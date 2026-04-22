import { useState } from "react";
import type { StoryPriority, StoryType } from "../../types/board";

// ---------------------------------------------------------------------------
// Filter state type — passed up to parent (BoardPage)
// ---------------------------------------------------------------------------

export interface BoardFilters {
  /** "all" | "mine" | "unassigned" */
  quick: "all" | "mine" | "unassigned";
  priorities: StoryPriority[];
  types: StoryType[];
  labels: string[];
}

export const DEFAULT_FILTERS: BoardFilters = {
  quick: "all",
  priorities: [],
  types: [],
  labels: [],
};

// ---------------------------------------------------------------------------
// FilterBar
// ---------------------------------------------------------------------------

interface FilterBarProps {
  filters: BoardFilters;
  onChange: (next: BoardFilters) => void;
  /** All unique labels present in the story list */
  availableLabels?: string[];
}

const PRIORITY_OPTIONS: { value: StoryPriority; label: string }[] = [
  { value: "critical", label: "Critical" },
  { value: "high",    label: "High" },
  { value: "medium",  label: "Medium" },
  { value: "low",     label: "Low" },
];

const TYPE_OPTIONS: { value: StoryType; label: string }[] = [
  { value: "task",     label: "Task" },
  { value: "human",   label: "Human Input" },
  { value: "pipeline", label: "Pipeline" },
];

function toggleInList<T>(list: T[], value: T): T[] {
  return list.includes(value) ? list.filter(x => x !== value) : [...list, value];
}

function activeCount(f: BoardFilters): number {
  return (f.quick !== "all" ? 1 : 0) + f.priorities.length + f.types.length + f.labels.length;
}

export function FilterBar({ filters, onChange, availableLabels = [] }: FilterBarProps) {
  const count = activeCount(filters);
  const [labelsOpen, setLabelsOpen] = useState(filters.labels.length > 0);

  return (
    <div className="filter-bar" role="toolbar" aria-label="Filter stories">
      {/* ── Quick pills ─────────────────────────────────────────────── */}
      <div className="filter-bar__pills">
        {(["all", "mine", "unassigned"] as const).map(q => (
          <button
            key={q}
            className={`filter-pill${filters.quick === q ? " filter-pill--active" : ""}`}
            onClick={() => onChange({ ...filters, quick: q })}
          >
            {q === "all" ? "All" : q === "mine" ? "Assigned" : "Unassigned"}
          </button>
        ))}
      </div>

      <div className="filter-bar__divider" />

      {/* ── Priority dropdown ───────────────────────────────────────── */}
      <div className="filter-bar__group">
        <span className="filter-bar__group-label">
          Priority{filters.priorities.length > 0 ? ` (${filters.priorities.length})` : ""}
        </span>
        <div className="filter-bar__check-group">
          {PRIORITY_OPTIONS.map(({ value, label }) => (
            <label key={value} className="filter-bar__check-label">
              <input
                type="checkbox"
                checked={filters.priorities.includes(value)}
                onChange={() =>
                  onChange({ ...filters, priorities: toggleInList(filters.priorities, value) })
                }
              />
              {label}
            </label>
          ))}
        </div>
      </div>

      {/* ── Type dropdown ───────────────────────────────────────────── */}
      <div className="filter-bar__group">
        <span className="filter-bar__group-label">
          Type{filters.types.length > 0 ? ` (${filters.types.length})` : ""}
        </span>
        <div className="filter-bar__check-group">
          {TYPE_OPTIONS.map(({ value, label }) => (
            <label key={value} className="filter-bar__check-label">
              <input
                type="checkbox"
                checked={filters.types.includes(value)}
                onChange={() =>
                  onChange({ ...filters, types: toggleInList(filters.types, value) })
                }
              />
              {label}
            </label>
          ))}
        </div>
      </div>

      {/* ── Labels ─────────────────────────────────────────────────── */}
      {availableLabels.length > 0 && (
        <div className="filter-bar__group">
          <div className="filter-bar__group-head">
            <span className="filter-bar__group-label">
              Label{filters.labels.length > 0 ? ` (${filters.labels.length})` : ""}
            </span>
            <button
              type="button"
              className="filter-bar__group-toggle"
              aria-expanded={labelsOpen}
              onClick={() => setLabelsOpen(v => !v)}
            >
              {labelsOpen ? "Hide" : `Show ${availableLabels.length}`}
            </button>
          </div>
          {labelsOpen && (
            <div className="filter-bar__check-group filter-bar__check-group--labels">
              {availableLabels.map(label => (
                <label key={label} className="filter-bar__check-label">
                  <input
                    type="checkbox"
                    checked={filters.labels.includes(label)}
                    onChange={() =>
                      onChange({ ...filters, labels: toggleInList(filters.labels, label) })
                    }
                  />
                  {label}
                </label>
              ))}
            </div>
          )}
        </div>
      )}

      {/* ── Active count & clear ────────────────────────────────────── */}
      {count > 0 && (
        <button
          className="filter-bar__clear"
          onClick={() =>
            onChange({ quick: "all", priorities: [], types: [], labels: [] })
          }
        >
          Clear {count} filter{count !== 1 ? "s" : ""}
        </button>
      )}
    </div>
  );
}
