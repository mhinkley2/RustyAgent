import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useState, useCallback, useEffect } from "react";
import { notifyError } from "../components/ui/Toast";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface Workspace {
  id: string;
  path: string;
  name: string;
  lastOpenedAt: string;
  createdAt: string;
}

interface RawWorkspace {
  id: string;
  path: string;
  name: string;
  last_opened_at: string;
  created_at: string;
}

function mapWorkspace(r: RawWorkspace): Workspace {
  return {
    id:           r.id,
    path:         r.path,
    name:         r.name,
    lastOpenedAt: r.last_opened_at,
    createdAt:    r.created_at,
  };
}

// ---------------------------------------------------------------------------
// useWorkspace
// ---------------------------------------------------------------------------

interface UseWorkspaceReturn {
  /** The currently active workspace (most recently opened). */
  activeWorkspace: Workspace | null;
  recentWorkspaces: Workspace[];
  loading: boolean;
  /** Open the OS folder-picker and register the chosen workspace. */
  pickAndOpenWorkspace: () => Promise<Workspace | null>;
  /** Re-open an existing workspace from the recent list. */
  reopenWorkspace: (workspace: Workspace) => Promise<void>;
  /** Remove a workspace from the recent list. */
  removeWorkspace: (id: string) => Promise<void>;
  refresh: () => Promise<void>;
}

export function useWorkspace(): UseWorkspaceReturn {
  const [recentWorkspaces, setRecentWorkspaces] = useState<Workspace[]>([]);
  const [activeWorkspace, setActiveWorkspace] = useState<Workspace | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const raw = await invoke<RawWorkspace[]>("get_recent_workspaces");
      const workspaces = raw.map(mapWorkspace);

      if (workspaces[0]) {
        const activeRaw = await invoke<RawWorkspace>("open_workspace", { path: workspaces[0].path });
        const active = mapWorkspace(activeRaw);
        setActiveWorkspace(active);
        setRecentWorkspaces([active, ...workspaces.filter((workspace) => workspace.path !== active.path)]);
      } else {
        setRecentWorkspaces([]);
        setActiveWorkspace(null);
      }
    } catch (e) {
      notifyError("Failed to load workspaces", errorMessage(e), { duration: 7000 });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  // Keep active workspace in sync when the backend switches workspace.
  useEffect(() => {
    const unlisten = listen<RawWorkspace>("workspace-changed", (event) => {
      const ws = mapWorkspace(event.payload);
      setActiveWorkspace(ws);
      setRecentWorkspaces(prev => {
        const filtered = prev.filter(w => w.id !== ws.id);
        return [ws, ...filtered];
      });
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  const pickAndOpenWorkspace = useCallback(async (): Promise<Workspace | null> => {
    const selected = await openDialog({ directory: true, multiple: false });
    if (!selected) return null;
    const path = typeof selected === "string" ? selected : selected;
    if (typeof path !== "string") return null;
    try {
      const raw = await invoke<RawWorkspace>("open_workspace", { path });
      const ws = mapWorkspace(raw);
      setActiveWorkspace(ws);
      setRecentWorkspaces(prev => {
        const filtered = prev.filter(w => w.id !== ws.id);
        return [ws, ...filtered];
      });
      return ws;
    } catch (e) {
      notifyError("Failed to open workspace", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  const reopenWorkspace = useCallback(async (workspace: Workspace) => {
    try {
      const raw = await invoke<RawWorkspace>("open_workspace", { path: workspace.path });
      const ws = mapWorkspace(raw);
      setActiveWorkspace(ws);
      setRecentWorkspaces(prev => {
        const filtered = prev.filter(w => w.id !== ws.id);
        return [ws, ...filtered];
      });
    } catch (e) {
      notifyError("Failed to reopen workspace", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  const removeWorkspace = useCallback(async (id: string) => {
    try {
      await invoke("remove_workspace", { id });
      setRecentWorkspaces(prev => prev.filter(w => w.id !== id));
      setActiveWorkspace(prev => (prev?.id === id ? null : prev));
    } catch (e) {
      notifyError("Failed to remove workspace", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  return {
    activeWorkspace,
    recentWorkspaces,
    loading,
    pickAndOpenWorkspace,
    reopenWorkspace,
    removeWorkspace,
    refresh,
  };
}
