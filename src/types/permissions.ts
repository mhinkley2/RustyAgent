// Permission types — mirrors the Rust AgentPermissions struct.

export interface AgentPermissions {
  profileId: string;
  /** Exact tool names allowed (empty = all tools allowed). */
  allowedTools: string[];
  /** Path prefixes for allowed file reads (empty = no restriction). */
  allowFileReadPaths: string[];
  /** Path prefixes for allowed file writes (empty = no restriction). */
  allowFileWritePaths: string[];
  /** Program names permitted for custom shell tools (empty = no restriction). */
  allowShellCommands: string[];
  /** When true, every file write tool call pauses for human approval. */
  requireApprovalOnWrite: boolean;
}

export function defaultPermissions(profileId: string): AgentPermissions {
  return {
    profileId,
    allowedTools: [],
    allowFileReadPaths: [],
    allowFileWritePaths: [],
    allowShellCommands: [],
    requireApprovalOnWrite: false,
  };
}
