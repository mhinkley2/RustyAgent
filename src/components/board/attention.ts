import type { HumanRequest, ApprovalRequest } from "../../types/human";

/**
 * What a single story is waiting on a person for.
 *
 * `requestId` and `kind` are the click target: the marker on a card has to
 * reopen one specific request, not "whatever is first" — the whole defect this
 * replaces was UI that could only ever reach the first non-dismissed request.
 */
export interface StoryAttention {
  /** Pending input questions raised by this story's runs. */
  inputs: number;
  /** Pending tool-call approvals raised by this story's runs. */
  approvals: number;
  /** The request the marker opens. */
  requestId: string;
  /** Which dialog that request belongs to. */
  kind: "input" | "approval";
}

/**
 * Which stories are blocked on a human, keyed by story id.
 *
 * Both request kinds already knew their run; what they now carry is the story
 * behind that run, which is the only thing that lets a count in a banner become
 * a mark on a card.
 *
 * Requests with no story — a question raised outside a run, an approval whose
 * story was deleted — are counted by the banner and skipped here. They are
 * still reachable through the banner's action; there is simply no card to put
 * them on, and inventing one would be worse than the banner.
 *
 * A story with both kinds pending resolves to the input request, matching the
 * dialogs' own precedence (`BoardPage` renders the approval gate only when no
 * input dialog is up). The marker opening one dialog and the board then showing
 * the other would be its own small lie.
 */
export function attentionByStory(
  humanRequests: HumanRequest[],
  approvalRequests: ApprovalRequest[],
): Map<string, StoryAttention> {
  const byStory = new Map<string, StoryAttention>();

  const bump = (
    storyId: string | null,
    kind: "input" | "approval",
    requestId: string,
  ) => {
    if (!storyId) return;

    const existing = byStory.get(storyId);
    if (!existing) {
      byStory.set(storyId, {
        inputs: kind === "input" ? 1 : 0,
        approvals: kind === "approval" ? 1 : 0,
        requestId,
        kind,
      });
      return;
    }

    if (kind === "input") existing.inputs += 1;
    else existing.approvals += 1;

    // Inputs win the click even when an approval got here first.
    if (kind === "input" && existing.kind === "approval") {
      existing.requestId = requestId;
      existing.kind = "input";
    }
  };

  for (const request of humanRequests) bump(request.taskStoryId, "input", request.id);
  for (const request of approvalRequests) bump(request.storyId, "approval", request.id);

  return byStory;
}

/** How many things this story is holding a person up on. */
export function attentionCount(attention: StoryAttention): number {
  return attention.inputs + attention.approvals;
}

/** The marker's tooltip, spelling out what the count is made of. */
export function attentionLabel(attention: StoryAttention): string {
  const parts: string[] = [];
  if (attention.inputs > 0) {
    parts.push(`${attention.inputs} question${attention.inputs === 1 ? "" : "s"}`);
  }
  if (attention.approvals > 0) {
    parts.push(`${attention.approvals} approval${attention.approvals === 1 ? "" : "s"}`);
  }
  return `Waiting on you — ${parts.join(", ")}`;
}
