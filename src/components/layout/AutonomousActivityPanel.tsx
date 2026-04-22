import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Activity, Clock3, ExternalLink, RefreshCw } from "lucide-react";
import { useAgentStatuses } from "../../hooks/useAgentStatus";
import { useAgents } from "../../hooks/useAgents";
import type { AgentRuntimeStatus } from "../../types/agent";

interface RawRun {
  id: string;
  story_id: string;
  story_title: string | null;
  agent_profile_id: string;
  agent_name: string | null;
  status: "running" | "done" | "failed" | "cancelled";
  started_at: string;
}

interface RawEvent {
  event_type: string;
  role: string | null;
  content: string | null;
  tool_name: string | null;
  is_error: boolean;
}

interface ActivityRow {
  id: string;
  agentName: string;
  storyTitle: string;
  stateLabel: string;
  elapsedLabel: string;
  lastAction: string;
  runId: string | null;
  isActive: boolean;
}

const POLL_INTERVAL_MS = 4000;

function elapsedLabel(startedAtIso: string | null): string {
  if (!startedAtIso) return "-";
  const startedAt = new Date(startedAtIso).getTime();
  if (Number.isNaN(startedAt)) return "-";
  const secs = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = secs % 60;
  if (mins < 60) return `${mins}m ${remSecs}s`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  return `${hours}h ${remMins}m`;
}

function summarizeAction(event: RawEvent | null, runtime: AgentRuntimeStatus | undefined): string {
  if (runtime?.state === "waiting_for_approval") return "Waiting for approval";
  if (runtime?.state === "waiting_for_human_input") return "Waiting for human input";
  if (!event) {
    if (runtime?.state === "running_story") return "Working on story tasks";
    if (runtime?.state === "checking_for_work") return "Checking for ready work";
    if (runtime?.state === "completed_recently") return "Completed latest run";
    if (runtime?.state === "failed") return runtime.failureSummary ?? "Run failed";
    return "No recent action";
  }

  if (event.event_type === "tool_call") {
    return event.tool_name ? `Using ${event.tool_name}` : "Using a tool";
  }
  if (event.event_type === "tool_result") {
    if (event.is_error) {
      return event.tool_name ? `${event.tool_name} returned an error` : "Tool returned an error";
    }
    return event.tool_name ? `${event.tool_name} completed` : "Tool call completed";
  }
  if (event.event_type === "approval_request") {
    return event.tool_name ? `Approval requested for ${event.tool_name}` : "Approval requested";
  }
  if (event.event_type === "approval_response") {
    return "Approval decision received";
  }
  if (event.event_type === "error") {
    return event.content?.trim() || "Run reported an error";
  }
  if (event.event_type === "thought") {
    return "Planning next step";
  }
  if (event.event_type === "message") {
    if (event.role === "assistant") return "Drafting response";
    if (event.role === "tool") return "Processing tool output";
    return "Processing message";
  }
  return "Updating run";
}

export default function AutonomousActivityPanel() {
  const { statuses } = useAgentStatuses();
  const { profiles } = useAgents();

  const [runs, setRuns] = useState<RawRun[]>([]);
  const [latestEvents, setLatestEvents] = useState<Record<string, RawEvent | null>>({});
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const inFlightRef = useRef(false);

  const profileNameById = useMemo(() => {
    const map: Record<string, string> = {};
    for (const p of profiles) map[p.id] = p.name;
    return map;
  }, [profiles]);

  const refresh = useCallback(async () => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    setRefreshing(true);
    try {
      const runList = await invoke<RawRun[]>("get_runs", { filters: null });
      const sorted = [...runList].sort(
        (a, b) => new Date(b.started_at).getTime() - new Date(a.started_at).getTime(),
      );
      setRuns(sorted);

      const activeRunIds = new Set(
        Object.values(statuses)
          .map((s) => s.activeRunId)
          .filter((id): id is string => Boolean(id)),
      );

      const targetRunIds: string[] = [];
      for (const run of sorted) {
        if (run.status === "running" || activeRunIds.has(run.id)) {
          targetRunIds.push(run.id);
        }
        if (targetRunIds.length >= 10) break;
      }

      const eventEntries = await Promise.all(
        targetRunIds.map(async (runId) => {
          try {
            const events = await invoke<RawEvent[]>("get_run_events", { runId });
            return [runId, events.length > 0 ? events[events.length - 1] : null] as const;
          } catch {
            return [runId, null] as const;
          }
        }),
      );

      setLatestEvents((prev) => {
        const next = { ...prev };
        for (const [runId, evt] of eventEntries) {
          next[runId] = evt;
        }
        return next;
      });
    } finally {
      setLoading(false);
      setRefreshing(false);
      inFlightRef.current = false;
    }
  }, [statuses]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const timer = setInterval(() => {
      void refresh();
    }, POLL_INTERVAL_MS);
    const unlisten = listen("workspace-changed", () => {
      void refresh();
    });

    return () => {
      clearInterval(timer);
      unlisten.then((fn) => fn());
    };
  }, [refresh]);

  const rows = useMemo<ActivityRow[]>(() => {
    const rowsByRunId = new Map<string, ActivityRow>();
    const statusList = Object.values(statuses);

    for (const runtime of statusList) {
      if (runtime.state === "idle") continue;

      const run = runtime.activeRunId ? runs.find((r) => r.id === runtime.activeRunId) : undefined;
      const row: ActivityRow = {
        id: runtime.activeRunId ?? `runtime-${runtime.profileId}`,
        agentName:
          profileNameById[runtime.profileId] ||
          run?.agent_name ||
          runtime.profileId,
        storyTitle: runtime.activeStoryTitle || run?.story_title || "Autonomous task",
        stateLabel: runtime.stateLabel,
        elapsedLabel: elapsedLabel(run?.started_at ?? null),
        lastAction: summarizeAction(
          runtime.activeRunId ? latestEvents[runtime.activeRunId] ?? null : null,
          runtime,
        ),
        runId: runtime.activeRunId,
        isActive: true,
      };

      if (row.runId) {
        rowsByRunId.set(row.runId, row);
      } else {
        rowsByRunId.set(row.id, row);
      }
    }

    const recentRows: ActivityRow[] = [];
    for (const run of runs) {
      if (rowsByRunId.has(run.id)) continue;
      recentRows.push({
        id: run.id,
        agentName: run.agent_name || profileNameById[run.agent_profile_id] || run.agent_profile_id,
        storyTitle: run.story_title || run.story_id,
        stateLabel:
          run.status === "running"
            ? "Running"
            : run.status === "done"
              ? "Completed"
              : run.status === "failed"
                ? "Failed"
                : "Cancelled",
        elapsedLabel: elapsedLabel(run.started_at),
        lastAction: summarizeAction(latestEvents[run.id] ?? null, undefined),
        runId: run.id,
        isActive: run.status === "running",
      });
      if (recentRows.length >= 12) break;
    }

    const activeRows = Array.from(rowsByRunId.values());
    return [...activeRows, ...recentRows].slice(0, 14);
  }, [latestEvents, profileNameById, runs, statuses]);

  const activeCount = rows.filter((r) => r.isActive).length;

  return (
    <section className="activity-panel" aria-label="Autonomous Activity">
      <header className="activity-panel__header">
        <div className="activity-panel__title-wrap">
          <Activity size={14} />
          <h2 className="activity-panel__title">Autonomous Activity</h2>
        </div>
        <span className="activity-panel__meta">
          {loading ? "Loading..." : `${activeCount} active`}
        </span>
      </header>

      {rows.length === 0 && !loading ? (
        <div className="activity-panel__empty">
          <p className="activity-panel__empty-title">No agents are active right now.</p>
          <p className="activity-panel__empty-sub">
            Start a run from Board or enable continuous mode to see live autonomous activity here.
          </p>
        </div>
      ) : (
        <ul className="activity-panel__list" role="list">
          {rows.map((row) => (
            <li key={row.id} className="activity-row">
              <div className="activity-row__head">
                <span className="activity-row__agent">{row.agentName}</span>
                <span className={`activity-row__state${row.isActive ? " activity-row__state--active" : ""}`}>
                  {row.stateLabel}
                </span>
              </div>
              <p className="activity-row__story" title={row.storyTitle}>{row.storyTitle}</p>
              <p className="activity-row__action" title={row.lastAction}>{row.lastAction}</p>
              <div className="activity-row__foot">
                <span className="activity-row__elapsed">
                  <Clock3 size={11} />
                  {row.elapsedLabel}
                </span>
                {row.runId ? (
                  <Link to={`/runs?runId=${row.runId}`} className="activity-row__link" title="Open run details">
                    Run details
                    <ExternalLink size={11} />
                  </Link>
                ) : (
                  <span className="activity-row__link activity-row__link--muted">No run details</span>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}

      {refreshing && rows.length > 0 && (
        <div className="activity-panel__refreshing" aria-live="polite">
          <RefreshCw size={11} className="activity-panel__refresh-icon" />
          Refreshing
        </div>
      )}
    </section>
  );
}