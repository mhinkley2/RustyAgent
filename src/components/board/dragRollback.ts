import type { Story, StoryStatus } from "../../types/board";

/** The board as the columns render it: each status, in display order. */
export type ColMap = Record<StoryStatus, Story[]>;

/** Which of a drop's two writes was refused. */
export type FailedWrite = "move" | "reorder";

/**
 * The board to fall back to when a drop's write is rejected.
 *
 * A drop can persist two things: the card's new status, and the new order of
 * the column it landed in. They are separate writes, so they fail separately
 * — and they have different previous states. Restoring the wrong one is worse
 * than restoring nothing, because it undoes a write that actually landed and
 * leaves the board disagreeing with the database in the opposite direction.
 *
 * - The status write failed. Nothing was persisted, so the whole drag comes
 *   back: the card returns to the column it was dragged out of, in the order
 *   that column had before.
 * - The order write failed after a status write that succeeded. The move is
 *   real and must survive. `beforeReorder` is the board with the card already
 *   in its new column but the order untouched, which is exactly what a failed
 *   reorder should leave behind.
 * - The order write failed with no status change — a plain within-column
 *   reorder. `beforeDrag` is both the pre-drag and the pre-reorder state here,
 *   so it is the one to restore.
 *
 * Returns `null` when there is nothing to restore, which is not an error:
 *
 * - A drop whose drag start was never recorded has no previous state to name,
 *   and guessing at one would move cards the user did not touch.
 * - A drag is already in flight. The persist is awaited, so the user can pick
 *   up another card before the first drop's write comes back; restoring
 *   underneath that drag would yank a card out from under the pointer. A
 *   failure that arrives that late is dropped — the same condition the resync
 *   effect uses, for the same reason. The toast still stands either way.
 *
 * Lives apart from `KanbanView` because it is a rule about which state is
 * correct rather than about rendering, and because a test for it should not
 * have to mount a drag-and-drop board to ask which snapshot wins.
 */
export function rollbackTarget(args: {
  failed: FailedWrite;
  crossColumn: boolean;
  beforeDrag: ColMap | null;
  beforeReorder: ColMap;
  dragInFlight: boolean;
}): ColMap | null {
  const { failed, crossColumn, beforeDrag, beforeReorder, dragInFlight } = args;
  if (dragInFlight) return null;
  if (failed === "move") return beforeDrag;
  return crossColumn ? beforeReorder : beforeDrag;
}
