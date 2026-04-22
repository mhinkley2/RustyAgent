import { invoke } from "@tauri-apps/api/core";
import { useState, useCallback, useEffect } from "react";
import { notifyError } from "../components/ui/Toast";
import type {
  McpServer,
  ToolBinding,
  CreateMcpServerInput,
  UpdateMcpServerInput,
  CreateToolBindingInput,
} from "../types/mcp";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// ---------------------------------------------------------------------------
// useMcpServers
// ---------------------------------------------------------------------------

interface UseMcpServersReturn {
  servers: McpServer[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  createServer: (input: CreateMcpServerInput) => Promise<McpServer>;
  updateServer: (id: string, input: UpdateMcpServerInput) => Promise<McpServer>;
  deleteServer: (id: string) => Promise<void>;
}

export function useMcpServers(): UseMcpServersReturn {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<McpServer[]>("get_mcp_servers");
      setServers(data);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      notifyError("Failed to load MCP servers", message, { duration: 7000 });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const createServer = useCallback(
    async (input: CreateMcpServerInput): Promise<McpServer> => {
      try {
        const server = await invoke<McpServer>("create_mcp_server", { input });
        setServers((prev) => [...prev, server]);
        return server;
      } catch (e) {
        notifyError("Failed to create MCP server", errorMessage(e), { duration: 7000 });
        throw e;
      }
    },
    []
  );

  const updateServer = useCallback(
    async (id: string, input: UpdateMcpServerInput): Promise<McpServer> => {
      try {
        const updated = await invoke<McpServer>("update_mcp_server", { id, input });
        setServers((prev) => prev.map((s) => (s.id === id ? updated : s)));
        return updated;
      } catch (e) {
        notifyError("Failed to update MCP server", errorMessage(e), { duration: 7000 });
        throw e;
      }
    },
    []
  );

  const deleteServer = useCallback(async (id: string): Promise<void> => {
    try {
      await invoke("delete_mcp_server", { id });
      setServers((prev) => prev.filter((s) => s.id !== id));
    } catch (e) {
      notifyError("Failed to delete MCP server", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  return { servers, loading, error, refresh, createServer, updateServer, deleteServer };
}

// ---------------------------------------------------------------------------
// useToolBindings — bindings for a single agent profile
// ---------------------------------------------------------------------------

interface UseToolBindingsReturn {
  bindings: ToolBinding[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  createBinding: (input: CreateToolBindingInput) => Promise<ToolBinding>;
  updateBindingTools: (id: string, allowedTools: string[] | null) => Promise<ToolBinding>;
  deleteBinding: (id: string) => Promise<void>;
}

export function useToolBindings(agentProfileId: string): UseToolBindingsReturn {
  const [bindings, setBindings] = useState<ToolBinding[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<ToolBinding[]>("get_tool_bindings", {
        agentProfileId,
      });
      setBindings(data);
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      notifyError("Failed to load tool bindings", message, { duration: 7000 });
    } finally {
      setLoading(false);
    }
  }, [agentProfileId]);

  useEffect(() => {
    if (agentProfileId) refresh();
  }, [agentProfileId, refresh]);

  const createBinding = useCallback(
    async (input: CreateToolBindingInput): Promise<ToolBinding> => {
      try {
        const binding = await invoke<ToolBinding>("create_tool_binding", { input });
        setBindings((prev) => [...prev, binding]);
        return binding;
      } catch (e) {
        notifyError("Failed to create tool binding", errorMessage(e), { duration: 7000 });
        throw e;
      }
    },
    []
  );

  const updateBindingTools = useCallback(
    async (id: string, allowedTools: string[] | null): Promise<ToolBinding> => {
      try {
        const updated = await invoke<ToolBinding>(
          "update_tool_binding_allowed_tools",
          { id, allowedTools }
        );
        setBindings((prev) => prev.map((b) => (b.id === id ? updated : b)));
        return updated;
      } catch (e) {
        notifyError("Failed to update tool binding", errorMessage(e), { duration: 7000 });
        throw e;
      }
    },
    []
  );

  const deleteBinding = useCallback(async (id: string): Promise<void> => {
    try {
      await invoke("delete_tool_binding", { id });
      setBindings((prev) => prev.filter((b) => b.id !== id));
    } catch (e) {
      notifyError("Failed to delete tool binding", errorMessage(e), { duration: 7000 });
      throw e;
    }
  }, []);

  return {
    bindings,
    loading,
    error,
    refresh,
    createBinding,
    updateBindingTools,
    deleteBinding,
  };
}
