/**
 * Where one dragged card lands in a column whose other cards a filter hid.
 *
 * The Kanban renders `filteredStories`, so with a filter active a column shows
 * a subset of what it holds. Numbering the rendered column `0..n` — which is
 * what persisting the visible order does — hands a hidden card and a visible
 * card the same `sortOrder`, and the column comes back scrambled the moment
 * the filter clears. On the Ready column that is not cosmetic: `sort_order` is
 * what settles the order agents pick in, within a priority band
 * (`db::story_status::queue_order_sql` sorts by priority first, then
 * `sort_order`). Colliding it reshuffles the band.
 *
 * The rule here is the narrowest one that matches what the drag actually
 * expressed: **one card moved, and everything else stayed put.** The moved
 * card is lifted out of the full column and reinserted immediately before the
 * visible card that now follows it — the card the user dropped it above. The
 * hidden cards are never renumbered relative to each other.
 *
 * Dropped at the bottom of the visible list, it lands after every hidden card
 * too. There is no visible follower to anchor to, and putting it above a card
 * the user could not see would be a guess.
 *
 * With no filter active `full` and `visibleAfter` are the same list and this
 * returns exactly what numbering the visible order would have.
 *
 * A card arriving from another column is not in `full` yet; it is taken from
 * `visibleAfter` and inserted by the same rule.
 */
export function placeInFullColumn<T extends { id: string }>(
  full: T[],
  visibleAfter: T[],
  movedId: string,
): T[] {
  const moved =
    visibleAfter.find(s => s.id === movedId) ?? full.find(s => s.id === movedId);
  // Nothing to place. Returning `full` unchanged keeps the caller from writing
  // an order it did not mean to.
  if (!moved) return full;

  const rest = full.filter(s => s.id !== movedId);

  const movedAt = visibleAfter.findIndex(s => s.id === movedId);
  const follower = movedAt === -1 ? undefined : visibleAfter[movedAt + 1];
  const insertAt = follower ? rest.findIndex(s => s.id === follower.id) : -1;

  if (insertAt === -1) return [...rest, moved];
  return [...rest.slice(0, insertAt), moved, ...rest.slice(insertAt)];
}
