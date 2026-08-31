// @vitest-environment node
import { describe, expect, it } from "vitest";

import { nextUpIds } from "./queue";
import type { Story } from "../../types/board";

function story(id: string, assignedAgentId?: string): Story {
  return {
    id,
    key: `#${id}`,
    title: `Story ${id}`,
    status: "ready",
    priority: "medium",
    type: "task",
    assignedAgentId,
    requiresApproval: false,
    trackHistory: true,
    sortOrder: 0,
    labels: [],
    createdAt: new Date("2026-04-13T00:00:00Z"),
    updatedAt: new Date("2026-04-13T00:00:00Z"),
  };
}

describe("nextUpIds", () => {
  // The scheduler picks the first Ready story *for a given profile*, so two
  // agents drawing from one column each have their own next.
  it("marks one card per agent, not one for the whole column", () => {
    const next = nextUpIds([
      story("a1-first", "agent-1"),
      story("a2-first", "agent-2"),
      story("a1-second", "agent-1"),
    ]);

    expect(next).toEqual(new Set(["a1-first", "a2-first"]));
  });

  // An unassigned story is never picked at all, so calling it "next up" would
  // promise something that will not happen.
  it("never marks an unassigned story", () => {
    const next = nextUpIds([story("nobody"), story("mine", "agent-1")]);

    expect(next.has("nobody")).toBe(false);
    expect(next.has("mine")).toBe(true);
  });

  // "First" means first in the order the column is already in, which is the
  // order the scheduler picks by.
  it("takes the earliest card in the given order", () => {
    const next = nextUpIds([
      story("top", "agent-1"),
      story("middle", "agent-1"),
      story("bottom", "agent-1"),
    ]);

    expect(next).toEqual(new Set(["top"]));
  });

  it("marks nothing in an empty column", () => {
    expect(nextUpIds([])).toEqual(new Set());
  });

  it("marks nothing when no story is assigned", () => {
    expect(nextUpIds([story("a"), story("b")])).toEqual(new Set());
  });
});
