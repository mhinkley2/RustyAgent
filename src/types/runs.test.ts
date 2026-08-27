// @vitest-environment node
import { describe, expect, it } from "vitest";

import {
  formatCost,
  formatDuration,
  formatTokens,
  RUN_STATUS_LABELS,
  type RunStatus,
} from "./runs";

describe("formatCost", () => {
  it("renders exactly zero as $0.00", () => {
    expect(formatCost(0)).toBe("$0.00");
  });

  it("uses four decimals below one cent so sub-cent runs are not all $0.00", () => {
    expect(formatCost(0.004)).toBe("$0.0040");
    expect(formatCost(0.0001)).toBe("$0.0001");
  });

  it("switches to two decimals at one cent", () => {
    expect(formatCost(0.01)).toBe("$0.01");
  });

  it("rounds larger amounts to cents", () => {
    expect(formatCost(12.3456)).toBe("$12.35");
    expect(formatCost(7)).toBe("$7.00");
  });
});

describe("formatTokens", () => {
  it("renders counts below a thousand verbatim", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
  });

  it("abbreviates thousands with one decimal", () => {
    expect(formatTokens(1_000)).toBe("1.0k");
    expect(formatTokens(1_500)).toBe("1.5k");
    expect(formatTokens(999_999)).toBe("1000.0k");
  });

  it("abbreviates millions with one decimal", () => {
    expect(formatTokens(1_000_000)).toBe("1.0M");
    expect(formatTokens(2_400_000)).toBe("2.4M");
  });
});

describe("formatDuration", () => {
  it("renders a missing duration as an em dash", () => {
    expect(formatDuration(null)).toBe("—");
  });

  it("renders sub-minute durations in whole seconds", () => {
    expect(formatDuration(0)).toBe("0s");
    expect(formatDuration(59.6)).toBe("60s");
  });

  it("drops the seconds component when it rounds to zero", () => {
    expect(formatDuration(60)).toBe("1m");
    expect(formatDuration(3600)).toBe("60m");
  });

  it("renders minutes and seconds together otherwise", () => {
    expect(formatDuration(61)).toBe("1m 1s");
    expect(formatDuration(125)).toBe("2m 5s");
  });
});

describe("RUN_STATUS_LABELS", () => {
  // Regression guard. The conversation runtime used to finish runs with the
  // status 'completed' while this map (and the RunStatus union) only knew
  // 'done', so every successful run rendered a blank badge and the "Done"
  // filter never matched. `finish_run` now writes 'done'; if it ever drifts
  // again, this fails.
  //
  // Keep in sync with the statuses written by:
  //   crates/runtime/src/runtime.rs  (finish_run)
  //   crates/pipeline/src/lib.rs     (final_status)
  const STATUSES_THE_BACKEND_WRITES: RunStatus[] = [
    "running",
    "done",
    "failed",
    "cancelled",
  ];

  it.each(STATUSES_THE_BACKEND_WRITES)("has a label for %s", (status) => {
    expect(RUN_STATUS_LABELS[status]).toBeTruthy();
  });

  it("has no labels beyond the statuses the backend writes", () => {
    expect(Object.keys(RUN_STATUS_LABELS).sort()).toEqual(
      [...STATUSES_THE_BACKEND_WRITES].sort(),
    );
  });

  it("does not silently render undefined for an unknown status", () => {
    // What the bug looked like from the UI's side.
    const stale = "completed" as unknown as RunStatus;
    expect(RUN_STATUS_LABELS[stale]).toBeUndefined();
  });
});
