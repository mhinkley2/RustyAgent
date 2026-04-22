import { invoke } from "@tauri-apps/api/core";
import { Bot, Cpu, Settings2, Play, Square, Clock } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import type { AgentProfile } from "../../types/agent";
import { PROVIDER_LABELS } from "../../types/agent";
import { useAgentStatus } from "../../hooks/useAgentStatus";

interface RawRun {
  id: string;
  story_id: string;
  story_title: string | null;
  agent_profile_id: string;
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

interface AgentActivity {
  runId: string | null;
  storyTitle: string | null;
  lastAction: string | null;
  recentFailure: string | null;
}

const POLL_INTERVAL_MS = 4000;

function summarizeAction(event: RawEvent | null): string {
  if (!event) return "Working on story tasks";
  if (event.event_type === "tool_call") {
    return event.tool_name ? `Calling ${event.tool_name}` : "Calling a tool";
  }
  if (event.event_type === "tool_result") {
    if (event.is_error) return event.tool_name ? `${event.tool_name} error` : "Tool error";
    return event.tool_name ? `${event.tool_name} completed` : "Tool completed";
  }
  if (event.event_type === "approval_request") return "Waiting for approval";
  if (event.event_type === "approval_response") return "Approval decision received";
  if (event.event_type === "error") return event.content?.trim() || "Run failed";
  if (event.event_type === "thought") return "Planning next step";
  if (event.event_type === "message") {
    if (event.role === "assistant") return "Drafting response";
    if (event.role === "tool") return "Processing tool output";
    return "Processing message";
  }
  return "Updating run";
}

function useAgentCardActivity(profiles: AgentProfile[]): Record<string, AgentActivity> {
  const [activity, setActivity] = useState<Record<string, AgentActivity>>({});
  const profileIds = useMemo(() => profiles.map((p) => p.id), [profiles]);

  useEffect(() => {
    let disposed = false;

    async function refresh() {
      try {
        const runs = await invoke<RawRun[]>("get_runs", { filters: null });
        if (disposed) return;
        const sorted = [...runs].sort(
          (a, b) => new Date(b.started_at).getTime() - new Date(a.started_at).getTime(),
        );

        const latestByProfile = new Map<string, RawRun>();
        const activeByProfile = new Map<string, RawRun>();
        for (const run of sorted) {
          if (!latestByProfile.has(run.agent_profile_id)) {
            latestByProfile.set(run.agent_profile_id, run);
          }
          if (run.status === "running" && !activeByProfile.has(run.agent_profile_id)) {
            activeByProfile.set(run.agent_profile_id, run);
          }
        }

        const eventRunIds = new Set<string>();
        for (const run of activeByProfile.values()) {
          eventRunIds.add(run.id);
        }
        for (const run of latestByProfile.values()) {
          if (run.status === "failed") eventRunIds.add(run.id);
        }

        const eventPairs = await Promise.all(
          Array.from(eventRunIds).map(async (runId) => {
            try {
              const events = await invoke<RawEvent[]>("get_run_events", { runId });
              return [runId, events.length > 0 ? events[events.length - 1] : null] as const;
            } catch {
              return [runId, null] as const;
            }
          }),
        );
        if (disposed) return;
        const eventByRunId: Record<string, RawEvent | null> = {};
        for (const [runId, evt] of eventPairs) {
          eventByRunId[runId] = evt;
        }

        const next: Record<string, AgentActivity> = {};
        for (const profileId of profileIds) {
          const active = activeByProfile.get(profileId) ?? null;
          const latest = latestByProfile.get(profileId) ?? null;
          const activeEvent = active ? eventByRunId[active.id] ?? null : null;
          const failedEvent = latest && latest.status === "failed" ? eventByRunId[latest.id] ?? null : null;

          next[profileId] = {
            runId: active?.id ?? latest?.id ?? null,
            storyTitle: active?.story_title ?? null,
            lastAction: active ? summarizeAction(activeEvent) : null,
            recentFailure:
              !active && latest?.status === "failed"
                ? summarizeAction(failedEvent)
                : null,
          };
        }
        setActivity(next);
      } catch {
        // Non-critical UI polling.
      }
    }

    void refresh();
    const timer = setInterval(() => {
      void refresh();
    }, POLL_INTERVAL_MS);

    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, [profileIds]);

  return activity;
}

// ---------------------------------------------------------------------------
// AgentCard
// ---------------------------------------------------------------------------

interface AgentCardProps {
  profile: AgentProfile;
  activity: AgentActivity | null;
  onEdit: (profile: AgentProfile) => void;
  onDelete: (profile: AgentProfile) => void;
}

function AgentCard({ profile, activity, onEdit, onDelete }: AgentCardProps) {
  const navigate = useNavigate();
  const providerLabel = PROVIDER_LABELS[profile.provider as keyof typeof PROVIDER_LABELS] ?? profile.provider;
  const { status, startContinuous, stopScheduler } = useAgentStatus(profile.id);

  const runtimeLabel = () => {
    if (!status || status.state === "idle") return null;
    if (status.schedulerMode === "scheduled" && status.state === "checking_for_work") {
      if (status.nextRunAt) {
        const d = new Date(status.nextRunAt);
        return `Next: ${d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
      }
      return "Scheduled";
    }
    if (status.state === "failed") {
      return status.failureSummary ?? status.stateLabel;
    }
    return status.stateLabel;
  };

  const isActive = status ? status.state !== "idle" : false;
  const hasRecentFailure = !isActive && Boolean(activity?.recentFailure);

  const activityAction = () => {
    if (status?.state === "waiting_for_approval" || status?.state === "waiting_for_human_input") {
      return status.stateLabel;
    }
    if (isActive) return activity?.lastAction ?? runtimeLabel() ?? "Working on story tasks";
    if (hasRecentFailure) return activity?.recentFailure;
    return null;
  };

  const storyTitle =
    status?.state === "running_story"
      ? (status.activeStoryTitle ?? activity?.storyTitle)
      : null;

  return (
    <div className="agent-card">
      <div className="agent-card__header">
        <div className={`agent-card__icon${isActive ? " agent-card__icon--active" : ""}`}>
          <Bot size={20} />
        </div>
        <div className="agent-card__meta">
          <span className="agent-card__name">{profile.name}</span>
          <span className="agent-card__sub">{providerLabel} · {profile.model}</span>
        </div>
        <span className={`agent-card__scope-badge agent-card__scope-badge--${profile.scope}`}>
          {profile.scope === "workspace" ? "Workspace" : "Global"}
        </span>
        <button
          className="agent-card__edit-btn"
          onClick={() => onEdit(profile)}
          aria-label={`Edit ${profile.name}`}
        >
          <Settings2 size={14} />
        </button>
      </div>

      {profile.description && (
        <p className="agent-card__description">{profile.description}</p>
      )}

      {(storyTitle || activityAction()) && (
        <div className="agent-card__activity" aria-live="polite">
          {storyTitle && (
            <p className="agent-card__activity-story" title={storyTitle}>
              {storyTitle}
            </p>
          )}
          {activityAction() && (
            <p
              className={[
                "agent-card__activity-action",
                hasRecentFailure ? "agent-card__activity-action--error" : "",
              ].join(" ")}
              title={activityAction() ?? ""}
            >
              {activityAction()}
            </p>
          )}
        </div>
      )}

      <div className="agent-card__footer">
        {/* Runtime mode badge */}
        <span className={`agent-card__badge agent-card__badge--mode-${profile.run_mode}`}>
          {profile.run_mode === "manual" ? "Manual" :
           profile.run_mode === "continuous" ? "Continuous" : "Scheduled"}
        </span>

        {/* Live status indicator */}
        {status && status.state !== "idle" && (
          <span className={`agent-card__runtime-status agent-card__runtime-status--${status.state}`}>
            {status.state === "running_story" && <span className="agent-card__pulse" />}
            {status.schedulerMode === "scheduled" && status.state !== "running_story" && <Clock size={10} />}
            {runtimeLabel()}
          </span>
        )}

        <span className="agent-card__badge">
          <Cpu size={10} />
          &nbsp;{profile.max_iterations} iter
        </span>

        {activity?.runId && (isActive || hasRecentFailure) && (
          <button
            className="agent-card__view-run"
            onClick={() => navigate(`/runs?runId=${activity.runId}`)}
            aria-label={`View run for ${profile.name}`}
          >
            View run
          </button>
        )}

        {/* Continuous mode toggle (only for continuous-mode profiles) */}
        {profile.run_mode === "continuous" && (
          <button
            className={`agent-card__toggle-btn${isActive ? " agent-card__toggle-btn--stop" : ""}`}
            onClick={async () => {
              if (isActive) {
                await stopScheduler();
              } else {
                await startContinuous();
              }
            }}
            aria-label={isActive ? `Stop continuous mode for ${profile.name}` : `Start continuous mode for ${profile.name}`}
            title={isActive ? "Stop" : "Start continuous polling"}
          >
            {isActive ? <Square size={11} /> : <Play size={11} />}
          </button>
        )}

        <button
          className="agent-card__delete"
          onClick={() => onDelete(profile)}
          aria-label={`Delete ${profile.name}`}
        >
          Delete
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// AgentList
// ---------------------------------------------------------------------------

interface AgentListProps {
  profiles: AgentProfile[];
  onEdit: (profile: AgentProfile) => void;
  onDelete: (profile: AgentProfile) => void;
}

export function AgentList({ profiles, onEdit, onDelete }: AgentListProps) {
  const activityByProfile = useAgentCardActivity(profiles);

  if (profiles.length === 0) {
    return (
      <div className="empty-state">
        <Bot size={40} className="empty-state__icon" />
        <p className="empty-state__title">No agent profiles yet</p>
        <p className="empty-state__body">
          Create a profile to configure an AI agent with a provider, model, and system prompt.
        </p>
      </div>
    );
  }

  return (
    <div className="agent-list">
      {profiles.map(p => (
        <AgentCard
          key={p.id}
          profile={p}
          activity={activityByProfile[p.id] ?? null}
          onEdit={onEdit}
          onDelete={onDelete}
        />
      ))}
    </div>
  );
}
