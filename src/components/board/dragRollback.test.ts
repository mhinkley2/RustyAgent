// @vitest-environment node
import { describe, expect, it } from "vitest";

import { rollbackTarget, type ColMap } from "./dragRollback";
import type { Story, StoryStatus } from "../../types/board";

function story(id: string, status: StoryStatus): Story {
  return {
    id,
    key: `#${id}`,
    title: `Story ${id}`,
    status,
    priority: "medium",
    type: "task",
    requiresApproval: false,
    trackHistory: true,
    sortOrder: 0,
    labels: [],
    createdAt: new Date("2026-04-13T00:00:00Z"),
    updatedAt: new Date("2026-04-13T00:00:00Z"),
  };
}

function colMap(partial: Partial<Record<StoryStatus, Story[]>>): ColMap {
  return {
    backlog: [],
    ready: [],
    in_progress: [],
    blocked: [],
    review: [],
    done: [],
    ...partial,
  };
}

/** Card `a` sitting in Ready, alongside `b`. */
const BEFORE_DRAG = colMap({ ready: [story("a", "ready"), story("b", "ready")] });

/** `a` dragged into In Progress; Ready keeps `b`. */
const BEFORE_REORDER = colMap({
  ready: [story("b", "ready")],
  in_progress: [story("a", "in_progress")],
});

describe("rollbackTarget", () => {
  it("returns the whole drag when the status write is refused", () => {
    // Nothing was persisted, so the card belongs back in the column it was
    // dragged out of. Leaving it where the failed write tried to put it is
    // the bug this exists for: an error toast, and a board that disagrees
    // with the database until something else reloads it.
    const target = rollbackTarget({
      failed: "move",
      crossColumn: true,
      beforeDrag: BEFORE_DRAG,
      beforeReorder: BEFORE_REORDER,
      dragInFlight: false,
    });

    expect(target).toBe(BEFORE_DRAG);
    expect(target?.ready.map((s) => s.id)).toEqual(["a", "b"]);
    expect(target?.in_progress).toEqual([]);
  });

  it("keeps a status write that landed when only the reorder is refused", () => {
    // The move is real. Rolling the card back to its old column here would
    // undo a write that succeeded — the board would then disagree with the
    // database in the opposite direction, which is worse than not restoring.
    const target = rollbackTarget({
      failed: "reorder",
      crossColumn: true,
      beforeDrag: BEFORE_DRAG,
      beforeReorder: BEFORE_REORDER,
      dragInFlight: false,
    });

    expect(target).toBe(BEFORE_REORDER);
    expect(target?.in_progress.map((s) => s.id)).toEqual(["a"]);
  });

  it("restores the original order when a within-column reorder is refused", () => {
    // No status changed, so the pre-drag board is also the pre-reorder board.
    const target = rollbackTarget({
      failed: "reorder",
      crossColumn: false,
      beforeDrag: BEFORE_DRAG,
      beforeReorder: colMap({ ready: [story("b", "ready"), story("a", "ready")] }),
      dragInFlight: false,
    });

    expect(target).toBe(BEFORE_DRAG);
    expect(target?.ready.map((s) => s.id)).toEqual(["a", "b"]);
  });

  it("restores nothing when the drag start was never recorded", () => {
    // No previous state to name. Guessing at one would move cards the user
    // never touched, so the board is left alone and the toast stands on its own.
    expect(
      rollbackTarget({
        failed: "move",
        crossColumn: true,
        beforeDrag: null,
        beforeReorder: BEFORE_REORDER,
        dragInFlight: false,
      }),
    ).toBeNull();

    expect(
      rollbackTarget({
        failed: "reorder",
        crossColumn: false,
        beforeDrag: null,
        beforeReorder: BEFORE_REORDER,
        dragInFlight: false,
      }),
    ).toBeNull();
  });
});

describe("rollbackTarget — a drag started after the failure", () => {
  it("restores nothing while another drag is in flight", () => {
    // The persist is awaited, so the user can pick up a second card before the
    // first drop's write comes back. Restoring here would yank that card out
    // from under the pointer, which is a worse failure than the stale column
    // this whole rule exists to fix.
    for (const failed of ["move", "reorder"] as const) {
      expect(
        rollbackTarget({
          failed,
          crossColumn: true,
          beforeDrag: BEFORE_DRAG,
          beforeReorder: BEFORE_REORDER,
          dragInFlight: true,
        }),
      ).toBeNull();
    }
  });
});
