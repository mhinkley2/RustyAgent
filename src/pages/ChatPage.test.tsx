import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";

import { invokeMock } from "../test/tauriMock";
import { createChatBackend } from "../test/backends/chatBackend";

// ChatPage keeps module-level singletons (`runEventBuffer`, `trackedRuns`, and
// a `globalEventListenerReady` flag) so a stream survives navigation. The setup
// file resets the module registry before each test; re-importing here gives
// every case its own instance of those singletons, which is what makes this
// suite order-independent.
let ChatPage: typeof import("./ChatPage").default;

function getSessionOpenButton(sessionTitle: string) {
  const candidates = screen.getAllByRole("button", { name: new RegExp(sessionTitle, "i") });
  const button = candidates.find((candidate) =>
    candidate.className.includes("chat-page__session-open"),
  );
  if (!button) {
    throw new Error(`Could not find session open button for ${sessionTitle}`);
  }
  return button;
}

describe("ChatPage", () => {
  beforeEach(async () => {
    ChatPage = (await import("./ChatPage")).default;
  });

  it("creates an initial persisted draft session when none exist", async () => {
    createChatBackend({ sessions: [] });

    render(<ChatPage />);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("create_chat_session", { title: null });
    });
    expect(await screen.findByText("New Chat")).toBeInTheDocument();
  });

  it("reuses an existing empty draft session instead of creating a duplicate", async () => {
    createChatBackend({
      sessions: [
        {
          id: "draft-1",
          title: "Draft A",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: "2026-04-13T00:00:01Z",
        },
      ],
    });

    render(<ChatPage />);

    expect(await screen.findByText("Draft A")).toBeInTheDocument();
    await waitFor(() => {
      expect(invokeMock).not.toHaveBeenCalledWith("create_chat_session", { title: null });
    });
  });

  it("persists user and assistant messages and allows switching sessions after completion", async () => {
    const backend = createChatBackend({
      sessions: [
        {
          id: "draft-1",
          title: "Draft Session",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: "2026-04-13T00:00:01Z",
        },
        {
          id: "session-2",
          title: "Previous Session",
          agent_profile_id: "agent-1",
          agent_name: "Agent One",
          last_message_preview: "Earlier answer",
          last_updated_at: "2026-04-13T00:00:02Z",
        },
      ],
      messages: {
        "draft-1": [],
        "session-2": [
          {
            id: "message-1",
            session_id: "session-2",
            role: "user",
            content: "Earlier question",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:02Z",
          },
          {
            id: "message-2",
            session_id: "session-2",
            role: "assistant",
            content: "Earlier answer",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:03Z",
          },
        ],
      },
    });

    const user = userEvent.setup();
    const view = render(<ChatPage />);

    const textarea = await screen.findByPlaceholderText(/message the agent/i);
    await user.type(textarea, "Hello agent");
    await user.keyboard("{Enter}");

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "start_chat_run",
        expect.objectContaining({
          profileId: "agent-1",
          sessionId: "draft-1",
          sessionTitle: "Hello agent",
        }),
      );
    });

    const previousSessionButton = getSessionOpenButton("Previous Session");
    expect(previousSessionButton).toBeDisabled();

    await backend.emitRunEvent({ type: "token", run_id: "run-1", content: "Hello back" });
    await backend.emitRunEvent({ type: "complete", run_id: "run-1", stop_reason: "end_turn" });

    await waitFor(() => {
      expect(previousSessionButton).not.toBeDisabled();
    });

    expect(backend.getMessages("draft-1").map((message) => message.role)).toEqual(["user", "assistant"]);
    expect(backend.getMessages("draft-1")[0]?.content).toBe("Hello agent");
    expect(backend.getMessages("draft-1")[1]?.content).toBe("Hello back");

    await user.click(previousSessionButton);

    const thread = view.container.querySelector(".chat-page__thread");
    expect(thread).not.toBeNull();
    const threadQueries = within(thread as HTMLElement);
    expect(await threadQueries.findByText("Earlier question")).toBeInTheDocument();
    expect(await threadQueries.findByText("Earlier answer")).toBeInTheDocument();
  });

  it("creates and selects a persisted session when starting a new chat", async () => {
    const backend = createChatBackend({
      sessions: [
        {
          id: "session-1",
          title: "Existing Session",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: "Saved",
          last_updated_at: "2026-04-13T00:00:01Z",
        },
      ],
      messages: {
        "session-1": [],
      },
    });

    const user = userEvent.setup();
    render(<ChatPage />);

    await screen.findByText("Existing Session");
    await user.click(screen.getByRole("button", { name: /^\+ New$/ }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("create_chat_session", { title: null });
    });

    const sessions = backend.getSessions();
    expect(sessions[0]?.title).toBe("New Chat");
    expect(await screen.findAllByText("New Chat")).not.toHaveLength(0);
  });

  it("shows an empty-session state when a draft session is selected", async () => {
    createChatBackend({
      sessions: [
        {
          id: "draft-1",
          title: "New Chat",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: "2026-04-13T00:00:01Z",
        },
      ],
    });

    render(<ChatPage />);

    expect(await screen.findByText("This session has no messages yet. Start chatting to begin the conversation.")).toBeInTheDocument();
  });

  it("renames a draft session from the first user message", async () => {
    const backend = createChatBackend({
      sessions: [
        {
          id: "draft-1",
          title: "New Chat",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: "2026-04-13T00:00:01Z",
        },
      ],
      messages: {
        "draft-1": [],
      },
    });

    const user = userEvent.setup();
    render(<ChatPage />);

    const textarea = await screen.findByPlaceholderText(/message the agent/i);
    await user.type(textarea, "Ask about release blockers");
    await user.keyboard("{Enter}");

    await waitFor(() => {
      expect(getSessionOpenButton("Ask about release blockers")).toBeInTheDocument();
    });

    expect(backend.getSessions()[0]?.title).toBe("Ask about release blockers");
  });

  it("switches back to a newly created empty session after opening another session", async () => {
    const user = userEvent.setup();
    createChatBackend({
      sessions: [
        {
          id: "session-1",
          title: "Existing Session",
          agent_profile_id: "agent-1",
          agent_name: "Agent One",
          last_message_preview: "Saved answer",
          last_updated_at: "2026-04-13T00:00:01Z",
        },
      ],
      messages: {
        "session-1": [
          {
            id: "message-1",
            session_id: "session-1",
            role: "user",
            content: "Saved question",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:01Z",
          },
          {
            id: "message-2",
            session_id: "session-1",
            role: "assistant",
            content: "Saved answer",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:02Z",
          },
        ],
      },
    });

    const view = render(<ChatPage />);

    await screen.findByText("Existing Session");
    await user.click(screen.getByRole("button", { name: /^\+ New$/ }));

    const newChatButton = getSessionOpenButton("New Chat");
    await user.click(getSessionOpenButton("Existing Session"));

    const thread = view.container.querySelector(".chat-page__thread");
    expect(thread).not.toBeNull();
    const threadQueries = within(thread as HTMLElement);
    expect(await threadQueries.findByText("Saved question")).toBeInTheDocument();
    expect(await threadQueries.findByText("Saved answer")).toBeInTheDocument();

    await user.click(newChatButton);

    await waitFor(() => {
      expect(threadQueries.queryByText("Saved question")).not.toBeInTheDocument();
      expect(threadQueries.queryByText("Saved answer")).not.toBeInTheDocument();
    });
  });

  it("opens a session when clicking anywhere on the session row", async () => {
    const user = userEvent.setup();
    createChatBackend({
      sessions: [
        {
          id: "session-1",
          title: "First Session",
          agent_profile_id: "agent-1",
          agent_name: "Agent One",
          last_message_preview: "First answer",
          last_updated_at: "2026-04-13T00:00:01Z",
        },
        {
          id: "session-2",
          title: "Second Session",
          agent_profile_id: "agent-1",
          agent_name: "Agent One",
          last_message_preview: "Second answer",
          last_updated_at: "2026-04-13T00:00:02Z",
        },
      ],
      messages: {
        "session-1": [
          {
            id: "message-1",
            session_id: "session-1",
            role: "user",
            content: "First question",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:01Z",
          },
          {
            id: "message-2",
            session_id: "session-1",
            role: "assistant",
            content: "First answer",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:02Z",
          },
        ],
        "session-2": [
          {
            id: "message-3",
            session_id: "session-2",
            role: "user",
            content: "Second question",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:03Z",
          },
          {
            id: "message-4",
            session_id: "session-2",
            role: "assistant",
            content: "Second answer",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:04Z",
          },
        ],
      },
    });

    const view = render(<ChatPage />);

    await screen.findByText("Second Session");
    const secondSessionPreview = screen.getByText("Second answer");
    await user.click(secondSessionPreview);

    const thread = view.container.querySelector(".chat-page__thread");
    expect(thread).not.toBeNull();
    const threadQueries = within(thread as HTMLElement);
    expect(await threadQueries.findByText("Second question")).toBeInTheDocument();
    expect(await threadQueries.findByText("Second answer")).toBeInTheDocument();
  });

  it("restores the active session after the chat page remounts", async () => {
    const user = userEvent.setup();
    createChatBackend({
      sessions: [
        {
          id: "draft-1",
          title: "New Chat",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: "2026-04-13T00:00:01Z",
        },
        {
          id: "session-2",
          title: "Saved Session",
          agent_profile_id: "agent-1",
          agent_name: "Agent One",
          last_message_preview: "Saved answer",
          last_updated_at: "2026-04-13T00:00:02Z",
        },
      ],
      messages: {
        "draft-1": [],
        "session-2": [
          {
            id: "message-1",
            session_id: "session-2",
            role: "user",
            content: "Saved question",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:02Z",
          },
          {
            id: "message-2",
            session_id: "session-2",
            role: "assistant",
            content: "Saved answer",
            agent_profile_id: "agent-1",
            created_at: "2026-04-13T00:00:03Z",
          },
        ],
      },
    });

    const firstRender = render(<ChatPage />);
    await screen.findByText("Saved Session");

    await user.click(getSessionOpenButton("Saved Session"));

    let thread = firstRender.container.querySelector(".chat-page__thread");
    expect(thread).not.toBeNull();
    let threadQueries = within(thread as HTMLElement);
    expect(await threadQueries.findByText("Saved question")).toBeInTheDocument();
    expect(await threadQueries.findByText("Saved answer")).toBeInTheDocument();
    expect(window.localStorage.getItem("rustyagent.chat.activeSessionId")).toBe("session-2");

    firstRender.unmount();

    const secondRender = render(<ChatPage />);
    thread = secondRender.container.querySelector(".chat-page__thread");
    expect(thread).not.toBeNull();
    threadQueries = within(thread as HTMLElement);

    expect(await threadQueries.findByText("Saved question")).toBeInTheDocument();
    expect(await threadQueries.findByText("Saved answer")).toBeInTheDocument();
  });

  it("preserves streamed assistant response after navigating away and back", async () => {
    const backend = createChatBackend({
      sessions: [
        {
          id: "draft-1",
          title: "New Chat",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: "2026-04-13T00:00:01Z",
        },
      ],
      messages: {
        "draft-1": [],
      },
    });

    const user = userEvent.setup();
    const firstRender = render(<ChatPage />);

    const textarea = await screen.findByPlaceholderText(/message the agent/i);
    await user.type(textarea, "Keep streaming while I navigate");
    await user.keyboard("{Enter}");

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "start_chat_run",
        expect.objectContaining({
          sessionId: "draft-1",
          sessionTitle: "Keep streaming while I navigate",
        }),
      );
    });

    firstRender.unmount();

    await backend.emitRunEvent({ type: "token", run_id: "run-1", content: "Recovered" });
    await backend.emitRunEvent({ type: "token", run_id: "run-1", content: " response" });
    await backend.emitRunEvent({ type: "complete", run_id: "run-1", stop_reason: "end_turn" });

    const secondRender = render(<ChatPage />);
    const thread = secondRender.container.querySelector(".chat-page__thread");
    expect(thread).not.toBeNull();
    const threadQueries = within(thread as HTMLElement);

    expect(await threadQueries.findByText("Keep streaming while I navigate")).toBeInTheDocument();
    expect(await threadQueries.findByText("Recovered response")).toBeInTheDocument();
  });

  // -------------------------------------------------------------------------
  // Streaming edge cases
  // -------------------------------------------------------------------------

  /** A backend with one empty draft session, ready to send into. */
  function draftBackend() {
    return createChatBackend({
      sessions: [
        {
          id: "draft-1",
          title: "New Chat",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: "2026-04-13T00:00:01Z",
        },
      ],
      messages: { "draft-1": [] },
    });
  }

  /** Type a message and press Enter, then wait for the run to start. */
  async function send(text: string) {
    const user = userEvent.setup();
    const textarea = await screen.findByPlaceholderText(/message the agent/i);
    await user.type(textarea, text);
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "start_chat_run",
        expect.objectContaining({ sessionId: "draft-1" }),
      );
    });
    return user;
  }

  it("renders tool calls and their results in the transcript", async () => {
    const backend = draftBackend();
    const view = render(<ChatPage />);
    await send("Write a file");

    await backend.emitRunEvent({
      type: "tool_call",
      run_id: "run-1",
      tool_name: "file_write",
      input: { path: "a.txt" },
    });
    await backend.emitRunEvent({
      type: "tool_result",
      run_id: "run-1",
      tool_name: "file_write",
      output: "Successfully wrote to 'a.txt'",
      is_error: false,
    });
    await backend.emitRunEvent({ type: "complete", run_id: "run-1", stop_reason: "end_turn" });

    // The transcript summarises tool activity rather than echoing raw names.
    const thread = view.container.querySelector(".chat-page__thread") as HTMLElement;
    await waitFor(() => {
      expect(thread.textContent).toContain("tool action");
    });
    expect(thread.textContent).toContain("file write");
  });

  it("surfaces the message from a failed run and re-enables the composer", async () => {
    const backend = draftBackend();
    render(<ChatPage />);
    await send("Do the thing");

    await backend.emitRunEvent({
      type: "failed",
      run_id: "run-1",
      message: "LLM call failed: upstream is down",
    });

    // The message renders inside a status entry (and so also matches its
    // ancestors) — assert at least one match rather than exactly one.
    const matches = await screen.findAllByText(/upstream is down/i);
    expect(matches.length).toBeGreaterThan(0);
    const textarea = await screen.findByPlaceholderText(/message the agent/i);
    await waitFor(() => expect(textarea).not.toBeDisabled());
  });

  it("persists partial assistant text when a run is cancelled", async () => {
    const backend = draftBackend();
    render(<ChatPage />);
    await send("Start something long");

    await backend.emitRunEvent({ type: "token", run_id: "run-1", content: "Partial " });
    await backend.emitRunEvent({ type: "token", run_id: "run-1", content: "answer" });
    await backend.emitRunEvent({ type: "cancelled", run_id: "run-1" });

    await waitFor(() => {
      const roles = backend.getMessages("draft-1").map((m) => m.role);
      expect(roles).toEqual(["user", "assistant"]);
    });
    expect(backend.getMessages("draft-1")[1].content).toBe("Partial answer");
  });

  it("does not persist an assistant message when the stream produced no text", async () => {
    const backend = draftBackend();
    render(<ChatPage />);
    await send("Say nothing");

    await backend.emitRunEvent({ type: "complete", run_id: "run-1", stop_reason: "end_turn" });

    await waitFor(() => {
      expect(backend.getMessages("draft-1").map((m) => m.role)).toEqual(["user"]);
    });
  });

  it("buffers events for a session that is not on screen and replays them on return", async () => {
    // The whole point of the module-level run buffer: a stream that continues
    // while the user is looking at another session must not be lost.
    const backend = createChatBackend({
      sessions: [
        {
          id: "draft-1",
          title: "New Chat",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: "2026-04-13T00:00:02Z",
        },
        {
          id: "session-2",
          title: "Other Session",
          agent_profile_id: "agent-1",
          agent_name: "Agent One",
          last_message_preview: "Older",
          last_updated_at: "2026-04-13T00:00:01Z",
        },
      ],
      messages: { "draft-1": [], "session-2": [] },
    });

    const view = render(<ChatPage />);
    await send("Stream while I look away");

    view.unmount();

    await backend.emitRunEvent({ type: "token", run_id: "run-1", content: "Buffered " });
    await backend.emitRunEvent({ type: "token", run_id: "run-1", content: "reply" });
    await backend.emitRunEvent({ type: "complete", run_id: "run-1", stop_reason: "end_turn" });

    await waitFor(() => {
      expect(backend.getMessages("draft-1")[1]?.content).toBe("Buffered reply");
    });

    const second = render(<ChatPage />);
    const thread = second.container.querySelector(".chat-page__thread") as HTMLElement;
    expect(await within(thread).findByText("Buffered reply")).toBeInTheDocument();
  });

  it("ignores run events belonging to a different run", async () => {
    const backend = draftBackend();
    const view = render(<ChatPage />);
    await send("Only mine");

    // An event from an unrelated run must not land in this transcript.
    await backend.emitRunEvent({ type: "token", run_id: "run-999", content: "NOT MINE" });
    await backend.emitRunEvent({ type: "token", run_id: "run-1", content: "mine" });
    await backend.emitRunEvent({ type: "complete", run_id: "run-1", stop_reason: "end_turn" });

    const thread = view.container.querySelector(".chat-page__thread") as HTMLElement;
    await waitFor(() => expect(thread.textContent).toContain("mine"));
    expect(thread.textContent).not.toContain("NOT MINE");
    await waitFor(() => {
      expect(backend.getMessages("draft-1")[1]?.content).toBe("mine");
    });
  });
});
