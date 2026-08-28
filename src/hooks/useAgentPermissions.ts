import { invoke } from "@tauri-apps/api/core";
import { useState, useCallback, useEffect } from "react";
import type { AgentPermissions } from "../types/permissions";
import { defaultPermissions } from "../types/permissions";

// ---------------------------------------------------------------------------
// Raw -> typed mapping (snake_case from Rust -> camelCase)
// ---------------------------------------------------------------------------

function mapPerms(raw: Record<string, unknown>): AgentPermissions {
  return {
    profileId:                String(raw.profile_id ?? raw.profileId ?? ""),
    allowedTools:             Array.isArray(raw.allowed_tools ?? raw.allowedTools)
                                ? (raw.allowed_tools ?? raw.allowedTools) as string[]
                                : [],
    allowFileReadPaths:       Array.isArray(raw.allow_file_read_paths ?? raw.allowFileReadPaths)
                                ? (raw.allow_file_read_paths ?? raw.allowFileReadPaths) as string[]
                                : [],
    allowFileWritePaths:      Array.isArray(raw.allow_file_write_paths ?? raw.allowFileWritePaths)
                                ? (raw.allow_file_write_paths ?? raw.allowFileWritePaths) as string[]
                                : [],
    allowShellCommands:       Array.isArray(raw.allow_shell_commands ?? raw.allowShellCommands)
                                ? (raw.allow_shell_commands ?? raw.allowShellCommands) as string[]
                                : [],
    requireApprovalOnWrite:   Boolean(raw.require_approval_on_write ?? raw.requireApprovalOnWrite ?? false),
  };
}

// ---------------------------------------------------------------------------
// useAgentPermissions
// ---------------------------------------------------------------------------

export interface UseAgentPermissionsReturn {
  permissions: AgentPermissions;
  loading: boolean;
  error: string | null;
  save: (perms: AgentPermissions) => Promise<void>;
}

export function useAgentPermissions(profileId: string | null): UseAgentPermissionsReturn {
  const [permissions, setPermissions] = useState<AgentPermissions>(
    defaultPermissions(profileId ?? "")
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (id: string) => {
    setLoading(true);
    setError(null);
    try {
      const raw = await invoke<Record<string, unknown>>("get_agent_permissions", {
        profileId: id,
      });
      setPermissions(mapPerms(raw));
    } catch (e) {
      setError(String(e));
      setPermissions(defaultPermissions(id));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (profileId) {
      load(profileId);
    } else {
      setPermissions(defaultPermissions(""));
    }
  }, [profileId, load]);

  const save = useCallback(async (perms: AgentPermissions) => {
    await invoke("upsert_agent_permissions", { perms });
    setPermissions(perms);
  }, []);

  return { permissions, loading, error, save };
}
