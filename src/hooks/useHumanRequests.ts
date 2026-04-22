import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useState, useEffect, useCallback, useRef } from "react";
import type { HumanRequest, ApprovalRequest } from "../types/human";

// ---------------------------------------------------------------------------
// Mapping helpers
// ---------------------------------------------------------------------------

function mapRequest(raw: Record<string, unknown>): HumanRequest {
  return {
    id:          String(raw.id ?? ""),
    storyId:     String(raw.story_id ?? raw.id ?? ""),
    storyTitle:  String(raw.story_title ?? ""),
    runId:       raw.run_id != null ? String(raw.run_id) : null,
    question:    raw.question != null ? String(raw.question) : null,
    status:      String(raw.status ?? ""),
    createdAt:   String(raw.created_at ?? ""),
  };
}

function mapApproval(raw: Record<string, unknown>): ApprovalRequest {
  return {
    id:          String(raw.id ?? ""),
    runId:       String(raw.run_id ?? ""),
    storyTitle:  raw.story_title != null ? String(raw.story_title) : null,
    toolName:    String(raw.tool_name ?? ""),
    toolInput:   String(raw.tool_input ?? "{}"),
    status:      (raw.status ?? "pending") as ApprovalRequest["status"],
    createdAt:   String(raw.created_at ?? ""),
  };
}

// ---------------------------------------------------------------------------
// useHumanRequests
// ---------------------------------------------------------------------------

export interface UseHumanRequestsReturn {
  humanRequests:  HumanRequest[];
  approvalRequests: ApprovalRequest[];
  loading: boolean;
  refresh: () => Promise<void>;
  respondToHuman: (storyId: string, response: string) => Promise<void>;
  decideApproval: (id: string, approved: boolean, rejectionReason?: string) => Promise<void>;
}

export function useHumanRequests(pollInterval = 5_000): UseHumanRequestsReturn {
  const [humanRequests, setHumanRequests] = useState<HumanRequest[]>([]);
  const [approvalRequests, setApprovalRequests] = useState<ApprovalRequest[]>([]);
  const [loading, setLoading] = useState(true);
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [humans, approvals] = await Promise.all([
        invoke<Record<string, unknown>[]>("get_pending_human_requests"),
        invoke<Record<string, unknown>[]>("get_pending_approvals"),
      ]);
      setHumanRequests(humans.map(mapRequest));
      setApprovalRequests(approvals.map(mapApproval));
    } catch (e) {
      console.error("useHumanRequests refresh error:", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();

    // Poll periodically as a fallback (disabled when pollInterval is 0).
    if (pollInterval > 0) {
      pollingRef.current = setInterval(refresh, pollInterval);
    }

    // Also refresh immediately when Tauri emits events.
    const unlistenHuman = listen("human-request-created", refresh);
    const unlistenApproval = listen("approval-request-created", refresh);

    return () => {
      if (pollingRef.current) clearInterval(pollingRef.current);
      unlistenHuman.then(fn => fn());
      unlistenApproval.then(fn => fn());
    };
  }, [refresh]);

  const respondToHuman = useCallback(
    async (storyId: string, response: string) => {
      await invoke("respond_to_human_request", { storyId, response });
      await refresh();
    },
    [refresh]
  );

  const decideApproval = useCallback(
    async (id: string, approved: boolean, rejectionReason?: string) => {
      await invoke("decide_approval", {
        id,
        approved,
        rejectionReason: rejectionReason ?? null,
      });
      await refresh();
    },
    [refresh]
  );

  return { humanRequests, approvalRequests, loading, refresh, respondToHuman, decideApproval };
}

// ---------------------------------------------------------------------------
// requestDesktopNotification — best-effort, no throw
// ---------------------------------------------------------------------------

export function requestDesktopNotification(title: string, body: string): void {
  if (!("Notification" in window)) return;
  if (Notification.permission === "granted") {
    new Notification(title, { body, icon: "/tauri.svg" });
  } else if (Notification.permission !== "denied") {
    Notification.requestPermission().then(permission => {
      if (permission === "granted") {
        new Notification(title, { body, icon: "/tauri.svg" });
      }
    });
  }
}
