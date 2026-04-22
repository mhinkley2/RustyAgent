import { HashRouter, Routes, Route, Navigate } from "react-router-dom";
import { ModalProvider } from "./context/ModalContext";
import { WorkspaceProvider } from "./context/WorkspaceContext";
import { ToastProvider } from "./components/ui/Toast";
import AppShell from "./components/layout/AppShell";
import AgentsPage from "./pages/AgentsPage";
import BoardPage from "./pages/BoardPage";
import RunsPage from "./pages/RunsPage";
import McpPage from "./pages/McpPage";
import ToolsPage from "./pages/ToolsPage";
import SettingsPage from "./pages/SettingsPage";
import ChatPage from "./pages/ChatPage";
import EditorPage from "./pages/EditorPage";
import LogsPage from "./pages/LogsPage";
import "./App.css";

export default function App() {
  return (
    <HashRouter>
      <ModalProvider>
        <WorkspaceProvider>
          <ToastProvider>
            <Routes>
              <Route element={<AppShell />}>
                <Route index element={<Navigate to="/agents" replace />} />
                <Route path="/agents"   element={<AgentsPage />} />
                <Route path="/board"    element={<BoardPage />} />
                <Route path="/runs"     element={<RunsPage />} />
                <Route path="/chat"     element={<ChatPage />} />
                <Route path="/editor"   element={<EditorPage />} />
                <Route path="/logs"     element={<LogsPage />} />
                <Route path="/mcp"      element={<McpPage />} />
                <Route path="/tools"    element={<ToolsPage />} />
                <Route path="/settings" element={<SettingsPage />} />
              </Route>
            </Routes>
          </ToastProvider>
        </WorkspaceProvider>
      </ModalProvider>
    </HashRouter>
  );
}
