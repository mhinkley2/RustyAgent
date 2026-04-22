// Hook for polling agent runtime statuses from the scheduler.
import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AgentRuntimeStatus } from "../types/agent";

const POLL_INTERVAL_MS = 3000;

/**
 * Returns a map of profileId → AgentRuntimeStatus, polled every 3 seconds.
 */
export function useAgentStatuses(): {
  statuses: Record<string, AgentRuntimeStatus>;
  startContinuous: (profileId: string, pollIntervalSecs?: number) => Promise<void>;
  startScheduled: (profileId: string, cronExpression: string) => Promise<void>;
  stopScheduler: (profileId: string) => Promise<void>;
} {
  const [statuses, setStatuses] = useState<Record<string, AgentRuntimeStatus>>({});
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetchStatuses = useCallback(async () => {
    try {
      const list = await invoke<AgentRuntimeStatus[]>("get_all_agent_runtime_statuses");
      const map: Record<string, AgentRuntimeStatus> = {};
      for (const s of list) {
        map[s.profileId] = s;
      }
      setStatuses(map);
    } catch {
      // non-critical — ignore polling errors
    }
  }, []);

  useEffect(() => {
    fetchStatuses();
    timerRef.current = setInterval(fetchStatuses, POLL_INTERVAL_MS);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [fetchStatuses]);

  const startContinuous = useCallback(async (profileId: string, pollIntervalSecs = 30) => {
    await invoke("start_continuous_mode", { profileId, pollIntervalSecs });
    await fetchStatuses();
  }, [fetchStatuses]);

  const startScheduled = useCallback(async (profileId: string, cronExpression: string) => {
    await invoke("start_scheduled_mode", { profileId, cronExpression });
    await fetchStatuses();
  }, [fetchStatuses]);

  const stopScheduler = useCallback(async (profileId: string) => {
    await invoke("stop_agent_scheduler", { profileId });
    await fetchStatuses();
  }, [fetchStatuses]);

  return { statuses, startContinuous, startScheduled, stopScheduler };
}

/**
 * Single-profile convenience hook.
 */
export function useAgentStatus(profileId: string): {
  status: AgentRuntimeStatus | null;
  startContinuous: () => Promise<void>;
  stopScheduler: () => Promise<void>;
} {
  const [status, setStatus] = useState<AgentRuntimeStatus | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetch = useCallback(async () => {
    try {
      const s = await invoke<AgentRuntimeStatus>("get_agent_runtime_status", { profileId });
      setStatus(s);
    } catch {
      // ignore
    }
  }, [profileId]);

  useEffect(() => {
    fetch();
    timerRef.current = setInterval(fetch, POLL_INTERVAL_MS);
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [fetch]);

  const startContinuous = useCallback(async () => {
    await invoke("start_continuous_mode", { profileId });
    await fetch();
  }, [profileId, fetch]);

  const stopScheduler = useCallback(async () => {
    await invoke("stop_agent_scheduler", { profileId });
    await fetch();
  }, [profileId, fetch]);

  return { status, startContinuous, stopScheduler };
}
