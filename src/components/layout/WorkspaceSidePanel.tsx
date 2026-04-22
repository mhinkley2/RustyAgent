import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Link } from "react-router-dom";
import { Activity, MessageSquare, Send, Square } from "lucide-react";
import AutonomousActivityPanel from "./AutonomousActivityPanel";
import { useSidePanel } from "./SidePanelContext";

interface Profile {
  id: string;
  name: string;
}

interface ChatSessionSummary {
  id: string;
  title: string;
  agent_profile_id: string | null;
  last_message_preview: string | null;
}

interface ChatSessionMessage {
  id: string;
  session_id: string;
  role: "user" | "assistant";
  content: string;
}

type RunEventPayload =
  | { type: "token"; run_id: string; content: string }
  | { type: "complete"; run_id: string; stop_reason: string }
  | { type: "cancelled"; run_id: string }
  | { type: "failed"; run_id: string; message: string };

interface PanelMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
}

const ACTIVE_CHAT_SESSION_KEY = "rustyagent.chat.activeSessionId";

function readStoredActiveSessionId(): string | null {
  try {
    return localStorage.getItem(ACTIVE_CHAT_SESSION_KEY);
  } catch {
    return null;
  }
}

function writeStoredActiveSessionId(sessionId: string | null) {
  try {
    if (sessionId) localStorage.setItem(ACTIVE_CHAT_SESSION_KEY, sessionId);
    else localStorage.removeItem(ACTIVE_CHAT_SESSION_KEY);
  } catch {
    // ignore storage failures
  }
}

function toPanelMessages(messages: ChatSessionMessage[]): PanelMessage[] {
  return messages.map((m) => ({
    id: m.id,
    role: m.role,
    content: m.content,
  }));
}

function buildChatMessages(messages: PanelMessage[]): Array<{ role: string; content: string }> {
  return messages
    .filter((m) => (m.role === "user" || m.role === "assistant") && m.content.trim().length > 0)
    .map((m) => ({ role: m.role, content: m.content }));
}

function ChatSidePanel() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [profileId, setProfileId] = useState<string>("");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<PanelMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const runIdRef = useRef<string | null>(null);
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const assistantMsgIdRef = useRef<string | null>(null);
  const assistantTextRef = useRef("");
  const sessionIdRef = useRef<string | null>(null);
  const threadRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    sessionIdRef.current = sessionId;
    writeStoredActiveSessionId(sessionId);
  }, [sessionId]);

  useEffect(() => {
    if (!threadRef.current) return;
    threadRef.current.scrollTop = threadRef.current.scrollHeight;
  }, [messages]);

  const loadProfiles = useCallback(async () => {
    const nextProfiles = await invoke<Profile[]>("get_profiles");
    setProfiles(nextProfiles);
    setProfileId((prev) => {
      if (prev && nextProfiles.some((p) => p.id === prev)) return prev;
      return nextProfiles[0]?.id ?? "";
    });
  }, []);

  const loadMessages = useCallback(async (id: string) => {
    const history = await invoke<ChatSessionMessage[]>("get_chat_session_messages", { sessionId: id });
    setMessages(toPanelMessages(history));
  }, []);

  const restoreSession = useCallback(async () => {
    const sessions = await invoke<ChatSessionSummary[]>("list_chat_sessions", { limit: 50 });
    const storedId = readStoredActiveSessionId();
    const target =
      (storedId ? sessions.find((s) => s.id === storedId) : null) ??
      sessions.find((s) => Boolean(s.last_message_preview)) ??
      sessions[0] ??
      null;

    if (!target) return;

    setSessionId(target.id);
    sessionIdRef.current = target.id;
    if (target.agent_profile_id) setProfileId(target.agent_profile_id);
    await loadMessages(target.id);
  }, [loadMessages]);

  useEffect(() => {
    let mounted = true;

    (async () => {
      try {
        await loadProfiles();
        if (!mounted) return;
        await restoreSession();
      } catch (err) {
        if (!mounted) return;
        const msg = err instanceof Error ? err.message : String(err);
        setError(`Failed to initialize panel chat: ${msg}`);
      }
    })();

    return () => {
      mounted = false;
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, [loadProfiles, restoreSession]);

  const ensureSession = useCallback(async (): Promise<string> => {
    if (sessionIdRef.current) return sessionIdRef.current;
    const created = await invoke<ChatSessionSummary>("create_chat_session", { title: null });
    setSessionId(created.id);
    sessionIdRef.current = created.id;
    return created.id;
  }, []);

  const stopRun = useCallback(async () => {
    const rid = runIdRef.current;
    if (!rid) return;
    try {
      await invoke("stop_run", { runId: rid });
    } catch {
      // ignore
    }
  }, []);

  const sendMessage = useCallback(async () => {
    const text = draft.trim();
    if (!text || isRunning || !profileId) return;

    setError(null);
    setDraft("");
    setIsRunning(true);

    const userMsg: PanelMessage = {
      id: `u-${Date.now()}`,
      role: "user",
      content: text,
    };
    const assistantMsgId = `a-${Date.now()}`;
    const assistantMsg: PanelMessage = {
      id: assistantMsgId,
      role: "assistant",
      content: "",
    };

    assistantMsgIdRef.current = assistantMsgId;
    assistantTextRef.current = "";

    setMessages((prev) => [...prev, userMsg, assistantMsg]);

    try {
      unlistenRef.current?.();
      unlistenRef.current = await listen<RunEventPayload>("run-event", ({ payload }) => {
        if (!runIdRef.current || payload.run_id !== runIdRef.current) return;

        if (payload.type === "token") {
          assistantTextRef.current += payload.content;
          const currentId = assistantMsgIdRef.current;
          if (!currentId) return;
          setMessages((prev) =>
            prev.map((msg) =>
              msg.id === currentId
                ? { ...msg, content: (msg.content || "") + payload.content }
                : msg,
            ),
          );
          return;
        }

        if (payload.type === "failed") {
          const currentId = assistantMsgIdRef.current;
          if (currentId && !assistantTextRef.current.trim()) {
            setMessages((prev) =>
              prev.map((msg) =>
                msg.id === currentId ? { ...msg, content: `Error: ${payload.message}` } : msg,
              ),
            );
          }
        }

        if (payload.type === "complete" || payload.type === "failed" || payload.type === "cancelled") {
          const sid = sessionIdRef.current;
          if (sid && assistantTextRef.current.trim()) {
            void invoke("append_chat_session_message", {
              sessionId: sid,
              role: "assistant",
              content: assistantTextRef.current.trim(),
              agentProfileId: profileId,
            });
          }
          setIsRunning(false);
          runIdRef.current = null;
          unlistenRef.current?.();
          unlistenRef.current = null;
        }
      });

      const sid = await ensureSession();
      await invoke("append_chat_session_message", {
        sessionId: sid,
        role: "user",
        content: text,
        agentProfileId: profileId,
      });

      const chatMessages = buildChatMessages([...messages, userMsg]);
      const response = await invoke<{ run_id: string; session_id: string }>("start_chat_run", {
        profileId,
        messages: chatMessages,
        sessionId: sid,
        sessionTitle: text.slice(0, 60),
      });

      runIdRef.current = response.run_id;
      if (!sessionIdRef.current || sessionIdRef.current !== response.session_id) {
        sessionIdRef.current = response.session_id;
        setSessionId(response.session_id);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      setIsRunning(false);
      runIdRef.current = null;
      unlistenRef.current?.();
      unlistenRef.current = null;
    }
  }, [draft, ensureSession, isRunning, messages, profileId]);

  return (
    <section className="chat-side-panel" aria-label="Global chat panel">
      <header className="chat-side-panel__header">
        <h2 className="chat-side-panel__title">Chat</h2>
        <Link className="chat-side-panel__open-full" to="/chat">
          Open full chat
        </Link>
      </header>

      <div className="chat-side-panel__controls">
        <select
          className="chat-side-panel__profile"
          value={profileId}
          onChange={(event) => setProfileId(event.target.value)}
          disabled={isRunning}
        >
          {profiles.length === 0 && <option value="">No agents available</option>}
          {profiles.map((profile) => (
            <option key={profile.id} value={profile.id}>
              {profile.name}
            </option>
          ))}
        </select>
      </div>

      <div className="chat-side-panel__thread" ref={threadRef}>
        {messages.length === 0 ? (
          <div className="chat-side-panel__empty">Start a message to create or resume a session.</div>
        ) : (
          messages.map((msg) => (
            <div
              key={msg.id}
              className={[
                "chat-side-panel__msg",
                msg.role === "user" ? "chat-side-panel__msg--user" : "chat-side-panel__msg--assistant",
              ].join(" ")}
            >
              <div className="chat-side-panel__msg-role">{msg.role === "user" ? "You" : "Assistant"}</div>
              <div className="chat-side-panel__msg-body">{msg.content || (isRunning ? "..." : "")}</div>
            </div>
          ))
        )}
      </div>

      {error && <div className="chat-side-panel__error">{error}</div>}

      <div className="chat-side-panel__composer">
        <textarea
          className="chat-side-panel__textarea"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void sendMessage();
            }
          }}
          placeholder="Message the agent..."
          disabled={isRunning || !profileId}
          rows={2}
        />
        {isRunning ? (
          <button
            type="button"
            className="chat-side-panel__send chat-side-panel__send--stop"
            onClick={() => void stopRun()}
            aria-label="Stop generation"
          >
            <Square size={14} />
          </button>
        ) : (
          <button
            type="button"
            className="chat-side-panel__send"
            onClick={() => void sendMessage()}
            disabled={!draft.trim() || !profileId}
            aria-label="Send message"
          >
            <Send size={14} />
          </button>
        )}
      </div>
    </section>
  );
}

export default function WorkspaceSidePanel() {
  const { mode, setMode } = useSidePanel();

  return (
    <section className="workspace-side-panel" aria-label="Side panel">
      <div className="workspace-side-panel__tabs" role="tablist" aria-label="Side panel tabs">
        <button
          type="button"
          role="tab"
          className={`workspace-side-panel__tab${mode === "chat" ? " is-active" : ""}`}
          aria-selected={mode === "chat"}
          onClick={() => setMode("chat")}
        >
          <MessageSquare size={14} />
          Chat
        </button>
        <button
          type="button"
          role="tab"
          className={`workspace-side-panel__tab${mode === "activity" ? " is-active" : ""}`}
          aria-selected={mode === "activity"}
          onClick={() => setMode("activity")}
        >
          <Activity size={14} />
          Activity
        </button>
      </div>

      <div className="workspace-side-panel__body">
        {mode === "chat" ? <ChatSidePanel /> : <AutonomousActivityPanel />}
      </div>
    </section>
  );
}
