import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AgentPicker } from "./AgentPicker";
import type { AgentProfile } from "../../types/agent";

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

const picker = () => screen.getByRole("combobox", { name: "Assigned agent" });

describe("AgentPicker", () => {
  it("shows the assigned agent", () => {
    render(
      <AgentPicker agents={AGENTS} value="a2" onChange={() => {}} ariaLabel="Assigned agent" />,
    );

    expect((picker() as HTMLSelectElement).value).toBe("a2");
  });

  it("shows unassigned as an empty value, not as the first agent", () => {
    render(
      <AgentPicker agents={AGENTS} value={null} onChange={() => {}} ariaLabel="Assigned agent" />,
    );

    expect((picker() as HTMLSelectElement).value).toBe("");
    expect(screen.getByRole("option", { name: "Unassigned" })).toBeTruthy();
  });

  it("reports the chosen agent", async () => {
    const onChange = vi.fn();
    render(
      <AgentPicker agents={AGENTS} value={null} onChange={onChange} ariaLabel="Assigned agent" />,
    );

    await userEvent.selectOptions(picker(), "a1");

    expect(onChange).toHaveBeenCalledWith("a1");
  });

  it("reports null when unassigned is chosen", async () => {
    // A select's value is a string, so unassigning has to be translated back
    // into the null the caller means — the empty string reaches `update_story`
    // as "clear", but nothing above this should have to know that.
    const onChange = vi.fn();
    render(
      <AgentPicker agents={AGENTS} value="a1" onChange={onChange} ariaLabel="Assigned agent" />,
    );

    await userEvent.selectOptions(picker(), "");

    expect(onChange).toHaveBeenCalledWith(null);
  });

  it("keeps showing an assignment whose profile is gone", async () => {
    render(
      <AgentPicker
        agents={AGENTS}
        value="deleted-id"
        onChange={() => {}}
        ariaLabel="Assigned agent"
      />,
    );

    expect((picker() as HTMLSelectElement).value).toBe("deleted-id");
    expect(screen.getByRole("option", { name: /unavailable/ })).toBeTruthy();
  });

  it("takes a different word for the empty option", () => {
    // The footer's "run with" picker means "use the assignee", not "nobody".
    render(
      <AgentPicker
        agents={AGENTS}
        value={null}
        onChange={() => {}}
        ariaLabel="Run with"
        unassignedLabel="Run as Agent One"
      />,
    );

    expect(screen.getByRole("option", { name: "Run as Agent One" })).toBeTruthy();
    expect(screen.queryByRole("option", { name: "Unassigned" })).toBeNull();
  });

  it("can be disabled while a write is in flight", () => {
    render(
      <AgentPicker
        agents={AGENTS}
        value="a1"
        onChange={() => {}}
        ariaLabel="Assigned agent"
        disabled
      />,
    );

    expect((picker() as HTMLSelectElement).disabled).toBe(true);
  });
});
