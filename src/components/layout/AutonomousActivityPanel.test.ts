// @vitest-environment node
import { describe, expect, it } from "vitest";

import { mergeLastAction } from "./AutonomousActivityPanel";

function event(overrides: Partial<Parameters<typeof mergeLastAction>[2]> = {}) {
  return {
    event_type: "message",
    role: "assistant",
    content: null,
    tool_name: null,
    is_error: false,
    ...overrides,
  };
}

describe("mergeLastAction", () => {
  it("records the first action for a run", () => {
    const next = mergeLastAction({}, "run-1", event({ event_type: "tool_call", role: null }));

    expect(next["run-1"]?.event_type).toBe("tool_call");
  });

  // `summarizeAction` renders every assistant message as "Drafting response",
  // so a second token cannot change the panel — and tokens arrive by the
  // thousand. Returning the same reference is what lets React skip the render.
  it("returns the same map when another token cannot change the label", () => {
    const prev = { "run-1": event({ content: "Hel" }) };

    const next = mergeLastAction(prev, "run-1", event({ content: "lo" }));

    expect(next).toBe(prev);
  });

  it("replaces a message with the tool call that follows it", () => {
    const prev = { "run-1": event({ content: "drafting" }) };

    const next = mergeLastAction(
      prev,
      "run-1",
      event({ event_type: "tool_call", role: null, tool_name: "file_write" }),
    );

    expect(next).not.toBe(prev);
    expect(next["run-1"]?.tool_name).toBe("file_write");
  });

  it("shows a run starting to draft after a tool result", () => {
    const prev = { "run-1": event({ event_type: "tool_result", role: null }) };

    const next = mergeLastAction(prev, "run-1", event({ content: "Hel" }));

    expect(next).not.toBe(prev);
    expect(next["run-1"]?.event_type).toBe("message");
  });

  it("keeps runs separate", () => {
    const prev = { "run-1": event({ content: "one" }) };

    const next = mergeLastAction(prev, "run-2", event({ content: "two" }));

    expect(next["run-1"]).toBe(prev["run-1"]);
    expect(next["run-2"]?.content).toBe("two");
  });
});
