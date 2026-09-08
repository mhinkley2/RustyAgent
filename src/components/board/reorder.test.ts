// @vitest-environment node
import { describe, expect, it } from "vitest";

import { placeInFullColumn } from "./reorder";

const card = (id: string) => ({ id });
const ids = (list: { id: string }[]) => list.map(c => c.id);

describe("placeInFullColumn", () => {
  // With nothing hidden this has to agree with what numbering the rendered
  // column already did, or turning the filter on would change how a plain
  // drag behaves.
  it("reduces to the visible order when no filter is hiding anything", () => {
    const full = [card("a"), card("b"), card("c")];
    const visibleAfter = [card("b"), card("a"), card("c")];

    expect(ids(placeInFullColumn(full, visibleAfter, "a"))).toEqual(["b", "a", "c"]);
  });

  it("leaves hidden cards where they were and moves only the dragged one", () => {
    // b and d are filtered out; the user drags c above a.
    const full = [card("a"), card("b"), card("c"), card("d")];
    const visibleAfter = [card("c"), card("a")];

    // c lands immediately before a, its visible follower. b stays after a and
    // before d, exactly as it was — it is not dragged along by a move it had
    // no part in.
    expect(ids(placeInFullColumn(full, visibleAfter, "c"))).toEqual(["c", "a", "b", "d"]);
  });

  // No visible follower means no card to anchor above, and guessing a position
  // among cards the user could not see would be worse than the honest answer.
  it("puts a card dropped at the bottom of the visible list after the hidden tail", () => {
    const full = [card("a"), card("b"), card("c")];
    const visibleAfter = [card("c"), card("a")];

    expect(ids(placeInFullColumn(full, visibleAfter, "a"))).toEqual(["b", "c", "a"]);
  });

  it("inserts a card arriving from another column, which is not in the full list yet", () => {
    const full = [card("a"), card("b")];
    const visibleAfter = [card("new"), card("b")];

    expect(ids(placeInFullColumn(full, visibleAfter, "new"))).toEqual(["a", "new", "b"]);
  });

  it("appends a card arriving from another column onto the end", () => {
    const full = [card("a"), card("b")];
    const visibleAfter = [card("a"), card("new")];

    expect(ids(placeInFullColumn(full, visibleAfter, "new"))).toEqual(["a", "b", "new"]);
  });

  it("returns the column untouched when the moved id is in neither list", () => {
    const full = [card("a"), card("b")];

    expect(ids(placeInFullColumn(full, [card("a")], "ghost"))).toEqual(["a", "b"]);
  });

  it("never drops or duplicates a card", () => {
    const full = [card("a"), card("b"), card("c"), card("d"), card("e")];
    const visibleAfter = [card("e"), card("b")];

    const out = placeInFullColumn(full, visibleAfter, "e");
    expect(ids(out).sort()).toEqual(["a", "b", "c", "d", "e"]);
    expect(out).toHaveLength(full.length);
  });
});
