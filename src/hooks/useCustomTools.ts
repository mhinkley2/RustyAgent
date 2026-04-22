import { invoke } from "@tauri-apps/api/core";
import { useState, useCallback, useEffect } from "react";
import type {
  CustomTool,
  CustomToolBinding,
  CreateCustomToolInput,
  UpdateCustomToolInput,
} from "../types/custom_tools";
import { useWorkspaceContext } from "../context/WorkspaceContext";

// ---------------------------------------------------------------------------
// useCustomTools
// ---------------------------------------------------------------------------

interface UseCustomToolsReturn {
  tools: CustomTool[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  createTool: (input: CreateCustomToolInput) => Promise<CustomTool>;
  updateTool: (id: string, input: UpdateCustomToolInput) => Promise<CustomTool>;
  deleteTool: (id: string) => Promise<void>;
}

export function useCustomTools(): UseCustomToolsReturn {
  const [tools, setTools] = useState<CustomTool[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const { activeWorkspace } = useWorkspaceContext();

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<CustomTool[]>("get_custom_tools", {
        workspaceId: activeWorkspace?.id ?? null,
      });
      setTools(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [activeWorkspace?.id]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const createTool = useCallback(
    async (input: CreateCustomToolInput): Promise<CustomTool> => {
      const tool = await invoke<CustomTool>("create_custom_tool", { input });
      setTools((prev) => [...prev, tool]);
      return tool;
    },
    []
  );

  const updateTool = useCallback(
    async (id: string, input: UpdateCustomToolInput): Promise<CustomTool> => {
      const updated = await invoke<CustomTool>("update_custom_tool", { id, input });
      setTools((prev) => prev.map((t) => (t.id === id ? updated : t)));
      return updated;
    },
    []
  );

  const deleteTool = useCallback(async (id: string): Promise<void> => {
    await invoke("delete_custom_tool", { id });
    setTools((prev) => prev.filter((t) => t.id !== id));
  }, []);

  return { tools, loading, error, refresh, createTool, updateTool, deleteTool };
}

// ---------------------------------------------------------------------------
// useCustomToolBindings — bindings for a single agent profile
// ---------------------------------------------------------------------------

interface UseCustomToolBindingsReturn {
  bindings: CustomToolBinding[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  createBinding: (customToolId: string) => Promise<CustomToolBinding>;
  deleteBinding: (customToolId: string) => Promise<void>;
}

export function useCustomToolBindings(agentProfileId: string): UseCustomToolBindingsReturn {
  const [bindings, setBindings] = useState<CustomToolBinding[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<CustomToolBinding[]>("get_custom_tool_bindings", {
        agentProfileId,
      });
      setBindings(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [agentProfileId]);

  useEffect(() => {
    if (agentProfileId) refresh();
  }, [agentProfileId, refresh]);

  const createBinding = useCallback(
    async (customToolId: string): Promise<CustomToolBinding> => {
      const binding = await invoke<CustomToolBinding>("create_custom_tool_binding", {
        agentProfileId,
        customToolId,
      });
      setBindings((prev) => [...prev, binding]);
      return binding;
    },
    [agentProfileId]
  );

  const deleteBinding = useCallback(
    async (customToolId: string): Promise<void> => {
      await invoke("delete_custom_tool_binding", { agentProfileId, customToolId });
      setBindings((prev) => prev.filter((b) => b.custom_tool_id !== customToolId));
    },
    [agentProfileId]
  );

  return { bindings, loading, error, refresh, createBinding, deleteBinding };
}
