// Permission types — mirrors the Rust AgentPermissions struct.

export interface AgentPermissions {
  profileId: string;
  /** Exact tool names allowed (empty = all tools allowed). */
  allowedTools: string[];
  /** Path prefixes for allowed file reads (empty = no restriction). */
  allowFileReadPaths: string[];
  /** Path prefixes for allowed file writes (empty = no restriction). */
  allowFileWritePaths: string[];
  /** Shell command name prefixes permitted (empty = no restriction). */
  allowShellCommands: string[];
  /** Network hostname allow-list (empty = no restriction). */
  allowNetworkHosts: string[];
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
    allowNetworkHosts: [],
    requireApprovalOnWrite: false,
  };
}
