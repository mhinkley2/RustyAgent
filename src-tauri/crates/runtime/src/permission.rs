#[derive(Debug, Clone, PartialEq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
    RequiresApproval,
}

const WRITE_TOOLS: &[&str] = &[
    "file_write",
    "write_file_text",
    "create_empty_file",
    "create_dir_fs",
    "delete_path",
    "rename_path",
    "duplicate_file",
];

fn is_write_tool(name: &str) -> bool {
    WRITE_TOOLS.contains(&name)
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    pub allowed_tools: Vec<String>,
    pub allow_file_read_paths: Vec<String>,
    pub allow_file_write_paths: Vec<String>,
    pub allow_shell_commands: Vec<String>,
    pub allow_network_hosts: Vec<String>,
    pub require_approval_on_write: bool,
}

impl PermissionPolicy {
    pub fn allow_all() -> Self {
        Self {
            allowed_tools: vec![],
            allow_file_read_paths: vec![],
            allow_file_write_paths: vec![],
            allow_shell_commands: vec![],
            allow_network_hosts: vec![],
            require_approval_on_write: false,
        }
    }

    pub fn restricted(tools: Vec<String>) -> Self {
        Self {
            allowed_tools: tools,
            ..Self::allow_all()
        }
    }

    pub fn from_db_json(value: Option<serde_json::Value>) -> Self {
        match value {
            None | Some(serde_json::Value::Null) => Self::allow_all(),
            Some(serde_json::Value::Array(arr)) => {
                let names = arr
                    .into_iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                Self::restricted(names)
            }
            _ => Self::allow_all(),
        }
    }

    pub fn from_db_permissions(
        allowed_tools_json: &str,
        allow_file_read_paths_json: &str,
        allow_file_write_paths_json: &str,
        allow_shell_commands_json: &str,
        allow_network_hosts_json: &str,
        require_approval_on_write: bool,
    ) -> Self {
        let parse_list = |s: &str| -> Vec<String> {
            serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
        };
        Self {
            allowed_tools: parse_list(allowed_tools_json),
            allow_file_read_paths: parse_list(allow_file_read_paths_json),
            allow_file_write_paths: parse_list(allow_file_write_paths_json),
            allow_shell_commands: parse_list(allow_shell_commands_json),
            allow_network_hosts: parse_list(allow_network_hosts_json),
            require_approval_on_write,
        }
    }

    pub fn check(&self, tool_name: &str) -> bool {
        if self.allowed_tools.is_empty() {
            return true;
        }
        self.allowed_tools.iter().any(|t| t == tool_name)
    }

    pub fn check_tool(&self, tool_name: &str, inputs: &serde_json::Value) -> PolicyDecision {
        if !self.check(tool_name) {
            return PolicyDecision::Deny(format!(
                "Tool '{}' is not permitted for this agent profile",
                tool_name
            ));
        }

        if is_write_tool(tool_name) {
            if !self.allow_file_write_paths.is_empty() {
                let path = inputs
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let allowed = self
                    .allow_file_write_paths
                    .iter()
                    .any(|prefix| path.starts_with(prefix.as_str()));
                if !allowed {
                    return PolicyDecision::Deny(format!(
                        "Write to '{}' is outside the allowed file write paths",
                        path
                    ));
                }
            }

            if self.require_approval_on_write {
                return PolicyDecision::RequiresApproval;
            }
        }

        PolicyDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_all_permits_any_tool() {
        let policy = PermissionPolicy::allow_all();
        assert_eq!(policy.check_tool("get_story", &serde_json::json!({})), PolicyDecision::Allow);
    }

    #[test]
    fn restricted_blocks_unlisted_tools() {
        let policy = PermissionPolicy::restricted(vec!["get_story".into()]);
        assert!(matches!(
            policy.check_tool("memory_write", &serde_json::json!({})),
            PolicyDecision::Deny(_)
        ));
    }

    #[test]
    fn write_path_restriction_blocks_outside_path() {
        let mut policy = PermissionPolicy::allow_all();
        policy.allow_file_write_paths = vec!["/workspace/".into()];
        assert!(matches!(
            policy.check_tool("write_file_text", &serde_json::json!({"path": "/etc/passwd"})),
            PolicyDecision::Deny(_)
        ));
    }

    #[test]
    fn write_path_restriction_allows_inside_path() {
        let mut policy = PermissionPolicy::allow_all();
        policy.allow_file_write_paths = vec!["/workspace/".into()];
        assert_eq!(
            policy.check_tool(
                "write_file_text",
                &serde_json::json!({"path": "/workspace/src/main.rs"})
            ),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn require_approval_on_write_returns_requires_approval() {
        let mut policy = PermissionPolicy::allow_all();
        policy.require_approval_on_write = true;
        assert_eq!(
            policy.check_tool(
                "write_file_text",
                &serde_json::json!({"path": "/workspace/src/main.rs"})
            ),
            PolicyDecision::RequiresApproval
        );
    }

    #[test]
    fn require_approval_does_not_affect_non_write_tools() {
        let mut policy = PermissionPolicy::allow_all();
        policy.require_approval_on_write = true;
        assert_eq!(
            policy.check_tool("get_story", &serde_json::json!({})),
            PolicyDecision::Allow
        );
    }
}