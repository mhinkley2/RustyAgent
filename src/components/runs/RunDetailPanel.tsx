import { useState } from "react";
import { ChevronDown, ChevronRight, Copy, Download, Trash2 } from "lucide-react";
import { SlidePanel } from "../forms";
import { ConfirmDialog } from "../forms";
import type { StoryRun, RunEvent, RunDiff } from "../../types/runs";
import {
  formatEstimatedCost,
  formatTokens,
  formatDuration,
  totalInputTokens,
  RUN_STATUS_LABELS,
} from "../../types/runs";
import { useRunEvents, useRunDiff, exportRun } from "../../hooks/useRuns";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function timeAgo(date: Date): string {
  const s = Math.round((Date.now() - date.getTime()) / 1000);
  if (s < 60) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

function tryPrettyJson(raw: string | null): string {
  if (!raw) return "";
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

function copyText(text: string) {
  navigator.clipboard.writeText(text).catch(() => {});
}

// ---------------------------------------------------------------------------
// EventRow
// ---------------------------------------------------------------------------

function MessageEvent({ event }: { event: RunEvent }) {
  const roleLabel = event.role
    ? event.role.charAt(0).toUpperCase() + event.role.slice(1)
    : "System";
  return (
    <div className={`run-event run-event--message run-event--role-${event.role ?? "system"}`}>
      <span className="run-event__label">{roleLabel}</span>
      <p className="run-event__content">{event.content}</p>
    </div>
  );
}

function ToolCallEvent({ event }: { event: RunEvent }) {
  const [expanded, setExpanded] = useState(false);
  const pretty = tryPrettyJson(event.toolInput);
  return (
    <div className="run-event run-event--tool-call">
      <button
        className="run-event__tool-header"
        onClick={() => setExpanded(e => !e)}
      >
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
        <span className="run-event__label run-event__label--tool">⚙ tool_call</span>
        <span className="run-event__tool-name">{event.toolName}</span>
      </button>
      {expanded && (
        <div className="run-event__code-block">
          <button
            className="run-event__copy-btn"
            onClick={() => copyText(pretty)}
            title="Copy"
          >
            <Copy size={11} />
          </button>
          <pre>{pretty}</pre>
        </div>
      )}
    </div>
  );
}

function ToolResultEvent({ event }: { event: RunEvent }) {
  const [expanded, setExpanded] = useState(false);
  const raw = event.toolOutput ?? event.content ?? "";
  const preview = raw.length > 300 ? raw.slice(0, 300) + "…" : raw;
  const pretty = tryPrettyJson(raw);
  const hasMore = raw.length > 300;
  return (
    <div className={`run-event run-event--tool-result${event.isError ? " run-event--error" : ""}`}>
      <span className="run-event__label run-event__label--result">
        {event.isError ? "✗ tool_error" : "↩ tool_result"}
      </span>
      <span className="run-event__tool-name">{event.toolName}</span>
      <div className="run-event__result-body">
        <pre className="run-event__result-pre">{expanded ? pretty : preview}</pre>
        {hasMore && (
          <button
            className="run-event__expand-btn"
            onClick={() => setExpanded(e => !e)}
          >
            {expanded ? "Show less ▲" : "Show full result ▾"}
          </button>
        )}
      </div>
    </div>
  );
}

function ErrorEvent({ event }: { event: RunEvent }) {
  return (
    <div className="run-event run-event--error">
      <span className="run-event__label run-event__label--error">✗ error</span>
      <pre className="run-event__error-content">{event.content}</pre>
    </div>
  );
}

interface CompactionDetail {
  strategy: string;
  before_tokens: number;
  after_tokens: number;
  budget_tokens: number;
  evicted_messages: number;
  summarized: boolean;
}

/**
 * A compaction is the one event that explains an agent forgetting something,
 * so the timeline shows what it cost rather than the raw payload: how much
 * context went away, against which budget, and whether a summary replaced it.
 */
function ContextCompactedEvent({ event }: { event: RunEvent }) {
  let detail: CompactionDetail | null = null;
  try {
    detail = JSON.parse(event.content ?? "") as CompactionDetail;
  } catch {
    detail = null;
  }
  if (!detail) return <GenericEvent event={event} />;

  return (
    <div className="run-event run-event--compaction">
      <span className="run-event__label run-event__label--compaction">
        ✂ context compacted
      </span>
      <p className="run-event__content">
        {detail.strategy} · {formatTokens(detail.before_tokens)} →{" "}
        {formatTokens(detail.after_tokens)} tokens against a{" "}
        {formatTokens(detail.budget_tokens)} budget ·{" "}
        {detail.evicted_messages} message
        {detail.evicted_messages === 1 ? "" : "s"} dropped
        {detail.summarized ? " · replaced by a summary" : ""}
      </p>
    </div>
  );
}

function GenericEvent({ event }: { event: RunEvent }) {
  return (
    <div className="run-event run-event--generic">
      <span className="run-event__label">{event.eventType}</span>
      {event.content && <p className="run-event__content">{event.content}</p>}
    </div>
  );
}

function EventRow({ event }: { event: RunEvent }) {
  switch (event.eventType) {
    case "message":         return <MessageEvent event={event} />;
    case "tool_call":       return <ToolCallEvent event={event} />;
    case "tool_result":     return <ToolResultEvent event={event} />;
    case "error":           return <ErrorEvent event={event} />;
    case "context_compacted": return <ContextCompactedEvent event={event} />;
    default:                return <GenericEvent event={event} />;
  }
}

// ---------------------------------------------------------------------------
// ChangesTab — inline unified diff renderer
// ---------------------------------------------------------------------------

type DiffLineKind = "header" | "hunk" | "added" | "removed" | "context" | "meta";

interface DiffLine {
  kind: DiffLineKind;
  text: string;
}

function parseDiff(raw: string): DiffLine[] {
  const lines: DiffLine[] = [];
  for (const line of raw.split("\n")) {
    if (line.startsWith("diff --git") || line.startsWith("---") || line.startsWith("+++")) {
      lines.push({ kind: "header", text: line });
    } else if (line.startsWith("@@")) {
      lines.push({ kind: "hunk", text: line });
    } else if (line.startsWith("+")) {
      lines.push({ kind: "added", text: line });
    } else if (line.startsWith("-")) {
      lines.push({ kind: "removed", text: line });
    } else if (line.startsWith("index ") || line.startsWith("new file") || line.startsWith("deleted file")) {
      lines.push({ kind: "meta", text: line });
    } else {
      lines.push({ kind: "context", text: line });
    }
  }
  return lines;
}

function ChangesTab({ run, diff, loading, error }: {
  run: StoryRun;
  diff: RunDiff | null;
  loading: boolean;
  error: string | null;
}) {
  if (!run.beforeSha) {
    return (
      <p className="run-detail__muted">
        Git not detected — file diffs are unavailable for this run.
      </p>
    );
  }
  if (loading) {
    return <div className="run-detail__loading">Loading diff…</div>;
  }
  if (error) {
    return <p className="run-detail__muted">Failed to load diff: {error}</p>;
  }
  if (!diff?.diffOutput) {
    return (
      <p className="run-detail__muted">
        No file changes recorded for this run.
      </p>
    );
  }

  const lines = parseDiff(diff.diffOutput);

  return (
    <div className="run-diff">
      <div className="run-diff__sha-row">
        <span className="run-diff__sha-label">Base commit</span>
        <code className="run-diff__sha">{run.beforeSha.slice(0, 12)}</code>
      </div>
      <div className="run-diff__body">
        {lines.map((line, i) => (
          <div key={i} className={`run-diff__line run-diff__line--${line.kind}`}>
            <span className="run-diff__line-text">{line.text || " "}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// RunDetailPanel
// ---------------------------------------------------------------------------

interface RunDetailPanelProps {
  run: StoryRun | null;
  onClose: () => void;
  onDelete?: (run: StoryRun) => void;
}

export function RunDetailPanel({ run, onClose, onDelete }: RunDetailPanelProps) {
  const { events, loading: eventsLoading } = useRunEvents(run?.id ?? null);
  const { diff, loading: diffLoading, error: diffError } = useRunDiff(run?.id ?? null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [activeTab, setActiveTab] = useState<"events" | "changes">("events");

  async function handleExport() {
    if (!run) return;
    const storySlug = (run.storyTitle ?? run.storyId).slice(0, 30).replace(/\s+/g, "-").toLowerCase();
    await exportRun(run.id, `run-${storySlug}-${run.id.slice(0, 8)}.jsonl`);
  }

  const footer = run ? (
    <div className="run-detail__footer">
      <button className="btn btn--ghost btn--sm" onClick={() => setConfirmDelete(true)}>
        <Trash2 size={13} />
        Delete
      </button>
      <button className="btn btn--secondary btn--sm" onClick={handleExport}>
        <Download size={13} />
        Export .jsonl
      </button>
    </div>
  ) : undefined;

  const statusClass = run ? `run-detail__status--${run.status}` : "";

  return (
    <>
      <SlidePanel
        open={run !== null}
        onClose={onClose}
        title={run ? (run.storyTitle ?? "Run Detail") : "Run Detail"}
        footer={footer}
        width={560}
      >
        {run && (
          <div className="run-detail">
            {/* ── Header meta ─────────────────────────────────────────── */}
            <div className="run-detail__meta">
              <span className={`run-detail__status-badge ${statusClass}`}>
                {RUN_STATUS_LABELS[run.status]}
              </span>
              <span className="run-detail__agent">{run.agentName ?? run.agentProfileId}</span>
              <span className="run-detail__time">{timeAgo(run.startedAt)}</span>
            </div>

            {/* ── Stats row ───────────────────────────────────────────── */}
            <div className="run-detail__stats">
              <div className="run-detail__stat">
                <span className="run-detail__stat-label">Duration</span>
                <span className="run-detail__stat-value">{formatDuration(run.durationSecs)}</span>
              </div>
              <div className="run-detail__stat">
                <span className="run-detail__stat-label">Tokens</span>
                <span className="run-detail__stat-value">
                  {formatTokens(totalInputTokens(run))} in · {formatTokens(run.outputTokens)} out
                </span>
              </div>
              <div className="run-detail__stat">
                <span className="run-detail__stat-label">Cached</span>
                <span className="run-detail__stat-value">
                  {formatTokens(run.cacheReadTokens)} read
                  {run.cacheCreationTokens > 0
                    ? ` · ${formatTokens(run.cacheCreationTokens)} written`
                    : ""}
                </span>
              </div>
              <div className="run-detail__stat">
                <span className="run-detail__stat-label">Est. cost</span>
                <span className="run-detail__stat-value">{formatEstimatedCost(run)}</span>
              </div>
              <div className="run-detail__stat">
                <span className="run-detail__stat-label">Iterations</span>
                <span className="run-detail__stat-value">{run.iterationCount}</span>
              </div>
            </div>

            <hr className="run-detail__divider" />

            {/* ── Tab bar ───────────────────────────────────────────── */}
            <div className="run-detail__tabs" role="tablist">
              <button
                role="tab"
                aria-selected={activeTab === "events"}
                className={`run-detail__tab${activeTab === "events" ? " run-detail__tab--active" : ""}`}
                onClick={() => setActiveTab("events")}
              >
                Event Log
              </button>
              <button
                role="tab"
                aria-selected={activeTab === "changes"}
                className={`run-detail__tab${activeTab === "changes" ? " run-detail__tab--active" : ""}`}
                onClick={() => setActiveTab("changes")}
              >
                File Changes
              </button>
            </div>

            {/* ── Tab content ───────────────────────────────────────── */}
            {activeTab === "events" ? (
              <section className="run-detail__events">
                {eventsLoading ? (
                  <div className="run-detail__loading">Loading events…</div>
                ) : events.length === 0 ? (
                  <p className="run-detail__muted">No events recorded for this run.</p>
                ) : (
                  <div className="run-detail__event-list">
                    {events.map(evt => (
                      <EventRow key={evt.id} event={evt} />
                    ))}
                  </div>
                )}
              </section>
            ) : (
              <section className="run-detail__events">
                <ChangesTab
                  run={run}
                  diff={diff}
                  loading={diffLoading}
                  error={diffError}
                />
              </section>
            )}
          </div>
        )}
      </SlidePanel>

      <ConfirmDialog
        open={confirmDelete}
        title={`Delete this run?`}
        body="This will permanently delete the run and all its events. This cannot be undone."
        confirmLabel="Delete Run"
        onClose={() => setConfirmDelete(false)}
        onConfirm={() => {
          setConfirmDelete(false);
          if (run) onDelete?.(run);
          onClose();
        }}
      />
    </>
  );
}
