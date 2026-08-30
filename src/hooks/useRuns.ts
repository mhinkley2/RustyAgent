import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useState, useCallback, useEffect, useMemo } from "react";
import type { StoryRun, RunEvent, RunFilters, RunDiff } from "../types/runs";

// ---------------------------------------------------------------------------
// Raw backend types (snake_case from serde)
// ---------------------------------------------------------------------------

interface RawRun {
  id: string;
  story_id: string;
  story_title: string | null;
  agent_profile_id: string;
  agent_name: string | null;
  status: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_input_tokens: number;
  cache_creation_input_tokens: number;
  estimated_cost_usd: number;
  iteration_count: number;
  started_at: string;
  finished_at: string | null;
  duration_secs: number | null;
  before_sha: string | null;
  worktree_path: string | null;
  branch_name: string | null;
  after_sha: string | null;
  isolation_status: string | null;
  isolation_note: string | null;
}

interface RawEvent {
  id: string;
  run_id: string;
  event_type: string;
  role: string | null;
  content: string | null;
  tool_name: string | null;
  tool_input: string | null;
  tool_output: string | null;
  is_error: boolean;
  sequence_num: number;
  created_at: string;
}

// ---------------------------------------------------------------------------
// Mappers
// ---------------------------------------------------------------------------

function mapRun(r: RawRun): StoryRun {
  return {
    id:               r.id,
    storyId:          r.story_id,
    storyTitle:       r.story_title,
    agentProfileId:   r.agent_profile_id,
    agentName:        r.agent_name,
    status:           r.status as StoryRun["status"],
    inputTokens:      r.input_tokens,
    outputTokens:     r.output_tokens,
    cacheReadTokens:     r.cache_read_input_tokens,
    cacheCreationTokens: r.cache_creation_input_tokens,
    estimatedCostUsd: r.estimated_cost_usd,
    iterationCount:   r.iteration_count,
    startedAt:        new Date(r.started_at),
    finishedAt:       r.finished_at ? new Date(r.finished_at) : null,
    durationSecs:     r.duration_secs,
    beforeSha:        r.before_sha,
    worktreePath:     r.worktree_path,
    branchName:       r.branch_name,
    afterSha:         r.after_sha,
    isolationStatus:  r.isolation_status as StoryRun["isolationStatus"],
    isolationNote:    r.isolation_note,
  };
}

function mapEvent(e: RawEvent): RunEvent {
  return {
    id:          e.id,
    runId:       e.run_id,
    eventType:   e.event_type as RunEvent["eventType"],
    role:        e.role,
    content:     e.content,
    toolName:    e.tool_name,
    toolInput:   e.tool_input,
    toolOutput:  e.tool_output,
    isError:     e.is_error,
    sequenceNum: e.sequence_num,
    createdAt:   new Date(e.created_at),
  };
}

// ---------------------------------------------------------------------------
// Live run events
// ---------------------------------------------------------------------------

/**
 * The `run-event` payload — `runtime::RunEvent` serialized with its
 * `#[serde(tag = "type")]` discriminator.
 *
 * A different shape from {@link RunEvent}, which mirrors a `run_events` *row*.
 * The two meet in {@link liveEventToRow}.
 */
type LiveRunEvent =
  | { type: "token"; run_id: string; content: string }
  | { type: "tool_call"; run_id: string; tool_name: string; input: unknown }
  | { type: "tool_result"; run_id: string; tool_name: string; output: string; is_error: boolean }
  | { type: "awaiting_approval"; run_id: string; approval_request_id: string; tool_name: string }
  | {
      type: "approval_resolved";
      run_id: string;
      approval_request_id: string;
      tool_name: string;
      approved: boolean;
      outcome: string;
    }
  | { type: "complete"; run_id: string; stop_reason: string }
  | { type: "cancelled"; run_id: string }
  | { type: "failed"; run_id: string; message: string }
  | { type: "context_compacted"; run_id: string };

/**
 * Whether a row is assistant text still being streamed, and so may absorb the
 * next token.
 *
 * Recognised by shape because {@link liveEventToRow} produces this shape for
 * `token` and nothing else. Note that the *database* keeps one `token` row per
 * delta, so a refresh does not return the coalesced row this builds — the live
 * view is the tidier of the two, not the divergent one.
 */
function isStreamedText(row: Omit<RunEvent, "sequenceNum">): boolean {
  return row.eventType === "message" && row.role === "assistant";
}

/**
 * Turn a live event into the row the timeline renders.
 *
 * Only the kinds the runtime also *persists* are mapped, and they are mapped
 * to the same `event_type` and columns `persist_event` writes. That is the
 * property worth keeping: reopening the panel refetches from the database, and
 * anything appended live that the database would not return turns a refresh
 * into an unexplained disappearance. `null` means "watch it happen, but do not
 * write it into the timeline".
 */
function liveEventToRow(
  event: LiveRunEvent,
  runId: string,
): Omit<RunEvent, "sequenceNum"> | null {
  const base = {
    id: `live-${runId}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    runId,
    role: null,
    content: null,
    toolName: null,
    toolInput: null,
    toolOutput: null,
    isError: false,
    createdAt: new Date(),
  };

  switch (event.type) {
    case "token":
      return { ...base, eventType: "message", role: "assistant", content: event.content };
    case "tool_call":
      return {
        ...base,
        eventType: "tool_call",
        toolName: event.tool_name,
        toolInput: JSON.stringify(event.input),
      };
    case "tool_result":
      return {
        ...base,
        eventType: "tool_result",
        toolName: event.tool_name,
        toolOutput: event.output,
        isError: event.is_error,
      };
    case "awaiting_approval":
      return {
        ...base,
        eventType: "approval_request",
        toolName: event.tool_name,
        content: `Waiting for you to approve '${event.tool_name}'.`,
      };
    case "approval_resolved":
      return {
        ...base,
        eventType: "approval_response",
        toolName: event.tool_name,
        content: event.approved
          ? `'${event.tool_name}' was approved.`
          : `'${event.tool_name}' was not run (${event.outcome}).`,
      };
    case "failed":
      return { ...base, eventType: "error", content: event.message, isError: true };
    default:
      // `complete`, `cancelled` and `context_compacted` end up in the timeline
      // through their own persisted rows, which carry detail this payload does
      // not; duplicating them here would double them on the next refresh.
      return null;
  }
}

// ---------------------------------------------------------------------------
// useRuns
// ---------------------------------------------------------------------------

interface UseRunsReturn {
  runs: StoryRun[];
  loading: boolean;
  error: string | null;
  refresh: (filters?: RunFilters) => Promise<void>;
  deleteRun: (id: string) => Promise<void>;
}

export function useRuns(initialFilters?: RunFilters): UseRunsReturn {
  const [runs, setRuns] = useState<StoryRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async (filters?: RunFilters) => {
    setLoading(true);
    setError(null);
    try {
      const raw = await invoke<RawRun[]>("get_runs", { filters: filters ?? null });
      setRuns(raw.map(mapRun));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh(initialFilters);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refresh]);

  // Refresh whenever the active workspace changes.
  useEffect(() => {
    const unlisten = listen("workspace-changed", () => { refresh(initialFilters); });
    return () => { unlisten.then(fn => fn()); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refresh]);

  const deleteRun = useCallback(async (id: string) => {
    await invoke("delete_run", { id });
    setRuns(prev => prev.filter(r => r.id !== id));
  }, []);

  return { runs, loading, error, refresh, deleteRun };
}

// ---------------------------------------------------------------------------
// useRunEvents — load events for a single run on demand
// ---------------------------------------------------------------------------

interface UseRunEventsReturn {
  events: RunEvent[];
  loading: boolean;
  error: string | null;
}

export function useRunEvents(runId: string | null): UseRunEventsReturn {
  const [fetched, setFetched] = useState<RunEvent[]>([]);
  const [live, setLive] = useState<Omit<RunEvent, "sequenceNum">[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setLive([]);
    if (!runId) {
      setFetched([]);
      return;
    }
    setLoading(true);
    setError(null);
    invoke<RawEvent[]>("get_run_events", { runId })
      .then(raw => setFetched(raw.map(mapEvent)))
      .catch(e => setError(String(e)))
      .finally(() => setLoading(false));
  }, [runId]);

  // Follow the run as it executes.
  //
  // Without this the timeline is whatever the fetch above returned, so an
  // autonomous run — the kind nobody is sitting in front of — could only be
  // watched by closing the panel and opening it again. The subscription is
  // filtered to `runId`: every run in the app emits on the same `run-event`
  // channel, and an unfiltered listener would interleave three agents' tool
  // calls into one timeline.
  //
  // Live events accumulate separately from the fetched ones rather than being
  // appended to them, so an event that arrives while the fetch is still in
  // flight is not wiped by the response when it lands.
  useEffect(() => {
    if (!runId) return;

    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void listen<LiveRunEvent>("run-event", ({ payload }) => {
      if (payload.run_id !== runId) return;
      const row = liveEventToRow(payload, runId);
      if (!row) return;
      setLive(prev => {
        // Grow the assistant's message rather than appending a row per token.
        // The runtime emits one `Token` per text delta, so a long reply would
        // otherwise add a thousand rows and re-render every one of them a
        // thousand times.
        const last = prev[prev.length - 1];
        if (isStreamedText(row) && last && isStreamedText(last)) {
          const merged = { ...last, content: (last.content ?? "") + (row.content ?? "") };
          return [...prev.slice(0, -1), merged];
        }
        return [...prev, row];
      });
    }).then(fn => {
      // `listen` resolves after an await, by which point the effect may
      // already have been torn down; without this the handler outlives the
      // panel and leaks one subscription per open.
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [runId]);

  const events = useMemo(() => {
    // Continue the fetched rows' own numbering rather than counting them.
    // `get_run_events` returns a whole run ordered by `sequence_num`, and
    // pruning drops whole runs rather than rows within one, so today the count
    // and the last number agree. Deriving it from the row means they cannot
    // disagree later either — if that query ever grows a limit, counting would
    // silently hand two rows the same number.
    const last = fetched[fetched.length - 1];
    const base = last ? last.sequenceNum + 1 : 0;
    return [
      ...fetched,
      ...live.map((row, i) => ({ ...row, sequenceNum: base + i })),
    ];
  }, [fetched, live]);

  return { events, loading, error };
}

// ---------------------------------------------------------------------------
// exportRun — trigger a .jsonl download of run events
// ---------------------------------------------------------------------------

export async function exportRun(runId: string, filename: string): Promise<void> {
  const json = await invoke<string>("export_run_events", { runId });
  const events: unknown[] = JSON.parse(json);
  const jsonl = events.map(e => JSON.stringify(e)).join("\n");
  const blob = new Blob([jsonl], { type: "application/jsonl" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

// ---------------------------------------------------------------------------
// useRunDiff — lazily fetch the git diff for a single run
// ---------------------------------------------------------------------------

interface UseRunDiffReturn {
  diff: RunDiff | null;
  loading: boolean;
  error: string | null;
}

export function useRunDiff(runId: string | null): UseRunDiffReturn {
  const [diff, setDiff] = useState<RunDiff | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!runId) {
      setDiff(null);
      return;
    }
    setLoading(true);
    setError(null);
    invoke<{ run_id: string; before_sha: string | null; diff_output: string | null }>(
      "get_run_diff",
      { runId }
    )
      .then(raw => setDiff({ runId: raw.run_id, beforeSha: raw.before_sha, diffOutput: raw.diff_output }))
      .catch(e => setError(String(e)))
      .finally(() => setLoading(false));
  }, [runId]);

  return { diff, loading, error };
}

// ---------------------------------------------------------------------------
// Accepting and reverting an isolated run
// ---------------------------------------------------------------------------

/**
 * Merge a finished run's branch into the user's working tree.
 *
 * The backend uses `git merge --squash`, so the changes land staged and
 * uncommitted, and git refuses rather than overwriting uncommitted local work.
 * Resolves with the backend's description of what happened.
 */
export async function acceptRun(runId: string): Promise<string> {
  return invoke<string>("accept_run", { runId });
}

/**
 * Throw a finished run's changes away.
 *
 * Deletes only the run's own worktree and branch. The user's working tree is
 * never written to, which is why this is an exact undo rather than a
 * best-effort one.
 */
export async function revertRun(runId: string): Promise<string> {
  return invoke<string>("revert_run", { runId });
}
