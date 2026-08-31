import type { Story } from "../../types/board";

/**
 * The ids of the stories agents would pick up next — one per assigned agent.
 *
 * Mirrors the scheduler's pick rule (`scheduler::next_ready_story_sql`), which
 * takes the first Ready story *for a given profile*. So "next" is per agent
 * rather than one global next: two agents draw from the same column
 * independently, and the top card is next for someone without being next for
 * the agent you have in mind. That is the whole reason the board marks it —
 * ordering alone does not say whose turn it is.
 *
 * A story nobody is assigned to is never picked at all, so it is never next.
 * Marking one would promise something that will not happen.
 *
 * Takes the column in the order it is already in, which is the order the
 * scheduler picks by (`db::story_status::queue_order_sql`). This function does
 * not sort: if those two orders ever diverge again, the chip is what makes it
 * visible rather than papering over it.
 *
 * Lives apart from `KanbanView` because it is a rule about the queue rather
 * than about rendering — and because a test for it should not have to mount a
 * drag-and-drop board to ask a question about ordering.
 */
export function nextUpIds(ready: Story[]): Set<string> {
  const seen = new Set<string>();
  const next = new Set<string>();

  for (const story of ready) {
    const agent = story.assignedAgentId;
    if (!agent || seen.has(agent)) continue;
    seen.add(agent);
    next.add(story.id);
  }

  return next;
}
