import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ListView } from "./ListView";
import type { AgentProfile } from "../../types/agent";
import type { Story } from "../../types/board";

function story(id: string, title: string, overrides: Partial<Story> = {}): Story {
  return {
    id,
    key: `#${id}`,
    title,
    status: "ready",
    priority: "medium",
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

const STORIES = [story("s1", "Migrate the database"), story("s2", "Cap the read path")];
const AGENTS = [agent("a1", "Agent One"), agent("a2", "Agent Two")];

const bulkBar = () => screen.queryByRole("toolbar", { name: "Bulk actions" });
const bulkPicker = () => screen.getByRole("combobox", { name: /Assign an agent to/ });

async function selectRow(key: string) {
  await userEvent.click(screen.getByRole("checkbox", { name: `Select ${key}` }));
}

describe("ListView — bulk assign", () => {
  it("shows no bulk bar until something is selected", () => {
    render(<ListView stories={STORIES} onSelect={() => {}} agents={AGENTS} onAssignStories={vi.fn()} />);

    expect(bulkBar()).toBeNull();
  });

  it("offers no assign action without a handler", async () => {
    render(<ListView stories={STORIES} onSelect={() => {}} />);
    await selectRow("#s1");

    expect(bulkBar()).not.toBeNull();
    expect(screen.queryByRole("combobox", { name: /Assign an agent to/ })).toBeNull();
  });

  it("names how many stories the action will touch", async () => {
    render(<ListView stories={STORIES} onSelect={() => {}} agents={AGENTS} onAssignStories={vi.fn()} />);
    await selectRow("#s1");
    await selectRow("#s2");

    expect(screen.getByRole("option", { name: "Assign 2…" })).toBeTruthy();
  });

  it("assigns every selected story", async () => {
    const onAssignStories = vi.fn().mockResolvedValue(undefined);
    render(
      <ListView
        stories={STORIES}
        onSelect={() => {}}
        agents={AGENTS}
        onAssignStories={onAssignStories}
      />,
    );
    await selectRow("#s1");
    await selectRow("#s2");

    await userEvent.selectOptions(bulkPicker(), "a2");

    expect(onAssignStories).toHaveBeenCalledWith(["s1", "s2"], "a2");
  });

  it("unassigns a selection", async () => {
    const onAssignStories = vi.fn().mockResolvedValue(undefined);
    render(
      <ListView
        stories={STORIES}
        onSelect={() => {}}
        agents={AGENTS}
        onAssignStories={onAssignStories}
      />,
    );
    await selectRow("#s1");

    // The picker's empty option is the command's own label, so choosing it has
    // to mean "unassign these" rather than "do nothing".
    await userEvent.selectOptions(bulkPicker(), "");

    expect(onAssignStories).toHaveBeenCalledWith(["s1"], null);
  });

  it("clears the selection so the action cannot be fired twice by accident", async () => {
    const onAssignStories = vi.fn().mockResolvedValue(undefined);
    render(
      <ListView
        stories={STORIES}
        onSelect={() => {}}
        agents={AGENTS}
        onAssignStories={onAssignStories}
      />,
    );
    await selectRow("#s1");

    await userEvent.selectOptions(bulkPicker(), "a1");

    expect(bulkBar()).toBeNull();
  });

  it("does not leave a failed bulk assign as an unhandled rejection", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const onAssignStories = vi.fn().mockRejectedValue(new Error("no workspace is open"));
    render(
      <ListView
        stories={STORIES}
        onSelect={() => {}}
        agents={AGENTS}
        onAssignStories={onAssignStories}
      />,
    );
    await selectRow("#s1");

    await userEvent.selectOptions(bulkPicker(), "a1");

    await vi.waitFor(() =>
      expect(error).toHaveBeenCalledWith("Assignment failed:", expect.any(Error)),
    );
  });
});

describe("ListView — selection", () => {
  it("selects and clears every row at once", async () => {
    render(<ListView stories={STORIES} onSelect={() => {}} />);

    await userEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
    expect(screen.getByText("2 selected")).toBeTruthy();

    await userEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
    expect(bulkBar()).toBeNull();
  });

  it("opens a story from its row but not from its checkbox", async () => {
    const onSelect = vi.fn();
    render(<ListView stories={STORIES} onSelect={onSelect} />);

    await selectRow("#s1");
    expect(onSelect).not.toHaveBeenCalled();

    await userEvent.click(screen.getByText("Migrate the database"));
    expect(onSelect).toHaveBeenCalledWith(STORIES[0]);
  });
});
