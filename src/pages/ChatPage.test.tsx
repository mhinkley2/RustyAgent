import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ChatPage from "./ChatPage";

const invokeMock = vi.fn();
const listenMock = vi.fn();

type RunEventPayload =
  | { type: "token"; run_id: string; content: string }
  | { type: "tool_call"; run_id: string; tool_name: string; input: unknown }
  | { type: "tool_result"; run_id: string; tool_name: string; output: string; is_error: boolean }
  | { type: "complete"; run_id: string; stop_reason: string }
  | { type: "cancelled"; run_id: string }
  | { type: "failed"; run_id: string; message: string };

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

interface Profile {
  id: string;
  name: string;
}

interface ChatSessionSummary {
  id: string;
  title: string;
  agent_profile_id: string | null;
  agent_name: string | null;
  last_message_preview: string | null;
  last_updated_at: string;
}

interface ChatSessionMessage {
  id: string;
  session_id: string;
  role: "user" | "assistant";
  content: string;
  agent_profile_id: string | null;
  created_at: string;
}

function cloneSessions(sessions: ChatSessionSummary[]) {
  return sessions.map((session) => ({ ...session }));
}

function cloneMessages(messages: Record<string, ChatSessionMessage[]>) {
  return Object.fromEntries(
    Object.entries(messages).map(([sessionId, entries]) => [
      sessionId,
      entries.map((entry) => ({ ...entry })),
    ]),
  ) as Record<string, ChatSessionMessage[]>;
}

function createChatBackend(options?: {
  sessions?: ChatSessionSummary[];
  messages?: Record<string, ChatSessionMessage[]>;
  profiles?: Profile[];
}) {
  const profiles = options?.profiles ?? [{ id: "agent-1", name: "Agent One" }];
  const sessions = cloneSessions(options?.sessions ?? []);
  const messagesBySession = cloneMessages(options?.messages ?? {});
  const runEventListeners = new Set<(event: { payload: RunEventPayload }) => void>();
  let nextSessionCounter = sessions.length + 1;
  let nextMessageCounter = 1;
  let nextRunCounter = 1;

  const findProfileName = (profileId: string | null | undefined) =>
    profiles.find((profile) => profile.id === profileId)?.name ?? null;

  const updateSessionFromMessage = (
    sessionId: string,
    role: "user" | "assistant",
    content: string,
    agentProfileId: string | null,
  ) => {
    const session = sessions.find((entry) => entry.id === sessionId);
    if (!session) return;
    session.last_message_preview = content;
    session.last_updated_at = `2026-04-13T00:00:${String(nextMessageCounter).padStart(2, "0")}Z`;
    session.agent_profile_id = agentProfileId;
    session.agent_name = findProfileName(agentProfileId);

    const nextMessage: ChatSessionMessage = {
      id: `message-${nextMessageCounter++}`,
      session_id: sessionId,
      role,
      content,
      agent_profile_id: agentProfileId,
      created_at: session.last_updated_at,
    };

    if (!messagesBySession[sessionId]) messagesBySession[sessionId] = [];
    messagesBySession[sessionId].push(nextMessage);
  };

  listenMock.mockImplementation(async (eventName: string, callback: (event: { payload: RunEventPayload }) => void) => {
    if (eventName === "run-event") {
      runEventListeners.add(callback);
      return () => {
        runEventListeners.delete(callback);
      };
    }
    return () => {};
  });

  invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "get_profiles":
        return profiles.map((profile) => ({ ...profile }));
      case "list_chat_sessions":
        return cloneSessions(sessions);
      case "create_chat_session": {
        const newSession: ChatSessionSummary = {
          id: `session-${nextSessionCounter++}`,
          title: typeof args?.title === "string" && args.title.trim() ? args.title.trim() : "New Chat",
          agent_profile_id: null,
          agent_name: null,
          last_message_preview: null,
          last_updated_at: `2026-04-13T00:00:${String(nextSessionCounter).padStart(2, "0")}Z`,
        };
        sessions.unshift(newSession);
        messagesBySession[newSession.id] = [];
        return { ...newSession };
      }
      case "start_chat_run": {
        const sessionId = typeof args?.sessionId === "string" && args.sessionId ? args.sessionId : sessions[0]?.id;
        const sessionTitle = typeof args?.sessionTitle === "string" ? args.sessionTitle.trim() : "";
        const session = sessions.find((entry) => entry.id === sessionId);
        if (session && sessionTitle && ["", "new chat", "chat session"].includes(session.title.trim().toLowerCase())) {
          session.title = sessionTitle;
        }
        return {
          run_id: `run-${nextRunCounter++}`,
          session_id: sessionId,
        };
      }
      case "append_chat_session_message": {
        updateSessionFromMessage(
          String(args?.sessionId),
          args?.role as "user" | "assistant",
          String(args?.content ?? ""),
          (args?.agentProfileId as string | null | undefined) ?? null,
        );
        return undefined;
      }
      case "get_chat_session_messages":
        return (messagesBySession[String(args?.sessionId)] ?? []).map((entry) => ({ ...entry }));
      default:
        throw new Error(`Unhandled invoke command: ${command}`);
    }
  });

  return {
    getSessions: () => cloneSessions(sessions),
    getMessages: (sessionId: string) => (messagesBySession[sessionId] ?? []).map((entry) => ({ ...entry })),
    emitRunEvent: async (payload: RunEventPayload) => {
      await act(async () => {
        for (const listener of runEventListeners) {
          listener({ payload });
        }
      });
    },
  };
}

function getSessionOpenButton(sessionTitle: string) {
  const candidates = screen.getAllByRole("button", { name: new RegExp(sessionTitle, "i") });
  const button = candidates.find((candidate) => candidate.className.includes("chat-page__session-open"));
  if (!button) {
    throw new Error(`Could not find session open button for ${sessionTitle}`);
  }
  return button;
}

describe("ChatPage", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    window.localStorage.clear();
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
});
