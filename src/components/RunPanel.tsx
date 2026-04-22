import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Types — must mirror the RunEvent enum in runtime/src/runtime.rs
// ---------------------------------------------------------------------------

type RunEventKind =
  | { type: "token"; run_id: string; content: string }
  | { type: "tool_call"; run_id: string; tool_name: string; input: unknown }
  | { type: "tool_result"; run_id: string; tool_name: string; output: string; is_error: boolean }
  | { type: "complete"; run_id: string; stop_reason: string }
  | { type: "cancelled"; run_id: string }
  | { type: "failed"; run_id: string; message: string }
  | { type: "approval_request"; run_id: string; request_id: string; tool_name: string; input: unknown }
  | { type: "human_input"; run_id: string; request_id: string; prompt: string };

type RunStatus = "idle" | "running" | "complete" | "cancelled" | "failed";

interface LogEntry {
  id: number;
  kind: RunEventKind["type"];
  // token
  content?: string;
  // tool_call
  toolName?: string;
  toolInput?: unknown;
  expanded?: boolean;
  // tool_result
  toolOutput?: string;
  isError?: boolean;
  // complete / failed
  stopReason?: string;
  message?: string;
  // approval_request
  requestId?: string;
  approvalToolName?: string;
  approvalInput?: unknown;
  approvalResponded?: boolean;
  approved?: boolean;
  // human_input
  humanPrompt?: string;
  humanResponded?: boolean;
  humanResponse?: string;
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface RunPanelProps {
  storyId: string;
  profileId: string;
  storyTitle?: string;
  /** If true, start the run immediately on mount. */
  autoStart?: boolean;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function statusLabel(status: RunStatus): string {
  switch (status) {
    case "idle":      return "Idle";
    case "running":   return "● Running";
    case "complete":  return "✔ Complete";
    case "cancelled": return "⏹ Cancelled";
    case "failed":    return "✖ Failed";
  }
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export function RunPanel({ storyId, profileId, storyTitle, autoStart }: RunPanelProps) {
  const [runId, setRunId] = useState<string | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [status, setStatus] = useState<RunStatus>("idle");
  const [autoScroll, setAutoScroll] = useState(true);
  const [hasNewContent, setHasNewContent] = useState(false);
  const [outChars, setOutChars] = useState(0);
  const [duration, setDuration] = useState<number | null>(null);
  const [rawEvents, setRawEvents] = useState<RunEventKind[]>([]);

  const entryIdRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const autoScrollRef = useRef(true);
  const startTimeRef = useRef<number | null>(null);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  // Keep ref in sync with state for use inside event callbacks.
  useEffect(() => { autoScrollRef.current = autoScroll; }, [autoScroll]);

  // Auto-scroll on new entries.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (autoScrollRef.current) {
      el.scrollTop = el.scrollHeight;
      setHasNewContent(false);
    } else {
      setHasNewContent(true);
    }
  }, [entries]);

  // Clean up the Tauri event listener on unmount.
  useEffect(() => () => { unlistenRef.current?.(); }, []);

  // Auto-start refs — populated after startRun is defined below.
  const autoStartedRef = useRef(false);
  const startRunRef = useRef<(() => void) | null>(null);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom) {
      setAutoScroll(true);
      setHasNewContent(false);
    } else {
      setAutoScroll(false);
    }
  }, []);

  const jumpToBottom = useCallback(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    setAutoScroll(true);
    setHasNewContent(false);
  }, []);

  const appendEntry = useCallback((partial: Omit<LogEntry, "id">) => {
    setEntries(prev => [...prev, { id: ++entryIdRef.current, ...partial }]);
  }, []);

  // Coalesce consecutive token events into one entry to avoid thousands of DOM nodes.
  const appendToken = useCallback((token: string) => {
    setOutChars(c => c + token.length);
    setEntries(prev => {
      if (prev.length > 0 && prev[prev.length - 1].kind === "token") {
        const copy = [...prev];
        copy[copy.length - 1] = {
          ...copy[copy.length - 1],
          content: (copy[copy.length - 1].content ?? "") + token,
        };
        return copy;
      }
      return [...prev, { id: ++entryIdRef.current, kind: "token", content: token }];
    });
  }, []);

  const toggleExpand = useCallback((id: number) => {
    setEntries(prev => prev.map(e => e.id === id ? { ...e, expanded: !e.expanded } : e));
  }, []);

  const handleApprove = useCallback((_requestId: string, id: number, approved: boolean) => {
    setEntries(prev => prev.map(e => e.id === id ? { ...e, approvalResponded: true, approved } : e));
    // TODO: invoke Tauri command when backend supports approval gates.
  }, []);

  const handleHumanInput = useCallback((_requestId: string, id: number, value: string) => {
    setEntries(prev => prev.map(e => e.id === id ? { ...e, humanResponded: true, humanResponse: value } : e));
    // TODO: invoke Tauri command when backend supports human input.
  }, []);

  const startRun = useCallback(async () => {
    setEntries([]);
    setRawEvents([]);
    setStatus("running");
    setIsRunning(true);
    setOutChars(0);
    setDuration(null);
    setAutoScroll(true);
    setHasNewContent(false);
    autoScrollRef.current = true;
    startTimeRef.current = Date.now();

    try {
      const id = await invoke<string>("start_run", { storyId, profileId });
      setRunId(id);

      const unlisten = await listen<RunEventKind>("run-event", ({ payload }) => {
        if (payload.run_id !== id) return;
        setRawEvents(prev => [...prev, payload]);

        switch (payload.type) {
          case "token":
            appendToken(payload.content);
            break;
          case "tool_call":
            appendEntry({ kind: "tool_call", toolName: payload.tool_name, toolInput: payload.input, expanded: false });
            break;
          case "tool_result":
            appendEntry({ kind: "tool_result", toolName: payload.tool_name, toolOutput: payload.output, isError: payload.is_error });
            break;
          case "complete":
            appendEntry({ kind: "complete", stopReason: payload.stop_reason });
            setStatus("complete");
            setIsRunning(false);
            setDuration(Date.now() - (startTimeRef.current ?? Date.now()));
            break;
          case "cancelled":
            appendEntry({ kind: "cancelled" });
            setStatus("cancelled");
            setIsRunning(false);
            setDuration(Date.now() - (startTimeRef.current ?? Date.now()));
            break;
          case "failed":
            appendEntry({ kind: "failed", message: payload.message });
            setStatus("failed");
            setIsRunning(false);
            setDuration(Date.now() - (startTimeRef.current ?? Date.now()));
            break;
          case "approval_request":
            appendEntry({
              kind: "approval_request",
              requestId: payload.request_id,
              approvalToolName: payload.tool_name,
              approvalInput: payload.input,
              approvalResponded: false,
            });
            break;
          case "human_input":
            appendEntry({
              kind: "human_input",
              requestId: payload.request_id,
              humanPrompt: payload.prompt,
              humanResponded: false,
            });
            break;
        }
      });

      unlistenRef.current = unlisten;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      appendEntry({ kind: "failed", message: msg });
      setStatus("failed");
      setIsRunning(false);
    }
  }, [storyId, profileId, appendEntry, appendToken]);

  // Keep startRunRef current so the autoStart effect can call it without a stale closure.
  startRunRef.current = startRun;

  useEffect(() => {
    if (autoStart && !autoStartedRef.current) {
      autoStartedRef.current = true;
      startRunRef.current?.();
    }
  }, [autoStart]);

  const stopRun = useCallback(async () => {
    if (!runId) return;
    try {
      await invoke("stop_run", { runId });
    } catch (err) {
      console.error("stop_run failed:", err);
    }
  }, [runId]);

  const exportJsonl = useCallback(() => {
    const lines = rawEvents.map(e => JSON.stringify(e)).join("\n");
    const blob = new Blob([lines], { type: "application/x-ndjson" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `run-${runId ?? "export"}.jsonl`;
    a.click();
    URL.revokeObjectURL(url);
  }, [rawEvents, runId]);

  const approxTokens = Math.round(outChars / 4);
  const isDone = status === "complete" || status === "cancelled" || status === "failed";

  return (
    <div className="rp" data-status={status}>
      <header className="rp__header">
        <span className="rp__title">{storyTitle ?? "Agent Run"}</span>
        <div className="rp__header-right">
          {approxTokens > 0 && <TokenCounter tokens={approxTokens} />}
          <span className="rp__status-badge" data-status={status}>
            {statusLabel(status)}
          </span>
        </div>
      </header>

      <div className="rp__log" ref={scrollRef} onScroll={handleScroll}>
        {entries.length === 0 && status === "idle" && (
          <div className="rp__idle-msg">Press "Start Run" to begin.</div>
        )}
        {entries.map((entry, idx) => (
          <EventRow
            key={entry.id}
            entry={entry}
            isLast={idx === entries.length - 1}
            isRunning={isRunning}
            onToggle={() => toggleExpand(entry.id)}
            onApprove={(approved) => handleApprove(entry.requestId!, entry.id, approved)}
            onHumanInput={(value) => handleHumanInput(entry.requestId!, entry.id, value)}
          />
        ))}
      </div>

      {!autoScroll && hasNewContent && (
        <button className="rp__jump-btn" onClick={jumpToBottom} aria-label="Jump to bottom">
          ↓ New content below
        </button>
      )}

      <footer className="rp__footer">
        <div className="rp__footer-left">
          {isRunning && (
            <button className="btn btn--destructive btn--sm" onClick={stopRun}>
              ⏹ Stop Run
            </button>
          )}
          {!isRunning && status === "idle" && (
            <button className="btn btn--primary" onClick={startRun}>
              ▶ Start Run
            </button>
          )}
          {!isRunning && isDone && (
            <button className="btn btn--secondary btn--sm" onClick={startRun}>
              ↺ Re-run
            </button>
          )}
        </div>
        <div className="rp__footer-right">
          {isDone && (
            <>
              {duration !== null && (
                <span className="rp__duration">{formatDuration(duration)}</span>
              )}
              <button className="btn btn--ghost btn--sm" onClick={exportJsonl}>
                ↓ Export .jsonl
              </button>
            </>
          )}
          <button
            className={`rp__autoscroll-btn${autoScroll ? " rp__autoscroll-btn--on" : ""}`}
            onClick={() => setAutoScroll(v => !v)}
          >
            {autoScroll ? "↓ Auto-scroll: ON" : "↓ Auto-scroll: OFF"}
          </button>
        </div>
      </footer>
    </div>
  );
}

// ---------------------------------------------------------------------------
// EventRow dispatcher
// ---------------------------------------------------------------------------

interface EventRowProps {
  entry: LogEntry;
  isLast: boolean;
  isRunning: boolean;
  onToggle: () => void;
  onApprove: (approved: boolean) => void;
  onHumanInput: (value: string) => void;
}

function EventRow({ entry, isLast, isRunning, onToggle, onApprove, onHumanInput }: EventRowProps) {
  switch (entry.kind) {
    case "token":
      return <TokenRow entry={entry} isLast={isLast} isRunning={isRunning} />;
    case "tool_call":
      return <ToolCallRow entry={entry} onToggle={onToggle} />;
    case "tool_result":
      return <ToolResultRow entry={entry} />;
    case "complete":
    case "cancelled":
    case "failed":
      return <StatusRow entry={entry} />;
    case "approval_request":
      return <ApprovalRow entry={entry} onApprove={onApprove} />;
    case "human_input":
      return <HumanInputRow entry={entry} onSubmit={onHumanInput} />;
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Token row — streaming assistant text with blinking cursor
// ---------------------------------------------------------------------------

function TokenRow({
  entry,
  isLast,
  isRunning,
}: {
  entry: LogEntry;
  isLast: boolean;
  isRunning: boolean;
}) {
  return (
    <div className="rp-row rp-row--token">
      {entry.content}
      {isLast && isRunning && <span className="rp__cursor" aria-hidden />}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tool call row — collapsible with JSON viewer
// ---------------------------------------------------------------------------

function ToolCallRow({ entry, onToggle }: { entry: LogEntry; onToggle: () => void }) {
  return (
    <div className="rp-row rp-row--tool-call">
      <button className="rp-row__toggle" onClick={onToggle} aria-expanded={!!entry.expanded}>
        <span className="rp-row__icon">⚙</span>
        <span className="rp-row__label">tool_call</span>
        <span className="rp-row__tool-name">{entry.toolName}</span>
        <span className="rp-row__chevron">{entry.expanded ? "▴" : "▾"}</span>
      </button>
      {entry.expanded && (
        <div className="rp-row__body">
          <JsonViewer value={entry.toolInput} />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tool result row — truncated at 300 chars with expand toggle
// ---------------------------------------------------------------------------

const TRUNCATE_LEN = 300;

function ToolResultRow({ entry }: { entry: LogEntry }) {
  const [expanded, setExpanded] = useState(false);
  const output = entry.toolOutput ?? "";
  const isLong = output.length > TRUNCATE_LEN;
  const displayText = !expanded && isLong ? output.slice(0, TRUNCATE_LEN) + "…" : output;

  return (
    <div className={`rp-row rp-row--tool-result${entry.isError ? " rp-row--error" : ""}`}>
      <div className="rp-row__header">
        <span className="rp-row__icon">{entry.isError ? "✖" : "←"}</span>
        <span className={`rp-row__label${entry.isError ? " rp-row__label--error" : ""}`}>
          tool_result
        </span>
        <span className="rp-row__tool-name">{entry.toolName}</span>
      </div>
      <div className="rp-row__body rp-row__output">
        <pre>{displayText}</pre>
        {isLong && (
          <button className="rp-row__expand-btn" onClick={() => setExpanded(v => !v)}>
            {expanded ? "Show less ▴" : "Show full result ▾"}
          </button>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Status row — complete / cancelled / failed
// ---------------------------------------------------------------------------

function StatusRow({ entry }: { entry: LogEntry }) {
  const meta: Record<string, { icon: string; label: string }> = {
    complete:  { icon: "✔", label: "Complete" },
    cancelled: { icon: "⏹", label: "Cancelled" },
    failed:    { icon: "✖", label: "Error" },
  };
  const text: Record<string, string | undefined> = {
    complete:  entry.stopReason ? `stop_reason: ${entry.stopReason}` : undefined,
    cancelled: "Run was cancelled.",
    failed:    entry.message,
  };
  const { icon, label } = meta[entry.kind] ?? { icon: "?", label: entry.kind };

  return (
    <div className={`rp-row rp-row--${entry.kind}`}>
      <div className="rp-row__header">
        <span className="rp-row__icon">{icon}</span>
        <span className="rp-row__label">{label}</span>
        {text[entry.kind] && (
          <span className="rp-row__status-text">{text[entry.kind]}</span>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Approval gate row — Approve / Reject with amber border
// ---------------------------------------------------------------------------

function ApprovalRow({
  entry,
  onApprove,
}: {
  entry: LogEntry;
  onApprove: (approved: boolean) => void;
}) {
  return (
    <div className="rp-row rp-row--approval">
      <div className="rp-row__header">
        <span className="rp-row__icon">⚠</span>
        <span className="rp-row__label rp-row__label--approval">approval_request</span>
      </div>
      <div className="rp-row__body">
        <p className="rp-row__approval-prompt">Approve tool call?</p>
        <p className="rp-row__approval-meta">
          <strong>Tool:</strong> {entry.approvalToolName}
        </p>
        <JsonViewer value={entry.approvalInput} />
        {!entry.approvalResponded ? (
          <div className="rp-row__approval-actions">
            <button className="btn btn--destructive btn--sm" onClick={() => onApprove(false)}>
              Reject
            </button>
            <button className="btn btn--primary btn--sm" onClick={() => onApprove(true)}>
              Approve
            </button>
          </div>
        ) : (
          <p className="rp-row__approval-result">
            {entry.approved ? "✔ Approved" : "✖ Rejected"}
          </p>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Human input row — auto-focused textarea + Send
// ---------------------------------------------------------------------------

function HumanInputRow({
  entry,
  onSubmit,
}: {
  entry: LogEntry;
  onSubmit: (value: string) => void;
}) {
  const [value, setValue] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (!entry.humanResponded) textareaRef.current?.focus();
  }, [entry.humanResponded]);

  const handleSubmit = () => {
    const trimmed = value.trim();
    if (!trimmed) return;
    onSubmit(trimmed);
    setValue("");
  };

  return (
    <div className="rp-row rp-row--human-input">
      <div className="rp-row__header">
        <span className="rp-row__icon">👤</span>
        <span className="rp-row__label rp-row__label--human">human_input</span>
      </div>
      <div className="rp-row__body">
        <p className="rp-row__human-prompt">"{entry.humanPrompt}"</p>
        {!entry.humanResponded ? (
          <>
            <textarea
              ref={textareaRef}
              className="form-input"
              placeholder="Type your response…"
              value={value}
              onChange={e => setValue(e.target.value)}
              onKeyDown={e => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  handleSubmit();
                }
              }}
              rows={3}
            />
            <div className="rp-row__human-actions">
              <button
                className="btn btn--primary btn--sm"
                onClick={handleSubmit}
                disabled={!value.trim()}
              >
                Send
              </button>
            </div>
          </>
        ) : (
          <p className="rp-row__human-response">
            <em>You:</em> {entry.humanResponse}
          </p>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// JSON viewer — formatted pre + hover copy button
// ---------------------------------------------------------------------------

function JsonViewer({ value }: { value: unknown }) {
  const [copied, setCopied] = useState(false);
  const formatted = JSON.stringify(value, null, 2);

  const handleCopy = async () => {
    await navigator.clipboard.writeText(formatted);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="json-viewer">
      <button className="json-viewer__copy" onClick={handleCopy} aria-label="Copy JSON">
        {copied ? "✔ Copied" : "Copy"}
      </button>
      <pre className="json-viewer__pre">{formatted}</pre>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Token counter — header widget with breakdown tooltip
// ---------------------------------------------------------------------------

function TokenCounter({ tokens }: { tokens: number }) {
  const [show, setShow] = useState(false);
  return (
    <div
      className="token-counter"
      onMouseEnter={() => setShow(true)}
      onMouseLeave={() => setShow(false)}
    >
      <span className="token-counter__value">~{tokens.toLocaleString()} tokens</span>
      {show && (
        <div className="token-counter__tooltip" role="tooltip">
          <div>Output tokens: ~{tokens.toLocaleString()}</div>
          <div className="token-counter__note">Approximated from token stream (chars ÷ 4)</div>
        </div>
      )}
    </div>
  );
}


