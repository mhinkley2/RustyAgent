import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Check, ChevronLeft, ChevronRight, Pencil, RefreshCw, Search, Send, Square, Trash2, X } from "lucide-react";

type RunEventPayload =
  | { type: "token"; run_id: string; content: string }
  | { type: "tool_call"; run_id: string; tool_name: string; input: unknown }
  | { type: "tool_result"; run_id: string; tool_name: string; output: string; is_error: boolean }
  | { type: "complete"; run_id: string; stop_reason: string }
  | { type: "cancelled"; run_id: string }
  | { type: "failed"; run_id: string; message: string };

interface LogEntry {
  id: number;
  kind: "token" | "tool_call" | "tool_result" | "complete" | "failed" | "cancelled";
  content?: string;
  toolName?: string;
  toolInput?: unknown;
  toolOutput?: string;
  isError?: boolean;
  stopReason?: string;
  message?: string;
}

type VariantStatus = "running" | "complete" | "failed" | "cancelled";

interface AssistantVariant {
  entries: LogEntry[];
  status: VariantStatus;
  runId?: string;
}

interface ToolSummary {
  totalActions: number;
  errors: number;
  readFiles: number;
  writeFiles: number;
  shellCommands: number;
  searchActions: number;
}

type ChatFilterMode = "answers" | "answers_tools" | "tools_only" | "errors";

interface ChatSessionSummary {
  id: string;
  title: string;
  agent_profile_id: string | null;
  agent_name: string | null;
  last_message_preview: string | null;
  last_updated_at: string;
}

interface ChatSessionMessage {
  id: string;
  session_id: string;
  role: "user" | "assistant";
  content: string;
  agent_profile_id: string | null;
  created_at: string;
}

interface UserTurn {
  role: "user";
  content: string;
}

interface AssistantTurn {
  role: "assistant";
  variants: AssistantVariant[];
  activeIndex: number;
}

type Turn = UserTurn | AssistantTurn;

const ACTIVE_CHAT_SESSION_KEY = "rustyagent.chat.activeSessionId";

// Module-level event buffer to preserve events when component unmounts during streaming.
// Maps run_id → accumulated events. This survives component mount/unmount.
const runEventBuffer = new Map<string, RunEventPayload[]>();

interface TrackedRunState {
  sessionId: string;
  agentProfileId: string;
  assistantText: string;
  persisted: boolean;
}

const trackedRuns = new Map<string, TrackedRunState>();

function isTerminalEvent(payload: RunEventPayload): payload is Extract<RunEventPayload, { type: "complete" | "failed" | "cancelled" }> {
  return payload.type === "complete" || payload.type === "failed" || payload.type === "cancelled";
}

async function persistTrackedAssistant(runId: string, fallbackText = ""): Promise<void> {
  const run = trackedRuns.get(runId);
  if (!run || run.persisted) return;

  const text = (run.assistantText.trim() || fallbackText.trim()).trim();
  run.persisted = true;

  if (!text) {
    trackedRuns.delete(runId);
    return;
  }

  try {
    await invoke("append_chat_session_message", {
      sessionId: run.sessionId,
      role: "assistant",
      content: text,
      agentProfileId: run.agentProfileId,
    });
  } catch (err) {
    run.persisted = false;
    console.error("Failed to persist assistant message for run", err);
    return;
  }

  trackedRuns.delete(runId);
}

// Global event listener that persists across component instances.
// Set up once per module load; never cleaned up by individual component unmounts.
let globalEventListenerReady = false;

function ensureGlobalEventListener() {
  if (globalEventListenerReady) return;
  globalEventListenerReady = true;

  listen<RunEventPayload>("run-event", ({ payload }) => {
    // Buffer all events by run_id, regardless of component state
    if (!runEventBuffer.has(payload.run_id)) {
      runEventBuffer.set(payload.run_id, []);
    }
    runEventBuffer.get(payload.run_id)!.push(payload);

    const tracked = trackedRuns.get(payload.run_id);
    if (tracked && payload.type === "token") {
      tracked.assistantText += payload.content;
    }

    if (isTerminalEvent(payload)) {
      const fallback = payload.type === "failed" ? payload.message : "";
      void persistTrackedAssistant(payload.run_id, fallback);
      runEventBuffer.delete(payload.run_id);
    }
  }).catch(console.error);
}

interface Profile {
  id: string;
  name: string;
}

function isUntitledChatSession(title?: string | null): boolean {
  const normalized = (title ?? "").trim().toLowerCase();
  return !normalized || normalized === "new chat" || normalized === "chat session";
}

function extractText(variant: AssistantVariant): string {
  return variant.entries
    .filter(e => e.kind === "token")
    .map(e => e.content ?? "")
    .join("");
}

function buildMessages(turns: Turn[]): Array<{ role: string; content: string }> {
  const out: Array<{ role: string; content: string }> = [];
  for (const turn of turns) {
    if (turn.role === "user") {
      out.push({ role: "user", content: turn.content });
    } else {
      const variant = turn.variants[turn.activeIndex];
      const text = extractText(variant);
      if (text) out.push({ role: "assistant", content: text });
    }
  }
  return out;
}

function formatRelativeTime(iso: string): string {
  const ts = new Date(iso).getTime();
  if (Number.isNaN(ts)) return "";
  const delta = Date.now() - ts;
  const mins = Math.floor(delta / 60000);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

function readStoredActiveSessionId(): string | null {
  try {
    return localStorage.getItem(ACTIVE_CHAT_SESSION_KEY);
  } catch {
    return null;
  }
}

function writeStoredActiveSessionId(sessionId: string | null) {
  try {
    if (sessionId) {
      localStorage.setItem(ACTIVE_CHAT_SESSION_KEY, sessionId);
    } else {
      localStorage.removeItem(ACTIVE_CHAT_SESSION_KEY);
    }
  } catch {
    // ignore storage failures
  }
}

function classifyToolAction(toolName?: string): "read" | "write" | "shell" | "search" | "other" {
  const n = (toolName ?? "").toLowerCase();
  if (n.includes("read") || n.includes("list_directory") || n.includes("file_search")) return "read";
  if (n.includes("write") || n.includes("edit") || n.includes("rename") || n.includes("delete") || n.includes("create")) return "write";
  if (n.includes("terminal") || n.includes("shell") || n.includes("run_in_terminal") || n.includes("command")) return "shell";
  if (n.includes("search") || n.includes("grep") || n.includes("semantic")) return "search";
  return "other";
}

function buildToolSummary(entries: LogEntry[]): ToolSummary {
  const summary: ToolSummary = {
    totalActions: 0,
    errors: 0,
    readFiles: 0,
    writeFiles: 0,
    shellCommands: 0,
    searchActions: 0,
  };

  for (const e of entries) {
    if (e.kind === "tool_call") {
      summary.totalActions += 1;
      const kind = classifyToolAction(e.toolName);
      if (kind === "read") summary.readFiles += 1;
      if (kind === "write") summary.writeFiles += 1;
      if (kind === "shell") summary.shellCommands += 1;
      if (kind === "search") summary.searchActions += 1;
    }
    if ((e.kind === "tool_result" && e.isError) || e.kind === "failed") {
      summary.errors += 1;
    }
  }

  return summary;
}

function buildSummaryLabels(summary: ToolSummary): string[] {
  const labels: string[] = [];
  if (summary.totalActions > 0) {
    labels.push(`${summary.totalActions} tool action${summary.totalActions === 1 ? "" : "s"}`);
  }
  if (summary.readFiles > 0) labels.push(`${summary.readFiles} read file${summary.readFiles === 1 ? "" : "s"}`);
  if (summary.writeFiles > 0) labels.push(`${summary.writeFiles} file write${summary.writeFiles === 1 ? "" : "s"}`);
  if (summary.shellCommands > 0) labels.push(`${summary.shellCommands} shell command${summary.shellCommands === 1 ? "" : "s"}`);
  if (summary.searchActions > 0) labels.push(`${summary.searchActions} search action${summary.searchActions === 1 ? "" : "s"}`);
  if (summary.errors > 0) labels.push(`${summary.errors} error${summary.errors === 1 ? "" : "s"}`);
  return labels;
}

function ToolCallEntry({ entry }: { entry: LogEntry }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="chat-tool-call">
      <button className="chat-tool-call__toggle" onClick={() => setOpen(v => !v)}>
        <span className="chat-tool-call__icon">⚙</span>
        <span className="chat-tool-call__name">{entry.toolName}</span>
        <span className="chat-tool-call__chevron">{open ? "▴" : "▾"}</span>
      </button>
      {open && (
        <pre className="chat-tool-call__body">{JSON.stringify(entry.toolInput, null, 2)}</pre>
      )}
    </div>
  );
}

function ToolResultEntry({ entry }: { entry: LogEntry }) {
  const [open, setOpen] = useState(false);
  const text = entry.toolOutput ?? "";
  const isLong = text.length > 300;
  return (
    <div className={`chat-tool-result${entry.isError ? " chat-tool-result--error" : ""}`}>
      <div className="chat-tool-result__header">
        <span className="chat-tool-result__icon">{entry.isError ? "✖" : "←"}</span>
        <span className="chat-tool-result__name">{entry.toolName}</span>
        {isLong && (
          <button className="chat-tool-result__expand" onClick={() => setOpen(v => !v)}>
            {open ? "less ▴" : "more ▾"}
          </button>
        )}
      </div>
      <pre className="chat-tool-result__body">{!open && isLong ? `${text.slice(0, 300)}...` : text}</pre>
    </div>
  );
}

function StatusEntry({ entry }: { entry: LogEntry }) {
  if (entry.kind === "complete") return null;
  const isErr = entry.kind === "failed";
  return (
    <div className={`chat-status-entry chat-status-entry--${entry.kind}`}>
      {isErr ? `✖ Error: ${entry.message}` : "⏹ Cancelled"}
    </div>
  );
}

interface AssistantBubbleProps {
  turn: AssistantTurn;
  isLast: boolean;
  isRunning: boolean;
  filterMode: ChatFilterMode;
  onShowErrors: () => void;
  onRetry: () => void;
  onPrev: () => void;
  onNext: () => void;
}

function AssistantBubble({ turn, isLast, isRunning, filterMode, onShowErrors, onRetry, onPrev, onNext }: AssistantBubbleProps) {
  const [inspectOpen, setInspectOpen] = useState(false);
  const variant = turn.variants[turn.activeIndex];
  const numVar = turn.variants.length;
  const isActive = variant.status === "running";
  const toolSummary = buildToolSummary(variant.entries);
  const summaryLabels = buildSummaryLabels(toolSummary);
  const hasToolActivity = toolSummary.totalActions > 0 || toolSummary.errors > 0;

  const answerEntries = variant.entries.filter(entry =>
    entry.kind === "token" || entry.kind === "complete" || entry.kind === "failed" || entry.kind === "cancelled"
  );

  const toolEntries = variant.entries.filter(entry =>
    entry.kind === "tool_call" || entry.kind === "tool_result"
  );

  const errorEntries = variant.entries.filter(entry =>
    (entry.kind === "tool_result" && !!entry.isError) || entry.kind === "failed"
  );

  const visibleEntries =
    filterMode === "answers_tools"
      ? [...answerEntries, ...(inspectOpen ? toolEntries : [])]
      : filterMode === "answers"
        ? [...answerEntries, ...(inspectOpen ? toolEntries : [])]
        : filterMode === "tools_only"
          ? toolEntries
          : errorEntries;

  const hiddenToolErrorCount = variant.entries.filter(entry => entry.kind === "tool_result" && !!entry.isError).length;

  return (
    <div className="chat-assistant">
      <div className="chat-bubble chat-bubble--assistant">
        {(filterMode === "answers" || filterMode === "answers_tools") && hasToolActivity && (
          <div className="chat-tool-summary" role="status" aria-live="polite">
            <div className="chat-tool-summary__labels">
              {summaryLabels.map((label) => (
                <span key={label} className="chat-tool-summary__chip">{label}</span>
              ))}
            </div>
            <button
              className="chat-tool-summary__toggle"
              onClick={() => setInspectOpen(v => !v)}
            >
              {inspectOpen ? "Hide details" : "Inspect tools"}
            </button>
          </div>
        )}

        {visibleEntries.map((entry, i) => {
          if (entry.kind === "token") {
            return (
              <span key={entry.id}>
                {entry.content}
                {i === visibleEntries.length - 1 && isActive && <span className="chat-cursor" aria-hidden />}
              </span>
            );
          }
          if (entry.kind === "tool_call") return <ToolCallEntry key={entry.id} entry={entry} />;
          if (entry.kind === "tool_result") return <ToolResultEntry key={entry.id} entry={entry} />;
          return <StatusEntry key={entry.id} entry={entry} />;
        })}
        {visibleEntries.length === 0 && isActive && <span className="chat-cursor" aria-hidden />}
        {visibleEntries.length === 0 && !isActive && (
          <div className="chat-assistant__filtered-empty">No entries match this filter for this response.</div>
        )}
        {filterMode === "answers" && hiddenToolErrorCount > 0 && (
          <button className="chat-assistant__error-hint" onClick={onShowErrors}>
            {hiddenToolErrorCount} tool error{hiddenToolErrorCount === 1 ? "" : "s"} hidden. Show errors.
          </button>
        )}
      </div>

      <div className="chat-assistant__meta">
        {numVar > 1 && (
          <div className="chat-variant-nav">
            <button className="chat-variant-nav__btn" onClick={onPrev} disabled={turn.activeIndex === 0} aria-label="Previous response">
              <ChevronLeft size={12} />
            </button>
            <span className="chat-variant-nav__label">{turn.activeIndex + 1} / {numVar}</span>
            <button className="chat-variant-nav__btn" onClick={onNext} disabled={turn.activeIndex === numVar - 1} aria-label="Next response">
              <ChevronRight size={12} />
            </button>
          </div>
        )}

        {isLast && !isRunning && (
          <button className="chat-retry-btn" onClick={onRetry} title="Generate another response">
            <RefreshCw size={11} />
            <span>Retry</span>
          </button>
        )}
      </div>
    </div>
  );
}

export default function ChatPage() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [profileId, setProfileId] = useState<string>("");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [sessions, setSessions] = useState<ChatSessionSummary[]>([]);
  const [sessionsLoaded, setSessionsLoaded] = useState(false);
  const [loadingSessionId, setLoadingSessionId] = useState<string | null>(null);
  const [sessionSearch, setSessionSearch] = useState<string>("");
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renamingTitle, setRenamingTitle] = useState<string>("");
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState<string>("");
  const [isRunning, setIsRunning] = useState<boolean>(false);
  const [filterMode, setFilterMode] = useState<ChatFilterMode>("answers");

  const entryIdRef = useRef(0);
    const [editingMessageIdx, setEditingMessageIdx] = useState<number | null>(null);
    const [editingText, setEditingText] = useState<string>("");
  const scrollRef = useRef<HTMLDivElement>(null);
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const runIdRef = useRef<string | null>(null);
  const initializedDraftSessionRef = useRef(false);

  useEffect(() => {
    sessionIdRef.current = sessionId;
    if (sessionId) {
      writeStoredActiveSessionId(sessionId);
      return;
    }

    if (sessionsLoaded && initializedDraftSessionRef.current) {
      writeStoredActiveSessionId(null);
    }
  }, [sessionId, sessionsLoaded]);

  const nextEntryId = useCallback(() => {
    entryIdRef.current += 1;
    return entryIdRef.current;
  }, []);

  const loadSessions = useCallback(async () => {
    try {
      const data = await invoke<ChatSessionSummary[]>("list_chat_sessions", { limit: 100 });
      setSessions(data);
    } catch (err) {
      console.error("Failed to load chat sessions", err);
    } finally {
      setSessionsLoaded(true);
    }
  }, []);

  const createEmptySession = useCallback(async (title?: string) => {
    const session = await invoke<ChatSessionSummary>("create_chat_session", {
      title: title ?? null,
    });

    setSessions((prev) => [session, ...prev]);
    setTurns([]);
    setSessionId(session.id);
    sessionIdRef.current = session.id;
    runIdRef.current = null;
    setIsRunning(false);
    setDraft("");

    return session;
  }, []);

  const loadProfiles = useCallback(async () => {
    try {
      const nextProfiles = await invoke<Profile[]>("get_profiles");
      setProfiles(nextProfiles);
      setProfileId((current) => {
        if (current && nextProfiles.some((profile) => profile.id === current)) {
          return current;
        }
        return nextProfiles[0]?.id ?? "";
      });
    } catch (err) {
      console.error("Failed to load profiles", err);
    }
  }, []);

  useEffect(() => {
    void loadProfiles();
    void loadSessions();
  }, [loadProfiles, loadSessions]);

  useEffect(() => {
    const refreshChatState = () => {
      setSessionsLoaded(false);
      void loadProfiles();
      void loadSessions();
    };

    const handleVisibilityChange = () => {
      if (!document.hidden) {
        refreshChatState();
      }
    };

    window.addEventListener("focus", refreshChatState);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    const unlistenWorkspace = listen("workspace-changed", () => {
      initializedDraftSessionRef.current = false;
      setSessionId(null);
      sessionIdRef.current = null;
      setTurns([]);
      setDraft("");
      setLoadingSessionId(null);
      runIdRef.current = null;
      setIsRunning(false);
      refreshChatState();
    });

    return () => {
      window.removeEventListener("focus", refreshChatState);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      void unlistenWorkspace.then((dispose) => dispose());
    };
  }, [loadProfiles, loadSessions]);

  const visibleSessions = sessions.filter((s) => {
    const q = sessionSearch.trim().toLowerCase();
    if (!q) return true;
    return (
      (s.title ?? "").toLowerCase().includes(q) ||
      (s.last_message_preview ?? "").toLowerCase().includes(q)
    );
  });

  const beginRename = useCallback((target: ChatSessionSummary) => {
    setRenamingSessionId(target.id);
    setRenamingTitle(target.title || "");
  }, []);

  const cancelRename = useCallback(() => {
    setRenamingSessionId(null);
    setRenamingTitle("");
  }, []);

  const commitRename = useCallback(async () => {
    const id = renamingSessionId;
    const nextTitle = renamingTitle.trim();
    if (!id) return;
    if (!nextTitle) {
      cancelRename();
      return;
    }
    try {
      await invoke("update_story", { id, input: { title: nextTitle } });
      setSessions((prev) => prev.map((s) => (s.id === id ? { ...s, title: nextTitle } : s)));
    } catch (err) {
      console.error("Failed to rename chat session", err);
    } finally {
      cancelRename();
      loadSessions();
    }
  }, [cancelRename, loadSessions, renamingSessionId, renamingTitle]);

  const deleteSession = useCallback(async (target: ChatSessionSummary) => {
    if (!window.confirm(`Delete session "${target.title || "Chat Session"}"? This cannot be undone.`)) {
      return;
    }
    try {
      await invoke("delete_story", { id: target.id });
      setSessions((prev) => prev.filter((s) => s.id !== target.id));
      if (sessionIdRef.current === target.id) {
        unlistenRef.current?.();
        setTurns([]);
        setSessionId(null);
        sessionIdRef.current = null;
        runIdRef.current = null;
        setIsRunning(false);
        setDraft("");
      }
    } catch (err) {
      console.error("Failed to delete chat session", err);
    }
  }, []);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [turns]);

  useEffect(() => {
    // Don't clean up the event listener on unmount.
    // The listener persists across tab switches so we continue receiving
    // streaming events even when the component unmounts.
    // handleTerminalEvent will clean it up when the run completes.
    return () => {
      // Intentionally empty — listener cleanup is handled by handleTerminalEvent
    };
  }, []);

  const hydrateTurns = useCallback((messages: ChatSessionMessage[]) => {
    const hydrated: Turn[] = [];
    for (const msg of messages) {
      if (msg.role === "user") {
        hydrated.push({ role: "user", content: msg.content });
      } else {
        hydrated.push({
          role: "assistant",
          activeIndex: 0,
          variants: [
            {
              status: "complete",
              entries: msg.content
                ? [
                    { id: nextEntryId(), kind: "token", content: msg.content },
                    { id: nextEntryId(), kind: "complete" },
                  ]
                : [{ id: nextEntryId(), kind: "complete" }],
            },
          ],
        });
      }
    }
    return hydrated;
  }, [nextEntryId]);

  const streamInto = useCallback(async (
    messages: Array<{ role: string; content: string }>,
    targetTurnIdx: number,
    targetVarIdx: number,
    sessionTitle: string,
    persistUserText?: string,
  ) => {
    // Ensure global listener is set up
    ensureGlobalEventListener();

    unlistenRef.current?.();

    const mkEntry = (partial: Omit<LogEntry, "id">): LogEntry => ({
      id: ++entryIdRef.current,
      ...partial,
    });

    let activeRunId: string | null = null;
    let assistantText = "";
    let lastProcessedEventIndex = 0;

    const handleTerminalEvent = (payload: Extract<RunEventPayload, { type: "complete" | "failed" | "cancelled" }>) => {
      if (activeRunId) {
        const tracked = trackedRuns.get(activeRunId);
        if (tracked) {
          tracked.assistantText = assistantText;
        }
        const fallback = payload.type === "failed" ? payload.message : "";
        void persistTrackedAssistant(activeRunId, fallback).then(() => loadSessions()).catch(console.error);
      }

      setIsRunning(false);
      runIdRef.current = null;
      unlistenRef.current?.();
      unlistenRef.current = null;

      // Clear in-memory stream buffer when the run is terminal.
      if (activeRunId) {
        runEventBuffer.delete(activeRunId);
      }
    };

    const processEvent = (payload: RunEventPayload) => {
      if (!activeRunId || payload.run_id !== activeRunId) return false;
      if (payload.type === "token") assistantText += payload.content;

      setTurns(prev => {
        const next = [...prev];
        const turn = next[targetTurnIdx];
        if (!turn || turn.role !== "assistant") return prev;

        const variants = [...turn.variants];
        const variant = { ...variants[targetVarIdx], entries: [...variants[targetVarIdx].entries] };

        switch (payload.type) {
          case "token": {
            const last = variant.entries[variant.entries.length - 1];
            if (last?.kind === "token") {
              variant.entries = [
                ...variant.entries.slice(0, -1),
                { ...last, content: (last.content ?? "") + payload.content },
              ];
            } else {
              variant.entries = [...variant.entries, mkEntry({ kind: "token", content: payload.content })];
            }
            break;
          }
          case "tool_call":
            variant.entries = [...variant.entries, mkEntry({ kind: "tool_call", toolName: payload.tool_name, toolInput: payload.input })];
            break;
          case "tool_result":
            variant.entries = [...variant.entries, mkEntry({
              kind: "tool_result",
              toolName: payload.tool_name,
              toolOutput: payload.output,
              isError: payload.is_error,
            })];
            break;
          case "complete":
            variant.entries = [...variant.entries, mkEntry({ kind: "complete", stopReason: payload.stop_reason })];
            variant.status = "complete";
            break;
          case "failed":
            variant.entries = [...variant.entries, mkEntry({ kind: "failed", message: payload.message })];
            variant.status = "failed";
            break;
          case "cancelled":
            variant.entries = [...variant.entries, mkEntry({ kind: "cancelled" })];
            variant.status = "cancelled";
            break;
        }

        variants[targetVarIdx] = variant;
        next[targetTurnIdx] = { ...turn, variants } as AssistantTurn;
        return next;
      });

      if (payload.type === "complete" || payload.type === "failed" || payload.type === "cancelled") {
        handleTerminalEvent(payload);
      }
      return true;
    };

    // Set up listener that processes both real-time and buffered events
    try {
      const unlisten = await listen<RunEventPayload>("run-event", ({ payload }) => {
        processEvent(payload);
      });

      unlistenRef.current = unlisten;

      const resp = await invoke<{ run_id: string; session_id: string }>("start_chat_run", {
        profileId,
        messages,
        sessionId: sessionIdRef.current,
        sessionTitle,
      });

      if (persistUserText && persistUserText.trim()) {
        void invoke("append_chat_session_message", {
          sessionId: resp.session_id,
          role: "user",
          content: persistUserText,
          agentProfileId: profileId,
        }).then(loadSessions).catch(console.error);
      }

      activeRunId = resp.run_id;
      runIdRef.current = resp.run_id;
      if (!sessionIdRef.current) {
        sessionIdRef.current = resp.session_id;
        setSessionId(resp.session_id);
      }

      const seedText = (runEventBuffer.get(activeRunId) ?? [])
        .filter((event): event is Extract<RunEventPayload, { type: "token" }> => event.type === "token")
        .map((event) => event.content)
        .join("");

      trackedRuns.set(activeRunId, {
        sessionId: resp.session_id,
        agentProfileId: profileId,
        assistantText: seedText,
        persisted: false,
      });

      // Process any buffered events that came in while we were setting up
      const buffered = runEventBuffer.get(activeRunId) || [];
      for (let i = lastProcessedEventIndex; i < buffered.length; i++) {
        processEvent(buffered[i]);
        lastProcessedEventIndex = i + 1;
      }
    } catch (err) {
      unlistenRef.current?.();
      unlistenRef.current = null;
      const msg = err instanceof Error ? err.message : String(err);
      setTurns(prev => {
        const next = [...prev];
        const turn = next[targetTurnIdx];
        if (!turn || turn.role !== "assistant") return prev;
        const variants = [...turn.variants];
        variants[targetVarIdx] = {
          ...variants[targetVarIdx],
          entries: [...variants[targetVarIdx].entries, mkEntry({ kind: "failed", message: msg })],
          status: "failed",
        };
        next[targetTurnIdx] = { ...turn, variants } as AssistantTurn;
        return next;
      });
      setIsRunning(false);
    }
  }, [profileId, loadSessions]);

  const sendMessage = useCallback(async () => {
    const text = draft.trim();
    if (!text || isRunning || !profileId) return;

    const nextTitle = text.slice(0, 60);
    const selectedProfile = profiles.find((profile) => profile.id === profileId) ?? null;
    const activeSessionId = sessionIdRef.current;

    if (activeSessionId) {
      setSessions((prev) => prev.map((session) => {
        if (session.id !== activeSessionId) return session;
        return {
          ...session,
          title: isUntitledChatSession(session.title) ? nextTitle : session.title,
          last_message_preview: text,
          last_updated_at: new Date().toISOString(),
          agent_profile_id: profileId,
          agent_name: selectedProfile?.name ?? session.agent_name,
        };
      }));
    }

    setDraft("");
    setIsRunning(true);

    const userTurn: UserTurn = { role: "user", content: text };
    const assistantTurn: AssistantTurn = {
      role: "assistant",
      variants: [{ entries: [], status: "running" }],
      activeIndex: 0,
    };

    const prevTurns = turns;
    const targetTurnIdx = prevTurns.length + 1;

    setTurns([...prevTurns, userTurn, assistantTurn]);

    const messages = buildMessages([...prevTurns, userTurn]);
    await streamInto(messages, targetTurnIdx, 0, nextTitle, text);
  }, [draft, isRunning, profileId, profiles, turns, streamInto]);

  const retry = useCallback(async (turnIdx: number) => {
    if (isRunning) return;

    const turn = turns[turnIdx];
    if (!turn || turn.role !== "assistant") return;

    setIsRunning(true);

    const newVariantIdx = turn.variants.length;
    const newVariant: AssistantVariant = { entries: [], status: "running" };

    setTurns(prev => {
      const next = [...prev];
      const t = next[turnIdx] as AssistantTurn;
      next[turnIdx] = {
        ...t,
        variants: [...t.variants, newVariant],
        activeIndex: newVariantIdx,
      };
      return next;
    });

    const messages = buildMessages(turns.slice(0, turnIdx));
    await streamInto(messages, turnIdx, newVariantIdx, "");
  }, [isRunning, turns, streamInto]);

  const switchVariant = useCallback((turnIdx: number, delta: number) => {
    setTurns(prev => {
      const next = [...prev];
      const turn = next[turnIdx];
      if (!turn || turn.role !== "assistant") return prev;
      const newIdx = Math.min(Math.max(turn.activeIndex + delta, 0), turn.variants.length - 1);
      next[turnIdx] = { ...turn, activeIndex: newIdx };
      return next;
    });
  }, []);

    const editMessage = useCallback(async (turnIdx: number, newText: string) => {
      if (isRunning || !newText.trim()) return;

      const userTurn = turns[turnIdx];
      if (!userTurn || userTurn.role !== "user") return;

      // Update the user message
      setTurns(prev => {
        const next = [...prev];
        (next[turnIdx] as UserTurn).content = newText;
      
        // Remove the assistant response that follows (if it exists)
        if (next[turnIdx + 1]?.role === "assistant") {
          next.splice(turnIdx + 1, 1);
        }
      
        return next;
      });

      setEditingMessageIdx(null);
      setEditingText("");
      setIsRunning(true);

      // Regenerate response with updated message
      const messagesBeforeEdit = buildMessages(turns.slice(0, turnIdx));
      const newMessages = [...messagesBeforeEdit, { role: "user", content: newText }];
      const newTitle = newText.slice(0, 60);

      // Add a new assistant turn for the regenerated response
      setTurns(prev => {
        const next = [...prev];
        next.push({
          role: "assistant",
          variants: [{ entries: [], status: "running" }],
          activeIndex: 0,
        });
        return next;
      });

      await streamInto(newMessages, turnIdx + 1, 0, newTitle, undefined);
    }, [isRunning, turns, streamInto, buildMessages]);

  const stopRun = useCallback(async () => {
    const rid = runIdRef.current;
    if (!rid) return;
    try {
      await invoke("stop_run", { runId: rid });
    } catch {
      // ignore
    }
  }, []);

  const newChat = useCallback(async () => {
    unlistenRef.current?.();
    await createEmptySession();
  }, [createEmptySession]);

  // Reattach the component-level streaming listener to an already-running run (e.g. after
  // navigating away and back while a response was still streaming or pending).
  const reattachToRun = useCallback((runId: string, targetTurnIdx: number, targetVarIdx: number) => {
    ensureGlobalEventListener();
    runIdRef.current = runId;

    const mkEntry = (partial: Omit<LogEntry, "id">): LogEntry => ({
      id: ++entryIdRef.current,
      ...partial,
    });

    let assistantText = trackedRuns.get(runId)?.assistantText ?? "";

    const applyEvent = (payload: RunEventPayload) => {
      if (payload.run_id !== runId) return;
      if (payload.type === "token") assistantText += payload.content;

      setTurns(prev => {
        const next = [...prev];
        const turn = next[targetTurnIdx];
        if (!turn || turn.role !== "assistant") return prev;
        const variants = [...turn.variants];
        const variant = { ...variants[targetVarIdx], entries: [...variants[targetVarIdx].entries] };

        switch (payload.type) {
          case "token": {
            const last = variant.entries[variant.entries.length - 1];
            if (last?.kind === "token") {
              variant.entries = [...variant.entries.slice(0, -1), { ...last, content: (last.content ?? "") + payload.content }];
            } else {
              variant.entries = [...variant.entries, mkEntry({ kind: "token", content: payload.content })];
            }
            break;
          }
          case "tool_call":
            variant.entries = [...variant.entries, mkEntry({ kind: "tool_call", toolName: payload.tool_name, toolInput: payload.input })];
            break;
          case "tool_result":
            variant.entries = [...variant.entries, mkEntry({ kind: "tool_result", toolName: payload.tool_name, toolOutput: payload.output, isError: payload.is_error })];
            break;
          case "complete":
            variant.entries = [...variant.entries, mkEntry({ kind: "complete", stopReason: payload.stop_reason })];
            variant.status = "complete";
            break;
          case "failed":
            variant.entries = [...variant.entries, mkEntry({ kind: "failed", message: payload.message })];
            variant.status = "failed";
            break;
          case "cancelled":
            variant.entries = [...variant.entries, mkEntry({ kind: "cancelled" })];
            variant.status = "cancelled";
            break;
        }

        variants[targetVarIdx] = variant;
        next[targetTurnIdx] = { ...turn, variants } as AssistantTurn;
        return next;
      });

      if (payload.type === "complete" || payload.type === "failed" || payload.type === "cancelled") {
        const tracked = trackedRuns.get(runId);
        if (tracked) tracked.assistantText = assistantText;
        const fallback = payload.type === "failed" ? payload.message : "";
        void persistTrackedAssistant(runId, fallback).then(() => loadSessions()).catch(console.error);
        setIsRunning(false);
        runIdRef.current = null;
        unlistenRef.current?.();
        unlistenRef.current = null;
        runEventBuffer.delete(runId);
      }
    };

    // Seed with any already-buffered events
    const buffered = runEventBuffer.get(runId) ?? [];
    for (const ev of buffered) applyEvent(ev);

    // Set up a per-component listener for events that arrive after this point
    listen<RunEventPayload>("run-event", ({ payload }) => {
      applyEvent(payload);
    }).then(unlisten => {
      unlistenRef.current = unlisten;
    }).catch(console.error);
  }, [loadSessions]);

  const openSession = useCallback(async (id: string) => {
    if (isRunning) return;
    setLoadingSessionId(id);
    try {
      const messages = await invoke<ChatSessionMessage[]>("get_chat_session_messages", { sessionId: id });
      const selected = sessions.find(s => s.id === id) ?? null;
      const preferredProfile = messages.find(m => !!m.agent_profile_id)?.agent_profile_id ?? selected?.agent_profile_id ?? null;

      unlistenRef.current?.();
      runIdRef.current = null;
      setDraft("");
      setSessionId(id);
      sessionIdRef.current = id;

      // Check if there's an in-progress run for this session that we should reattach to
      const activeEntry = [...trackedRuns.entries()].find(([, s]) => s.sessionId === id && !s.persisted);
      if (activeEntry) {
        const [activeRunId] = activeEntry;
        const hydrated = hydrateTurns(messages);
        // Append a skeleton running assistant turn for the in-progress response
        const skeletonTurn: AssistantTurn = {
          role: "assistant",
          activeIndex: 0,
          variants: [{ entries: [], status: "running" }],
        };
        setTurns([...hydrated, skeletonTurn]);
        setIsRunning(true);
        reattachToRun(activeRunId, hydrated.length, 0);
      } else {
        setIsRunning(false);
        setTurns(hydrateTurns(messages));
      }

      if (preferredProfile && profiles.some(p => p.id === preferredProfile)) {
        setProfileId(preferredProfile);
      }
    } catch (err) {
      console.error("Failed to open chat session", err);
    } finally {
      setLoadingSessionId(null);
    }
  }, [isRunning, sessions, profiles, hydrateTurns, reattachToRun]);

  useEffect(() => {
    if (initializedDraftSessionRef.current) return;
    if (!sessionsLoaded) return;
    if (sessionIdRef.current) return;

    const storedSessionId = readStoredActiveSessionId();
    if (storedSessionId) {
      const storedSession = sessions.find((session) => session.id === storedSessionId);
      if (storedSession) {
        initializedDraftSessionRef.current = true;
        if (storedSession.last_message_preview) {
          void openSession(storedSession.id).catch((err) => {
            console.error("Failed to restore active chat session", err);
          });
        } else {
          setSessionId(storedSession.id);
          sessionIdRef.current = storedSession.id;
          setTurns([]);
          if (storedSession.agent_profile_id && profiles.some((profile) => profile.id === storedSession.agent_profile_id)) {
            setProfileId(storedSession.agent_profile_id);
          }
        }
        return;
      }

      writeStoredActiveSessionId(null);
    }

    const existingDraft = sessions.find((session) => !session.last_message_preview);
    if (existingDraft) {
      initializedDraftSessionRef.current = true;
      setSessionId(existingDraft.id);
      sessionIdRef.current = existingDraft.id;
      setTurns([]);
      if (existingDraft.agent_profile_id && profiles.some((profile) => profile.id === existingDraft.agent_profile_id)) {
        setProfileId(existingDraft.agent_profile_id);
      }
      return;
    }

    initializedDraftSessionRef.current = true;
    void createEmptySession().catch((err) => {
      initializedDraftSessionRef.current = false;
      console.error("Failed to create initial chat session", err);
    });
  }, [createEmptySession, openSession, profiles, sessions, sessionsLoaded]);

  const handleKey = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  }, [sendMessage]);

  const lastAssistantIdx = (() => {
    for (let i = turns.length - 1; i >= 0; i--) {
      if (turns[i].role === "assistant") return i;
    }
    return -1;
  })();

  const hiddenErrorCount = turns.reduce((count, turn) => {
    if (turn.role !== "assistant") return count;
    const variant = turn.variants[turn.activeIndex];
    return count + variant.entries.filter(entry => entry.kind === "tool_result" && !!entry.isError).length;
  }, 0);

  const filterOptions: Array<{ mode: ChatFilterMode; label: string }> = [
    { mode: "answers", label: "Answers" },
    { mode: "answers_tools", label: "Answers + Tools" },
    { mode: "tools_only", label: "Tools Only" },
    { mode: "errors", label: "Errors" },
  ];

  const activeSession = sessions.find((s) => s.id === sessionId) ?? null;

  return (
    <div className="chat-page">
      <div className="chat-page__content">
        <aside className="chat-page__sessions" aria-label="Chat session history">
          <div className="chat-page__sessions-head">
            <h2 className="chat-page__sessions-title">Sessions</h2>
            <button className="btn btn--ghost btn--sm" onClick={() => void newChat()} disabled={isRunning} title="Start a new chat session">
              + New
            </button>
          </div>

          <div className="chat-page__sessions-search">
            <Search size={13} className="chat-page__sessions-search-icon" />
            <input
              type="search"
              className="chat-page__sessions-search-input"
              value={sessionSearch}
              onChange={(e) => setSessionSearch(e.target.value)}
              placeholder="Search sessions"
              aria-label="Search chat sessions"
            />
          </div>

          {visibleSessions.length === 0 ? (
            <div className="chat-page__sessions-empty">
              {sessions.length === 0
                ? "No previous chats yet. Start a conversation and it will appear here."
                : "No sessions match your search."}
            </div>
          ) : (
            <div className="chat-page__session-list">
              {visibleSessions.map(session => {
                const isActive = session.id === sessionId;
                const loading = loadingSessionId === session.id;
                const isRenaming = renamingSessionId === session.id;
                const canOpen = !isRunning && !loading && !isRenaming;
                return (
                  <div
                    key={session.id}
                    className={`chat-page__session-item${isActive ? " is-active" : ""}${canOpen ? " is-clickable" : ""}`}
                    role="button"
                    tabIndex={canOpen ? 0 : -1}
                    aria-pressed={isActive}
                    onClick={() => {
                      if (canOpen) {
                        void openSession(session.id);
                      }
                    }}
                    onKeyDown={(event) => {
                      if (!canOpen) return;
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        void openSession(session.id);
                      }
                    }}
                  >
                    <div className="chat-page__session-top">
                      {isRenaming ? (
                        <input
                          type="text"
                          className="chat-page__session-rename-input"
                          value={renamingTitle}
                          onChange={(e) => setRenamingTitle(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              void commitRename();
                            }
                            if (e.key === "Escape") {
                              e.preventDefault();
                              cancelRename();
                            }
                          }}
                          onClick={(event) => event.stopPropagation()}
                          autoFocus
                          aria-label="Rename session"
                        />
                      ) : (
                        <button
                          type="button"
                          className="chat-page__session-open"
                          onClick={(event) => {
                            event.stopPropagation();
                            if (canOpen) {
                              void openSession(session.id);
                            }
                          }}
                          disabled={!canOpen}
                        >
                          <span className="chat-page__session-title">{session.title || "Chat Session"}</span>
                        </button>
                      )}

                      <span className="chat-page__session-time">{formatRelativeTime(session.last_updated_at)}</span>

                      <div className="chat-page__session-actions">
                        {isRenaming ? (
                          <>
                            <button
                              type="button"
                              className="chat-page__session-action"
                              onClick={(event) => {
                                event.stopPropagation();
                                void commitRename();
                              }}
                              aria-label="Save session name"
                            >
                              <Check size={12} />
                            </button>
                            <button
                              type="button"
                              className="chat-page__session-action"
                              onClick={(event) => {
                                event.stopPropagation();
                                cancelRename();
                              }}
                              aria-label="Cancel rename"
                            >
                              <X size={12} />
                            </button>
                          </>
                        ) : (
                          <>
                            <button
                              type="button"
                              className="chat-page__session-action"
                              onClick={(event) => {
                                event.stopPropagation();
                                beginRename(session);
                              }}
                              aria-label={`Rename ${session.title || "chat session"}`}
                              disabled={isRunning}
                            >
                              <Pencil size={12} />
                            </button>
                            <button
                              type="button"
                              className="chat-page__session-action chat-page__session-action--danger"
                              onClick={(event) => {
                                event.stopPropagation();
                                void deleteSession(session);
                              }}
                              aria-label={`Delete ${session.title || "chat session"}`}
                              disabled={isRunning}
                            >
                              <Trash2 size={12} />
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                    <div className="chat-page__session-meta">{session.agent_name ?? "Unknown agent"}</div>
                    <div className="chat-page__session-preview">{loading ? "Loading..." : (session.last_message_preview || "No messages yet")}</div>
                  </div>
                );
              })}
            </div>
          )}
        </aside>

        <div className="chat-page__main">
          <header className="chat-page__header">
            <div className="chat-page__header-main">
              <select
                className="chat-page__profile-select"
                value={profileId}
                onChange={e => setProfileId(e.target.value)}
                disabled={isRunning}
              >
                {profiles.length === 0 && <option value="">No agents — create one first</option>}
                {profiles.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
              </select>

              <button className="btn btn--ghost btn--sm" onClick={() => void newChat()} disabled={isRunning} title="Start a new chat session">
                + New chat
              </button>

              {activeSession && renamingSessionId !== activeSession.id && (
                <>
                  <button
                    className="btn btn--ghost btn--sm"
                    onClick={() => beginRename(activeSession)}
                    disabled={isRunning}
                    title="Rename active session"
                  >
                    Rename
                  </button>
                  <button
                    className="btn btn--ghost btn--sm"
                    onClick={() => void deleteSession(activeSession)}
                    disabled={isRunning}
                    title="Delete active session"
                  >
                    Delete
                  </button>
                </>
              )}
            </div>

            <div className="chat-page__filters" role="group" aria-label="Chat transcript filters">
              {filterOptions.map(option => (
                <button
                  key={option.mode}
                  type="button"
                  className={`chat-page__filter-btn${filterMode === option.mode ? " is-active" : ""}`}
                  onClick={() => setFilterMode(option.mode)}
                  aria-pressed={filterMode === option.mode}
                >
                  {option.label}
                </button>
              ))}
            </div>

            {filterMode === "answers" && hiddenErrorCount > 0 && (
              <div className="chat-page__error-pill" role="status" aria-live="polite">
                {hiddenErrorCount} hidden tool error{hiddenErrorCount === 1 ? "" : "s"}
              </div>
            )}
          </header>

          <div className="chat-page__thread" ref={scrollRef}>
            {turns.length === 0 && (
              <div className="chat-page__empty">
                {activeSession
                  ? "This session has no messages yet. Start chatting to begin the conversation."
                  : "Select an existing session or start chatting."}
              </div>
            )}

            {turns.map((turn, idx) => {
              if (turn.role === "user") {
                  const isLast = idx === turns.length - 1;
                  const isEditing = editingMessageIdx === idx;

                  if (isEditing) {
                    return (
                      <div key={idx} className="chat-user">
                        <div className="chat-bubble chat-bubble--user">
                          <textarea
                            autoFocus
                            value={editingText}
                            onChange={e => setEditingText(e.target.value)}
                            className="chat-edit-textarea"
                            rows={3}
                          />
                          <div className="chat-edit-actions">
                            <button
                              onClick={() => editMessage(idx, editingText)}
                              disabled={!editingText.trim()}
                              className="chat-edit-btn chat-edit-btn--confirm"
                            >
                              <Check size={14} /> Save
                            </button>
                            <button
                              onClick={() => {
                                setEditingMessageIdx(null);
                                setEditingText("");
                              }}
                              className="chat-edit-btn chat-edit-btn--cancel"
                            >
                              <X size={14} /> Cancel
                            </button>
                          </div>
                        </div>
                      </div>
                    );
                  }

                  return (
                    <div key={idx} className="chat-user">
                      <div className="chat-bubble chat-bubble--user">
                        {turn.content}
                        {isLast && !isRunning && (
                          <button
                            onClick={() => {
                              setEditingMessageIdx(idx);
                              setEditingText(turn.content);
                            }}
                            className="chat-edit-btn-small"
                            title="Edit message"
                          >
                            <Pencil size={14} />
                          </button>
                        )}
                      </div>
                    </div>
                  );
              }

              return (
                <AssistantBubble
                    key={idx}  
                  turn={turn}
                  isLast={idx === lastAssistantIdx}
                  isRunning={isRunning}
                  filterMode={filterMode}
                  onShowErrors={() => setFilterMode("errors")}
                  onRetry={() => retry(idx)}
                  onPrev={() => switchVariant(idx, -1)}
                  onNext={() => switchVariant(idx, +1)}
                />
              );
            })}
          </div>

          <div className="chat-page__input-wrap">
            <div className="chat-page__input-box">
              <textarea
                className="chat-page__textarea"
                value={draft}
                onChange={e => setDraft(e.target.value)}
                onKeyDown={handleKey}
                placeholder="Message the agent... (Enter to send, Shift+Enter for newline)"
                disabled={isRunning || !profileId}
                rows={1}
              />
              {isRunning ? (
                <button className="chat-page__send-btn chat-page__send-btn--stop" onClick={stopRun} title="Stop generation">
                  <Square size={16} />
                </button>
              ) : (
                <button
                  className="chat-page__send-btn"
                  onClick={sendMessage}
                  disabled={!draft.trim() || !profileId}
                  title="Send message"
                >
                  <Send size={16} />
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
