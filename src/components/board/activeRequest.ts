import type { HumanRequest, ApprovalRequest } from "../../types/human";

/** Which dialog, if any, the board should be showing. */
export interface ActiveRequests {
  human: HumanRequest | null;
  approval: ApprovalRequest | null;
}

/**
 * The request in front of the user right now.
 *
 * Two rules, in order:
 *
 * 1. An explicit focus — a card's marker, a banner's button — wins. Without
 *    this the only reachable request was the first *non-dismissed* one, so
 *    dismissing everything left a blocked run with no UI that could reopen it.
 *    The button meant to bring one back had nothing to act on.
 * 2. Otherwise, the first request nobody has dismissed.
 *
 * The two dialogs never appear together: the approval gate renders only when
 * no input dialog does. That is deliberate and stays. It does mean a focused
 * approval has to suppress the input dialog — otherwise clicking an approval
 * marker while any question is outstanding opens the wrong one and looks
 * broken in the same way the old code did.
 *
 * A pure function rather than a tangle of `??` inside the page, because this is
 * where both defects lived and a test should be able to reach it without
 * mounting a drag-and-drop board.
 */
export function activeRequests(
  humanRequests: HumanRequest[],
  approvalRequests: ApprovalRequest[],
  dismissedHumanIds: ReadonlySet<string>,
  dismissedApprovalIds: ReadonlySet<string>,
  focusedRequestId: string | null,
): ActiveRequests {
  const focusedHuman = humanRequests.find(r => r.id === focusedRequestId) ?? null;
  const focusedApproval = approvalRequests.find(r => r.id === focusedRequestId) ?? null;

  const approval =
    focusedApproval ?? approvalRequests.find(r => !dismissedApprovalIds.has(r.id)) ?? null;

  const human = focusedApproval
    ? null
    : focusedHuman ?? humanRequests.find(r => !dismissedHumanIds.has(r.id)) ?? null;

  return { human, approval };
}

/**
 * Drop `id` from a dismissal set, keeping the same set when it was not there.
 *
 * Returning the identical reference matters: these are React state setters, and
 * a fresh `Set` every poll would re-render the page on a timer for no reason.
 */
export function undismiss(prev: Set<string>, id: string): Set<string> {
  if (!prev.has(id)) return prev;
  const next = new Set(prev);
  next.delete(id);
  return next;
}

/**
 * Drop dismissals of requests that no longer exist.
 *
 * Dismissal is keyed by request id, so a stale entry cannot hide a *new*
 * request — a new one gets a new id. What it can do is accumulate for the life
 * of the page, remembering decisions about things that are gone. Same identity
 * rule as [`undismiss`]: unchanged means the same set back.
 */
export function pruneDismissed(prev: Set<string>, liveIds: ReadonlySet<string>): Set<string> {
  const kept = [...prev].filter(id => liveIds.has(id));
  return kept.length === prev.size ? prev : new Set(kept);
}
