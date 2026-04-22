import { createContext, useContext, ReactNode } from "react";
import { useWorkspace, type Workspace } from "../hooks/useWorkspace";

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

interface WorkspaceContextValue {
  activeWorkspace: Workspace | null;
  recentWorkspaces: Workspace[];
  loading: boolean;
  pickAndOpenWorkspace: () => Promise<Workspace | null>;
  reopenWorkspace: (workspace: Workspace) => Promise<void>;
  removeWorkspace: (id: string) => Promise<void>;
  refresh: () => Promise<void>;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const ws = useWorkspace();
  return (
    <WorkspaceContext.Provider value={ws}>
      {children}
    </WorkspaceContext.Provider>
  );
}

// ---------------------------------------------------------------------------
// Consumer hook
// ---------------------------------------------------------------------------

export function useWorkspaceContext(): WorkspaceContextValue {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) {
    throw new Error("useWorkspaceContext must be used within <WorkspaceProvider>");
  }
  return ctx;
}
