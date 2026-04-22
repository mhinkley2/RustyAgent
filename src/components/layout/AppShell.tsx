import { Outlet } from "react-router-dom";
import { useLocation } from "react-router-dom";
import Titlebar from "./Titlebar";
import ActivityBar from "./Sidebar";
import SidePanel from "./SidePanel";
import StatusBar from "./StatusBar";
import WorkspaceSidePanel from "./WorkspaceSidePanel";
import { SidePanelProvider } from "./SidePanelContext";
import { useHumanRequests } from "../../hooks/useHumanRequests";

/**
 * Persistent outer shell: titlebar + activity bar + side panel + content + status bar.
 *
 * Layout:
 *   ┌──────────────────────────────────────────────┐
 *   │  Titlebar (drag region, window controls)     │  36px
 *   ├────┬──────────────┬───────────────────────────┤
 *   │    │              │                           │
 *   │ AB │  SidePanel   │  <Outlet />               │  flex-1
 *   │ 48 │  240px       │  (page content)           │
 *   │    │  (resizable) │                           │
 *   ├────┴──────────────┴───────────────────────────┤
 *   │  StatusBar                                   │  28px
 *   └──────────────────────────────────────────────┘
 */
export default function AppShell() {
  const { pathname } = useLocation();
  const { humanRequests, approvalRequests } = useHumanRequests(0);
  const isChatRoute = pathname === "/chat" || pathname.startsWith("/chat/");

  const pendingHumanCount = humanRequests.filter(
    (r) => r.status !== "done" && r.status !== "failed"
  ).length;
  const pendingApprovalCount = approvalRequests.length;

  return (
    <SidePanelProvider>
      <div className="app-shell">
        <Titlebar />

        <div className="app-shell__body">
          <ActivityBar
            pendingHumanStories={pendingHumanCount}
            pendingApprovals={pendingApprovalCount}
          />

          <SidePanel>
            <WorkspaceSidePanel />
          </SidePanel>

          <main
            className={[
              "app-shell__content",
              isChatRoute ? "app-shell__content--chat" : "",
            ].join(" ")}
            id="main-content"
          >
            <div className={[
              "page-container",
              isChatRoute ? "page-container--chat" : "",
            ].join(" ")}>
              <Outlet />
            </div>
          </main>
        </div>

        <StatusBar
          pendingHumanCount={pendingHumanCount}
          pendingApprovalCount={pendingApprovalCount}
        />
      </div>
    </SidePanelProvider>
  );
}
