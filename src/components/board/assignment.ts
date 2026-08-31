import type { AgentProfile } from "../../types/agent";
import type { Story } from "../../types/board";
import type { UpdateStoryInput } from "../../hooks/useStories";

/**
 * The `<select>` value standing for "no agent".
 *
 * A select's value is always a string, and `""` is the one string that cannot
 * collide with a profile id.
 */
export const UNASSIGNED = "";

export interface AgentOption {
  id: string;
  label: string;
}

/**
 * The options a picker should offer for a story assigned to `value`.
 *
 * If `value` names a profile that is not in `agents` — deleted since the
 * assignment, belonging to another workspace, or simply not loaded yet — it is
 * added as its own option. Without that the select falls back to its first
 * option, so a story would *look* assigned to whichever agent happens to sort
 * first, and the next change would silently reassign it.
 */
export function pickerOptions(agents: AgentProfile[], value: string | null): AgentOption[] {
  const options = agents.map(a => ({ id: a.id, label: a.name }));
  if (value && !agents.some(a => a.id === value)) {
    options.unshift({ id: value, label: `${value.slice(0, 8)} (unavailable)` });
  }
  return options;
}

/**
 * The update that assigns `agentId`, or clears the assignment for `null`.
 *
 * `update_story` reads three states out of one optional field: absent keeps the
 * current assignee, `""` clears it, anything else sets it. So unassigning by
 * sending `undefined` is a no-op that looks like a save — the trap this exists
 * to keep in one place.
 */
export function assignmentInput(agentId: string | null): UpdateStoryInput {
  return { assigned_agent_id: agentId ?? UNASSIGNED };
}

/**
 * Whether a story's newest run is still going.
 *
 * Reassigning during a run is allowed and applies to the *next* run: a run
 * records its profile on its own row when it starts, so nothing about it can
 * change afterwards. The UI says so rather than blocking the edit.
 */
export function hasActiveRun(story: Story): boolean {
  return story.latestRun?.status === "running";
}

/**
 * Which profile a run should use: an explicit one-off choice, else the story's
 * own assignment.
 *
 * The one-off is deliberately not persisted, so "try this agent on that story"
 * does not quietly become "this story belongs to that agent".
 */
export function runProfileId(story: Story, override: string | null): string | null {
  return override ?? story.assignedAgentId ?? null;
}

/** The display name for a profile id, for confirmations and titles. */
export function agentName(agents: AgentProfile[], id: string | null): string | null {
  if (!id) return null;
  return agents.find(a => a.id === id)?.name ?? null;
}
