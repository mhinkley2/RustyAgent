//! Tests for [`crate::permissions`] — the commands layer over the
//! `agent_permissions` table.

use db::testing::{make_test_pool, seed_profile};

use crate::permissions::{get_agent_permissions, upsert_agent_permissions, AgentPermissions};

fn perms(profile_id: &str) -> AgentPermissions {
    AgentPermissions {
        profile_id: profile_id.to_string(),
        allowed_tools: vec!["file_read".into(), "file_write".into()],
        allow_file_read_paths: vec!["docs".into()],
        allow_file_write_paths: vec!["src".into()],
        allow_shell_commands: vec!["git".into()],
        require_approval_on_write: true,
    }
}

#[tokio::test]
async fn permissions_round_trip_through_the_database() {
    let db = make_test_pool().await;
    seed_profile(&db, "agent-1", "Agent").await;

    upsert_agent_permissions(perms("agent-1"), &db).await.expect("upsert");
    let loaded = get_agent_permissions("agent-1".into(), &db).await.expect("get");

    assert_eq!(loaded.allowed_tools, vec!["file_read", "file_write"]);
    assert_eq!(loaded.allow_file_read_paths, vec!["docs"]);
    assert_eq!(loaded.allow_file_write_paths, vec!["src"]);
    assert_eq!(loaded.allow_shell_commands, vec!["git"]);
    assert!(loaded.require_approval_on_write);
}

#[tokio::test]
async fn a_second_upsert_replaces_the_first() {
    let db = make_test_pool().await;
    seed_profile(&db, "agent-1", "Agent").await;

    upsert_agent_permissions(perms("agent-1"), &db).await.expect("first upsert");

    let mut relaxed = perms("agent-1");
    relaxed.allow_shell_commands = vec![];
    relaxed.require_approval_on_write = false;
    upsert_agent_permissions(relaxed, &db).await.expect("second upsert");

    let loaded = get_agent_permissions("agent-1".into(), &db).await.expect("get");
    assert!(loaded.allow_shell_commands.is_empty());
    assert!(!loaded.require_approval_on_write);
}

/// A profile with no row is unrestricted — the "permissive when unset" default
/// the whole permission model leans on.
#[tokio::test]
async fn a_profile_with_no_row_reads_back_as_unrestricted() {
    let db = make_test_pool().await;
    seed_profile(&db, "agent-1", "Agent").await;

    let loaded = get_agent_permissions("agent-1".into(), &db).await.expect("get");

    assert_eq!(loaded.profile_id, "agent-1");
    assert!(loaded.allowed_tools.is_empty());
    assert!(loaded.allow_file_read_paths.is_empty());
    assert!(loaded.allow_file_write_paths.is_empty());
    assert!(loaded.allow_shell_commands.is_empty());
    assert!(!loaded.require_approval_on_write);
}

/// The permission editor posts camelCase. A payload that still carries the
/// removed `allowNetworkHosts` — a stale frontend, or a saved request — must
/// deserialize rather than fail, and must not resurrect the field.
#[test]
fn a_payload_still_carrying_allow_network_hosts_is_accepted_and_ignored() {
    let json = serde_json::json!({
        "profileId": "agent-1",
        "allowedTools": [],
        "allowFileReadPaths": ["docs"],
        "allowFileWritePaths": [],
        "allowShellCommands": [],
        "allowNetworkHosts": ["api.github.com"],
        "requireApprovalOnWrite": false,
    });

    let parsed: AgentPermissions = serde_json::from_value(json).expect("deserialize");

    assert_eq!(parsed.profile_id, "agent-1");
    assert_eq!(parsed.allow_file_read_paths, vec!["docs"]);
}

#[test]
fn the_serialized_shape_no_longer_advertises_a_network_allow_list() {
    let json = serde_json::to_value(perms("agent-1")).expect("serialize");

    assert!(
        json.get("allowNetworkHosts").is_none(),
        "the removed control must not reappear on the wire: {json}"
    );
    assert!(json.get("allowShellCommands").is_some());
}
