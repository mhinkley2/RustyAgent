import { useEffect, useRef, useState } from "react";
import { Search, X } from "lucide-react";
import type { StoryPriority, StoryStatus, StoryType } from "../../types/board";
import { KANBAN_COLUMNS } from "../../types/board";

// ---------------------------------------------------------------------------
// Filter state type — passed up to parent (BoardPage)
// ---------------------------------------------------------------------------

export interface BoardFilters {
  /** "all" | "mine" | "unassigned" */
  quick: "all" | "mine" | "unassigned";
  priorities: StoryPriority[];
  types: StoryType[];
  labels: string[];
  /** Free text over title and description. Empty means no text filter. */
  search: string;
  /**
   * Statuses to show. Empty means all of them.
   *
   * Most useful in List view, which has no other way to ask for one status.
   * The Kanban applies it by drawing only those columns, so the filter and the
   * column layout say the same thing rather than leaving five columns of
   * "Nothing here".
   */
  statuses: StoryStatus[];
}

export const DEFAULT_FILTERS: BoardFilters = {
  quick: "all",
  priorities: [],
  types: [],
  labels: [],
  search: "",
  statuses: [],
};

/** How long typing settles before the board re-filters. */
const SEARCH_DEBOUNCE_MS = 200;

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

export function activeCount(f: BoardFilters): number {
  return (
    (f.quick !== "all" ? 1 : 0) +
    f.priorities.length +
    f.types.length +
    f.labels.length +
    f.statuses.length +
    (f.search.trim() ? 1 : 0)
  );
}

export function FilterBar({ filters, onChange, availableLabels = [] }: FilterBarProps) {
  const count = activeCount(filters);
  const [labelsOpen, setLabelsOpen] = useState(filters.labels.length > 0);

  /**
   * What the box shows, held locally so a keystroke is never waiting on a
   * re-filter of the whole board.
   *
   * Seeded from the prop and resynced when the prop changes to something the
   * box is not already showing — that is what makes "Clear filters" empty the
   * input rather than leaving stale text over an unfiltered board.
   */
  const [draft, setDraft] = useState(filters.search);
  useEffect(() => {
    setDraft(current => (current === filters.search ? current : filters.search));
  }, [filters.search]);

  /**
   * The debounce depends on the draft and nothing else.
   *
   * Depending on `filters` and `onChange` directly would clear and restart the
   * timer on every identity change in either — safe only while the parent
   * happens to pass a stable `onChange` and hold `filters` in state. A caller
   * passing an inline arrow, on a page that re-renders on a timer, would get a
   * debounce that never fires and a search box that silently does nothing.
   * Read them through a ref instead, so what restarts the timer is typing.
   */
  const latest = useRef({ filters, onChange });
  latest.current = { filters, onChange };

  useEffect(() => {
    if (draft === latest.current.filters.search) return;
    const timer = setTimeout(() => {
      const { filters: current, onChange: fire } = latest.current;
      fire({ ...current, search: draft });
    }, SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [draft]);

  return (
    <div className="filter-bar" role="toolbar" aria-label="Filter stories">
      {/* ── Search ──────────────────────────────────────────────────── */}
      <div className="filter-bar__search">
        <Search size={13} className="filter-bar__search-icon" aria-hidden />
        <input
          type="search"
          className="filter-bar__search-input"
          placeholder="Search stories…"
          aria-label="Search stories by title and description"
          value={draft}
          onChange={e => setDraft(e.target.value)}
          onKeyDown={e => {
            // Escape clears rather than blurring: the box is inside a toolbar
            // with no other way back to an unfiltered board without reaching
            // for the mouse.
            if (e.key === "Escape") {
              // Stopped here, or the same keypress reaches the window listeners
              // behind `ModalContext` and `SlidePanel` and closes an open story
              // panel as well. One Escape, one thing.
              e.stopPropagation();
              setDraft("");
              onChange({ ...filters, search: "" });
            }
          }}
        />
        {draft && (
          <button
            type="button"
            className="filter-bar__search-clear"
            aria-label="Clear search"
            onClick={() => {
              setDraft("");
              onChange({ ...filters, search: "" });
            }}
          >
            <X size={12} />
          </button>
        )}
      </div>

      <div className="filter-bar__divider" />

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

      {/* ── Status ─────────────────────────────────────────────────── */}
      <div className="filter-bar__group">
        <span className="filter-bar__group-label">
          Status{filters.statuses.length > 0 ? ` (${filters.statuses.length})` : ""}
        </span>
        <div className="filter-bar__check-group">
          {KANBAN_COLUMNS.map(({ status, label }) => (
            <label key={status} className="filter-bar__check-label">
              <input
                type="checkbox"
                checked={filters.statuses.includes(status)}
                onChange={() =>
                  onChange({ ...filters, statuses: toggleInList(filters.statuses, status) })
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
          onClick={() => {
            // The box is uncontrolled between keystrokes, so clearing the
            // state is not enough — the draft has to go with it or the input
            // keeps showing a query that is no longer filtering anything.
            setDraft("");
            onChange(DEFAULT_FILTERS);
          }}
        >
          Clear {count} filter{count !== 1 ? "s" : ""}
        </button>
      )}
    </div>
  );
}
