import { tauriMock } from "../tauriMock";

// ---------------------------------------------------------------------------
// An in-memory stand-in for the chat-session half of the backend.
//
// Mirrors the real command surface (`list_chat_sessions`, `create_chat_session`,
// `start_chat_run`, `append_chat_session_message`, `get_chat_session_messages`)
// closely enough that ChatPage cannot tell the difference, including the
// rename-a-draft-from-its-first-message rule the backend applies.
// ---------------------------------------------------------------------------

export type RunEventPayload =
  | { type: "token"; run_id: string; content: string }
  | { type: "tool_call"; run_id: string; tool_name: string; input: unknown }
  | { type: "tool_result"; run_id: string; tool_name: string; output: string; is_error: boolean }
  | { type: "complete"; run_id: string; stop_reason: string }
  | { type: "cancelled"; run_id: string }
  | { type: "failed"; run_id: string; message: string };

export interface Profile {
  id: string;
  name: string;
}

export interface ChatSessionSummary {
  id: string;
  title: string;
  agent_profile_id: string | null;
  agent_name: string | null;
  last_message_preview: string | null;
  last_updated_at: string;
}

export interface ChatSessionMessage {
  id: string;
  session_id: string;
  role: "user" | "assistant";
  content: string;
  agent_profile_id: string | null;
  created_at: string;
}

/** Titles the backend treats as "still a draft", and so free to rename. */
const DRAFT_TITLES = ["", "new chat", "chat session"];

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

export function createChatBackend(options?: {
  sessions?: ChatSessionSummary[];
  messages?: Record<string, ChatSessionMessage[]>;
  profiles?: Profile[];
}) {
  const profiles = options?.profiles ?? [{ id: "agent-1", name: "Agent One" }];
  const sessions = cloneSessions(options?.sessions ?? []);
  const messagesBySession = cloneMessages(options?.messages ?? {});
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

  tauriMock.handleAll({
    get_profiles: () => profiles.map((profile) => ({ ...profile })),

    list_chat_sessions: () => cloneSessions(sessions),

    create_chat_session: (args) => {
      const title = args.title;
      const newSession: ChatSessionSummary = {
        id: `session-${nextSessionCounter++}`,
        title: typeof title === "string" && title.trim() ? title.trim() : "New Chat",
        agent_profile_id: null,
        agent_name: null,
        last_message_preview: null,
        last_updated_at: `2026-04-13T00:00:${String(nextSessionCounter).padStart(2, "0")}Z`,
      };
      sessions.unshift(newSession);
      messagesBySession[newSession.id] = [];
      return { ...newSession };
    },

    start_chat_run: (args) => {
      const sessionId =
        typeof args.sessionId === "string" && args.sessionId ? args.sessionId : sessions[0]?.id;
      const sessionTitle = typeof args.sessionTitle === "string" ? args.sessionTitle.trim() : "";
      const session = sessions.find((entry) => entry.id === sessionId);
      if (session && sessionTitle && DRAFT_TITLES.includes(session.title.trim().toLowerCase())) {
        session.title = sessionTitle;
      }
      return { run_id: `run-${nextRunCounter++}`, session_id: sessionId };
    },

    append_chat_session_message: (args) => {
      updateSessionFromMessage(
        String(args.sessionId),
        args.role as "user" | "assistant",
        String(args.content ?? ""),
        (args.agentProfileId as string | null | undefined) ?? null,
      );
      return undefined;
    },

    get_chat_session_messages: (args) =>
      (messagesBySession[String(args.sessionId)] ?? []).map((entry) => ({ ...entry })),

    stop_run: () => undefined,
  });

  return {
    getSessions: () => cloneSessions(sessions),
    getMessages: (sessionId: string) =>
      (messagesBySession[sessionId] ?? []).map((entry) => ({ ...entry })),
    emitRunEvent: (payload: RunEventPayload) => tauriMock.emit("run-event", payload),
  };
}
