// TOML representation of an agent profile.
// This is the on-disk format committed to version control.
//
// Example .rusty/agents/my-agent.toml:
//
//   [profile]
//   name = "My Agent"
//   description = "Does something useful"
//   provider = "anthropic"
//   model = "claude-opus-4-5"
//   system_prompt = """
//   You are a helpful assistant.
//   """
//
//   [behavior]
//   context_strategy = "recent"
//   persistent_memory = false
//   max_iterations = 20
//   run_mode = "manual"
//
//   [limits]
//   max_input_tokens = 100000
//   max_output_tokens = 4096
//
//   [permissions]
//   allow_read  = ["./"]
//   allow_write = []
//   allow_shell = []
//   require_approval_on_write = true

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToml {
    pub profile:     ProfileSection,
    #[serde(default)]
    pub behavior:    BehaviorSection,
    #[serde(default)]
    pub limits:      LimitsSection,
    #[serde(default)]
    pub permissions: PermissionsSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSection {
    pub name:          String,
    pub description:   Option<String>,
    pub provider:      String,
    pub model:         String,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BehaviorSection {
    #[serde(default = "default_context_strategy")]
    pub context_strategy:             String,
    #[serde(default)]
    pub persistent_memory:            bool,
    #[serde(default = "default_max_iterations")]
    pub max_iterations:               i64,
    #[serde(default = "default_run_mode")]
    pub run_mode:                     String,
    pub cron_expression:              Option<String>,
    #[serde(default = "default_poll_interval")]
    pub continuous_poll_interval_secs: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LimitsSection {
    pub max_input_tokens:  Option<i64>,
    pub max_output_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsSection {
    #[serde(default)]
    pub allow_read:                Vec<String>,
    #[serde(default)]
    pub allow_write:               Vec<String>,
    #[serde(default)]
    pub allow_shell:               Vec<String>,
    #[serde(default)]
    pub require_approval_on_write: bool,
}

fn default_context_strategy() -> String { "recent".into() }
fn default_max_iterations()    -> i64   { 20 }
fn default_run_mode()          -> String { "manual".into() }
fn default_poll_interval()     -> i64   { 30 }

impl AgentToml {
    /// Parse an [`AgentToml`] from a TOML string.
    pub fn from_str(src: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(src)
    }

    /// Serialize back to a pretty TOML string.
    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        toml::to_string_pretty(self).map_err(Into::into)
    }

    /// Derive a filesystem slug from the profile name (lowercase, kebab-case).
    pub fn slug(name: &str) -> String {
        name.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }
}
