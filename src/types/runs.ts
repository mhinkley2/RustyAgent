// Run history types — mirrors the Rust StoryRun / RunEvent structs.

export type RunStatus = "running" | "done" | "failed" | "cancelled";

export type RunEventType =
  | "message"
  | "tool_call"
  | "tool_result"
  | "thought"
  | "error"
  | "approval_request"
  | "approval_response"
  /** History was dropped to keep the request inside the model's input budget. */
  | "context_compacted";

export interface StoryRun {
  id: string;
  storyId: string;
  storyTitle: string | null;
  agentProfileId: string;
  agentName: string | null;
  status: RunStatus;
  /** Input tokens billed at the full rate; cached input is counted separately. */
  inputTokens: number;
  outputTokens: number;
  /** Input tokens served from the provider's prompt cache. */
  cacheReadTokens: number;
  /** Input tokens written into the provider's prompt cache. */
  cacheCreationTokens: number;
  /** Estimate from the per-model price table; 0 when the model is unpriced. */
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

/**
 * Every input token the run's provider calls read, cached or not.
 *
 * `inputTokens` alone is only the uncached remainder, so a run with a warm
 * prompt cache would otherwise appear to have read almost nothing.
 */
export function totalInputTokens(run: StoryRun): number {
  return run.inputTokens + run.cacheReadTokens + run.cacheCreationTokens;
}

export function totalTokens(run: StoryRun): number {
  return totalInputTokens(run) + run.outputTokens;
}

/**
 * Cost for display, distinguishing "free" from "not priced".
 *
 * A run on a model missing from the price table records real tokens against no
 * cost. Rendering that as $0.00 would state a number the app does not know, so
 * it shows an em dash instead.
 */
export function formatEstimatedCost(run: StoryRun): string {
  if (run.estimatedCostUsd === 0 && totalTokens(run) > 0) return "—";
  return formatCost(run.estimatedCostUsd);
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
