import { tauriMock } from "../tauriMock";

// ---------------------------------------------------------------------------
// In-memory stand-in for the story/board half of the backend.
// ---------------------------------------------------------------------------

export interface RawStory {
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

/** A raw story with sensible defaults; override just what a test cares about. */
export function rawStory(overrides: Partial<RawStory> & { id: string }): RawStory {
  return {
    title: `Story ${overrides.id}`,
    description: null,
    story_type: "feature",
    status: "backlog",
    priority: "medium",
    assigned_agent_id: null,
    assigned_agent_name: null,
    requires_approval: false,
    track_history: true,
    labels: [],
    sort_order: 0,
    created_at: "2026-04-13T00:00:00Z",
    updated_at: "2026-04-13T00:00:01Z",
    ...overrides,
  };
}

export function createStoryBackend(initial: RawStory[] = []) {
  let stories = initial.map((s) => ({ ...s }));
  let nextId = 1;
  /** Resolvers for pending batch_update_story_order calls, when deferred. */
  let deferReorder: (() => void) | null = null;

  tauriMock.handleAll({
    get_stories: () => stories.map((s) => ({ ...s })),

    create_story: (args) => {
      const input = (args.input ?? {}) as Partial<RawStory>;
      const created = rawStory({ id: `new-${nextId++}`, ...input });
      stories.push(created);
      return { ...created };
    },

    update_story: (args) => {
      const id = String(args.id);
      const input = (args.input ?? {}) as Partial<RawStory>;
      const idx = stories.findIndex((s) => s.id === id);
      if (idx === -1) throw new Error(`No such story: ${id}`);
      stories[idx] = { ...stories[idx], ...input };
      return { ...stories[idx] };
    },

    delete_story: (args) => {
      stories = stories.filter((s) => s.id !== String(args.id));
      return undefined;
    },

    batch_update_story_order: async (args) => {
      if (deferReorder) {
        await new Promise<void>((resolve) => {
          deferReorder = resolve;
        });
      }
      for (const u of (args.updates ?? []) as { id: string; sort_order: number }[]) {
        const story = stories.find((s) => s.id === u.id);
        if (story) story.sort_order = u.sort_order;
      }
      return undefined;
    },
  });

  return {
    /** Current server-side state. */
    getStories: () => stories.map((s) => ({ ...s })),
    /** Make the next reorder hang until `releaseReorder()` is called. */
    holdReorder() {
      deferReorder = () => {};
    },
    releaseReorder() {
      const resolve = deferReorder;
      deferReorder = null;
      resolve?.();
    },
    emitWorkspaceChanged: () => tauriMock.emit("workspace-changed", {}),
    /** Replace the server-side set, e.g. to prove a refresh actually refetched. */
    setStories(next: RawStory[]) {
      stories = next.map((s) => ({ ...s }));
    },
  };
}
