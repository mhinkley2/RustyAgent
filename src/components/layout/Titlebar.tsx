import { getCurrentWindow, Window as TauriWindow } from "@tauri-apps/api/window";
import { Minus, Square, X, Bot, FolderOpen, Check, ChevronDown, Trash2 } from "lucide-react";
import { useLocation, useNavigate } from "react-router-dom";
import { useRef, useState, useEffect } from "react";
import { useWorkspaceContext } from "../../context/WorkspaceContext";

function safeGetWindow(): TauriWindow | null {
  try {
    return getCurrentWindow();
  } catch (e) {
    console.warn("Tauri window API unavailable:", e);
    return null;
  }
}

/** Map of route paths → human-readable page titles. */
const PAGE_TITLES: Record<string, string> = {
  "/agents": "Agents",
  "/board": "Board",
  "/runs": "Runs",
  "/mcp": "MCP Servers",
  "/logs": "Logs",
  "/settings": "Settings",
};

export default function Titlebar() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const title = PAGE_TITLES[pathname] ?? null;
  const { activeWorkspace, recentWorkspaces, pickAndOpenWorkspace, reopenWorkspace, removeWorkspace } = useWorkspaceContext();

  const appWin = safeGetWindow();

  const [dropdownOpen, setDropdownOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Close dropdown when clicking outside.
  useEffect(() => {
    if (!dropdownOpen) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setDropdownOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [dropdownOpen]);

  const handleDragStart = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    appWin?.startDragging();
  };

  return (
    <header className="titlebar" onMouseDown={handleDragStart}>
      {/* App wordmark — left side, non-draggable, links to /agents */}
      <div
        className="titlebar__wordmark"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <button
          className="titlebar__wordmark-btn"
          onClick={() => navigate("/agents")}
          aria-label="Go to Agents (home)"
        >
          <Bot size={14} className="titlebar__wordmark-icon" aria-hidden="true" />
          <span className="titlebar__wordmark-name">RustyAgent</span>
        </button>
      </div>

      {/* Workspace pill — left of center, non-draggable */}
      <div
        className="titlebar__workspace"
        ref={dropdownRef}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <button
          className="titlebar__workspace-btn"
          onClick={() => setDropdownOpen(prev => !prev)}
          title={activeWorkspace ? activeWorkspace.path : "Open a workspace folder"}
          aria-label={activeWorkspace ? `Workspace: ${activeWorkspace.name}` : "Open workspace"}
          aria-expanded={dropdownOpen}
          aria-haspopup="listbox"
        >
          <FolderOpen size={12} aria-hidden="true" />
          <span className="titlebar__workspace-name">
            {activeWorkspace ? activeWorkspace.name : "Open Workspace"}
          </span>
          <ChevronDown size={10} className={`titlebar__workspace-chevron${dropdownOpen ? " titlebar__workspace-chevron--open" : ""}`} aria-hidden="true" />
        </button>

        {dropdownOpen && (
          <div className="titlebar__workspace-dropdown" role="listbox">
            {recentWorkspaces.length === 0 ? (
              <div className="titlebar__ws-empty">No recent workspaces</div>
            ) : (
              recentWorkspaces.map(ws => (
                <div
                  key={ws.id}
                  className={`titlebar__ws-item${ws.id === activeWorkspace?.id ? " titlebar__ws-item--active" : ""}`}
                  role="option"
                  aria-selected={ws.id === activeWorkspace?.id}
                  onClick={() => {
                    setDropdownOpen(false);
                    if (ws.id !== activeWorkspace?.id) reopenWorkspace(ws);
                  }}
                >
                  <Check size={11} className="titlebar__ws-check" aria-hidden="true" />
                  <span className="titlebar__ws-name" title={ws.path}>{ws.name}</span>
                  <button
                    className="titlebar__ws-remove"
                    aria-label={`Remove ${ws.name} from recent`}
                    onClick={(e) => {
                      e.stopPropagation();
                      removeWorkspace(ws.id);
                    }}
                  >
                    <Trash2 size={11} />
                  </button>
                </div>
              ))
            )}
            <div className="titlebar__ws-divider" />
            <div
              className="titlebar__ws-item titlebar__ws-item--action"
              onClick={() => { setDropdownOpen(false); pickAndOpenWorkspace(); }}
            >
              <FolderOpen size={11} aria-hidden="true" />
              <span>Open folder…</span>
            </div>
          </div>
        )}
      </div>

      {/* Page title — centered */}
      <div className="titlebar__title">
        {title ? <span>{title}</span> : <span>RustyAgent</span>}
      </div>

      {/* Window controls */}
      <div className="titlebar__controls" onMouseDown={(e) => e.stopPropagation()}>
        <button
          className="titlebar__btn"
          aria-label="Minimize"
          onClick={() => appWin?.minimize()}
        >
          <Minus size={14} />
        </button>
        <button
          className="titlebar__btn"
          aria-label="Maximize"
          onClick={() => appWin?.toggleMaximize()}
        >
          <Square size={12} />
        </button>
        <button
          className="titlebar__btn titlebar__btn--close"
          aria-label="Close"
          onClick={() => appWin?.close()}
        >
          <X size={14} />
        </button>
      </div>
    </header>
  );
}
