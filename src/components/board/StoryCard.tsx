/**
 * One card on the board.
 *
 * Lifted out of `KanbanView` when it grew a clickable marker: what the card
 * shows and what happens when you press part of it is a question about the
 * card, and answering it should not require mounting a drag-and-drop board.
 * `KanbanView` is left with the dragging.
 */
import { GitBranch } from "lucide-react";

import type { Story, StoryPriority, StoryLatestRun } from "../../types/board";
import type { RunStatus } from "../../types/runs";
import { RUN_STATUS_LABELS, formatCost } from "../../types/runs";
import { attentionCount, attentionLabel, type StoryAttention } from "./attention";

// ---------------------------------------------------------------------------
// Priority helpers
// ---------------------------------------------------------------------------

const PRIORITY_COLORS: Record<StoryPriority, string> = {
  critical: "var(--error)",
  high:     "var(--warning)",
  medium:   "var(--info)",
  low:      "var(--text-muted)",
};

const PRIORITY_LABELS: Record<StoryPriority, string> = {
  critical: "Critical",
  high:     "High",
  medium:   "Medium",
  low:      "Low",
};

// ---------------------------------------------------------------------------
// Recency formatting
// ---------------------------------------------------------------------------

/**
 * A glyph per finished run state. Mirrors `RUN_STATUS_GLYPHS` in the detail
 * panel; both are small enough that sharing them across a component boundary
 * would cost more indirection than it saves.
 */
const RUN_STATUS_GLYPHS: Record<RunStatus, string> = {
  running:   "⟳",
  done:      "✓",
  failed:    "✗",
  cancelled: "⊘",
};

/**
 * How long an active run has been going, in the compact form a card can hold.
 *
 * The fetched row carries only `started_at`, so elapsed time is computed here.
 * It advances when the board refreshes rather than ticking per second — the
 * board follows the database on its own now, and a card that re-rendered every
 * second across six columns would cost more than the precision is worth.
 */
function elapsedSince(start: Date): string {
  const secs = Math.max(0, Math.round((Date.now() - start.getTime()) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  return `${hours}h ${mins % 60}m`;
}

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

// ---------------------------------------------------------------------------
// StoryCard (pure visual – no native drag)
// ---------------------------------------------------------------------------

interface StoryCardProps {
  story: Story;
  onSelect: (story: Story) => void;
  isDragging?: boolean;
  /** Spread onto the drag-handle element (from useSortable). */
  dragProps?: React.HTMLAttributes<HTMLDivElement>;
  /**
   * This is the story its assigned agent will pick up next.
   *
   * Shown because the ordering alone is not legible when several agents draw
   * from one column: the top card is next for *someone*, but not necessarily
   * for the agent you are thinking about.
   */
  isNextUp?: boolean;
  /** What this story is blocking a person on, if anything. */
  attention?: StoryAttention;
  /** Open the dialog for `attention`'s request. Absent means render no marker. */
  onAttention?: (attention: StoryAttention) => void;
}

export function StoryCard({
  story,
  onSelect,
  isDragging,
  dragProps,
  isNextUp,
  attention,
  onAttention,
}: StoryCardProps) {
  return (
    <div
      className={`story-card story-card--${story.type}${isDragging ? " story-card--dragging" : ""}`}
      onClick={() => onSelect(story)}
      role="button"
      tabIndex={0}
      aria-label={`Story ${story.key}: ${story.title}`}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onSelect(story); }}
      {...dragProps}
    >
      <div className="story-card__top">
        <span
          className="story-card__priority-dot"
          style={{ background: PRIORITY_COLORS[story.priority] }}
          title={PRIORITY_LABELS[story.priority]}
          aria-label={PRIORITY_LABELS[story.priority]}
        />
        <span className="story-card__title">{story.title}</span>
        {story.type === "human" && (
          <span className="story-card__human-badge" title="Waiting for your input">⚑</span>
        )}
        {story.type === "pipeline" && (
          <span className="story-card__pipeline-badge" title="Pipeline">
            <GitBranch size={11} />
          </span>
        )}
      </div>
      <div className="story-card__meta">
        <span className="story-card__assignee">
          {story.assignee ?? "Unassigned"}
        </span>
        {attention && onAttention && (
          <button
            type="button"
            className="story-card__attention"
            title={attentionLabel(attention)}
            aria-label={`${attentionLabel(attention)} — open`}
            // The card itself opens the detail panel. Reaching the marker has
            // to mean the dialog, not the panel behind it.
            onClick={(e) => { e.stopPropagation(); onAttention(attention); }}
            onKeyDown={(e) => e.stopPropagation()}
            // dnd-kit's listeners live on the card. Without this the press that
            // opens a dialog also arms a drag.
            onPointerDown={(e) => e.stopPropagation()}
          >
            waiting on you
            {attentionCount(attention) > 1 && ` · ${attentionCount(attention)}`}
          </button>
        )}
        {isNextUp && (
          <span
            className="story-card__next-up"
            title={`Next up for ${story.assignee ?? "its agent"}`}
          >
            next up
          </span>
        )}
        <RunIndicator run={story.latestRun} />
      </div>
      <div className="story-card__footer">
        <span className="story-card__key">{story.key}</span>
        <span className="story-card__sep">·</span>
        <span className="story-card__type">{story.type}</span>
        <span className="story-card__sep">·</span>
        <span className="story-card__time">{timeAgo(story.updatedAt)}</span>
      </div>
    </div>
  );
}

/**
 * What a story's most recent run is doing, on the card.
 *
 * This is the difference between the board of an agent-orchestration tool and
 * a generic task tracker: without it you cannot tell a card an agent is
 * working right now from one nobody has touched in a week.
 *
 * A story that has never run renders nothing at all — no empty slot, no
 * placeholder. The meta row is one line and the board shows six columns, so
 * the indicator has to earn its space.
 */
function RunIndicator({ run }: { run?: StoryLatestRun }) {
  if (!run) return null;

  if (run.status === "running") {
    return (
      <span
        className="story-card__run story-card__run--running"
        title={`Running · ${run.iterationCount} iteration${run.iterationCount === 1 ? "" : "s"}`}
      >
        <span className="story-card__run-pulse" aria-hidden="true" />
        {elapsedSince(run.startedAt)}
        {run.iterationCount > 0 && ` · ${run.iterationCount} iter`}
      </span>
    );
  }

  return (
    <span
      className={`story-card__run story-card__run--${run.status}`}
      title={`${RUN_STATUS_LABELS[run.status]} · ${formatCost(run.estimatedCostUsd)}`}
    >
      {RUN_STATUS_GLYPHS[run.status]} {timeAgo(run.finishedAt ?? run.startedAt)}
    </span>
  );
}
