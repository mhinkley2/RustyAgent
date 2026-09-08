// @vitest-environment node
import { describe, expect, it } from "vitest";

import { matchesSearch, searchTerms } from "./search";
import type { Story } from "../../types/board";

function story(title: string, description?: string): Story {
  return {
    id: title,
    key: `#${title}`,
    title,
    status: "backlog",
    priority: "medium",
    type: "task",
    requiresApproval: false,
    trackHistory: true,
    sortOrder: 0,
    labels: [],
    description,
    createdAt: new Date("2026-04-13T00:00:00Z"),
    updatedAt: new Date("2026-04-13T00:00:00Z"),
  };
}

describe("searchTerms", () => {
  it("drops the whitespace a trailing keystroke leaves behind", () => {
    expect(searchTerms("  cap   diff ")).toEqual(["cap", "diff"]);
  });

  it("treats a blank query as no terms at all", () => {
    expect(searchTerms("   ")).toEqual([]);
  });
});

describe("matchesSearch", () => {
  it("matches an empty query against everything, so an empty box filters nothing", () => {
    expect(matchesSearch(story("Anything"), "")).toBe(true);
    expect(matchesSearch(story("Anything"), "   ")).toBe(true);
  });

  it("matches the title", () => {
    expect(matchesSearch(story("Cap get_run_diff"), "run_diff")).toBe(true);
  });

  // The stories on this board name the files they touch in the body, which is
  // how you find the one you half-remember.
  it("matches the description, not only the title", () => {
    const s = story("Something else entirely", "Touches commands/src/runs.rs:266");
    expect(matchesSearch(s, "runs.rs")).toBe(true);
  });

  it("ignores case on both sides", () => {
    expect(matchesSearch(story("Cap GET_RUN_DIFF"), "get_run_diff")).toBe(true);
    expect(matchesSearch(story("cap get_run_diff"), "GET_RUN_DIFF")).toBe(true);
  });

  // Every term has to hit, or typing more would widen the result set.
  it("requires all terms, across title and description together", () => {
    const s = story("Cap get_run_diff", "on the board MCP surface");
    expect(matchesSearch(s, "cap surface")).toBe(true);
    expect(matchesSearch(s, "cap absent")).toBe(false);
  });

  it("does not fall over on a story with no description", () => {
    expect(matchesSearch(story("Titled only"), "titled")).toBe(true);
    expect(matchesSearch(story("Titled only"), "undefined")).toBe(false);
  });
});
