// Run history types — mirrors the Rust StoryRun / RunEvent structs.

export type RunStatus = "running" | "done" | "failed" | "cancelled";

export type RunEventType =
  | "message"
  | "tool_call"
  | "tool_result"
  | "thought"
  | "error"
  | "approval_request"
  | "approval_response";

export interface StoryRun {
  id: string;
  storyId: string;
  storyTitle: string | null;
  agentProfileId: string;
  agentName: string | null;
  status: RunStatus;
  inputTokens: number;
  outputTokens: number;
  estimatedCostUsd: number;
  iterationCount: number;
  startedAt: Date;
  finishedAt: Date | null;
  durationSecs: number | null;
  /** Git HEAD SHA at run start; null if workspace is not a git repo. */
  beforeSha: string | null;
}

/** Git diff payload — fetched separately due to potentially large size. */
export interface RunDiff {
  runId: string;
  beforeSha: string | null;
  diffOutput: string | null;
}

export interface RunEvent {
  id: string;
  runId: string;
  eventType: RunEventType;
  role: string | null;        // 'user' | 'assistant' | 'tool'
  content: string | null;
  toolName: string | null;
  toolInput: string | null;   // JSON string
  toolOutput: string | null;  // JSON string
  isError: boolean;
  sequenceNum: number;
  createdAt: Date;
}

export interface RunFilters {
  storyId?: string;
  agentProfileId?: string;
  status?: RunStatus;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

export function formatCost(usd: number): string {
  if (usd === 0) return "$0.00";
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  return `$${usd.toFixed(2)}`;
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function formatDuration(secs: number | null): string {
  if (secs == null) return "—";
  if (secs < 60) return `${Math.round(secs)}s`;
  const m = Math.floor(secs / 60);
  const s = Math.round(secs % 60);
  return s > 0 ? `${m}m ${s}s` : `${m}m`;
}

export const RUN_STATUS_LABELS: Record<RunStatus, string> = {
  running:   "Running",
  done:      "Done",
  failed:    "Failed",
  cancelled: "Cancelled",
};
