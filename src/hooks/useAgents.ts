import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useState, useCallback, useEffect } from "react";
import { notifyError } from "../components/ui/Toast";
import type { AgentProfile, CreateProfileInput, UpdateProfileInput } from "../types/agent";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// ---------------------------------------------------------------------------
// useAgents — hook for loading and mutating agent profiles via Tauri IPC
// ---------------------------------------------------------------------------

interface UseAgentsReturn {
  profiles: AgentProfile[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  createProfile: (input: CreateProfileInput) => Promise<AgentProfile>;
  updateProfile: (id: string, input: UpdateProfileInput) => Promise<AgentProfile>;
  deleteProfile: (id: string) => Promise<void>;
}

export function useAgents(): UseAgentsReturn {
  const [profiles, setProfiles] = useState<AgentProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<AgentProfile[]>("get_profiles");
      setProfiles(data);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      notifyError("Failed to load agents", message, { duration: 7000 });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Refresh whenever the active workspace changes.
  useEffect(() => {
    const unlisten = listen("workspace-changed", () => { refresh(); });
    return () => { unlisten.then(fn => fn()); };
  }, [refresh]);

  const createProfile = useCallback(async (input: CreateProfileInput): Promise<AgentProfile> => {
    try {
      const profile = await invoke<AgentProfile>("create_profile", { input });
      setProfiles(prev => [...prev, profile]);
      return profile;
    } catch (e) {
      notifyError("Failed to create agent", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  const updateProfile = useCallback(async (id: string, input: UpdateProfileInput): Promise<AgentProfile> => {
    try {
      const updated = await invoke<AgentProfile>("update_profile", { id, input });
      setProfiles(prev => prev.map(p => p.id === id ? updated : p));
      return updated;
    } catch (e) {
      notifyError("Failed to update agent", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  const deleteProfile = useCallback(async (id: string): Promise<void> => {
    try {
      await invoke("delete_profile", { id });
      setProfiles(prev => prev.filter(p => p.id !== id));
    } catch (e) {
      notifyError("Failed to delete agent", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  return { profiles, loading, error, refresh, createProfile, updateProfile, deleteProfile };
}
