import { useSyncExternalStore } from "react";
import { notifyError } from "../components/ui/Toast";
import type { FrontendLogEntry, LogLevel } from "../types/logs";

const STORAGE_KEY = "rustyagent.frontendLogs";
const MAX_LOG_ENTRIES = 500;
const ERROR_TOAST_DEDUPE_WINDOW_MS = 8000;

type Listener = () => void;

const listeners = new Set<Listener>();
const originalConsole = {
  debug: console.debug.bind(console),
  info: console.info.bind(console),
  log: console.log.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
};

let installed = false;
let entries: FrontendLogEntry[] = loadEntries();
const recentErrorToasts = new Map<string, number>();

function loadEntries(): FrontendLogEntry[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isFrontendLogEntry);
  } catch {
    return [];
  }
}

function isFrontendLogEntry(value: unknown): value is FrontendLogEntry {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<FrontendLogEntry>;
  return typeof candidate.id === "string"
    && typeof candidate.timestamp === "string"
    && typeof candidate.level === "string"
    && typeof candidate.scope === "string"
    && typeof candidate.message === "string"
    && typeof candidate.origin === "string";
}

function saveEntries() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
  } catch {
    // ignore storage failures
  }
}

function notify() {
  saveEntries();
  listeners.forEach((listener) => listener());
}

function nextId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function normalizeValue(value: unknown): string {
  if (value instanceof Error) {
    return `${value.name}: ${value.message}${value.stack ? `\n${value.stack}` : ""}`;
  }
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean" || value == null) {
    return String(value);
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return Object.prototype.toString.call(value);
  }
}

function appendEntry(entry: FrontendLogEntry) {
  entries = [...entries.slice(-(MAX_LOG_ENTRIES - 1)), entry];
  maybeNotifyErrorToast(entry);
  notify();
}

function truncateForToast(value: string, maxLength = 200): string {
  const normalized = value.replace(/\s+/g, " ").trim();
  if (normalized.length <= maxLength) return normalized;
  return `${normalized.slice(0, maxLength - 1)}…`;
}

function toastSignature(entry: FrontendLogEntry): string {
  return [entry.origin, entry.scope, entry.message, entry.details ?? ""].join("|").slice(0, 400);
}

function buildToastMessage(entry: FrontendLogEntry): string {
  if (entry.message && !entry.message.startsWith("Unhandled promise rejection")) {
    return truncateForToast(entry.message);
  }
  if (entry.details) {
    return truncateForToast(entry.details);
  }
  return "Check the Logs page for details.";
}

function maybeNotifyErrorToast(entry: FrontendLogEntry) {
  if (entry.level !== "error") return;

  const now = Date.now();
  for (const [signature, timestamp] of recentErrorToasts) {
    if (now - timestamp > ERROR_TOAST_DEDUPE_WINDOW_MS) {
      recentErrorToasts.delete(signature);
    }
  }

  const signature = toastSignature(entry);
  if (recentErrorToasts.has(signature)) return;
  recentErrorToasts.set(signature, now);

  notifyError("Something failed", buildToastMessage(entry), {
    duration: 7000,
    action: {
      label: "View logs",
      onClick: () => {
        window.location.hash = "/logs";
      },
    },
  });
}

function recordConsole(level: LogLevel, args: unknown[]) {
  const [first, ...rest] = args;
  appendEntry({
    id: nextId(),
    timestamp: new Date().toISOString(),
    level,
    scope: "console",
    message: normalizeValue(first),
    details: rest.length > 0 ? rest.map(normalizeValue).join("\n\n") : undefined,
    origin: "console",
  });
}

export function logFrontend(level: LogLevel, scope: string, message: string, details?: unknown) {
  appendEntry({
    id: nextId(),
    timestamp: new Date().toISOString(),
    level,
    scope,
    message,
    details: details === undefined ? undefined : normalizeValue(details),
    origin: "manual",
  });
}

export function clearFrontendLogs() {
  entries = [];
  recentErrorToasts.clear();
  notify();
}

export function subscribeToFrontendLogs(listener: Listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getFrontendLogsSnapshot() {
  return entries;
}

export function useFrontendLogs() {
  return useSyncExternalStore(subscribeToFrontendLogs, getFrontendLogsSnapshot, getFrontendLogsSnapshot);
}

export function installFrontendLogging() {
  if (installed) return;
  installed = true;

  console.debug = (...args: unknown[]) => {
    recordConsole("debug", args);
    originalConsole.debug(...args);
  };
  console.info = (...args: unknown[]) => {
    recordConsole("info", args);
    originalConsole.info(...args);
  };
  console.log = (...args: unknown[]) => {
    recordConsole("info", args);
    originalConsole.log(...args);
  };
  console.warn = (...args: unknown[]) => {
    recordConsole("warn", args);
    originalConsole.warn(...args);
  };
  console.error = (...args: unknown[]) => {
    recordConsole("error", args);
    originalConsole.error(...args);
  };

  window.addEventListener("error", (event) => {
    appendEntry({
      id: nextId(),
      timestamp: new Date().toISOString(),
      level: "error",
      scope: "window",
      message: event.message || "Unhandled window error",
      details: event.error ? normalizeValue(event.error) : undefined,
      origin: "runtime",
    });
  });

  window.addEventListener("unhandledrejection", (event) => {
    appendEntry({
      id: nextId(),
      timestamp: new Date().toISOString(),
      level: "error",
      scope: "promise",
      message: "Unhandled promise rejection",
      details: normalizeValue(event.reason),
      origin: "runtime",
    });
  });
}