import { NavLink, useNavigate, useLocation } from "react-router-dom";
import { useEffect } from "react";
import {
  Bot,
  Kanban,
  History,
  MessageSquare,
  Code,
  ScrollText,
  Wrench,
  Settings,
} from "lucide-react";
import { useSidePanel } from "./SidePanelContext";

interface NavItem {
  path: string;
  label: string;
  icon: React.ReactNode;
  shortcut: number;
}

const NAV_ITEMS: NavItem[] = [
  { path: "/agents",   label: "Agents",      icon: <Bot size={20} />,          shortcut: 1 },
  { path: "/board",    label: "Board",       icon: <Kanban size={20} />,        shortcut: 2 },
  { path: "/runs",     label: "Runs",        icon: <History size={20} />,       shortcut: 3 },
  { path: "/chat",     label: "Chat",        icon: <MessageSquare size={20} />, shortcut: 4 },
  { path: "/editor",   label: "Editor",      icon: <Code size={20} />,          shortcut: 5 },
  { path: "/tools",    label: "Tools",       icon: <Wrench size={20} />,        shortcut: 6 },
  { path: "/logs",     label: "Logs",        icon: <ScrollText size={20} />,    shortcut: 7 },
];

interface ActivityBarProps {
  pendingHumanStories?: number;
  pendingApprovals?: number;
}

export default function ActivityBar({ pendingHumanStories = 0, pendingApprovals = 0 }: ActivityBarProps) {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const { open, setOpen } = useSidePanel();

  // Ctrl+1–7 keyboard shortcuts.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!e.ctrlKey) return;
      const n = parseInt(e.key, 10);
      if (n >= 1 && n <= NAV_ITEMS.length) {
        e.preventDefault();
        navigate(NAV_ITEMS[n - 1].path);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate]);

  function handleNavClick(isActive: boolean) {
    if (isActive) {
      // Toggle side panel when clicking the already-active item.
      setOpen(!open);
    }
  }

  return (
    <nav className="activity-bar" aria-label="Main navigation">
      {/* App icon — links to /agents */}
      <NavLink
        to="/agents"
        className="activity-bar__app-icon"
        aria-label="RustyAgent home"
        tabIndex={-1}
      >
        <Bot size={22} />
      </NavLink>

      {/* Primary nav items */}
      <ul className="activity-bar__nav" role="list">
        {NAV_ITEMS.map((item) => {
          const hasDot =
            (item.path === "/board" && pendingHumanStories > 0) ||
            (item.path === "/runs" && pendingApprovals > 0);

          const isActive = pathname === item.path || pathname.startsWith(item.path + "/");

          return (
            <li key={item.path}>
              <NavLink
                to={item.path}
                className={({ isActive: navActive }) =>
                  ["activity-bar__item", navActive ? "activity-bar__item--active" : ""].join(" ")
                }
                aria-label={`${item.label}${hasDot ? " (pending)" : ""}. Ctrl+${item.shortcut}`}
                onClick={() => handleNavClick(isActive)}
              >
                <span className="activity-bar__icon" aria-hidden="true">
                  {item.icon}
                </span>
                {hasDot && <span className="activity-bar__badge" aria-hidden="true" />}
                <span className="activity-bar__tooltip" role="tooltip">
                  {item.label}
                  <span className="activity-bar__tooltip-shortcut">Ctrl+{item.shortcut}</span>
                </span>
              </NavLink>
            </li>
          );
        })}
      </ul>

      {/* Settings — pinned to bottom */}
      <div className="activity-bar__bottom">
        <NavLink
          to="/settings"
          className={({ isActive }) =>
            ["activity-bar__item", isActive ? "activity-bar__item--active" : ""].join(" ")
          }
          aria-label="Settings"
        >
          <span className="activity-bar__icon" aria-hidden="true">
            <Settings size={20} />
          </span>
          <span className="activity-bar__tooltip" role="tooltip">Settings</span>
        </NavLink>
      </div>
    </nav>
  );
}
