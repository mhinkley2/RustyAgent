import { useNavigate } from "react-router-dom";
import { Circle, Flag, OctagonAlert, Zap } from "lucide-react";
import { useAgentStatuses } from "../../hooks/useAgentStatus";
import { useSidePanel } from "./SidePanelContext";

interface StatusBarProps {
  pendingHumanCount: number;
  pendingApprovalCount: number;
}

function getTheme(): "dark" | "light" {
  return (document.documentElement.getAttribute("data-theme") ?? "dark") as "dark" | "light";
}

function toggleTheme() {
  const current = getTheme();
  const next = current === "dark" ? "light" : "dark";
  document.documentElement.setAttribute("data-theme", next);
  try { localStorage.setItem("rustyagent.theme", next); } catch { /* ignore */ }
}

export default function StatusBar({ pendingHumanCount, pendingApprovalCount }: StatusBarProps) {
  const navigate = useNavigate();
  const { statuses } = useAgentStatuses();
  const { open, setOpen, mode, setMode } = useSidePanel();

  const statusList = Object.values(statuses);
  const runningAgents = statusList.filter(
    (s) => s.state === "running_story",
  ).length;
  const runtimePending = statusList.filter(
    (s) => s.state === "waiting_for_approval" || s.state === "waiting_for_human_input",
  ).length;
  const failedAgents = statusList.filter((s) => s.state === "failed").length;
  const hitlPending = Math.max(pendingHumanCount + pendingApprovalCount, runtimePending);
  const mcpConnected = 0;

  return (
    <footer
      className={[
        "status-bar",
        failedAgents > 0 ? "status-bar--error" : hitlPending > 0 ? "status-bar--warning" : "",
      ].join(" ")}
      aria-label="App status"
    >
      {/* Left items */}
      <div className="status-bar__left">
        <button
          className="status-bar__item"
          onClick={() => {
            if (mode !== "chat") {
              setMode("chat");
              setOpen(true);
              return;
            }
            setOpen(!open);
          }}
          aria-label="Toggle chat side panel"
        >
          Chat
        </button>

        <button
          className="status-bar__item"
          onClick={() => {
            if (mode !== "activity") {
              setMode("activity");
              setOpen(true);
              return;
            }
            setOpen(!open);
          }}
          aria-label="Toggle autonomous activity panel"
        >
          Activity
        </button>

        {runningAgents > 0 && (
          <button
            className="status-bar__item"
            onClick={() => navigate("/runs?status=running")}
            aria-label={`${runningAgents} agents running`}
          >
            <Circle size={8} className="status-bar__dot status-bar__dot--green" />
            {runningAgents} running
          </button>
        )}

        {hitlPending > 0 && (
          <button
            className="status-bar__item"
            onClick={() => {
              setMode("activity");
              setOpen(true);
            }}
            aria-label={`${hitlPending} awaiting input`}
          >
            <Flag size={10} />
            {hitlPending} awaiting input
          </button>
        )}

        {failedAgents > 0 && (
          <button
            className="status-bar__item"
            onClick={() => navigate("/runs?status=failed")}
            aria-label={`${failedAgents} failed agents`}
          >
            <OctagonAlert size={10} />
            {failedAgents} failed
          </button>
        )}

        {mcpConnected > 0 && (
          <button
            className="status-bar__item"
            onClick={() => navigate("/mcp")}
            aria-label={`${mcpConnected} MCP servers connected`}
          >
            <Zap size={10} />
            {mcpConnected} MCP connected
          </button>
        )}
      </div>

      {/* Right items */}
      <div className="status-bar__right">
        <span className="status-bar__version">v0.1.0</span>
        <button
          className="status-bar__item"
          onClick={toggleTheme}
          aria-label="Toggle theme"
        >
          {getTheme() === "dark" ? "Dark" : "Light"}
        </button>
      </div>
    </footer>
  );
}
