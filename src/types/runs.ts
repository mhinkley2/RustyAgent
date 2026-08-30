// Run history types — mirrors the Rust StoryRun / RunEvent structs.

export type RunStatus = "running" | "done" | "failed" | "cancelled";

export type RunEventType =
  | "message"
  /** Whether — and how — the run was isolated from the user's checkout. */
  | "isolation"
  | "tool_call"
  | "tool_result"
  | "thought"
  | "error"
  | "approval_request"
  | "approval_response"
  /** History was dropped to keep the request inside the model's input budget. */
  | "context_compacted"
  /**
   * A provider call failed transiently and is being tried again, after the
   * delay the provider asked for or a capped backoff. Written before the wait,
   * so a run that sits still for thirty seconds says why while it happens.
   */
  | "retry"
  /**
   * The run moved its story's card off `in_progress`, and this says where to
   * and why. Written by all three paths that move a card on their own — a run
   * finishing, a pipeline finishing, and the startup sweep — so a card that
   * moves is always attributable to the run that moved it.
   *
   * Distinct from `interrupted`, which says the *run* was ended by a restart.
   * A swept run carries both: what happened to it, and what became of its card.
   */
  | "story_status"
  /**
   * The app exited while the run was still executing, and the startup sweep
   * closed the run out. Written only by `db::recovery::reconcile_orphaned_runs`
   * — the run itself was never there to write it.
   *
   * The run's `status` is plain `failed`: the reason for the failure lives
   * here rather than in a fifth `RunStatus` no filter or badge would know.
   */
  | "interrupted";

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
  /** Absolute path of the isolated worktree the run executed in. */
  worktreePath: string | null;
  /** Branch the run's worktree had checked out. */
  branchName: string | null;
  /** Commit made on that branch when the run finished; null if it changed nothing. */
  afterSha: string | null;
  /** How the run was isolated, and what has since been decided about it. */
  isolationStatus: IsolationStatus | null;
  /** Why a run was not isolated, or what was surprising about the one that was. */
  isolationNote: string | null;
}

/**
 * `story_runs.isolation_status`.
 *
 * `null` means the run predates worktree isolation. Only an `isolated` run can
 * be accepted or reverted — there is nothing of RustyAgent's own to apply or
 * throw away for any of the others.
 */
export type IsolationStatus =
  /** Ran in its own git worktree; awaiting a decision. */
  | "isolated"
  /** The workspace is not a git repository. Ran in the user's directory. */
  | "not_a_git_repo"
  /** A git repository, but a worktree could not be made. Ran un-isolated. */
  | "unavailable"
  /** The run had no workspace directory at all. */
  | "no_workspace"
  /** Its changes were merged into the user's working tree. */
  | "accepted"
  /** Its worktree and branch were thrown away. */
  | "reverted";

/** Whether this run still has a worktree and branch to accept or revert. */
export function isDecidable(run: StoryRun): boolean {
  return run.isolationStatus === "isolated" && run.status !== "running";
}

/** Whether the run wrote into the user's own checkout rather than a worktree. */
export function ranUnisolated(run: StoryRun): boolean {
  return (
    run.isolationStatus === "not_a_git_repo" ||
    run.isolationStatus === "unavailable" ||
    run.isolationStatus === "no_workspace"
  );
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
