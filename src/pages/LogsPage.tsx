import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Bug, Download, RefreshCw, RotateCcw, ShieldAlert, TerminalSquare } from "lucide-react";
import { PageHeader } from "../components/board/PageHeader";
import { clearFrontendLogs, logFrontend, useFrontendLogs } from "../lib/logging";
import type { BackendLogPayload, FrontendLogEntry, LogLevel } from "../types/logs";

type ViewMode = "frontend" | "backend" | "all";

const LEVELS: Array<{ value: LogLevel | ""; label: string }> = [
  { value: "", label: "All levels" },
  { value: "debug", label: "Debug" },
  { value: "info", label: "Info" },
  { value: "warn", label: "Warn" },
  { value: "error", label: "Error" },
];

function matchesLevel(entry: FrontendLogEntry, level: LogLevel | "") {
  return !level || entry.level === level;
}

function entrySearchText(entry: FrontendLogEntry) {
  return [entry.scope, entry.message, entry.details ?? ""].join(" ").toLowerCase();
}

function levelIcon(level: LogLevel) {
  if (level === "error") return <ShieldAlert size={14} />;
  if (level === "warn") return <AlertTriangle size={14} />;
  if (level === "debug") return <Bug size={14} />;
  return <TerminalSquare size={14} />;
}

export default function LogsPage() {
  const frontendLogs = useFrontendLogs();
  const [backendLogs, setBackendLogs] = useState<BackendLogPayload | null>(null);
  const [backendLoading, setBackendLoading] = useState(false);
  const [backendError, setBackendError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [level, setLevel] = useState<LogLevel | "">("");
  const [mode, setMode] = useState<ViewMode>("all");

  const refreshBackendLogs = async () => {
    setBackendLoading(true);
    setBackendError(null);
    try {
      const payload = await invoke<BackendLogPayload>("get_app_logs");
      setBackendLogs(payload);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setBackendError(message);
      logFrontend("error", "logs-page", "Failed to load backend logs", message);
    } finally {
      setBackendLoading(false);
    }
  };

  useEffect(() => {
    void refreshBackendLogs();
  }, []);

  const normalizedQuery = query.trim().toLowerCase();
  const filteredFrontendLogs = useMemo(() => frontendLogs.filter((entry) => {
    if (!matchesLevel(entry, level)) return false;
    if (!normalizedQuery) return true;
    return entrySearchText(entry).includes(normalizedQuery);
  }), [frontendLogs, level, normalizedQuery]);

  const backendLines = useMemo(() => {
    const lines = (backendLogs?.content ?? "")
      .split(/\r?\n/)
      .filter((line) => line.trim().length > 0);
    if (!normalizedQuery) return lines;
    return lines.filter((line) => line.toLowerCase().includes(normalizedQuery));
  }, [backendLogs?.content, normalizedQuery]);

  const clearBackendLogs = async () => {
    try {
      await invoke("clear_app_logs");
      setBackendLogs((prev) => prev ? { ...prev, content: "" } : prev);
      logFrontend("info", "logs-page", "Cleared backend logs");
    } catch (err) {
      logFrontend("error", "logs-page", "Failed to clear backend logs", err);
    }
  };

  const exportFrontendLogs = () => {
    const blob = new Blob([JSON.stringify(frontendLogs, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `rustyagent-frontend-logs-${Date.now()}.json`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="logs-page">
      <PageHeader
        title="Logs"
        sticky
        cta={(
          <div className="logs-page__actions">
            <button className="btn btn--ghost btn--sm" onClick={() => void refreshBackendLogs()} disabled={backendLoading}>
              <RefreshCw size={14} />
              {backendLoading ? "Refreshing…" : "Refresh backend"}
            </button>
            <button className="btn btn--ghost btn--sm" onClick={exportFrontendLogs}>
              <Download size={14} />
              Export frontend
            </button>
          </div>
        )}
      >
        <div className="logs-toolbar">
          <input
            type="search"
            className="logs-toolbar__search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search logs"
            aria-label="Search logs"
          />
          <select className="logs-toolbar__select" value={level} onChange={(event) => setLevel(event.target.value as LogLevel | "")}> 
            {LEVELS.map((option) => (
              <option key={option.label} value={option.value}>{option.label}</option>
            ))}
          </select>
          <div className="logs-toolbar__segmented" role="tablist" aria-label="Log sources">
            {(["all", "frontend", "backend"] as ViewMode[]).map((value) => (
              <button
                key={value}
                type="button"
                className={`logs-toolbar__segment${mode === value ? " is-active" : ""}`}
                onClick={() => setMode(value)}
                aria-pressed={mode === value}
              >
                {value[0].toUpperCase() + value.slice(1)}
              </button>
            ))}
          </div>
        </div>
      </PageHeader>

      <div className="logs-page__content">
        {(mode === "all" || mode === "frontend") && (
          <section className="logs-panel">
            <div className="logs-panel__header">
              <div>
                <h2 className="logs-panel__title">Frontend Logs</h2>
                <p className="logs-panel__meta">Captured from console output, render errors, and runtime failures.</p>
              </div>
              <button className="btn btn--ghost btn--sm" onClick={() => { clearFrontendLogs(); logFrontend("info", "logs-page", "Cleared frontend logs"); }}>
                <RotateCcw size={14} />
                Clear frontend
              </button>
            </div>

            {filteredFrontendLogs.length === 0 ? (
              <div className="logs-panel__empty">No frontend logs match the current filter.</div>
            ) : (
              <div className="logs-entries">
                {[...filteredFrontendLogs].reverse().map((entry) => (
                  <article key={entry.id} className={`logs-entry logs-entry--${entry.level}`}>
                    <div className="logs-entry__header">
                      <span className="logs-entry__level">{levelIcon(entry.level)} {entry.level.toUpperCase()}</span>
                      <span className="logs-entry__scope">{entry.scope}</span>
                      <time className="logs-entry__time">{new Date(entry.timestamp).toLocaleString()}</time>
                    </div>
                    <p className="logs-entry__message">{entry.message}</p>
                    {entry.details && <pre className="logs-entry__details">{entry.details}</pre>}
                  </article>
                ))}
              </div>
            )}
          </section>
        )}

        {(mode === "all" || mode === "backend") && (
          <section className="logs-panel">
            <div className="logs-panel__header">
              <div>
                <h2 className="logs-panel__title">Backend Logs</h2>
                <p className="logs-panel__meta">Tracing output persisted by the Tauri runtime.</p>
              </div>
              <button className="btn btn--ghost btn--sm" onClick={() => void clearBackendLogs()}>
                <RotateCcw size={14} />
                Clear backend
              </button>
            </div>

            {backendLogs?.path && <div className="logs-panel__path">{backendLogs.path}</div>}
            {backendError && <div className="logs-panel__error">Failed to load backend logs: {backendError}</div>}
            {!backendError && backendLines.length === 0 && <div className="logs-panel__empty">No backend logs match the current filter.</div>}
            {!backendError && backendLines.length > 0 && (
              <pre className="logs-backend">{backendLines.join("\n")}</pre>
            )}
          </section>
        )}
      </div>
    </div>
  );
}