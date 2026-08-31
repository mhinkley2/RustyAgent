import { useState, useEffect, useMemo } from "react";
import { Play, ExternalLink, GitBranch, ArrowLeft } from "lucide-react";
import { SlidePanel } from "../forms";
import type {
  Story,
  StoryStatus,
  StoryPriority,
  StoryType,
  StoryLatestRun,
} from "../../types/board";
import type { StoryRun, RunStatus } from "../../types/runs";
import {
  RUN_STATUS_LABELS,
  formatCost,
  formatDuration,
  formatEstimatedCost,
} from "../../types/runs";
import { runsForStory } from "../../hooks/useRuns";
import { usePipelineProgress } from "../../hooks/usePipelineProgress";
import { PipelineProgressPanel } from "../pipeline/PipelineProgressPanel";
import { RunPanel } from "../RunPanel";

// ---------------------------------------------------------------------------
// SimpleMarkdown — renders a small subset of Markdown without external deps
// ---------------------------------------------------------------------------

function SimpleMarkdown({ content, className }: { content: string; className?: string }) {
  const lines = content.split("\n");
  const elements: React.ReactNode[] = [];
  let i = 0;

  function parseInline(text: string): React.ReactNode[] {
    // Handle **bold**, *italic*, `code`
    const parts: React.ReactNode[] = [];
    const re = /(\*\*(.+?)\*\*|\*(.+?)\*|`([^`]+)`)/g;
    let last = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) {
      if (m.index > last) parts.push(text.slice(last, m.index));
      if (m[2] !== undefined) parts.push(<strong key={m.index}>{m[2]}</strong>);
      else if (m[3] !== undefined) parts.push(<em key={m.index}>{m[3]}</em>);
      else if (m[4] !== undefined) parts.push(<code key={m.index}>{m[4]}</code>);
      last = m.index + m[0].length;
    }
    if (last < text.length) parts.push(text.slice(last));
    return parts;
  }

  while (i < lines.length) {
    const line = lines[i];

    // Fenced code block
    if (line.startsWith("```")) {
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        codeLines.push(lines[i]);
        i++;
      }
      elements.push(<pre key={i} className="sdp__md-pre"><code>{codeLines.join("\n")}</code></pre>);
      i++;
      continue;
    }

    // Bullet list item
    if (/^[-*] /.test(line)) {
      const listItems: React.ReactNode[] = [];
      while (i < lines.length && /^[-*] /.test(lines[i])) {
        const text = lines[i].slice(2);
        listItems.push(<li key={i}>{parseInline(text)}</li>);
        i++;
      }
      elements.push(<ul key={i} className="sdp__md-list">{listItems}</ul>);
      continue;
    }

    // Blank line → spacer
    if (line.trim() === "") {
      i++;
      continue;
    }

    // Regular paragraph line
    elements.push(<p key={i} className="sdp__md-p">{parseInline(line)}</p>);
    i++;
  }

  return <div className={className}>{elements}</div>;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const STATUS_LABELS: Record<StoryStatus, string> = {
  backlog:     "Backlog",
  ready:       "Ready",
  in_progress: "In Progress",
  blocked:     "Blocked",
  review:      "Review",
  done:        "Done",
};

const PRIORITY_LABELS: Record<StoryPriority, string> = {
  critical: "Critical",
  high:     "High",
  medium:   "Medium",
  low:      "Low",
};

const TYPE_LABELS: Record<StoryType, string> = {
  task:     "Task",
  human:    "Human Input",
  pipeline: "Pipeline",
};

function timeAgo(date: Date): string {
  const diffMs = Date.now() - date.getTime();
  const s = Math.round(diffMs / 1000);
  if (s < 60)  return "just now";
  const m = Math.round(s / 60);
  if (m < 60)  return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24)  return `${h}h ago`;
  const d = Math.round(h / 24);
  return `${d}d ago`;
}

/**
 * A glyph per run state, alongside the shared `RUN_STATUS_LABELS` so the two
 * cannot come to describe different sets.
 */
const RUN_STATUS_GLYPHS: Record<RunStatus, string> = {
  running:   "⟳",
  done:      "✓",
  failed:    "✗",
  cancelled: "⊘",
};

/**
 * How long a run took, or how long it has been going.
 *
 * A running row has no `finishedAt`, so it counts against now — which is what
 * makes an active run read as active rather than as one that took zero time.
 * The value is a snapshot: the panel re-renders on the board's refresh rather
 * than ticking, which is enough for a detail view nobody watches by the
 * second.
 */
function runDuration(run: StoryLatestRun): string {
  const end = run.finishedAt?.getTime() ?? Date.now();
  return formatDuration(Math.max(0, end - run.startedAt.getTime()) / 1000);
}

// ---------------------------------------------------------------------------
// Row helper
// ---------------------------------------------------------------------------

function DetailRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="sdp__row">
      <span className="sdp__label">{label}</span>
      <span className="sdp__value">{children}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// StoryDetailPanel
// ---------------------------------------------------------------------------

interface StoryDetailPanelProps {
  story: Story | null;
  onClose: () => void;
  onEdit?: (story: Story) => void;
  onDelete?: (story: Story) => void;
  /** Called when user clicks "Run Now" */
  onRun?: (story: Story) => void;
  /**
   * Open the run detail view for a run id.
   *
   * The panel does not navigate itself: it is rendered inside the board, and
   * the board owns where a run opens.
   */
  onOpenRun?: (runId: string) => void;
}

export function StoryDetailPanel({ story, onClose, onEdit, onDelete, onRun, onOpenRun }: StoryDetailPanelProps) {
  /**
   * Every run for this story, so the panel can list the ones before the
   * latest. Fetched for the open story only — the board's own query carries
   * just the newest run per card.
   */
  const [allRuns, setAllRuns] = useState<StoryRun[]>([]);

  useEffect(() => {
    if (!story) {
      setAllRuns([]);
      return;
    }
    let cancelled = false;
    runsForStory(story.id)
      .then((runs) => {
        if (!cancelled) setAllRuns(runs);
      })
      // A missing history is not worth a toast; the section simply does not
      // render, and the latest run above it still does.
      .catch(() => {
        if (!cancelled) setAllRuns([]);
      });
    return () => {
      cancelled = true;
    };
  }, [story?.id]);

  /** Everything but the run already shown above. */
  const priorRuns = useMemo(
    () => allRuns.filter((run) => run.id !== story?.latestRun?.id),
    [allRuns, story?.latestRun?.id],
  );

  const [activePipelineRunId, setActivePipelineRunId] = useState<string | null>(null);
  const [showRun, setShowRun] = useState(false);
  const { progress, startPipelineRun } = usePipelineProgress(
    story?.type === "pipeline" ? activePipelineRunId : null
  );

  // Reset run view when story changes
  const storyId = story?.id;
  const [lastStoryId, setLastStoryId] = useState<string | undefined>(undefined);
  if (storyId !== lastStoryId) {
    setLastStoryId(storyId);
    if (showRun) setShowRun(false);
    if (activePipelineRunId) setActivePipelineRunId(null);
  }

  const isPipeline = story?.type === "pipeline";

  async function handleStartPipeline() {
    if (!story || !story.assignedAgentId) return;
    try {
      const runId = await startPipelineRun(story.id, story.assignedAgentId);
      setActivePipelineRunId(runId);
    } catch (err) {
      console.error("start_pipeline_run failed:", err);
    }
  }

  const isPipelineRunning = progress && (progress.status === "running");

  const footer = story ? (
    showRun ? (
      <div className="sdp__footer-actions">
        <button className="btn btn--ghost btn--sm" onClick={() => setShowRun(false)}>
          <ArrowLeft size={14} />
          Back to details
        </button>
      </div>
    ) : (
    <div className="sdp__footer-actions">
      <div style={{ display: "flex", gap: 8 }}>
        <button
          className="btn btn--secondary btn--sm"
          onClick={() => onEdit?.(story)}
        >
          Edit
        </button>
        <button
          className="btn btn--secondary btn--sm"
          style={{ color: "var(--error)" }}
          onClick={() => onDelete?.(story)}
        >
          Delete
        </button>
      </div>
      {isPipeline ? (
        <button
          className="btn btn--primary"
          onClick={handleStartPipeline}
          disabled={isPipelineRunning || !story.assignedAgentId}
          title={!story.assignedAgentId ? "Assign an agent profile first" : undefined}
        >
          <GitBranch size={14} />
          {isPipelineRunning ? "Running…" : "Start Pipeline"}
        </button>
      ) : (
        <button
          className="btn btn--primary"
          disabled={!story.assignedAgentId}
          title={!story.assignedAgentId ? "Assign an agent profile first" : undefined}
          onClick={() => { onRun?.(story); setShowRun(true); }}
        >
          <Play size={14} />
          Run Now
        </button>
      )}
    </div>
    )
  ) : undefined;

  return (
    <SlidePanel
      open={story !== null}
      onClose={onClose}
      title={story ? (story.key ? `${story.key}: ${story.title}` : story.title) : "Story Detail"}
      footer={footer}
      width={520}
    >
      {story && showRun && story.assignedAgentId && (
        <RunPanel
          storyId={story.id}
          profileId={story.assignedAgentId}
          storyTitle={story.title}
          autoStart
        />
      )}
      {story && !showRun && (
        <div className="sdp">
          {/* ── Badges ─────────────────────────────────────────────────── */}
          <div className="sdp__badges">
            <span className={`sdp__badge sdp__badge--status sdp__badge--status-${story.status}`}>
              {STATUS_LABELS[story.status]}
            </span>
            <span className={`sdp__badge sdp__badge--priority sdp__badge--priority-${story.priority}`}>
              {PRIORITY_LABELS[story.priority]}
            </span>
            <span className={`sdp__badge sdp__badge--type sdp__badge--type-${story.type}`}>
              {TYPE_LABELS[story.type]}
            </span>
          </div>

          {/* ── Details grid ────────────────────────────────────────────── */}
          <div className="sdp__meta">
            <DetailRow label="Assignee">
              {story.assignee ?? <span className="sdp__muted">Unassigned</span>}
            </DetailRow>
            <DetailRow label="Updated">
              {timeAgo(story.updatedAt)}
            </DetailRow>
            <DetailRow label="Created">
              {timeAgo(story.createdAt)}
            </DetailRow>
          </div>

          <hr className="sdp__divider" />

          {/* ── Description ─────────────────────────────────────────────── */}
          <section className="sdp__section">
            <h3 className="sdp__section-title">Description</h3>
            {story.description ? (
              <SimpleMarkdown className="sdp__description" content={story.description} />
            ) : (
              <p className="sdp__muted">No description provided.</p>
            )}
          </section>

          {/* ── Latest run ──────────────────────────────────────────────── */}
          {story.latestRun && (
            <>
              <hr className="sdp__divider" />
              <section className="sdp__section">
                <h3 className="sdp__section-title">Latest Run</h3>
                <div className="sdp__run-card">
                  <div className="sdp__run-top">
                    <span
                      className={`sdp__run-status sdp__run-status--${story.latestRun.status}`}
                    >
                      {RUN_STATUS_GLYPHS[story.latestRun.status]}
                      &nbsp;{RUN_STATUS_LABELS[story.latestRun.status]}
                    </span>
                    <span className="sdp__muted">{timeAgo(story.latestRun.startedAt)}</span>
                  </div>
                  <div className="sdp__run-meta">
                    <span>Duration: {runDuration(story.latestRun)}</span>
                    <span>
                      {story.latestRun.iterationCount} iteration
                      {story.latestRun.iterationCount === 1 ? "" : "s"}
                    </span>
                    <span>{formatCost(story.latestRun.estimatedCostUsd)}</span>
                  </div>
                  <button
                    className="btn btn--ghost btn--sm sdp__run-link"
                    onClick={() => onOpenRun?.(story.latestRun!.id)}
                  >
                    <ExternalLink size={12} />
                    View full run
                  </button>
                </div>
              </section>
            </>
          )}

          {/* ── Run history ─────────────────────────────────────────────── */}
          {priorRuns.length > 0 && (
            <>
              <hr className="sdp__divider" />
              <section className="sdp__section">
                <h3 className="sdp__section-title">
                  Earlier Runs ({priorRuns.length})
                </h3>
                <ul className="sdp__run-history">
                  {priorRuns.map((run) => (
                    <li key={run.id} className="sdp__run-history-row">
                      <button
                        className="sdp__run-history-link"
                        onClick={() => onOpenRun?.(run.id)}
                        title="Open this run"
                      >
                        <span
                          className={`sdp__run-status sdp__run-status--${run.status}`}
                        >
                          {RUN_STATUS_GLYPHS[run.status]}
                        </span>
                        <span className="sdp__run-history-when">
                          {timeAgo(run.startedAt)}
                        </span>
                        <span className="sdp__muted">
                          {run.iterationCount} iter · {formatEstimatedCost(run)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            </>
          )}
          {/* ── Pipeline progress ────────────────────────────────────── */}
          {isPipeline && progress && (
            <>
              <hr className="sdp__divider" />
              <section className="sdp__section">
                <h3 className="sdp__section-title">Pipeline Progress</h3>
                <PipelineProgressPanel progress={progress} />
              </section>
            </>
          )}
        </div>
      )}
    </SlidePanel>
  );
}
