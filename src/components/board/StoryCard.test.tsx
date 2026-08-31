import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { StoryCard } from "./StoryCard";
import { attentionByStory } from "./attention";
import type { Story } from "../../types/board";
import type { ApprovalRequest, HumanRequest } from "../../types/human";
import type { AgentProfile } from "../../types/agent";

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

// ---------------------------------------------------------------------------
// Assigning from the card
// ---------------------------------------------------------------------------

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

const AGENTS = [agent("a1", "Agent One"), agent("a2", "Agent Two")];

const assigneeButton = () => screen.getByRole("button", { name: /^Assignee:/ });
const assigneePicker = () => screen.queryByRole("combobox", { name: /^Assign an agent to/ });

describe("StoryCard - assigning from the card", () => {
  it("leaves the assignee as plain text with no handler", () => {
    // The drag overlay renders a card that is only being shown; a control on it
    // would be unreachable and would suggest otherwise.
    render(<StoryCard story={makeStory({ assignee: "Agent One" })} onSelect={() => {}} />);

    expect(screen.getByText("Agent One")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^Assignee:/ })).toBeNull();
  });

  it("names the current assignee for a screen reader", () => {
    render(
      <StoryCard
        story={makeStory({ assignee: undefined })}
        onSelect={() => {}}
        agents={AGENTS}
        onAssign={() => {}}
      />,
    );

    expect(assigneeButton().getAttribute("aria-label")).toBe(
      "Assignee: Unassigned. Press to change.",
    );
  });

  it("swaps the name for a picker when pressed, without opening the panel", async () => {
    const onSelect = vi.fn();
    render(
      <StoryCard
        story={makeStory()}
        onSelect={onSelect}
        agents={AGENTS}
        onAssign={() => {}}
      />,
    );

    expect(assigneePicker()).toBeNull();
    await userEvent.click(assigneeButton());

    expect(assigneePicker()).not.toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("assigns the chosen agent and collapses again", async () => {
    const onAssign = vi.fn();
    const onSelect = vi.fn();
    render(
      <StoryCard
        story={makeStory()}
        onSelect={onSelect}
        agents={AGENTS}
        onAssign={onAssign}
      />,
    );

    await userEvent.click(assigneeButton());
    await userEvent.selectOptions(assigneePicker()!, "a2");

    expect(onAssign).toHaveBeenCalledWith("a2");
    expect(assigneePicker()).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("unassigns", async () => {
    const onAssign = vi.fn();
    render(
      <StoryCard
        story={makeStory({ assignee: "Agent One", assignedAgentId: "a1" })}
        onSelect={() => {}}
        agents={AGENTS}
        onAssign={onAssign}
      />,
    );

    await userEvent.click(assigneeButton());
    await userEvent.selectOptions(assigneePicker()!, "");

    expect(onAssign).toHaveBeenCalledWith(null);
  });

  it("collapses on Escape without assigning", async () => {
    const onAssign = vi.fn();
    render(
      <StoryCard story={makeStory()} onSelect={() => {}} agents={AGENTS} onAssign={onAssign} />,
    );

    await userEvent.click(assigneeButton());
    await userEvent.keyboard("{Escape}");

    expect(assigneePicker()).toBeNull();
    expect(onAssign).not.toHaveBeenCalled();
  });

  it("does not open the panel from the keys used to work the picker", async () => {
    // The card opens on Enter and Space. Those are also how you drive a select,
    // so without stopping them a keyboard user would get the panel over the top.
    const onSelect = vi.fn();
    render(
      <StoryCard story={makeStory()} onSelect={onSelect} agents={AGENTS} onAssign={() => {}} />,
    );

    await userEvent.click(assigneeButton());
    await userEvent.keyboard("{Enter}");

    expect(onSelect).not.toHaveBeenCalled();
  });

  it("does not leave a failed assignment as an unhandled rejection", async () => {
    // `updateStory` toasts and then rethrows, and this handler cannot await.
    // Dropping the promise would surface the failure only in the console —
    // and would leave the card looking as though the change had stuck.
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const onAssign = vi.fn().mockRejectedValue(new Error("no workspace is open"));

    render(
      <StoryCard story={makeStory()} onSelect={() => {}} agents={AGENTS} onAssign={onAssign} />,
    );

    await userEvent.click(assigneeButton());
    await userEvent.selectOptions(assigneePicker()!, "a2");

    await vi.waitFor(() =>
      expect(error).toHaveBeenCalledWith("Assignment failed:", expect.any(Error)),
    );
    expect(assigneePicker()).toBeNull();
  });
});
