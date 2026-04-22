// Types mirroring the Rust structs in commands/src/custom_tools.rs

export interface CustomTool {
  id: string;
  name: string;
  description: string;
  command: string;
  working_dir: string;
  timeout_secs: number;
  workspace_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateCustomToolInput {
  name: string;
  description?: string;
  command: string;
  working_dir?: string;
  timeout_secs?: number;
  workspace_id?: string | null;
}

export interface UpdateCustomToolInput {
  name?: string;
  description?: string;
  command?: string;
  working_dir?: string;
  timeout_secs?: number;
}

/** A binding between an agent profile and a custom tool. */
export interface CustomToolBinding {
  agent_profile_id: string;
  custom_tool_id: string;
  /** Display name from JOIN with custom_tools. */
  tool_name: string | null;
}
