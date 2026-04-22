// Hook for polling pipeline progress.
import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PipelineProgress } from "../types/board";

const POLL_INTERVAL_MS = 2000;

/**
 * Returns current progress for a specific pipeline run, polled every 2 seconds.
 * Stops polling once the run is done/failed/cancelled.
 */
export function usePipelineProgress(pipelineRunId: string | null): {
  progress: PipelineProgress | null;
  startPipelineRun: (storyId: string, profileId: string) => Promise<string>;
} {
  const [progress, setProgress] = useState<PipelineProgress | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetchProgress = useCallback(async (runId: string) => {
    try {
      const p = await invoke<PipelineProgress | null>("get_pipeline_progress", {
        pipelineRunId: runId,
      });
      setProgress(p);
      // Stop polling when terminal state reached
      if (p && (p.status === "done" || p.status === "failed" || p.status === "cancelled")) {
        if (timerRef.current) {
          clearInterval(timerRef.current);
          timerRef.current = null;
        }
      }
    } catch {
      // non-critical
    }
  }, []);

  useEffect(() => {
    if (!pipelineRunId) {
      setProgress(null);
      return;
    }

    fetchProgress(pipelineRunId);
    timerRef.current = setInterval(() => fetchProgress(pipelineRunId), POLL_INTERVAL_MS);

    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [pipelineRunId, fetchProgress]);

  const startPipelineRun = useCallback(async (storyId: string, profileId: string): Promise<string> => {
    const runId = await invoke<string>("start_pipeline_run", { storyId, profileId });
    return runId;
  }, []);

  return { progress, startPipelineRun };
}

/**
 * Returns all currently active pipeline runs, polled every 3 seconds.
 */
export function useActivePipelines(): PipelineProgress[] {
  const [pipelines, setPipelines] = useState<PipelineProgress[]>([]);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetchAll = useCallback(async () => {
    try {
      const list = await invoke<PipelineProgress[]>("list_active_pipelines");
      setPipelines(list);
    } catch {
      // non-critical
    }
  }, []);

  useEffect(() => {
    fetchAll();
    timerRef.current = setInterval(fetchAll, 3000);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [fetchAll]);

  return pipelines;
}
