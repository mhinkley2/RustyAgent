import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

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
