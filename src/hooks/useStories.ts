import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useState, useCallback, useEffect } from "react";
import { notifyError } from "../components/ui/Toast";
import type { Story, StoryStatus, StoryPriority, StoryType } from "../types/board";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// ---------------------------------------------------------------------------
// Raw types — match the Rust / serde serialization (snake_case)
// ---------------------------------------------------------------------------

interface RawStory {
  id: string;
  title: string;
  description: string | null;
  story_type: string;
  status: string;
  priority: string;
  assigned_agent_id: string | null;
  assigned_agent_name: string | null;
  requires_approval: boolean;
  track_history: boolean;
  labels: string[];
  sort_order: number;
  created_at: string;
  updated_at: string;
}

export interface CreateStoryInput {
  title: string;
  description?: string | null;
  story_type?: string;
  status?: string;
  priority?: string;
  assigned_agent_id?: string;
  requires_approval?: boolean;
  track_history?: boolean;
  labels?: string[];
}

export interface UpdateStoryInput {
  title?: string;
  description?: string | null;
  story_type?: string;
  status?: string;
  priority?: string;
  /** Empty string to clear the assignee. */
  assigned_agent_id?: string;
  requires_approval?: boolean;
  track_history?: boolean;
  labels?: string[];
}

// ---------------------------------------------------------------------------
// Mapping
// ---------------------------------------------------------------------------

function mapStory(raw: RawStory): Story {
  return {
    id:               raw.id,
    key:              "#" + raw.id.slice(0, 6),
    title:            raw.title,
    description:      raw.description ?? undefined,
    status:           raw.status as StoryStatus,
    priority:         raw.priority as StoryPriority,
    type:             raw.story_type as StoryType,
    assignedAgentId:  raw.assigned_agent_id ?? undefined,
    assignee:         raw.assigned_agent_name ?? undefined,
    requiresApproval: raw.requires_approval,
    trackHistory:     raw.track_history,
    labels:           raw.labels,
    sortOrder:        raw.sort_order,
    createdAt:        new Date(raw.created_at),
    updatedAt:        new Date(raw.updated_at),
  };
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

interface UseStoriesReturn {
  stories: Story[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  createStory: (input: CreateStoryInput) => Promise<Story>;
  updateStory: (id: string, input: UpdateStoryInput) => Promise<Story>;
  deleteStory: (id: string) => Promise<void>;
  reorderStories: (updates: { id: string; sortOrder: number }[]) => Promise<void>;
}

export function useStories(): UseStoriesReturn {
  const [stories, setStories] = useState<Story[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const raw = await invoke<RawStory[]>("get_stories");
      setStories(raw.map(mapStory));
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      notifyError("Failed to load stories", message, { duration: 7000 });
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

  const createStory = useCallback(async (input: CreateStoryInput): Promise<Story> => {
    try {
      const raw = await invoke<RawStory>("create_story", { input });
      const story = mapStory(raw);
      setStories(prev => [...prev, story]);
      return story;
    } catch (e) {
      notifyError("Failed to create story", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  const updateStory = useCallback(async (id: string, input: UpdateStoryInput): Promise<Story> => {
    try {
      const raw = await invoke<RawStory>("update_story", { id, input });
      const story = mapStory(raw);
      setStories(prev => prev.map(s => s.id === id ? story : s));
      return story;
    } catch (e) {
      notifyError("Failed to update story", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  const deleteStory = useCallback(async (id: string): Promise<void> => {
    try {
      await invoke("delete_story", { id });
      setStories(prev => prev.filter(s => s.id !== id));
    } catch (e) {
      notifyError("Failed to delete story", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  const reorderStories = useCallback(async (updates: { id: string; sortOrder: number }[]): Promise<void> => {
    // Optimistic local update
    setStories(prev => {
      const orderMap = new Map(updates.map(u => [u.id, u.sortOrder]));
      return [...prev].sort((a, b) => {
        const oa = orderMap.get(a.id) ?? a.sortOrder;
        const ob = orderMap.get(b.id) ?? b.sortOrder;
        return oa - ob;
      }).map(s => orderMap.has(s.id) ? { ...s, sortOrder: orderMap.get(s.id)! } : s);
    });
    try {
      await invoke("batch_update_story_order", {
        updates: updates.map(u => ({ id: u.id, sort_order: u.sortOrder })),
      });
    } catch (e) {
      notifyError("Failed to reorder stories", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  return { stories, loading, error, refresh, createStory, updateStory, deleteStory, reorderStories };
}
