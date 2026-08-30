import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import { tauriMock } from "../../test/tauriMock";
import AutonomousActivityPanel, { mergeLastAction } from "./AutonomousActivityPanel";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function rawRun(overrides: Record<string, unknown> & { id: string }) {
  return {
    story_id: "story-1",
    story_title: "Ship the thing",
    agent_profile_id: "agent-1",
    agent_name: "Agent One",
    status: "running",
    started_at: new Date().toISOString(),
    ...overrides,
  };
}

function runtimeStatus(overrides: Record<string, unknown> = {}) {
  return {
    profileId: "agent-1",
    state: "running_story",
    stateLabel: "Running story",
    activeRunId: "run-1",
    activeStoryTitle: "Ship the thing",
    failureSummary: null,
    ...overrides,
  };
}

function renderPanel({
  runs = [rawRun({ id: "run-1" })],
  statuses = [runtimeStatus()],
  profiles = [{ id: "agent-1", name: "Agent One" }],
}: {
  runs?: ReturnType<typeof rawRun>[];
  statuses?: ReturnType<typeof runtimeStatus>[];
  profiles?: { id: string; name: string }[];
} = {}) {
  tauriMock.handleAll({
    get_runs: () => runs,
    get_all_agent_runtime_statuses: () => statuses,
    get_profiles: () => profiles,
    get_run_events: () => [],
  });

  return render(
    <MemoryRouter>
      <AutonomousActivityPanel />
    </MemoryRouter>,
  );
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

describe("AutonomousActivityPanel", () => {
  it("lists an active run with its agent, story and state", async () => {
    renderPanel();

    expect(await screen.findByText("Agent One")).toBeInTheDocument();
    expect(screen.getByText("Ship the thing")).toBeInTheDocument();
    expect(screen.getByText("Running story")).toBeInTheDocument();
    expect(screen.getByText("1 active")).toBeInTheDocument();
  });

  it("says so when nothing is running", async () => {
    renderPanel({ runs: [], statuses: [] });

    expect(
      await screen.findByText(/no agents are active right now/i),
    ).toBeInTheDocument();
  });

  // An idle runtime is not activity. It used to be filtered here and the
  // filter is easy to lose, since an idle agent still has a status row.
  it("leaves idle agents out of the list", async () => {
    renderPanel({
      runs: [],
      statuses: [runtimeStatus({ state: "idle", activeRunId: null })],
    });

    expect(
      await screen.findByText(/no agents are active right now/i),
    ).toBeInTheDocument();
  });

  it("shows a finished run without counting it as active", async () => {
    renderPanel({
      runs: [rawRun({ id: "run-9", status: "done", story_title: "Older work" })],
      statuses: [],
    });

    expect(await screen.findByText("Older work")).toBeInTheDocument();
    expect(screen.getByText("Completed")).toBeInTheDocument();
    expect(screen.getByText("0 active")).toBeInTheDocument();
  });

  it("links a row to its run", async () => {
    renderPanel();

    const link = await screen.findByRole("link", { name: /run details/i });
    expect(link).toHaveAttribute("href", "/runs?runId=run-1");
  });
});

// ---------------------------------------------------------------------------
// Live updates — what this panel stopped polling for
// ---------------------------------------------------------------------------

describe("AutonomousActivityPanel live updates", () => {
  it("shows a tool call the moment the run emits it", async () => {
    renderPanel();
    await screen.findByText("Agent One");
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-1",
      tool_name: "file_write",
    });

    expect(await screen.findByText("Using file_write")).toBeInTheDocument();
  });

  it("shows a run parked on an approval", async () => {
    renderPanel();
    await screen.findByText("Agent One");
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    await tauriMock.emit("run-event", {
      type: "awaiting_approval",
      run_id: "run-1",
      tool_name: "file_write",
    });

    expect(
      await screen.findByText("Approval requested for file_write"),
    ).toBeInTheDocument();
  });

  it("reports a tool that failed", async () => {
    renderPanel();
    await screen.findByText("Agent One");
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    await tauriMock.emit("run-event", {
      type: "tool_result",
      run_id: "run-1",
      tool_name: "file_write",
      is_error: true,
    });

    expect(
      await screen.findByText("file_write returned an error"),
    ).toBeInTheDocument();
  });

  // The panel no longer refetches ten runs' event logs every four seconds; the
  // one thing an event cannot tell it is that a run's *status* changed.
  it("refetches the run list when a run finishes", async () => {
    renderPanel();
    await screen.findByText("Agent One");
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));
    const before = tauriMock.callCount("get_runs");

    await tauriMock.emit("run-event", {
      type: "complete",
      run_id: "run-1",
      stop_reason: "end_turn",
    });

    await waitFor(() =>
      expect(tauriMock.callCount("get_runs")).toBeGreaterThan(before),
    );
  });

  it("ignores an event for a run it is not showing", async () => {
    renderPanel();
    await screen.findByText("Agent One");
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-other",
      tool_name: "should_not_appear",
    });

    expect(screen.queryByText("Using should_not_appear")).not.toBeInTheDocument();
  });

  // The mount seeds each active run's last action from `get_run_events`, which
  // for a run that has only just started comes back empty — i.e. `null`. A
  // live event arriving while that fetch is in flight must not then be blanked
  // by it. This failed only in CI, where the fetch is slow enough to lose.
  it("keeps a live event that arrives before the seeding fetch resolves", async () => {
    let release: (rows: unknown[]) => void = () => {};
    tauriMock.handleAll({
      get_runs: () => [rawRun({ id: "run-1" })],
      get_all_agent_runtime_statuses: () => [runtimeStatus()],
      get_profiles: () => [{ id: "agent-1", name: "Agent One" }],
      get_run_events: () => new Promise((resolve) => { release = resolve; }),
    });
    render(
      <MemoryRouter>
        <AutonomousActivityPanel />
      </MemoryRouter>,
    );
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-1",
      tool_name: "file_write",
    });
    expect(await screen.findByText("Using file_write")).toBeInTheDocument();

    // The seeding fetch now lands, carrying nothing.
    release([]);

    await waitFor(() => expect(tauriMock.called("get_run_events")).toBe(true));
    expect(screen.getByText("Using file_write")).toBeInTheDocument();
  });

  it("stops listening when the panel unmounts", async () => {
    const { unmount } = renderPanel();
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    unmount();

    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(0));
  });
});

// ---------------------------------------------------------------------------
// mergeLastAction — the guard that is invisible from the rendered output
// ---------------------------------------------------------------------------

function event(overrides: Partial<Parameters<typeof mergeLastAction>[2]> = {}) {
  return {
    event_type: "message",
    role: "assistant",
    content: null,
    tool_name: null,
    is_error: false,
    ...overrides,
  };
}

describe("mergeLastAction", () => {
  it("records the first action for a run", () => {
    const next = mergeLastAction({}, "run-1", event({ event_type: "tool_call", role: null }));

    expect(next["run-1"]?.event_type).toBe("tool_call");
  });

  // `summarizeAction` renders every assistant message as "Drafting response",
  // so a second token cannot change the panel — and tokens arrive by the
  // thousand. Returning the same reference is what lets React skip the render.
  it("returns the same map when another token cannot change the label", () => {
    const prev = { "run-1": event({ content: "Hel" }) };

    const next = mergeLastAction(prev, "run-1", event({ content: "lo" }));

    expect(next).toBe(prev);
  });

  it("replaces a message with the tool call that follows it", () => {
    const prev = { "run-1": event({ content: "drafting" }) };

    const next = mergeLastAction(
      prev,
      "run-1",
      event({ event_type: "tool_call", role: null, tool_name: "file_write" }),
    );

    expect(next).not.toBe(prev);
    expect(next["run-1"]?.tool_name).toBe("file_write");
  });

  it("shows a run starting to draft after a tool result", () => {
    const prev = { "run-1": event({ event_type: "tool_result", role: null }) };

    const next = mergeLastAction(prev, "run-1", event({ content: "Hel" }));

    expect(next).not.toBe(prev);
    expect(next["run-1"]?.event_type).toBe("message");
  });

  it("keeps runs separate", () => {
    const prev = { "run-1": event({ content: "one" }) };

    const next = mergeLastAction(prev, "run-2", event({ content: "two" }));

    expect(next["run-1"]).toBe(prev["run-1"]);
    expect(next["run-2"]?.content).toBe("two");
  });
});
