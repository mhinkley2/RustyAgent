import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { tauriMock } from "../test/tauriMock";

let useHumanRequests: typeof import("./useHumanRequests").useHumanRequests;

beforeEach(async () => {
  useHumanRequests = (await import("./useHumanRequests")).useHumanRequests;
});

afterEach(() => {
  vi.useRealTimers();
});

function rawHuman(overrides: Record<string, unknown> = {}) {
  return {
    id: "hr-1",
    story_id: "story-1",
    task_story_id: "task-1",
    story_title: "Needs a decision",
    run_id: "run-1",
    question: "Which database?",
    status: "pending",
    created_at: "2026-04-13T00:00:00Z",
    ...overrides,
  };
}

function rawApproval(overrides: Record<string, unknown> = {}) {
  return {
    id: "ap-1",
    run_id: "run-1",
    story_id: "task-1",
    story_title: "Needs approval",
    tool_name: "file_write",
    tool_input: '{"path":"a.txt"}',
    status: "pending",
    created_at: "2026-04-13T00:00:00Z",
    ...overrides,
  };
}

/** Install both endpoints, defaulting to the given rows. */
function backend(humans: unknown[] = [], approvals: unknown[] = []) {
  const state = { humans, approvals };
  tauriMock.handleAll({
    get_pending_human_requests: () => state.humans,
    get_pending_approvals: () => state.approvals,
    respond_to_human_request: () => undefined,
    decide_approval: () => undefined,
  });
  return state;
}

async function renderLoaded(pollInterval?: number) {
  const view = renderHook(() =>
    pollInterval === undefined ? useHumanRequests() : useHumanRequests(pollInterval),
  );
  await waitFor(() => expect(view.result.current.loading).toBe(false));
  return view;
}

describe("useHumanRequests — loading and mapping", () => {
  it("loads human requests and approvals together on mount", async () => {
    backend([rawHuman()], [rawApproval()]);

    const { result } = await renderLoaded();

    expect(result.current.humanRequests).toHaveLength(1);
    expect(result.current.approvalRequests).toHaveLength(1);
    expect(tauriMock.callCount("get_pending_human_requests")).toBe(1);
    expect(tauriMock.callCount("get_pending_approvals")).toBe(1);
  });

  it("maps snake_case human-request fields to camelCase", async () => {
    backend([rawHuman()]);

    const { result } = await renderLoaded();

    expect(result.current.humanRequests[0]).toEqual({
      id: "hr-1",
      storyId: "story-1",
      taskStoryId: "task-1",
      storyTitle: "Needs a decision",
      runId: "run-1",
      question: "Which database?",
      status: "pending",
      createdAt: "2026-04-13T00:00:00Z",
    });
  });

  it("maps snake_case approval fields to camelCase", async () => {
    backend([], [rawApproval()]);

    const { result } = await renderLoaded();

    expect(result.current.approvalRequests[0]).toEqual({
      id: "ap-1",
      runId: "run-1",
      storyId: "task-1",
      storyTitle: "Needs approval",
      toolName: "file_write",
      toolInput: '{"path":"a.txt"}',
      status: "pending",
      createdAt: "2026-04-13T00:00:00Z",
    });
  });

  it("keeps an absent story on either request kind as null", async () => {
    // A question raised outside a run, and an approval whose story has been
    // deleted. Both stay pending; neither has a card to mark. The string
    // "null" or "undefined" here would badge a story id that cannot exist.
    backend(
      [rawHuman({ task_story_id: null })],
      [rawApproval({ story_id: undefined, story_title: null })],
    );

    const { result } = await renderLoaded();

    expect(result.current.humanRequests[0].taskStoryId).toBeNull();
    expect(result.current.approvalRequests[0].storyId).toBeNull();
    expect(result.current.approvalRequests[0].storyTitle).toBeNull();
  });

  it("keeps a null run_id and question as null rather than the string 'null'", async () => {
    backend([rawHuman({ run_id: null, question: null })]);

    const { result } = await renderLoaded();

    expect(result.current.humanRequests[0].runId).toBeNull();
    expect(result.current.humanRequests[0].question).toBeNull();
  });

  it("falls back to the id when story_id is absent", async () => {
    backend([rawHuman({ story_id: undefined })]);

    const { result } = await renderLoaded();

    expect(result.current.humanRequests[0].storyId).toBe("hr-1");
  });

  it("defaults a missing approval tool_input to an empty object and status to pending", async () => {
    backend([], [rawApproval({ tool_input: undefined, status: undefined })]);

    const { result } = await renderLoaded();

    expect(result.current.approvalRequests[0].toolInput).toBe("{}");
    expect(result.current.approvalRequests[0].status).toBe("pending");
  });

  it("does not leave loading stuck on when a fetch rejects", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    tauriMock.handleAll({
      get_pending_human_requests: () => {
        throw new Error("no workspace is open");
      },
      get_pending_approvals: () => [],
    });

    const { result } = await renderLoaded();

    expect(result.current.loading).toBe(false);
    expect(result.current.humanRequests).toEqual([]);
  });
});

describe("useHumanRequests — live updates", () => {
  it("refetches when human-request-created is emitted", async () => {
    const state = backend([]);
    const { result } = await renderLoaded();
    expect(result.current.humanRequests).toHaveLength(0);

    state.humans = [rawHuman()];
    await tauriMock.emit("human-request-created", {});

    await waitFor(() => expect(result.current.humanRequests).toHaveLength(1));
  });

  it("refetches when approval-request-created is emitted", async () => {
    const state = backend([], []);
    const { result } = await renderLoaded();

    state.approvals = [rawApproval()];
    await tauriMock.emit("approval-request-created", {});

    await waitFor(() => expect(result.current.approvalRequests).toHaveLength(1));
  });

  it("polls on the configured interval", async () => {
    vi.useFakeTimers();
    backend();

    const view = renderHook(() => useHumanRequests(5_000));
    await vi.waitFor(() => expect(tauriMock.callCount("get_pending_approvals")).toBe(1));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(tauriMock.callCount("get_pending_approvals")).toBe(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(tauriMock.callCount("get_pending_approvals")).toBe(3);

    view.unmount();
  });

  it("does not poll when the interval is zero", async () => {
    vi.useFakeTimers();
    backend();

    const view = renderHook(() => useHumanRequests(0));
    await vi.waitFor(() => expect(tauriMock.callCount("get_pending_approvals")).toBe(1));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });

    expect(tauriMock.callCount("get_pending_approvals")).toBe(1);
    view.unmount();
  });

  it("stops polling and drops both listeners on unmount", async () => {
    vi.useFakeTimers();
    backend();

    const view = renderHook(() => useHumanRequests(5_000));
    await vi.waitFor(() => expect(tauriMock.callCount("get_pending_approvals")).toBe(1));
    await vi.waitFor(() =>
      expect(tauriMock.listenerCount("human-request-created")).toBe(1),
    );

    view.unmount();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(tauriMock.callCount("get_pending_approvals")).toBe(1);
    await vi.waitFor(() => {
      expect(tauriMock.listenerCount("human-request-created")).toBe(0);
      expect(tauriMock.listenerCount("approval-request-created")).toBe(0);
    });
  });
});

describe("useHumanRequests — decisions", () => {
  it("sends a null rejectionReason when none is given, then refreshes", async () => {
    backend([], [rawApproval()]);
    const { result } = await renderLoaded(0);
    const before = tauriMock.callCount("get_pending_approvals");

    await act(async () => {
      await result.current.decideApproval("ap-1", true);
    });

    expect(tauriMock.calls("decide_approval")).toEqual([
      { id: "ap-1", approved: true, rejectionReason: null },
    ]);
    expect(tauriMock.callCount("get_pending_approvals")).toBe(before + 1);
  });

  it("passes a rejection reason through when one is given", async () => {
    backend([], [rawApproval()]);
    const { result } = await renderLoaded(0);

    await act(async () => {
      await result.current.decideApproval("ap-1", false, "writes outside the repo");
    });

    expect(tauriMock.calls("decide_approval")).toEqual([
      { id: "ap-1", approved: false, rejectionReason: "writes outside the repo" },
    ]);
  });

  it("sends the story id and response when answering a human request, then refreshes", async () => {
    backend([rawHuman()]);
    const { result } = await renderLoaded(0);
    const before = tauriMock.callCount("get_pending_human_requests");

    await act(async () => {
      await result.current.respondToHuman("story-1", "use SQLite");
    });

    expect(tauriMock.calls("respond_to_human_request")).toEqual([
      { storyId: "story-1", response: "use SQLite" },
    ]);
    expect(tauriMock.callCount("get_pending_human_requests")).toBe(before + 1);
  });

  it("propagates a failure to decide rather than swallowing it", async () => {
    backend([], [rawApproval()]);
    const { result } = await renderLoaded(0);
    tauriMock.handle("decide_approval", () => {
      throw new Error("approval already resolved");
    });

    await expect(
      act(async () => {
        await result.current.decideApproval("ap-1", true);
      }),
    ).rejects.toThrow(/already resolved/);
  });
});
