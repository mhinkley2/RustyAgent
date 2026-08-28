import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { tauriMock } from "../test/tauriMock";
import { createStoryBackend, rawStory } from "../test/backends/storyBackend";

const notifyError = vi.fn();
vi.mock("../components/ui/Toast", () => ({
  notifyError: (...args: unknown[]) => notifyError(...args),
  notifyToast: vi.fn(),
}));

let useStories: typeof import("./useStories").useStories;

beforeEach(async () => {
  notifyError.mockClear();
  useStories = (await import("./useStories")).useStories;
});

/** Render the hook and wait for its initial load to settle. */
async function renderLoaded() {
  const view = renderHook(() => useStories());
  await waitFor(() => expect(view.result.current.loading).toBe(false));
  return view;
}

describe("useStories — loading and mapping", () => {
  it("loads stories on mount and maps them to the UI shape", async () => {
    createStoryBackend([
      rawStory({
        id: "abcdef123456",
        title: "Ship the thing",
        description: "details",
        story_type: "bug",
        status: "in_progress",
        priority: "high",
        assigned_agent_id: "agent-1",
        assigned_agent_name: "Agent One",
        labels: ["urgent"],
        sort_order: 3,
      }),
    ]);

    const { result } = await renderLoaded();

    expect(result.current.stories).toHaveLength(1);
    const story = result.current.stories[0];
    // The display key is the first six characters of the id.
    expect(story.key).toBe("#abcdef");
    expect(story.title).toBe("Ship the thing");
    expect(story.type).toBe("bug");
    expect(story.status).toBe("in_progress");
    expect(story.priority).toBe("high");
    // assignedAgentId and assignee come from two separate raw fields.
    expect(story.assignedAgentId).toBe("agent-1");
    expect(story.assignee).toBe("Agent One");
    expect(story.labels).toEqual(["urgent"]);
    expect(story.sortOrder).toBe(3);
    expect(story.createdAt).toBeInstanceOf(Date);
    expect(story.updatedAt).toBeInstanceOf(Date);
  });

  it("maps null description and assignee to undefined rather than null", async () => {
    createStoryBackend([rawStory({ id: "s1" })]);

    const { result } = await renderLoaded();

    expect(result.current.stories[0].description).toBeUndefined();
    expect(result.current.stories[0].assignedAgentId).toBeUndefined();
    expect(result.current.stories[0].assignee).toBeUndefined();
  });

  it("sets an error and raises a toast when the load fails", async () => {
    tauriMock.handle("get_stories", () => {
      throw new Error("database is locked");
    });

    const { result } = await renderLoaded();

    expect(result.current.error).toContain("database is locked");
    expect(result.current.stories).toEqual([]);
    expect(notifyError).toHaveBeenCalledWith(
      "Failed to load stories",
      "database is locked",
      expect.anything(),
    );
  });

  it("clears a previous error on a successful refresh", async () => {
    let fail = true;
    const backend = createStoryBackend([rawStory({ id: "s1" })]);
    tauriMock.handle("get_stories", () => {
      if (fail) throw new Error("boom");
      return backend.getStories();
    });

    const { result } = await renderLoaded();
    expect(result.current.error).toBeTruthy();

    fail = false;
    await act(async () => {
      await result.current.refresh();
    });

    expect(result.current.error).toBeNull();
    expect(result.current.stories).toHaveLength(1);
  });
});

describe("useStories — mutations", () => {
  it("appends the created story to local state", async () => {
    createStoryBackend([]);
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.createStory({ title: "New one" });
    });

    expect(result.current.stories.map((s) => s.title)).toEqual(["New one"]);
  });

  it("rethrows and toasts on a failed create, leaving state untouched", async () => {
    createStoryBackend([rawStory({ id: "s1" })]);
    const { result } = await renderLoaded();
    tauriMock.handle("create_story", () => {
      throw new Error("title is required");
    });

    await expect(
      act(async () => {
        await result.current.createStory({ title: "" });
      }),
    ).rejects.toThrow();

    expect(result.current.stories).toHaveLength(1);
    expect(notifyError).toHaveBeenCalledWith(
      "Failed to create story",
      "title is required",
      expect.anything(),
    );
  });

  it("replaces only the story that was updated", async () => {
    createStoryBackend([
      rawStory({ id: "s1", title: "First" }),
      rawStory({ id: "s2", title: "Second" }),
    ]);
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.updateStory("s2", { title: "Second (edited)" });
    });

    expect(result.current.stories.map((s) => s.title)).toEqual([
      "First",
      "Second (edited)",
    ]);
  });

  it("removes the deleted story from local state", async () => {
    createStoryBackend([rawStory({ id: "s1" }), rawStory({ id: "s2" })]);
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.deleteStory("s1");
    });

    expect(result.current.stories.map((s) => s.id)).toEqual(["s2"]);
    expect(tauriMock.calls("delete_story")).toEqual([{ id: "s1" }]);
  });

  it("rethrows and toasts on a failed delete without removing the story", async () => {
    createStoryBackend([rawStory({ id: "s1" })]);
    const { result } = await renderLoaded();
    tauriMock.handle("delete_story", () => {
      throw new Error("still referenced");
    });

    await expect(
      act(async () => {
        await result.current.deleteStory("s1");
      }),
    ).rejects.toThrow();

    expect(result.current.stories).toHaveLength(1);
  });
});

describe("useStories — reordering", () => {
  it("applies the new order optimistically, before the backend confirms", async () => {
    const backend = createStoryBackend([
      rawStory({ id: "a", sort_order: 0 }),
      rawStory({ id: "b", sort_order: 1 }),
      rawStory({ id: "c", sort_order: 2 }),
    ]);
    const { result } = await renderLoaded();
    expect(result.current.stories.map((s) => s.id)).toEqual(["a", "b", "c"]);

    backend.holdReorder();
    let pending!: Promise<void>;
    act(() => {
      pending = result.current.reorderStories([
        { id: "c", sortOrder: 0 },
        { id: "a", sortOrder: 1 },
        { id: "b", sortOrder: 2 },
      ]);
    });

    // The UI has already re-ordered while the invoke is still in flight.
    expect(result.current.stories.map((s) => s.id)).toEqual(["c", "a", "b"]);

    backend.releaseReorder();
    await act(async () => {
      await pending;
    });
    expect(result.current.stories.map((s) => s.id)).toEqual(["c", "a", "b"]);
  });

  it("sends snake_case sort_order to the backend", async () => {
    createStoryBackend([rawStory({ id: "a", sort_order: 0 })]);
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.reorderStories([{ id: "a", sortOrder: 7 }]);
    });

    expect(tauriMock.calls("batch_update_story_order")).toEqual([
      { updates: [{ id: "a", sort_order: 7 }] },
    ]);
  });

  it("leaves stories absent from the update at their existing sortOrder", async () => {
    createStoryBackend([
      rawStory({ id: "a", sort_order: 0 }),
      rawStory({ id: "untouched", sort_order: 5 }),
    ]);
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.reorderStories([{ id: "a", sortOrder: 9 }]);
    });

    const untouched = result.current.stories.find((s) => s.id === "untouched");
    expect(untouched?.sortOrder).toBe(5);
    // 'a' moved past it.
    expect(result.current.stories.map((s) => s.id)).toEqual(["untouched", "a"]);
  });

  it("rethrows and toasts when the reorder fails to persist", async () => {
    createStoryBackend([rawStory({ id: "a" })]);
    const { result } = await renderLoaded();
    tauriMock.handle("batch_update_story_order", () => {
      throw new Error("write conflict");
    });

    await expect(
      act(async () => {
        await result.current.reorderStories([{ id: "a", sortOrder: 1 }]);
      }),
    ).rejects.toThrow();

    expect(notifyError).toHaveBeenCalledWith(
      "Failed to reorder stories",
      "write conflict",
      expect.anything(),
    );
  });
});

describe("useStories — workspace changes", () => {
  it("refetches when a workspace-changed event arrives", async () => {
    const backend = createStoryBackend([rawStory({ id: "s1", title: "Old workspace" })]);
    const { result } = await renderLoaded();
    expect(result.current.stories.map((s) => s.title)).toEqual(["Old workspace"]);

    backend.setStories([rawStory({ id: "s2", title: "New workspace" })]);
    await backend.emitWorkspaceChanged();

    await waitFor(() =>
      expect(result.current.stories.map((s) => s.title)).toEqual(["New workspace"]),
    );
  });

  it("unsubscribes the workspace listener on unmount", async () => {
    createStoryBackend([]);
    const view = await renderLoaded();
    expect(tauriMock.listenerCount("workspace-changed")).toBe(1);

    view.unmount();

    await waitFor(() => expect(tauriMock.listenerCount("workspace-changed")).toBe(0));
  });
});
