import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { tauriMock } from "../../test/tauriMock";
import { RunDetailPanel } from "./RunDetailPanel";
import type { StoryRun } from "../../types/runs";

function makeRun(overrides: Partial<StoryRun> = {}): StoryRun {
  return {
    id: "run-1",
    storyId: "story-1",
    storyTitle: "A story",
    agentProfileId: "agent-1",
    agentName: "Agent One",
    status: "done",
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    estimatedCostUsd: 0,
    iterationCount: 1,
    startedAt: new Date("2026-04-13T00:00:00Z"),
    finishedAt: new Date("2026-04-13T00:01:00Z"),
    durationSecs: 60,
    beforeSha: null,
    worktreePath: null,
    branchName: null,
    afterSha: null,
    isolationStatus: null,
    isolationNote: null,
    ...overrides,
  };
}

function renderPanel(run: StoryRun) {
  tauriMock.handleAll({
    get_run_events: () => [],
    get_run_diff: () => ({ run_id: run.id, before_sha: null, diff_output: null }),
  });
  return render(<RunDetailPanel run={run} onClose={() => {}} />);
}

/** The value rendered next to a stat label, e.g. "Tokens" -> "1.2k in · 40 out". */
async function statValue(label: string): Promise<string> {
  const labelEl = await screen.findByText(label);
  const value = labelEl.parentElement?.querySelector(".run-detail__stat-value");
  return value?.textContent ?? "";
}

describe("RunDetailPanel token accounting", () => {
  it("shows the real token counts a run recorded", async () => {
    // The bug this closes: every run rendered "0 in · 0 out" forever, because
    // nothing wrote the columns.
    renderPanel(makeRun({ inputTokens: 1200, outputTokens: 340 }));

    await waitFor(async () => {
      expect(await statValue("Tokens")).toBe("1.2k in · 340 out");
    });
  });

  it("counts cached input in the input total rather than dropping it", async () => {
    renderPanel(
      makeRun({ inputTokens: 200, cacheReadTokens: 800, outputTokens: 50 }),
    );

    await waitFor(async () => {
      expect(await statValue("Tokens")).toBe("1.0k in · 50 out");
    });
  });

  it("reports cache reads separately so the saving is visible", async () => {
    renderPanel(makeRun({ inputTokens: 200, cacheReadTokens: 4500 }));

    await waitFor(async () => {
      expect(await statValue("Cached")).toBe("4.5k read");
    });
  });

  it("names cache writes distinctly, since they cost more than plain input", async () => {
    renderPanel(makeRun({ cacheReadTokens: 100, cacheCreationTokens: 2000 }));

    await waitFor(async () => {
      expect(await statValue("Cached")).toBe("100 read · 2.0k written");
    });
  });

  it("labels the cost as an estimate rather than a bill", async () => {
    renderPanel(makeRun({ inputTokens: 10, estimatedCostUsd: 0.5 }));

    expect(await screen.findByText("Est. cost")).toBeInTheDocument();
    expect(screen.queryByText("Cost")).not.toBeInTheDocument();
  });

  it("declines to quote $0.00 for a run on an unpriced model", async () => {
    // Real tokens, no price-table entry: the app does not know the cost and
    // must not claim it was free.
    renderPanel(makeRun({ inputTokens: 5000, outputTokens: 500, estimatedCostUsd: 0 }));

    await waitFor(async () => {
      expect(await statValue("Est. cost")).toBe("—");
    });
  });
});

describe("RunDetailPanel context compaction", () => {
  function compactionEvent(payload: Record<string, unknown>) {
    return {
      id: "e1",
      run_id: "run-1",
      event_type: "context_compacted",
      role: null,
      content: JSON.stringify(payload),
      tool_name: null,
      tool_input: null,
      tool_output: null,
      is_error: false,
      sequence_num: 0,
      created_at: "2026-04-13T00:00:30Z",
    };
  }

  function renderWithEvents(events: unknown[]) {
    tauriMock.handleAll({
      get_run_events: () => events,
      get_run_diff: () => ({ run_id: "run-1", before_sha: null, diff_output: null }),
    });
    return render(<RunDetailPanel run={makeRun()} onClose={() => {}} />);
  }

  it("tells the operator that history was dropped and how much", async () => {
    // Without this the timeline shows nothing, and an agent that forgot an
    // earlier decision looks like a model failure rather than a budget one.
    renderWithEvents([
      compactionEvent({
        strategy: "recent",
        before_tokens: 6100,
        after_tokens: 4100,
        budget_tokens: 6000,
        evicted_messages: 2,
        summarized: false,
      }),
    ]);

    expect(await screen.findByText("✂ context compacted")).toBeInTheDocument();
    const line = await screen.findByText(/6\.1k/);
    expect(line.textContent).toContain("recent");
    expect(line.textContent).toContain("4.1k");
    expect(line.textContent).toContain("6.0k budget");
    expect(line.textContent).toContain("2 messages dropped");
    expect(line.textContent).not.toContain("summary");
  });

  it("says when the dropped prefix was replaced by a summary", async () => {
    renderWithEvents([
      compactionEvent({
        strategy: "summary",
        before_tokens: 6100,
        after_tokens: 4300,
        budget_tokens: 6000,
        evicted_messages: 1,
        summarized: true,
      }),
    ]);

    const line = await screen.findByText(/replaced by a summary/);
    expect(line.textContent).toContain("1 message dropped");
  });

  it("falls back to the raw payload rather than blanking on a malformed event", async () => {
    renderWithEvents([
      {
        id: "e1",
        run_id: "run-1",
        event_type: "context_compacted",
        role: null,
        content: "not json",
        tool_name: null,
        tool_input: null,
        tool_output: null,
        is_error: false,
        sequence_num: 0,
        created_at: "2026-04-13T00:00:30Z",
      },
    ]);

    expect(await screen.findByText("not json")).toBeInTheDocument();
  });
});

describe("RunDetailPanel run isolation", () => {
  function renderIsolated(overrides: Partial<StoryRun>, diffOutput: string | null = null) {
    const run = makeRun({
      beforeSha: "abc123def456789",
      worktreePath: "/data/worktrees/run-1",
      branchName: "rustyagent/run-1",
      isolationStatus: "isolated",
      ...overrides,
    });
    tauriMock.handleAll({
      get_run_events: () => [],
      get_run_diff: () => ({
        run_id: run.id,
        before_sha: run.beforeSha,
        diff_output: diffOutput,
      }),
      accept_run: () => "Accepted into /repo from branch 'rustyagent/run-1'.",
      revert_run: () => "Reverted: worktree and branch were deleted.",
    });
    return { run, ...render(<RunDetailPanel run={run} onClose={() => {}} />) };
  }

  /** Move to the File Changes tab, where the isolation banner lives. */
  async function openChanges() {
    await userEvent.click(screen.getByRole("tab", { name: "File Changes" }));
  }

  it("offers accept and revert once an isolated run has finished", async () => {
    renderIsolated({});

    expect(await screen.findByRole("button", { name: /Accept/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: /Revert/ })).toBeEnabled();
  });

  it("offers neither while the run is still going", async () => {
    // Its changes are not committed yet, so there is nothing coherent to
    // accept or throw away.
    renderIsolated({ status: "running", finishedAt: null, durationSecs: null });

    await screen.findByRole("button", { name: /Export/ });
    expect(screen.queryByRole("button", { name: /Accept/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Revert/ })).not.toBeInTheDocument();
  });

  it("offers neither for a run that was never isolated", async () => {
    renderIsolated({
      isolationStatus: "not_a_git_repo",
      worktreePath: null,
      branchName: null,
      isolationNote: "'/tmp/plain' is not a git repository, so this run could not be isolated.",
    });

    await screen.findByRole("button", { name: /Export/ });
    expect(screen.queryByRole("button", { name: /Accept/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Revert/ })).not.toBeInTheDocument();
  });

  it("warns, in the run's own words, when a run wrote into the user's tree", async () => {
    renderIsolated({
      isolationStatus: "not_a_git_repo",
      isolationNote: "'/tmp/plain' is not a git repository, so this run could not be isolated.",
    });
    await openChanges();

    expect(await screen.findByText("This run was not isolated.")).toBeInTheDocument();
    expect(screen.getByText(/not a git repository/)).toBeInTheDocument();
  });

  it("names the branch an isolated run's changes are parked on", async () => {
    renderIsolated({});
    await openChanges();

    const banner = await screen.findByText(/isolated worktree on branch/);
    expect(banner.textContent).toContain("rustyagent/run-1");
    expect(banner.textContent).toContain("Nothing here has reached your working tree yet");
  });

  it("shows the isolation banner above the diff, not only when there is none", async () => {
    renderIsolated({}, "diff --git a/new.txt b/new.txt\n+created\n");
    await openChanges();

    expect(await screen.findByText(/isolated worktree on branch/)).toBeInTheDocument();
    expect(screen.getByText("diff --git a/new.txt b/new.txt")).toBeInTheDocument();
  });

  it("accepting asks the backend to merge this run and reports what happened", async () => {
    const onDecided = vi.fn();
    const run = makeRun({
      beforeSha: "abc123",
      worktreePath: "/data/worktrees/run-1",
      branchName: "rustyagent/run-1",
      isolationStatus: "isolated",
    });
    tauriMock.handleAll({
      get_run_events: () => [],
      get_run_diff: () => ({ run_id: run.id, before_sha: "abc123", diff_output: null }),
      accept_run: () => "Accepted into /repo from branch 'rustyagent/run-1'.",
    });
    render(<RunDetailPanel run={run} onClose={() => {}} onDecided={onDecided} />);

    await userEvent.click(await screen.findByRole("button", { name: /Accept/ }));

    await waitFor(() => {
      expect(tauriMock.calls("accept_run")).toEqual([{ runId: "run-1" }]);
    });
    expect(onDecided).toHaveBeenCalled();
  });

  it("reverting confirms first, and says the user's tree is not touched", async () => {
    renderIsolated({});

    await userEvent.click(await screen.findByRole("button", { name: /Revert/ }));

    expect(await screen.findByText("Throw away this run's changes?")).toBeInTheDocument();
    expect(screen.getByText(/never wrote there/)).toBeInTheDocument();
    // Nothing has happened yet — the confirmation is a real gate.
    expect(tauriMock.called("revert_run")).toBe(false);
  });

  it("reverting only calls the backend once the user confirms", async () => {
    const onDecided = vi.fn();
    const run = makeRun({
      beforeSha: "abc123",
      worktreePath: "/data/worktrees/run-1",
      branchName: "rustyagent/run-1",
      isolationStatus: "isolated",
    });
    tauriMock.handleAll({
      get_run_events: () => [],
      get_run_diff: () => ({ run_id: run.id, before_sha: "abc123", diff_output: null }),
      revert_run: () => "Reverted: worktree and branch were deleted.",
    });
    render(<RunDetailPanel run={run} onClose={() => {}} onDecided={onDecided} />);

    await userEvent.click(await screen.findByRole("button", { name: /Revert/ }));
    await userEvent.click(await screen.findByRole("button", { name: "Revert Run" }));

    await waitFor(() => {
      expect(tauriMock.calls("revert_run")).toEqual([{ runId: "run-1" }]);
    });
    expect(onDecided).toHaveBeenCalled();
  });

  it("keeps the run when accepting fails, so it can be retried or reverted", async () => {
    // git refusing to merge over uncommitted local work is the safe outcome,
    // and the panel must not pretend the decision was made.
    const onDecided = vi.fn();
    const run = makeRun({
      beforeSha: "abc123",
      worktreePath: "/data/worktrees/run-1",
      branchName: "rustyagent/run-1",
      isolationStatus: "isolated",
    });
    tauriMock.handleAll({
      get_run_events: () => [],
      get_run_diff: () => ({ run_id: run.id, before_sha: "abc123", diff_output: null }),
      accept_run: () => {
        throw new Error("local changes would be overwritten");
      },
    });
    render(<RunDetailPanel run={run} onClose={() => {}} onDecided={onDecided} />);

    await userEvent.click(await screen.findByRole("button", { name: /Accept/ }));

    await waitFor(() => {
      expect(tauriMock.called("accept_run")).toBe(true);
    });
    expect(onDecided).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /Accept/ })).toBeEnabled();
  });

  it("says so plainly once a run has been accepted or reverted", async () => {
    renderIsolated({
      isolationStatus: "reverted",
      isolationNote: "Reverted: worktree and branch were deleted.",
    });
    await openChanges();

    expect(await screen.findByText(/worktree and branch were deleted/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Accept/ })).not.toBeInTheDocument();
  });
});

describe("RunDetailPanel interrupted runs", () => {
  // Closing the app mid-run used to leave the row saying "running" forever.
  // It now ends as a plain `failed` run, so the only thing that distinguishes
  // "the agent broke" from "the app went away" is this timeline entry — which
  // means it has to actually reach the screen.
  const INTERRUPTED_MESSAGE =
    "RustyAgent exited while this run was still executing, so it was marked failed on " +
    "the next startup.";

  function interruptedEvent() {
    return {
      id: "e1",
      run_id: "run-1",
      event_type: "interrupted",
      role: null,
      content: INTERRUPTED_MESSAGE,
      tool_name: null,
      tool_input: null,
      tool_output: null,
      is_error: false,
      sequence_num: 3,
      created_at: "2026-04-13T00:00:30Z",
    };
  }

  function renderInterrupted(run: Partial<StoryRun> = {}) {
    tauriMock.handleAll({
      get_run_events: () => [interruptedEvent()],
      get_run_diff: () => ({ run_id: "run-1", before_sha: null, diff_output: null }),
    });
    return render(
      <RunDetailPanel run={makeRun({ status: "failed", ...run })} onClose={() => {}} />,
    );
  }

  it("tells the operator a restart ended the run, not the agent", async () => {
    renderInterrupted();

    expect(await screen.findByText(new RegExp(INTERRUPTED_MESSAGE))).toBeInTheDocument();
  });

  it("labels the entry so it is distinguishable from an agent error", async () => {
    renderInterrupted();

    expect(await screen.findByText("interrupted")).toBeInTheDocument();
  });

  it("still reports the iterations the run got through before it was cut off", async () => {
    // The sweep leaves `iteration_count` alone precisely so this is not zero.
    renderInterrupted({ iterationCount: 4 });

    await waitFor(async () => {
      expect(await statValue("Iterations")).toBe("4");
    });
  });
});
