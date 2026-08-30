import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useState, useCallback, useEffect, useRef } from "react";
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
  /** When the list on screen was last read from the database. */
  lastFetchedAt: Date | null;
  /**
   * Hold automatic refreshes while a card is being dragged.
   *
   * A refetch landing between a drop and the write that persists it would
   * stomp the optimistic order. The change that arrived is remembered and
   * applied on resume.
   */
  pauseAutoRefresh: () => void;
  resumeAutoRefresh: () => void;
  createStory: (input: CreateStoryInput) => Promise<Story>;
  updateStory: (id: string, input: UpdateStoryInput) => Promise<Story>;
  deleteStory: (id: string) => Promise<void>;
  reorderStories: (updates: { id: string; sortOrder: number }[]) => Promise<void>;
}

/**
 * Emitted by every in-process writer that changes a story. Mirrors
 * `db::story_status::STORIES_CHANGED_EVENT`.
 */
export const STORIES_CHANGED_EVENT = "stories-changed";

/**
 * How long to wait after a change before refetching.
 *
 * A pipeline settling several stories emits several times in quick succession;
 * the board only needs one fetch out of it.
 */
const CHANGE_DEBOUNCE_MS = 250;

/**
 * The polling floor, for writers that cannot emit — chiefly the out-of-process
 * MCP binary. Stories change on run boundaries rather than per token, so this
 * is deliberately slow.
 */
const POLL_INTERVAL_MS = 15_000;

export function useStories(): UseStoriesReturn {
  const [stories, setStories] = useState<Story[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [lastFetchedAt, setLastFetchedAt] = useState<Date | null>(null);
  /// While a card is being dragged, a refetch would fight the optimistic order.
  const pausedRef = useRef(false);
  /// A refresh that arrived while paused, to be honoured on resume.
  const missedRef = useRef(false);

  /**
   * Fetch the board.
   *
   * `background` is for the poll and the event listener: they must not put the
   * board into its loading state, or a card would blink out every fifteen
   * seconds. Only an explicit refresh — mount, workspace change, or the user
   * pressing the button — says it is loading.
   */
  const fetchStories = useCallback(async (background: boolean) => {
    if (!background) setLoading(true);
    setError(null);
    try {
      const raw = await invoke<RawStory[]>("get_stories");
      setStories(raw.map(mapStory));
      setLastFetchedAt(new Date());
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      // A failing poll must not shout every fifteen seconds; the staleness
      // indicator in the header is how a background failure shows.
      if (!background) {
        notifyError("Failed to load stories", message, { duration: 7000 });
      }
    } finally {
      if (!background) setLoading(false);
    }
  }, []);

  const refresh = useCallback(async () => {
    await fetchStories(false);
  }, [fetchStories]);

  /**
   * Refetch in the background, unless a drag is in flight.
   *
   * A refetch landing between a drop and the write that persists it stomps the
   * optimistic order, so an automatic refresh defers instead — and the deferred
   * one is remembered rather than dropped, or a change arriving mid-drag would
   * wait for the *next* one.
   */
  const backgroundRefresh = useCallback(() => {
    if (pausedRef.current) {
      missedRef.current = true;
      return;
    }
    void fetchStories(true);
  }, [fetchStories]);

  const pauseAutoRefresh = useCallback(() => {
    pausedRef.current = true;
  }, []);

  const resumeAutoRefresh = useCallback(() => {
    pausedRef.current = false;
    if (missedRef.current) {
      missedRef.current = false;
      void fetchStories(true);
    }
  }, [fetchStories]);

  useEffect(() => {
    void fetchStories(false);
  }, [fetchStories]);

  // Refresh whenever the active workspace changes.
  useEffect(() => {
    const unlisten = listen("workspace-changed", () => { void fetchStories(false); });
    return () => { unlisten.then(fn => fn()); };
  }, [fetchStories]);

  /**
   * Follow the board as other writers change it.
   *
   * Runs, pipelines, the crash sweep and the UI's own commands all write
   * stories, and until this existed only a restart or a workspace switch made
   * any of it visible — so the routine end of a run moved a card in SQL that
   * the open board never showed.
   *
   * Debounced, because one pipeline settling six stories is one board change,
   * not six.
   */
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void listen(STORIES_CHANGED_EVENT, () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => backgroundRefresh(), CHANGE_DEBOUNCE_MS);
    }).then(fn => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      unlisten?.();
    };
  }, [backgroundRefresh]);

  /**
   * The floor under the events.
   *
   * `rustyagent-board-mcp` runs as a separate process against the same SQLite
   * file and cannot emit a Tauri event at all, so an external client's writes
   * are only ever visible through this. It also covers any future writer that
   * forgets to announce itself.
   */
  useEffect(() => {
    const timer = setInterval(backgroundRefresh, POLL_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [backgroundRefresh]);

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

  return {
    stories,
    loading,
    error,
    refresh,
    lastFetchedAt,
    pauseAutoRefresh,
    resumeAutoRefresh,
    createStory,
    updateStory,
    deleteStory,
    reorderStories,
  };
}
