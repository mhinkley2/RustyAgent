import { describe, expect, it } from "vitest";

import { attentionByStory, attentionCount, attentionLabel } from "./attention";
import type { HumanRequest, ApprovalRequest } from "../../types/human";

function human(overrides: Partial<HumanRequest> = {}): HumanRequest {
  return {
    id: "hr-1",
    storyId: "human-1",
    taskStoryId: "task-1",
    storyTitle: "Which database?",
    runId: "run-1",
    question: "Postgres or SQLite?",
    status: "ready",
    createdAt: "2026-08-31T00:00:00.000Z",
    ...overrides,
  };
}

function approval(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "ap-1",
    runId: "run-1",
    storyId: "task-1",
    storyTitle: "Migrate the database",
    toolName: "file_write",
    toolInput: '{"path":"a.txt"}',
    status: "pending",
    createdAt: "2026-08-31T00:00:00.000Z",
    ...overrides,
  };
}

describe("attentionByStory", () => {
  it("marks nothing when nothing is pending", () => {
    expect(attentionByStory([], []).size).toBe(0);
  });

  it("puts an input request on the task story, not the human story", () => {
    const byStory = attentionByStory([human()], []);

    expect([...byStory.keys()]).toEqual(["task-1"]);
    expect(byStory.get("task-1")).toMatchObject({
      inputs: 1,
      approvals: 0,
      requestId: "hr-1",
      kind: "input",
    });
    expect(
      byStory.has("human-1"),
      "the human story is filtered out of the board — marking it marks nothing",
    ).toBe(false);
  });

  it("puts an approval on the story whose run wants the tool call", () => {
    const byStory = attentionByStory([], [approval()]);

    expect(byStory.get("task-1")).toMatchObject({
      approvals: 1,
      inputs: 0,
      requestId: "ap-1",
      kind: "approval",
    });
  });

  it("skips a request that has no story to mark", () => {
    const byStory = attentionByStory(
      [human({ taskStoryId: null })],
      [approval({ storyId: null })],
    );

    expect(byStory.size).toBe(0);
  });

  it("counts several requests against the same story", () => {
    const byStory = attentionByStory(
      [human({ id: "hr-1" }), human({ id: "hr-2" })],
      [approval({ id: "ap-1" }), approval({ id: "ap-2" })],
    );

    expect(byStory.get("task-1")).toMatchObject({ inputs: 2, approvals: 2 });
    expect(attentionCount(byStory.get("task-1")!)).toBe(4);
  });

  it("opens the input dialog when a story has both kinds pending", () => {
    // Ordering matters: the approval is seen first, so this pins that inputs
    // take the click back rather than whichever arrived first winning.
    const byStory = attentionByStory([human({ id: "hr-9" })], [approval({ id: "ap-9" })]);

    expect(byStory.get("task-1")).toMatchObject({
      requestId: "hr-9",
      kind: "input",
      inputs: 1,
      approvals: 1,
    });
  });

  it("keeps stories separate", () => {
    const byStory = attentionByStory(
      [human({ id: "hr-1", taskStoryId: "task-1" })],
      [approval({ id: "ap-2", storyId: "task-2" })],
    );

    expect(byStory.get("task-1")).toMatchObject({ kind: "input", inputs: 1, approvals: 0 });
    expect(byStory.get("task-2")).toMatchObject({ kind: "approval", inputs: 0, approvals: 1 });
  });

  it("does not mutate a shared entry across stories", () => {
    const byStory = attentionByStory(
      [human({ id: "hr-1", taskStoryId: "task-1" }), human({ id: "hr-2", taskStoryId: "task-2" })],
      [],
    );

    expect(byStory.get("task-1")!.inputs).toBe(1);
    expect(byStory.get("task-2")!.inputs).toBe(1);
  });
});

describe("attentionLabel", () => {
  it("names only the kinds that are actually pending", () => {
    const inputsOnly = attentionByStory([human()], []).get("task-1")!;
    expect(attentionLabel(inputsOnly)).toBe("Waiting on you — 1 question");

    const approvalsOnly = attentionByStory([], [approval()]).get("task-1")!;
    expect(attentionLabel(approvalsOnly)).toBe("Waiting on you — 1 approval");
  });

  it("pluralises and joins both kinds", () => {
    const both = attentionByStory(
      [human({ id: "a" }), human({ id: "b" })],
      [approval({ id: "c" }), approval({ id: "d" })],
    ).get("task-1")!;

    expect(attentionLabel(both)).toBe("Waiting on you — 2 questions, 2 approvals");
  });
});
