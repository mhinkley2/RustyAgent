import { describe, expect, it } from "vitest";

import {
  UNASSIGNED,
  agentName,
  assignmentInput,
  hasActiveRun,
  pickerOptions,
  runProfileId,
} from "./assignment";
import type { AgentProfile } from "../../types/agent";
import type { Story } from "../../types/board";

function agent(id: string, name: string): AgentProfile {
  return {
    id,
    name,
    description: null,
    system_prompt: "",
    provider: "anthropic",
    model: "claude-opus-5",
    context_strategy: "recent",
    persistent_memory: false,
    max_input_tokens: null,
    max_output_tokens: null,
    run_mode: "manual",
    cron_expression: null,
    continuous_poll_interval_secs: 60,
    max_iterations: 10,
    max_retries: 2,
    scope: "global",
    toml_path: null,
    created_at: "2026-08-31T00:00:00.000Z",
    updated_at: "2026-08-31T00:00:00.000Z",
  };
}

function story(overrides: Partial<Story> = {}): Story {
  return {
    id: "task-1",
    key: "RA-1",
    title: "Migrate the database",
    status: "ready",
    priority: "high",
    type: "task",
    labels: [],
    requiresApproval: false,
    trackHistory: true,
    sortOrder: 0,
    createdAt: new Date("2026-08-31T00:00:00Z"),
    updatedAt: new Date("2026-08-31T00:00:00Z"),
    ...overrides,
  };
}

const AGENTS = [agent("a1", "Agent One"), agent("a2", "Agent Two")];

describe("assignmentInput", () => {
  it("sets the chosen agent", () => {
    expect(assignmentInput("a1")).toEqual({ assigned_agent_id: "a1" });
  });

  it("clears with an empty string, never by omitting the field", () => {
    // `update_story` reads absent as "keep the current assignee", so sending
    // `undefined` would be a no-op that looked like a save.
    const input = assignmentInput(null);

    expect(input).toEqual({ assigned_agent_id: UNASSIGNED });
    expect("assigned_agent_id" in input).toBe(true);
    expect(input.assigned_agent_id).not.toBeUndefined();
  });
});

describe("pickerOptions", () => {
  it("offers every profile", () => {
    expect(pickerOptions(AGENTS, null)).toEqual([
      { id: "a1", label: "Agent One" },
      { id: "a2", label: "Agent Two" },
    ]);
  });

  it("keeps an assignment whose profile is missing", () => {
    // Without its own option the select falls back to the first one, so the
    // story would look assigned to Agent One and the next change would
    // silently reassign it.
    const options = pickerOptions(AGENTS, "deleted-profile-id");

    expect(options[0]).toEqual({ id: "deleted-profile-id", label: "deleted- (unavailable)" });
    expect(options).toHaveLength(3);
  });

  it("adds nothing extra while the profiles are still loading and nothing is assigned", () => {
    expect(pickerOptions([], null)).toEqual([]);
  });

  it("keeps the assignment while the profiles are still loading", () => {
    expect(pickerOptions([], "a1")).toHaveLength(1);
  });
});

describe("runProfileId", () => {
  it("uses the story's assignee when there is no override", () => {
    expect(runProfileId(story({ assignedAgentId: "a1" }), null)).toBe("a1");
  });

  it("prefers the one-off choice", () => {
    expect(runProfileId(story({ assignedAgentId: "a1" }), "a2")).toBe("a2");
  });

  it("lets an unassigned story run under a one-off choice", () => {
    // This is the whole point of "run with": no assignment, but still runnable.
    expect(runProfileId(story(), "a2")).toBe("a2");
  });

  it("is null when there is neither", () => {
    expect(runProfileId(story(), null)).toBeNull();
  });
});

describe("hasActiveRun", () => {
  const run = (status: "running" | "done" | "failed") => ({
    id: "run-1",
    status,
    startedAt: new Date("2026-08-31T00:00:00Z"),
    finishedAt: undefined,
    iterationCount: 1,
    inputTokens: 0,
    outputTokens: 0,
    estimatedCostUsd: 0,
  });

  it("is true only while the newest run is going", () => {
    expect(hasActiveRun(story({ latestRun: run("running") }))).toBe(true);
    expect(hasActiveRun(story({ latestRun: run("done") }))).toBe(false);
    expect(hasActiveRun(story({ latestRun: run("failed") }))).toBe(false);
    expect(hasActiveRun(story())).toBe(false);
  });
});

describe("agentName", () => {
  it("resolves a name", () => {
    expect(agentName(AGENTS, "a2")).toBe("Agent Two");
  });

  it("is null for nobody and for an unknown id", () => {
    expect(agentName(AGENTS, null)).toBeNull();
    expect(agentName(AGENTS, "gone")).toBeNull();
  });
});
