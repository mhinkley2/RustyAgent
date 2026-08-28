import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { tauriMock } from "../test/tauriMock";

let useRuns: typeof import("./useRuns").useRuns;
let useRunEvents: typeof import("./useRuns").useRunEvents;
let useRunDiff: typeof import("./useRuns").useRunDiff;
let exportRun: typeof import("./useRuns").exportRun;

beforeEach(async () => {
  const mod = await import("./useRuns");
  useRuns = mod.useRuns;
  useRunEvents = mod.useRunEvents;
  useRunDiff = mod.useRunDiff;
  exportRun = mod.exportRun;
});

function rawRun(overrides: Record<string, unknown> & { id: string }) {
  return {
    story_id: "story-1",
    story_title: "A story",
    agent_profile_id: "agent-1",
    agent_name: "Agent One",
    status: "done",
    input_tokens: 100,
    output_tokens: 50,
    cache_read_input_tokens: 900,
    cache_creation_input_tokens: 40,
    estimated_cost_usd: 0.0123,
    iteration_count: 2,
    started_at: "2026-04-13T00:00:00Z",
    finished_at: "2026-04-13T00:01:00Z",
    duration_secs: 60,
    before_sha: null,
    ...overrides,
  };
}

function rawEvent(overrides: Record<string, unknown> & { id: string }) {
  return {
    run_id: "run-1",
    event_type: "message",
    role: "assistant",
    content: "hello",
    tool_name: null,
    tool_input: null,
    tool_output: null,
    is_error: false,
    sequence_num: 0,
    created_at: "2026-04-13T00:00:00Z",
    ...overrides,
  };
}

describe("useRuns", () => {
  it("maps raw runs to the UI shape, coercing timestamps to Dates", async () => {
    tauriMock.handle("get_runs", () => [rawRun({ id: "run-1" })]);

    const { result } = renderHook(() => useRuns());
    await waitFor(() => expect(result.current.loading).toBe(false));

    const run = result.current.runs[0];
    expect(run.storyId).toBe("story-1");
    expect(run.agentName).toBe("Agent One");
    expect(run.status).toBe("done");
    expect(run.inputTokens).toBe(100);
    expect(run.cacheReadTokens).toBe(900);
    expect(run.cacheCreationTokens).toBe(40);
    expect(run.estimatedCostUsd).toBeCloseTo(0.0123);
    expect(run.startedAt).toBeInstanceOf(Date);
    expect(run.finishedAt).toBeInstanceOf(Date);
  });

  it("keeps a null finished_at as null rather than an Invalid Date", async () => {
    tauriMock.handle("get_runs", () => [
      rawRun({ id: "run-1", finished_at: null, duration_secs: null }),
    ]);

    const { result } = renderHook(() => useRuns());
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.runs[0].finishedAt).toBeNull();
    expect(result.current.runs[0].durationSecs).toBeNull();
  });

  it("sends null when no filters are supplied", async () => {
    tauriMock.handle("get_runs", () => []);

    const { result } = renderHook(() => useRuns());
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(tauriMock.calls("get_runs")).toEqual([{ filters: null }]);
  });

  it("passes filters through on the initial load and on refresh", async () => {
    tauriMock.handle("get_runs", () => []);

    const { result } = renderHook(() => useRuns({ status: "failed" }));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.refresh({ storyId: "story-9" });
    });

    expect(tauriMock.calls("get_runs")).toEqual([
      { filters: { status: "failed" } },
      { filters: { storyId: "story-9" } },
    ]);
  });

  it("records the error and stops loading when the fetch rejects", async () => {
    tauriMock.handle("get_runs", () => {
      throw new Error("no workspace is open");
    });

    const { result } = renderHook(() => useRuns());
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.error).toContain("no workspace is open");
    expect(result.current.runs).toEqual([]);
  });

  it("removes a deleted run locally without refetching", async () => {
    tauriMock.handle("get_runs", () => [rawRun({ id: "run-1" }), rawRun({ id: "run-2" })]);
    tauriMock.handle("delete_run", () => undefined);

    const { result } = renderHook(() => useRuns());
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      await result.current.deleteRun("run-1");
    });

    expect(result.current.runs.map((r) => r.id)).toEqual(["run-2"]);
    expect(tauriMock.callCount("get_runs")).toBe(1);
  });

  it("refetches when the workspace changes", async () => {
    let runs = [rawRun({ id: "run-1" })];
    tauriMock.handle("get_runs", () => runs);

    const { result } = renderHook(() => useRuns());
    await waitFor(() => expect(result.current.loading).toBe(false));

    runs = [rawRun({ id: "run-2" })];
    await tauriMock.emit("workspace-changed", {});

    await waitFor(() => expect(result.current.runs.map((r) => r.id)).toEqual(["run-2"]));
  });

  it("unsubscribes the workspace listener on unmount", async () => {
    tauriMock.handle("get_runs", () => []);

    const view = renderHook(() => useRuns());
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    expect(tauriMock.listenerCount("workspace-changed")).toBe(1);

    view.unmount();
    await waitFor(() => expect(tauriMock.listenerCount("workspace-changed")).toBe(0));
  });
});

describe("useRunEvents", () => {
  it("fetches and maps events for a run", async () => {
    tauriMock.handle("get_run_events", () => [
      rawEvent({ id: "e1", event_type: "tool_call", tool_name: "file_write" }),
    ]);

    const { result } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(result.current.events).toHaveLength(1));

    expect(result.current.events[0].eventType).toBe("tool_call");
    expect(result.current.events[0].toolName).toBe("file_write");
    expect(result.current.events[0].createdAt).toBeInstanceOf(Date);
  });

  it("clears events and does not invoke when the run id is null", async () => {
    tauriMock.handle("get_run_events", () => [rawEvent({ id: "e1" })]);

    const { result } = renderHook(() => useRunEvents(null));

    await waitFor(() => expect(result.current.events).toEqual([]));
    expect(tauriMock.called("get_run_events")).toBe(false);
  });

  it("refetches when the run id changes", async () => {
    tauriMock.handle("get_run_events", (args) => [
      rawEvent({ id: `e-${args.runId}`, run_id: String(args.runId) }),
    ]);

    const { result, rerender } = renderHook(({ id }) => useRunEvents(id), {
      initialProps: { id: "run-1" as string | null },
    });
    await waitFor(() => expect(result.current.events[0]?.runId).toBe("run-1"));

    rerender({ id: "run-2" });

    await waitFor(() => expect(result.current.events[0]?.runId).toBe("run-2"));
  });

  it("surfaces a fetch error without leaving loading stuck on", async () => {
    tauriMock.handle("get_run_events", () => {
      throw new Error("run not found");
    });

    const { result } = renderHook(() => useRunEvents("run-1"));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toContain("run not found");
  });

  // The point of the feature: an autonomous run could previously only be
  // watched by closing the detail panel and opening it again.
  it("appends tool calls and results as the run emits them", async () => {
    tauriMock.handle("get_run_events", () => []);

    const { result } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-1",
      tool_name: "file_write",
      input: { path: "a.txt" },
    });
    await tauriMock.emit("run-event", {
      type: "tool_result",
      run_id: "run-1",
      tool_name: "file_write",
      output: "written",
      is_error: false,
    });

    await waitFor(() => expect(result.current.events).toHaveLength(2));
    expect(result.current.events[0].eventType).toBe("tool_call");
    expect(result.current.events[0].toolInput).toBe(JSON.stringify({ path: "a.txt" }));
    expect(result.current.events[1].eventType).toBe("tool_result");
    expect(result.current.events[1].toolOutput).toBe("written");
    expect(result.current.events.map((e) => e.sequenceNum)).toEqual([0, 1]);
  });

  // The runtime emits one `Token` per text delta. A row per token would make a
  // long reply grow the timeline to a thousand rows and re-render every one of
  // them a thousand times.
  it("grows one message row as tokens stream instead of a row per token", async () => {
    tauriMock.handle("get_run_events", () => []);

    const { result } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    for (const content of ["Hel", "lo ", "there"]) {
      await tauriMock.emit("run-event", { type: "token", run_id: "run-1", content });
    }

    await waitFor(() => expect(result.current.events).toHaveLength(1));
    expect(result.current.events[0].eventType).toBe("message");
    expect(result.current.events[0].content).toBe("Hello there");
  });

  // Coalescing must not swallow the boundary between two replies.
  it("starts a new message row after a tool call interrupts the stream", async () => {
    tauriMock.handle("get_run_events", () => []);

    const { result } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await tauriMock.emit("run-event", { type: "token", run_id: "run-1", content: "before" });
    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-1",
      tool_name: "file_write",
      input: {},
    });
    await tauriMock.emit("run-event", { type: "token", run_id: "run-1", content: "after" });

    await waitFor(() => expect(result.current.events).toHaveLength(3));
    expect(result.current.events.map((e) => e.eventType)).toEqual([
      "message",
      "tool_call",
      "message",
    ]);
    expect(result.current.events[0].content).toBe("before");
    expect(result.current.events[2].content).toBe("after");
    expect(result.current.events.map((e) => e.sequenceNum)).toEqual([0, 1, 2]);
  });

  // Every run in the app emits on one `run-event` channel, so an unfiltered
  // listener would interleave three agents' work into one timeline.
  it("ignores events belonging to another run", async () => {
    tauriMock.handle("get_run_events", () => []);

    const { result } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-2",
      tool_name: "file_write",
      input: {},
    });

    expect(result.current.events).toEqual([]);
  });

  // A parked run looks identical to a slow one otherwise.
  it("shows a run parking on an approval and then moving again", async () => {
    tauriMock.handle("get_run_events", () => []);

    const { result } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await tauriMock.emit("run-event", {
      type: "awaiting_approval",
      run_id: "run-1",
      approval_request_id: "ap-1",
      tool_name: "file_write",
    });
    await waitFor(() => expect(result.current.events).toHaveLength(1));
    expect(result.current.events[0].eventType).toBe("approval_request");
    expect(result.current.events[0].content).toContain("file_write");

    await tauriMock.emit("run-event", {
      type: "approval_resolved",
      run_id: "run-1",
      approval_request_id: "ap-1",
      tool_name: "file_write",
      approved: true,
      outcome: "approved",
    });
    await waitFor(() => expect(result.current.events).toHaveLength(2));
    expect(result.current.events[1].eventType).toBe("approval_response");
  });

  // `complete` and `cancelled` are persisted with detail this payload does not
  // carry, so appending them live would double them on the next refresh.
  it("does not append events the database will return with more detail", async () => {
    tauriMock.handle("get_run_events", () => []);

    const { result } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await tauriMock.emit("run-event", {
      type: "complete",
      run_id: "run-1",
      stop_reason: "end_turn",
    });
    await tauriMock.emit("run-event", { type: "cancelled", run_id: "run-1" });

    expect(result.current.events).toEqual([]);
  });

  // The fetch and the subscription start together. If live events were
  // appended to the fetched array, the response landing second would wipe
  // whatever arrived while it was in flight.
  it("keeps a live event that arrives before the initial fetch resolves", async () => {
    let release: (rows: unknown[]) => void = () => {};
    tauriMock.handle(
      "get_run_events",
      () => new Promise((resolve) => { release = resolve; }),
    );

    const { result } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-1",
      tool_name: "file_write",
      input: {},
    });

    release([
      rawEvent({ id: "e1", event_type: "message", content: "from the database" }),
    ]);

    await waitFor(() => expect(result.current.events).toHaveLength(2));
    expect(result.current.events[0].content).toBe("from the database");
    expect(result.current.events[1].toolName).toBe("file_write");
    expect(result.current.events.map((e) => e.sequenceNum)).toEqual([0, 1]);
  });

  it("stops listening when the panel closes", async () => {
    tauriMock.handle("get_run_events", () => []);

    const { unmount } = renderHook(() => useRunEvents("run-1"));
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    unmount();

    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(0));
  });

  it("follows the new run when the panel switches between runs", async () => {
    tauriMock.handle("get_run_events", () => []);

    const { result, rerender } = renderHook(({ id }) => useRunEvents(id), {
      initialProps: { id: "run-1" as string | null },
    });
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    rerender({ id: "run-2" });
    await waitFor(() => expect(tauriMock.listenerCount("run-event")).toBe(1));

    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-1",
      tool_name: "stale",
      input: {},
    });
    expect(result.current.events).toEqual([]);

    await tauriMock.emit("run-event", {
      type: "tool_call",
      run_id: "run-2",
      tool_name: "current",
      input: {},
    });
    await waitFor(() => expect(result.current.events).toHaveLength(1));
    expect(result.current.events[0].toolName).toBe("current");
  });
});

describe("useRunDiff", () => {
  it("maps the raw diff payload to camelCase", async () => {
    tauriMock.handle("get_run_diff", () => ({
      run_id: "run-1",
      before_sha: "abc123",
      diff_output: "diff --git a/x b/x",
    }));

    const { result } = renderHook(() => useRunDiff("run-1"));
    await waitFor(() => expect(result.current.diff).not.toBeNull());

    expect(result.current.diff).toEqual({
      runId: "run-1",
      beforeSha: "abc123",
      diffOutput: "diff --git a/x b/x",
    });
  });

  it("clears the diff and does not invoke when the run id is null", async () => {
    tauriMock.handle("get_run_diff", () => ({
      run_id: "x",
      before_sha: null,
      diff_output: null,
    }));

    const { result } = renderHook(() => useRunDiff(null));

    await waitFor(() => expect(result.current.diff).toBeNull());
    expect(tauriMock.called("get_run_diff")).toBe(false);
  });
});

describe("exportRun", () => {
  it("writes one JSON object per line and revokes the object URL", async () => {
    const events = [{ id: "e1", type: "token" }, { id: "e2", type: "complete" }];
    tauriMock.handle("export_run_events", () => JSON.stringify(events));

    let blobText = "";
    const createObjectURL = vi
      .spyOn(URL, "createObjectURL")
      .mockImplementation((blob: Blob | MediaSource) => {
        blobText = (blob as Blob & { __text?: string }).__text ?? "";
        return "blob:mock";
      });
    const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    // jsdom's Blob does not expose its contents synchronously; capture the
    // parts as they are constructed instead.
    const OriginalBlob = globalThis.Blob;
    vi.stubGlobal(
      "Blob",
      class extends OriginalBlob {
        __text: string;
        constructor(parts: BlobPart[], options?: BlobPropertyBag) {
          super(parts, options);
          this.__text = parts.join("");
        }
      },
    );

    const anchor = document.createElement("a");
    const click = vi.spyOn(anchor, "click").mockImplementation(() => {});
    vi.spyOn(document, "createElement").mockReturnValue(anchor);

    await exportRun("run-1", "run-1.jsonl");

    expect(tauriMock.calls("export_run_events")).toEqual([{ runId: "run-1" }]);
    expect(blobText).toBe('{"id":"e1","type":"token"}\n{"id":"e2","type":"complete"}');
    expect(anchor.download).toBe("run-1.jsonl");
    expect(anchor.href).toContain("blob:mock");
    expect(click).toHaveBeenCalledOnce();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:mock");

    createObjectURL.mockRestore();
    revokeObjectURL.mockRestore();
    vi.unstubAllGlobals();
  });
});
