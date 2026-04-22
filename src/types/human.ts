// ---------------------------------------------------------------------------
// Types for human-in-the-loop feature (RUSTYAGE-6)
// ---------------------------------------------------------------------------

export interface HumanRequest {
  id: string;
  storyId: string;
  storyTitle: string;
  runId: string | null;
  question: string | null;
  status: string;
  createdAt: string;
}

export interface ApprovalRequest {
  id: string;
  runId: string;
  storyTitle: string | null;
  toolName: string;
  toolInput: string; // JSON string
  status: "pending" | "approved" | "rejected";
  createdAt: string;
}
