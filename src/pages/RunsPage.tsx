import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { Trash2, ChevronDown } from "lucide-react";
import { PageHeader } from "../components/board/PageHeader";
import { RunDetailPanel } from "../components/runs/RunDetailPanel";
import { useRuns } from "../hooks/useRuns";
import { useAgents } from "../hooks/useAgents";
import type { StoryRun, RunFilters, RunStatus } from "../types/runs";
import {
  formatEstimatedCost,
  formatTokens,
  formatDuration,
  totalTokens,
  RUN_STATUS_LABELS,
} from "../types/runs";

// ---------------------------------------------------------------------------
// Status filter options
// ---------------------------------------------------------------------------

const STATUS_OPTIONS: { label: string; value: RunStatus | "" }[] = [
  { label: "All statuses", value: "" },
  { label: "Running", value: "running" },
  { label: "Done", value: "done" },
  { label: "Failed", value: "failed" },
  { label: "Cancelled", value: "cancelled" },
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function StatusBadge({ status }: { status: RunStatus }) {
  return (
    <span className={`runs-status-badge runs-status-badge--${status}`}>
      {RUN_STATUS_LABELS[status]}
    </span>
  );
}

function timeAgo(date: Date): string {
  const s = Math.round((Date.now() - date.getTime()) / 1000);
  if (s < 60) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

function parseStatusParam(value: string | null): RunStatus | undefined {
  if (value === "running" || value === "done" || value === "failed" || value === "cancelled") {
    return value;
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// RunsPage
// ---------------------------------------------------------------------------

export default function RunsPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const [filters, setFilters] = useState<RunFilters>({});
  const [selectedRun, setSelectedRun] = useState<StoryRun | null>(null);
  const [storySearch, setStorySearch] = useState("");

  const { runs, loading, error, deleteRun, refresh } = useRuns(filters);
  const { profiles } = useAgents();

  // Apply client-side story title search (server filter only has storyId)
  const visible = storySearch.trim()
    ? runs.filter(r =>
        (r.storyTitle ?? "").toLowerCase().includes(storySearch.toLowerCase())
      )
    : runs;

  useEffect(() => {
    const runId = searchParams.get("runId");
    if (!runId || runs.length === 0) return;
    const matched = runs.find((r) => r.id === runId);
    if (!matched) return;
    setSelectedRun(matched);
    const next = new URLSearchParams(searchParams);
    next.delete("runId");
    setSearchParams(next, { replace: true });
  }, [runs, searchParams, setSearchParams]);

  useEffect(() => {
    const statusFromUrl = parseStatusParam(searchParams.get("status"));
    if (filters.status === statusFromUrl) return;
    setFilters((f) => ({ ...f, status: statusFromUrl }));
  }, [filters.status, searchParams]);

  function updateStatus(value: string) {
    setFilters(f => ({ ...f, status: (value as RunStatus) || undefined }));
  }

  function updateAgent(value: string) {
    setFilters(f => ({ ...f, agentProfileId: value || undefined }));
  }

  async function handleDelete(run: StoryRun) {
    await deleteRun(run.id);
    if (selectedRun?.id === run.id) setSelectedRun(null);
  }

  return (
    <div className="runs-page">
      <PageHeader title="Runs">
        {/* ── Filter bar ─────────────────────────────────────────────── */}
        <div className="runs-filters">
          {/* Story search */}
          <input
            type="search"
            className="runs-filters__search"
            placeholder="Filter by story…"
            value={storySearch}
            onChange={e => setStorySearch(e.target.value)}
          />

          {/* Status filter */}
          <div className="runs-filters__select-wrap">
            <select
              className="runs-filters__select"
              value={filters.status ?? ""}
              onChange={e => updateStatus(e.target.value)}
            >
              {STATUS_OPTIONS.map(o => (
                <option key={o.value} value={o.value}>{o.label}</option>
              ))}
            </select>
            <ChevronDown size={13} className="runs-filters__select-icon" />
          </div>

          {/* Agent filter */}
          <div className="runs-filters__select-wrap">
            <select
              className="runs-filters__select"
              value={filters.agentProfileId ?? ""}
              onChange={e => updateAgent(e.target.value)}
            >
              <option value="">All agents</option>
              {profiles.map(p => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
            <ChevronDown size={13} className="runs-filters__select-icon" />
          </div>
        </div>
      </PageHeader>

      <div className="runs-page__content">
        {/* ── Error state ──────────────────────────────────────────────── */}
        {error && (
          <div className="runs-page__error">
            Failed to load runs: {error}
          </div>
        )}

        {/* ── Loading skeletons ────────────────────────────────────────── */}
        {loading && !error && (
          <div className="runs-table">
            <table className="runs-table__el">
              <thead>
                <TableHead />
              </thead>
              <tbody>
                {Array.from({ length: 5 }).map((_, i) => (
                  <tr key={i} className="runs-table__row runs-table__row--skeleton">
                    {Array.from({ length: 8 }).map((_, j) => (
                      <td key={j} className="runs-table__cell">
                        <span className="skeleton-line" />
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {/* ── Empty state ──────────────────────────────────────────────── */}
        {!loading && !error && visible.length === 0 && (
          <div className="runs-page__empty">
            <p className="runs-page__empty-title">No runs yet.</p>
            <p className="runs-page__empty-sub">
              Start an agent run from the Board page to see history here.
            </p>
          </div>
        )}

        {/* ── Table ───────────────────────────────────────────────────── */}
        {!loading && visible.length > 0 && (
          <div className="runs-table">
            <table className="runs-table__el">
              <thead>
                <TableHead />
              </thead>
              <tbody>
                {visible.map(run => (
                  <tr
                    key={run.id}
                    className={`runs-table__row${selectedRun?.id === run.id ? " runs-table__row--active" : ""}`}
                    onClick={() => setSelectedRun(run)}
                  >
                    <td className="runs-table__cell runs-table__cell--story">
                      {run.storyTitle ?? run.storyId}
                    </td>
                    <td className="runs-table__cell runs-table__cell--agent">
                      {run.agentName ?? run.agentProfileId}
                    </td>
                    <td className="runs-table__cell">
                      <StatusBadge status={run.status} />
                    </td>
                    <td className="runs-table__cell runs-table__cell--num">
                      {run.iterationCount}
                    </td>
                    <td className="runs-table__cell runs-table__cell--num">
                      {formatTokens(totalTokens(run))}
                    </td>
                    <td className="runs-table__cell runs-table__cell--num">
                      {formatEstimatedCost(run)}
                    </td>
                    <td className="runs-table__cell runs-table__cell--num">
                      {formatDuration(run.durationSecs)}
                    </td>
                    <td className="runs-table__cell runs-table__cell--time">
                      {timeAgo(run.startedAt)}
                    </td>
                    <td
                      className="runs-table__cell runs-table__cell--actions"
                      onClick={e => e.stopPropagation()}
                    >
                      <button
                        className="runs-table__action-btn"
                        title="Delete run"
                        onClick={() => handleDelete(run)}
                      >
                        <Trash2 size={13} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* ── Run detail panel ─────────────────────────────────────────── */}
      <RunDetailPanel
        run={selectedRun}
        onClose={() => setSelectedRun(null)}
        onDelete={handleDelete}
        onDecided={() => refresh(filters)}
      />
    </div>
  );
}

function TableHead() {
  return (
    <tr className="runs-table__head">
      <th className="runs-table__th">Story</th>
      <th className="runs-table__th">Agent</th>
      <th className="runs-table__th">Status</th>
      <th className="runs-table__th runs-table__th--num">Iterations</th>
      <th className="runs-table__th runs-table__th--num">Tokens</th>
      <th className="runs-table__th runs-table__th--num">Est. cost</th>
      <th className="runs-table__th runs-table__th--num">Duration</th>
      <th className="runs-table__th">Started</th>
      <th className="runs-table__th" />
    </tr>
  );
}
