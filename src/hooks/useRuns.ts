import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useState, useCallback, useEffect } from "react";
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
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!runId) {
      setEvents([]);
      return;
    }
    setLoading(true);
    setError(null);
    invoke<RawEvent[]>("get_run_events", { runId })
      .then(raw => setEvents(raw.map(mapEvent)))
      .catch(e => setError(String(e)))
      .finally(() => setLoading(false));
  }, [runId]);

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
