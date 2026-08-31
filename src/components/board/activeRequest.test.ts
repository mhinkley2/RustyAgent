import { describe, expect, it } from "vitest";

import { activeRequests, pruneDismissed, undismiss } from "./activeRequest";
import type { HumanRequest, ApprovalRequest } from "../../types/human";

function human(id: string): HumanRequest {
  return {
    id,
    storyId: `human-${id}`,
    taskStoryId: "task-1",
    storyTitle: "Which database?",
    runId: "run-1",
    question: "Postgres or SQLite?",
    status: "ready",
    createdAt: "2026-08-31T00:00:00.000Z",
  };
}

function approval(id: string): ApprovalRequest {
  return {
    id,
    runId: "run-1",
    storyId: "task-1",
    storyTitle: "Migrate the database",
    toolName: "file_write",
    toolInput: "{}",
    status: "pending",
    createdAt: "2026-08-31T00:00:00.000Z",
  };
}

const none = new Set<string>();

describe("activeRequests", () => {
  it("shows nothing when nothing is pending", () => {
    expect(activeRequests([], [], none, none, null)).toEqual({
      human: null,
      approval: null,
    });
  });

  it("shows the first request nobody has dismissed", () => {
    const { human: h } = activeRequests(
      [human("hr-1"), human("hr-2")],
      [],
      new Set(["hr-1"]),
      none,
      null,
    );

    expect(h?.id).toBe("hr-2");
  });

  // ── The defect this story is about ─────────────────────────────────────────

  it("shows no input dialog once every question has been dismissed", () => {
    const { human: h } = activeRequests(
      [human("hr-1"), human("hr-2")],
      [],
      new Set(["hr-1", "hr-2"]),
      none,
      null,
    );

    expect(h).toBeNull();
  });

  it("reopens a dismissed question when it is focused", () => {
    // This is the case the old banner button could not reach: with everything
    // dismissed there was no "first non-dismissed" request for it to act on, so
    // the click did nothing and the run stayed blocked with no reachable UI.
    const { human: h } = activeRequests(
      [human("hr-1"), human("hr-2")],
      [],
      new Set(["hr-1", "hr-2"]),
      none,
      "hr-1",
    );

    expect(h?.id).toBe("hr-1");
  });

  it("reopens a dismissed approval when it is focused", () => {
    const { approval: a } = activeRequests(
      [],
      [approval("ap-1")],
      none,
      new Set(["ap-1"]),
      "ap-1",
    );

    expect(a?.id).toBe("ap-1");
  });

  it("focuses a later request over the first non-dismissed one", () => {
    const { human: h } = activeRequests(
      [human("hr-1"), human("hr-2"), human("hr-3")],
      [],
      none,
      none,
      "hr-3",
    );

    expect(h?.id).toBe("hr-3");
  });

  // ── The two dialogs must never be up at once ──────────────────────────────

  it("lets a focused approval suppress the input dialog", () => {
    // The gate renders only when no input dialog does. Without this rule,
    // clicking an approval marker while a question is outstanding opens the
    // question instead — the marker would point at one thing and produce
    // another.
    const { human: h, approval: a } = activeRequests(
      [human("hr-1")],
      [approval("ap-1")],
      none,
      none,
      "ap-1",
    );

    expect(a?.id).toBe("ap-1");
    expect(h).toBeNull();
  });

  it("brings the question back once the focused approval is let go", () => {
    const { human: h } = activeRequests(
      [human("hr-1")],
      [approval("ap-1")],
      none,
      none,
      null,
    );

    expect(h?.id).toBe("hr-1");
  });

  it("ignores a focus on a request that no longer exists", () => {
    const { human: h, approval: a } = activeRequests(
      [human("hr-1")],
      [approval("ap-1")],
      none,
      none,
      "answered-and-gone",
    );

    expect(h?.id).toBe("hr-1");
    expect(a?.id).toBe("ap-1");
  });
});

describe("undismiss", () => {
  it("removes the id", () => {
    expect([...undismiss(new Set(["a", "b"]), "a")]).toEqual(["b"]);
  });

  it("returns the same set when the id was not dismissed", () => {
    const before = new Set(["a"]);
    expect(undismiss(before, "z")).toBe(before);
  });
});

describe("pruneDismissed", () => {
  it("drops dismissals of requests that are gone", () => {
    expect([...pruneDismissed(new Set(["a", "b"]), new Set(["b"]))]).toEqual(["b"]);
  });

  it("returns the same set when every dismissal is still live", () => {
    // A new reference every poll would re-render the board on a 5s timer.
    const before = new Set(["a", "b"]);
    expect(pruneDismissed(before, new Set(["a", "b", "c"]))).toBe(before);
  });
});
