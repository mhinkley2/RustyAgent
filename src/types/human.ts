// ---------------------------------------------------------------------------
// Types for human-in-the-loop feature (RUSTYAGE-6)
// ---------------------------------------------------------------------------

export interface HumanRequest {
  id: string;
  /** The synthetic `human` story that carries the question. */
  storyId: string;
  /**
   * The task story whose run asked, or `null` when there is no such card.
   *
   * `storyId` is the human story, which the board keeps out of its columns on
   * purpose — marking it would mark a card nobody is looking at. This is the
   * work that is actually blocked.
   */
  taskStoryId: string | null;
  storyTitle: string;
  runId: string | null;
  question: string | null;
  status: string;
  createdAt: string;
}

export interface ApprovalRequest {
  id: string;
  runId: string;
  /** The story whose run wants the tool call; `null` if it has been deleted. */
  storyId: string | null;
  storyTitle: string | null;
  toolName: string;
  toolInput: string; // JSON string
  status: "pending" | "approved" | "rejected";
  createdAt: string;
}
