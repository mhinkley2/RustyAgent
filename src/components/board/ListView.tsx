import { useState } from "react";
import type { Story, StoryPriority } from "../../types/board";
import { ConfirmDialog } from "../forms";
import type { AgentProfile } from "../../types/agent";
import { AgentPicker } from "./AgentPicker";

const PRIORITY_COLORS: Record<StoryPriority, string> = {
  critical: "var(--error)",
  high:     "var(--warning)",
  medium:   "var(--info)",
  low:      "var(--text-muted)",
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
  if (d < 30)  return `${d}d ago`;
  return "a while ago";
}

// ---------------------------------------------------------------------------
// BulkActionBar
// ---------------------------------------------------------------------------

interface BulkActionBarProps {
  count: number;
  onClear: () => void;
  onDelete: () => void;
  agents?: AgentProfile[];
  onAssign?: (agentId: string | null) => Promise<void> | void;
}

function BulkActionBar({ count, onClear, onDelete, agents, onAssign }: BulkActionBarProps) {
  return (
    <div className="bulk-bar" role="toolbar" aria-label="Bulk actions">
      <span className="bulk-bar__count">{count} selected</span>
      <div className="bulk-bar__actions">
        {agents && onAssign && (
          <AgentPicker
            className="bulk-bar__assign"
            agents={agents}
            // Always empty: this is a command, not a field. The selection can
            // hold several different assignees, so there is no one value it
            // could be showing.
            value={null}
            unassignedLabel={`Assign ${count}…`}
            ariaLabel={`Assign an agent to ${count} selected stories`}
            onChange={(agentId) => { void onAssign(agentId); }}
          />
        )}
        <button className="btn btn--destructive btn--sm" onClick={onDelete}>Delete</button>
        <button className="btn btn--ghost btn--sm" onClick={onClear}>Clear selection</button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ListView
// ---------------------------------------------------------------------------

interface ListViewProps {
  stories: Story[];
  onSelect: (story: Story) => void;
  onDeleteStories?: (ids: string[]) => Promise<void>;
  agents?: AgentProfile[];
  onAssignStories?: (ids: string[], agentId: string | null) => Promise<void>;
}

export function ListView({
  stories,
  onSelect,
  onDeleteStories,
  agents,
  onAssignStories,
}: ListViewProps) {
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmDelete, setConfirmDelete] = useState(false);

  const toggleAll = () => {
    if (selected.size === stories.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(stories.map(s => s.id)));
    }
  };

  const toggleOne = (id: string) => {
    setSelected(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const allChecked = selected.size === stories.length && stories.length > 0;
  const indeterminate = selected.size > 0 && selected.size < stories.length;

  return (
    <div className="list-view">
      <table className="data-table list-view__table" role="grid">
        <thead>
          <tr>
            <th className="list-view__check-col">
              <input
                type="checkbox"
                checked={allChecked}
                ref={el => { if (el) el.indeterminate = indeterminate; }}
                onChange={toggleAll}
                aria-label="Select all"
              />
            </th>
            <th>Title</th>
            <th>Assignee</th>
            <th>Priority</th>
            <th>Type</th>
            <th>Updated</th>
          </tr>
        </thead>
        <tbody>
          {stories.map(story => (
            <tr
              key={story.id}
              className={`list-view__row${selected.has(story.id) ? " list-view__row--selected" : ""}`}
              onClick={(e) => {
                // Don't open detail panel if clicking the checkbox
                const target = e.target as HTMLElement;
                if (target.tagName === "INPUT") return;
                onSelect(story);
              }}
            >
              <td className="list-view__check-col" onClick={e => e.stopPropagation()}>
                <input
                  type="checkbox"
                  checked={selected.has(story.id)}
                  onChange={() => toggleOne(story.id)}
                  aria-label={`Select ${story.key}`}
                />
              </td>
              <td>
                <span className="list-view__title">{story.title}</span>
                <span className="list-view__key">{story.key}</span>
              </td>
              <td className="list-view__muted">{story.assignee ?? "Unassigned"}</td>
              <td>
                <span className="story-priority-dot" style={{ background: PRIORITY_COLORS[story.priority] }} />
                <span className="list-view__muted" style={{ marginLeft: 6 }}>
                  {story.priority.charAt(0).toUpperCase() + story.priority.slice(1)}
                </span>
              </td>
              <td>
                <span className={`list-view__type-badge list-view__type-badge--${story.type}`}>
                  {story.type}
                </span>
              </td>
              <td className="list-view__muted">{timeAgo(story.updatedAt)}</td>
            </tr>
          ))}
        </tbody>
      </table>

      {selected.size > 0 && (
        <BulkActionBar
          count={selected.size}
          onClear={() => setSelected(new Set())}
          onDelete={() => setConfirmDelete(true)}
          agents={agents}
          onAssign={
            onAssignStories &&
            (async (agentId) => {
              const ids = [...selected];
              // Cleared before the write, like Delete: leaving rows selected
              // after a bulk action invites a second, accidental one.
              setSelected(new Set());
              await onAssignStories(ids, agentId);
            })
          }
        />
      )}

      <ConfirmDialog
        open={confirmDelete}
        title={`Delete ${selected.size} stor${selected.size === 1 ? "y" : "ies"}?`}
        body="This will permanently delete the selected stories and all their runs. This cannot be undone."
        confirmLabel="Delete"
        onClose={() => setConfirmDelete(false)}
        onConfirm={async () => {
          setConfirmDelete(false);
          const ids = [...selected];
          setSelected(new Set());
          await onDeleteStories?.(ids);
        }}
      />
    </div>
  );
}
