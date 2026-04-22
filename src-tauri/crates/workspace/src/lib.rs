// workspace crate — .rusty/ directory discovery, TOML profile parsing,
// SQLite synchronisation, and live-reload watching.

pub mod toml_profile;
pub mod loader;
pub mod watcher;

pub use toml_profile::AgentToml;
pub use loader::{sync_profiles, sync_profiles_for_workspace, ensure_rusty_dir};
