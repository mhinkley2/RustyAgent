// Types mirroring the Rust structs in commands/src/mcp_servers.rs
// Field names match Rust snake_case (Tauri does not rename by default).

export interface McpServer {
  id: string;
  name: string;
  command: string;
  args: string[];
  env_vars: Record<string, string>;
  auto_restart: boolean;
  max_restart_attempts: number;
  created_at: string;
  updated_at: string;
}

export interface CreateMcpServerInput {
  name: string;
  command: string;
  args?: string[];
  env_vars?: Record<string, string>;
  auto_restart?: boolean;
  max_restart_attempts?: number;
}

export interface UpdateMcpServerInput {
  name?: string;
  command?: string;
  args?: string[];
  env_vars?: Record<string, string>;
  auto_restart?: boolean;
  max_restart_attempts?: number;
}

/** A binding between an agent profile and an MCP server. */
export interface ToolBinding {
  id: string;
  agent_profile_id: string;
  mcp_server_id: string;
  /** Display name from JOIN with mcp_servers. */
  mcp_server_name: string | null;
  /** null = allow all tools from this server */
  allowed_tools: string[] | null;
  created_at: string;
}

export interface CreateToolBindingInput {
  agent_profile_id: string;
  mcp_server_id: string;
  /** Omit or null = allow all tools. */
  allowed_tools?: string[] | null;
}
