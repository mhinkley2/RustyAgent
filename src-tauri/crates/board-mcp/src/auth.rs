//! Bearer-token and origin checks for the HTTP transport.
//!
//! Not used by the stdio transport, and deliberately gated behind the `http`
//! feature so it cannot leak into it: a stdio server is a child process talking
//! over inherited pipes, with no listening socket and no third party to
//! authenticate.

use std::path::{Path, PathBuf};

use tracing::warn;

/// Environment override, primarily for tests and CI.
pub const TOKEN_ENV: &str = "RUSTYAGENT_MCP_TOKEN";
/// Escape hatch: disables the token check entirely.
pub const ALLOW_ANONYMOUS_ENV: &str = "RUSTYAGENT_MCP_ALLOW_ANONYMOUS";

const TOKEN_FILE: &str = "mcp-token";

#[derive(Clone)]
pub struct AuthConfig {
    /// `None` when anonymous access is explicitly allowed.
    pub token: Option<String>,
    pub port: u16,
}

impl AuthConfig {
    /// Resolve the token: environment first, then the persisted file, then
    /// generate and persist one.
    ///
    /// The token persists across restarts on purpose. A per-launch token would
    /// force the user to re-paste a secret into their MCP client config every
    /// time the app restarts, and would guard nothing extra — the file sits
    /// beside `settings.json`, which already stores provider API keys in
    /// plaintext.
    pub fn resolve(app_data_dir: Option<&Path>, port: u16) -> Self {
        if std::env::var(ALLOW_ANONYMOUS_ENV).is_ok_and(|value| value == "1") {
            warn!(
                "{ALLOW_ANONYMOUS_ENV}=1 — the MCP HTTP server will accept unauthenticated \
                 requests from any process on this machine."
            );
            return Self { token: None, port };
        }

        if let Ok(value) = std::env::var(TOKEN_ENV) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Self {
                    token: Some(trimmed.to_string()),
                    port,
                };
            }
        }

        let token = app_data_dir
            .map(load_or_create_token)
            .unwrap_or_else(generate_token);

        Self {
            token: Some(token),
            port,
        }
    }

    pub fn token_path(app_data_dir: &Path) -> PathBuf {
        app_data_dir.join(TOKEN_FILE)
    }

    /// Check an `Authorization` header value.
    pub fn check_bearer(&self, header: Option<&str>) -> bool {
        let Some(expected) = self.token.as_deref() else {
            return true; // anonymous access explicitly enabled
        };
        let Some(header) = header else { return false };
        let Some(presented) = header.strip_prefix("Bearer ") else {
            return false;
        };
        constant_time_eq(presented.trim().as_bytes(), expected.as_bytes())
    }

    /// Validate `Origin` and `Host`.
    ///
    /// An absent `Origin` is allowed — native MCP clients send none, and
    /// rejecting them would break every intended caller. A present `Origin`
    /// must be a localhost form or one of the Tauri webview origins.
    ///
    /// `Host` is the real DNS-rebinding guard: a rebound name resolves to
    /// 127.0.0.1 but arrives carrying the attacker's hostname.
    pub fn check_origin(&self, origin: Option<&str>, host: Option<&str>) -> bool {
        if let Some(host) = host {
            if !is_local_host(host, self.port) {
                return false;
            }
        }
        match origin {
            None => true,
            Some(origin) => is_allowed_origin(origin),
        }
    }
}

fn is_allowed_origin(origin: &str) -> bool {
    const EXACT: &[&str] = &["tauri://localhost", "https://tauri.localhost", "null"];
    if EXACT.contains(&origin) {
        return true;
    }
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    matches!(hostname_of(rest), "127.0.0.1" | "localhost" | "::1")
}

/// Split the host part off an `authority` (`host`, `host:port`, `[v6]:port`).
///
/// The bracketed IPv6 form has to be handled before splitting on ':', or
/// `[::1]:3000` yields `[`.
fn hostname_of(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    }
}

fn is_local_host(host: &str, port: u16) -> bool {
    if !matches!(hostname_of(host), "127.0.0.1" | "localhost" | "::1") {
        return false;
    }

    // If a port is present it must be ours.
    match host.rsplit_once(':') {
        Some((_, maybe_port)) if !maybe_port.is_empty() && maybe_port.chars().all(|c| c.is_ascii_digit()) => {
            maybe_port.parse::<u16>().is_ok_and(|value| value == port)
        }
        _ => true,
    }
}

/// Compare without an early exit on the first differing byte, so a near-miss
/// token cannot be refined by timing. The token's *length* is fixed and not
/// secret, so returning early on a length mismatch is fine.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 244 bits from two v4 UUIDs. `uuid` is already a workspace dependency, so
/// this avoids pulling in `rand` for a single call site.
fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn load_or_create_token(app_data_dir: &Path) -> String {
    let path = AuthConfig::token_path(app_data_dir);

    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
        // An empty or whitespace-only file must never authorize the empty
        // string — regenerate instead.
    }

    let token = generate_token();
    if let Err(error) = std::fs::create_dir_all(app_data_dir)
        .and_then(|()| std::fs::write(&path, &token))
        .and_then(|()| restrict_permissions(&path))
    {
        warn!(
            "Failed to persist the MCP token at {}: {error}. A new token will be \
             generated on the next launch.",
            path.display()
        );
    }
    token
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> std::io::Result<()> {
    // The file inherits the app-data directory's ACL, as settings.json does.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(token: &str) -> AuthConfig {
        AuthConfig {
            token: Some(token.to_string()),
            port: 8765,
        }
    }

    #[test]
    fn a_correct_bearer_token_is_accepted() {
        assert!(cfg("secret").check_bearer(Some("Bearer secret")));
    }

    #[test]
    fn a_wrong_or_missing_token_is_rejected() {
        let config = cfg("secret");
        assert!(!config.check_bearer(Some("Bearer wrong")));
        assert!(!config.check_bearer(Some("Basic secret")));
        assert!(!config.check_bearer(Some("secret")));
        assert!(!config.check_bearer(None));
    }

    #[test]
    fn a_token_of_a_different_length_is_rejected() {
        let config = cfg("secret");
        assert!(!config.check_bearer(Some("Bearer secretsecret")));
        assert!(!config.check_bearer(Some("Bearer sec")));
        assert!(!config.check_bearer(Some("Bearer ")));
    }

    #[test]
    fn anonymous_mode_accepts_anything() {
        let config = AuthConfig { token: None, port: 8765 };
        assert!(config.check_bearer(None));
        assert!(config.check_bearer(Some("Bearer nonsense")));
    }

    #[test]
    fn an_absent_origin_is_allowed() {
        assert!(cfg("s").check_origin(None, Some("127.0.0.1:8765")));
    }

    #[test]
    fn localhost_and_tauri_origins_are_allowed() {
        let config = cfg("s");
        for origin in [
            "http://localhost:1420",
            "http://127.0.0.1:8765",
            "http://[::1]:3000",
            "tauri://localhost",
            "https://tauri.localhost",
        ] {
            assert!(
                config.check_origin(Some(origin), Some("127.0.0.1:8765")),
                "{origin} should be allowed"
            );
        }
    }

    #[test]
    fn a_foreign_origin_is_rejected() {
        let config = cfg("s");
        for origin in [
            "http://evil.com",
            "https://evil.com",
            "http://localhost.evil.com",
            "http://127.0.0.1.evil.com",
        ] {
            assert!(
                !config.check_origin(Some(origin), Some("127.0.0.1:8765")),
                "{origin} should be rejected"
            );
        }
    }

    #[test]
    fn a_rebound_host_header_is_rejected() {
        let config = cfg("s");
        assert!(!config.check_origin(None, Some("attacker.example")));
        assert!(!config.check_origin(None, Some("attacker.example:8765")));
    }

    #[test]
    fn a_host_naming_a_different_port_is_rejected() {
        assert!(!cfg("s").check_origin(None, Some("127.0.0.1:9999")));
    }

    #[test]
    fn a_generated_token_is_long_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn a_token_file_round_trips_and_an_empty_one_regenerates() {
        let dir = tempfile::TempDir::new().expect("temp dir");

        let first = load_or_create_token(dir.path());
        assert!(!first.is_empty());
        assert_eq!(load_or_create_token(dir.path()), first, "should persist");

        std::fs::write(AuthConfig::token_path(dir.path()), "   \n").expect("write");
        let regenerated = load_or_create_token(dir.path());
        assert!(!regenerated.trim().is_empty());
        assert_ne!(regenerated, "");
    }
}
