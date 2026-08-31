import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { StoryCard } from "./StoryCard";
import { attentionByStory } from "./attention";
import type { Story } from "../../types/board";
import type { ApprovalRequest, HumanRequest } from "../../types/human";

function makeStory(overrides: Partial<Story> = {}): Story {
  return {
    id: "task-1",
    key: "RA-1",
    title: "Migrate the database",
    status: "in_progress",
    priority: "high",
    type: "task",
    assignee: "Agent One",
    labels: [],
    requiresApproval: false,
    trackHistory: true,
    sortOrder: 0,
    createdAt: new Date("2026-08-31T00:00:00Z"),
    updatedAt: new Date("2026-08-31T00:00:00Z"),
    ...overrides,
  };
}

const humanRequest: HumanRequest = {
  id: "hr-1",
  storyId: "human-1",
  taskStoryId: "task-1",
  storyTitle: "Which database?",
  runId: "run-1",
  question: "Postgres or SQLite?",
  status: "ready",
  createdAt: "2026-08-31T00:00:00.000Z",
};

const approvalRequest: ApprovalRequest = {
  id: "ap-1",
  runId: "run-1",
  storyId: "task-1",
  storyTitle: "Migrate the database",
  toolName: "file_write",
  toolInput: "{}",
  status: "pending",
  createdAt: "2026-08-31T00:00:00.000Z",
};

describe("StoryCard — the waiting-on-you marker", () => {
  it("renders nothing when the story is not blocking anyone", () => {
    render(<StoryCard story={makeStory()} onSelect={() => {}} />);

    expect(screen.queryByText(/waiting on you/i)).toBeNull();
  });

  it("marks a card with a pending question", () => {
    const attention = attentionByStory([humanRequest], []).get("task-1");

    render(
      <StoryCard story={makeStory()} onSelect={() => {}} attention={attention} onAttention={() => {}} />,
    );

    expect(screen.getByRole("button", { name: /waiting on you — 1 question/i })).toBeTruthy();
  });

  it("marks a card with a pending approval", () => {
    const attention = attentionByStory([], [approvalRequest]).get("task-1");

    render(
      <StoryCard story={makeStory()} onSelect={() => {}} attention={attention} onAttention={() => {}} />,
    );

    expect(screen.getByRole("button", { name: /waiting on you — 1 approval/i })).toBeTruthy();
  });

  it("shows the count only when there is more than one thing waiting", () => {
    const one = attentionByStory([humanRequest], []).get("task-1");
    const { unmount } = render(
      <StoryCard story={makeStory()} onSelect={() => {}} attention={one} onAttention={() => {}} />,
    );
    expect(screen.getByRole("button", { name: /waiting on you/i }).textContent).toBe(
      "waiting on you",
    );
    unmount();

    const three = attentionByStory(
      [humanRequest, { ...humanRequest, id: "hr-2" }],
      [approvalRequest],
    ).get("task-1");
    render(
      <StoryCard story={makeStory()} onSelect={() => {}} attention={three} onAttention={() => {}} />,
    );
    expect(screen.getByRole("button", { name: /waiting on you/i }).textContent).toBe(
      "waiting on you · 3",
    );
  });

  it("opens the request behind the marker rather than the detail panel", async () => {
    // The card itself opens the panel. If the marker did too, clicking it would
    // put a panel over the dialog you asked for.
    const onAttention = vi.fn();
    const onSelect = vi.fn();
    const attention = attentionByStory([humanRequest], []).get("task-1");

    render(
      <StoryCard
        story={makeStory()}
        onSelect={onSelect}
        attention={attention}
        onAttention={onAttention}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: /waiting on you/i }));

    expect(onAttention).toHaveBeenCalledWith(attention);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("still opens the panel when the card itself is clicked", async () => {
    const onSelect = vi.fn();
    const story = makeStory();
    const attention = attentionByStory([humanRequest], []).get("task-1");

    render(
      <StoryCard story={story} onSelect={onSelect} attention={attention} onAttention={() => {}} />,
    );

    await userEvent.click(screen.getByText("Migrate the database"));

    expect(onSelect).toHaveBeenCalledWith(story);
  });

  it("renders no marker when there is nothing to open it with", () => {
    // A read-only surface can pass `attention` without a handler; a chip that
    // does nothing when clicked is worse than no chip.
    const attention = attentionByStory([humanRequest], []).get("task-1");

    render(<StoryCard story={makeStory()} onSelect={() => {}} attention={attention} />);

    expect(screen.queryByText(/waiting on you/i)).toBeNull();
  });
});
