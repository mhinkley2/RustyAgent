// @vitest-environment node
import { describe, expect, it } from "vitest";

import { KANBAN_COLUMNS } from "./board";
import type { StoryStatus } from "./board";

/**
 * The canonical vocabulary, mirroring `db::story_status::STORY_STATUSES`.
 *
 * Five places used to spell out what a story's status may be and no two
 * agreed: the board drew six columns, `update_story_status` accepted five of
 * them plus a `failed` no column rendered, `create_story` accepted the right
 * six, `update_story` and the `list_stories` filter each accepted seven, and
 * two doc comments disagreed with all of it.
 *
 * `story_status.rs` asserts this same list from the Rust side. Changing either
 * without the other fails a build, which is the only mechanism that would have
 * caught the drift in the first place.
 */
const CANONICAL_STATUSES = [
  "backlog",
  "ready",
  "in_progress",
  "blocked",
  "review",
  "done",
] as const;

describe("story status vocabulary", () => {
  it("draws a column for every status a writer can produce", () => {
    expect(KANBAN_COLUMNS.map((c) => c.status)).toEqual([...CANONICAL_STATUSES]);
  });

  // The value that made this necessary. A card set to `failed` left every
  // column and could only be found through search.
  it("has no column the backend cannot write, and no status without a column", () => {
    const columns = new Set<string>(KANBAN_COLUMNS.map((c) => c.status));
    expect(columns.has("failed")).toBe(false);
    expect(columns.size).toBe(CANONICAL_STATUSES.length);
  });

  // Every finished run now lands its card in `review`, so a board that could
  // not draw that column would hide the routine outcome of the product.
  it("draws the column every finished run lands in", () => {
    expect(KANBAN_COLUMNS.some((c) => c.status === "review")).toBe(true);
  });

  it("gives every column a label", () => {
    for (const column of KANBAN_COLUMNS) {
      expect(column.label.trim()).not.toBe("");
    }
  });

  // A compile-time check in both directions.
  //
  // `Record<StoryStatus, true>` requires a key for every member of the union,
  // so losing one is a missing-key error and gaining one is an excess-property
  // error. An earlier version of this asserted
  // `const x: StoryStatus[] = [...CANONICAL_STATUSES]`, which only proved the
  // canonical list was a *subset* — a union that grew an extra member would
  // have compiled happily, which is the exact drift this file exists to catch.
  it("keeps the union and the canonical list in step", () => {
    const everyMember: Record<StoryStatus, true> = {
      backlog: true,
      ready: true,
      in_progress: true,
      blocked: true,
      review: true,
      done: true,
    };

    expect(Object.keys(everyMember).sort()).toEqual([...CANONICAL_STATUSES].sort());
  });
});
