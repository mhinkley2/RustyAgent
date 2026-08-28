//! Tests for [`crate::permission`].
//!
//! Each enforced control is asserted from both sides — it permits the action it
//! is meant to permit and denies the one it is meant to deny. A test that only
//! shows a control saying "yes" would pass just as happily if the control were
//! never consulted, which is the exact failure this story exists to fix.

use std::path::{Path, PathBuf};

use serde_json::json;
use tools::ToolPermissionInfo;

use crate::permission::{PermissionPolicy, PolicyDecision, ToolRequest, READ_TOOLS, WRITE_TOOLS};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_info() -> ToolPermissionInfo {
    ToolPermissionInfo { reads_files: true, path_inputs: &["path"], ..Default::default() }
}

fn write_info() -> ToolPermissionInfo {
    ToolPermissionInfo { writes_files: true, path_inputs: &["path"], ..Default::default() }
}

/// A shell-style tool: reads and writes anywhere, exposes no path input.
fn shell_info(program: &str) -> ToolPermissionInfo {
    ToolPermissionInfo {
        reads_files: true,
        writes_files: true,
        path_inputs: &[],
        shell_program: Some(program.to_string()),
    }
}

fn inert_info() -> ToolPermissionInfo {
    ToolPermissionInfo::default()
}

/// A workspace root that exists on neither platform, so resolution stays
/// lexical and the assertions do not depend on this machine's filesystem.
fn root() -> PathBuf {
    PathBuf::from("/ws")
}

fn decide(
    policy: &PermissionPolicy,
    name: &str,
    inputs: serde_json::Value,
    info: Option<ToolPermissionInfo>,
) -> PolicyDecision {
    policy.check_tool(
        &ToolRequest::new(name, &inputs)
            .with_info(info)
            .with_workspace_root(Some(root().as_path())),
    )
}

fn assert_denied(decision: PolicyDecision, needle: &str) {
    match decision {
        PolicyDecision::Deny(reason) => assert!(
            reason.contains(needle),
            "deny reason {reason:?} does not mention {needle:?}"
        ),
        other => panic!("expected a denial, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tool allow-list — pre-existing behaviour, still pinned
// ---------------------------------------------------------------------------

#[test]
fn allow_all_permits_any_tool() {
    let policy = PermissionPolicy::allow_all();
    assert_eq!(
        decide(&policy, "get_story", json!({}), Some(inert_info())),
        PolicyDecision::Allow
    );
}

#[test]
fn restricted_blocks_unlisted_tools() {
    let policy = PermissionPolicy::restricted(vec!["get_story".into()]);
    assert_denied(
        decide(&policy, "memory_write", json!({}), Some(inert_info())),
        "not permitted",
    );
}

#[test]
fn restricted_permits_a_listed_tool() {
    let policy = PermissionPolicy::restricted(vec!["get_story".into()]);
    assert_eq!(
        decide(&policy, "get_story", json!({}), Some(inert_info())),
        PolicyDecision::Allow
    );
}

/// The migration note that matters: a profile that configured nothing must be
/// unaffected by enforcement existing.
#[test]
fn an_unrestricted_profile_is_unaffected_by_any_of_the_new_gates() {
    let policy = PermissionPolicy::allow_all();

    for (name, info) in [
        ("file_read", read_info()),
        ("file_write", write_info()),
        ("file_list", read_info()),
        ("run_tests", shell_info("cargo")),
    ] {
        assert_eq!(
            decide(&policy, name, json!({ "path": "/etc/passwd" }), Some(info)),
            PolicyDecision::Allow,
            "{name} should be unaffected"
        );
    }
}

// ---------------------------------------------------------------------------
// allow_file_write_paths
// ---------------------------------------------------------------------------

#[test]
fn write_path_restriction_allows_inside_path() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_eq!(
        decide(
            &policy,
            "file_write",
            json!({ "path": "allowed/src/main.rs" }),
            Some(write_info())
        ),
        PolicyDecision::Allow
    );
}

#[test]
fn write_path_restriction_blocks_outside_path() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_denied(
        decide(&policy, "file_write", json!({ "path": "other/x" }), Some(write_info())),
        "outside this profile's allowed write paths",
    );
}

/// The bug the old string `starts_with` had.
#[test]
fn a_prefix_of_allowed_does_not_permit_a_write_to_allowed_other() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_denied(
        decide(
            &policy,
            "file_write",
            json!({ "path": "allowed-other/x" }),
            Some(write_info())
        ),
        "outside",
    );
}

#[test]
fn a_traversal_that_escapes_the_allowed_prefix_is_rejected() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_denied(
        decide(
            &policy,
            "file_write",
            json!({ "path": "./allowed/../../etc/x" }),
            Some(write_info())
        ),
        "outside",
    );
}

#[test]
fn a_traversal_that_stays_inside_the_allowed_prefix_is_permitted() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_eq!(
        decide(
            &policy,
            "file_write",
            json!({ "path": "allowed/deep/../notes.md" }),
            Some(write_info())
        ),
        PolicyDecision::Allow
    );
}

/// The path key comes from the tool, not from a hardcoded `"path"`.
#[test]
fn a_write_tool_whose_path_parameter_is_not_named_path_is_still_checked() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    let info = ToolPermissionInfo {
        writes_files: true,
        path_inputs: &["destination"],
        ..Default::default()
    };

    assert_denied(
        decide(
            &policy,
            "archive_to",
            json!({ "destination": "other/x" }),
            Some(info.clone()),
        ),
        "outside",
    );
    assert_eq!(
        decide(&policy, "archive_to", json!({ "destination": "allowed/x" }), Some(info)),
        PolicyDecision::Allow
    );
}

/// Every declared path input is checked, not just the first one that happens
/// to be present.
#[test]
fn a_write_tool_with_two_path_inputs_has_both_checked() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    let info = ToolPermissionInfo {
        writes_files: true,
        path_inputs: &["from", "to"],
        ..Default::default()
    };

    assert_denied(
        decide(
            &policy,
            "rename",
            json!({ "from": "allowed/a", "to": "escape/b" }),
            Some(info.clone()),
        ),
        "escape/b",
    );
    assert_eq!(
        decide(
            &policy,
            "rename",
            json!({ "from": "allowed/a", "to": "allowed/b" }),
            Some(info)
        ),
        PolicyDecision::Allow
    );
}

/// A write tool that supplies no path at all cannot be checked, so it is
/// refused rather than waved through.
#[test]
fn a_write_call_supplying_no_path_is_refused_when_write_paths_are_restricted() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_denied(
        decide(&policy, "file_write", json!({ "content": "hi" }), Some(write_info())),
        "no path that could be checked",
    );
}

#[test]
fn a_blank_allow_list_entry_does_not_silently_disable_the_restriction() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["   ".into(), "/ws/allowed".into()];

    assert_denied(
        decide(&policy, "file_write", json!({ "path": "other/x" }), Some(write_info())),
        "outside",
    );
    assert_eq!(
        decide(&policy, "file_write", json!({ "path": "allowed/x" }), Some(write_info())),
        PolicyDecision::Allow
    );
}

#[test]
fn a_read_tool_is_not_bound_by_the_write_path_list() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_eq!(
        decide(&policy, "file_read", json!({ "path": "other/x" }), Some(read_info())),
        PolicyDecision::Allow
    );
}

// ---------------------------------------------------------------------------
// allow_file_read_paths
// ---------------------------------------------------------------------------

#[test]
fn read_path_restriction_allows_file_read_inside_the_allowed_paths() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_read_paths = vec!["/ws/docs".into()];

    assert_eq!(
        decide(&policy, "file_read", json!({ "path": "docs/report.md" }), Some(read_info())),
        PolicyDecision::Allow
    );
}

#[test]
fn read_path_restriction_blocks_file_read_outside_the_allowed_paths() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_read_paths = vec!["/ws/docs".into()];

    assert_denied(
        decide(&policy, "file_read", json!({ "path": "secrets/keys.json" }), Some(read_info())),
        "outside this profile's allowed read paths",
    );
}

#[test]
fn read_path_restriction_blocks_file_list_outside_the_allowed_paths() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_read_paths = vec!["/ws/docs".into()];

    assert_denied(
        decide(&policy, "file_list", json!({ "path": "." }), Some(read_info())),
        "outside",
    );
    assert_eq!(
        decide(&policy, "file_list", json!({ "path": "docs" }), Some(read_info())),
        PolicyDecision::Allow
    );
}

#[test]
fn read_path_restriction_rejects_a_traversal_out_of_the_allowed_paths() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_read_paths = vec!["/ws/docs".into()];

    assert_denied(
        decide(
            &policy,
            "file_read",
            json!({ "path": "docs/../secrets/keys.json" }),
            Some(read_info())
        ),
        "outside",
    );
}

#[test]
fn a_write_tool_is_not_bound_by_the_read_path_list() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_read_paths = vec!["/ws/docs".into()];

    assert_eq!(
        decide(&policy, "file_write", json!({ "path": "src/main.rs" }), Some(write_info())),
        PolicyDecision::Allow
    );
}

// ---------------------------------------------------------------------------
// allow_shell_commands
// ---------------------------------------------------------------------------

#[test]
fn shell_restriction_allows_a_listed_program() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_shell_commands = vec!["cargo".into()];

    assert_eq!(
        decide(&policy, "run_tests", json!({}), Some(shell_info("cargo"))),
        PolicyDecision::Allow
    );
}

#[test]
fn shell_restriction_blocks_an_unlisted_program() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_shell_commands = vec!["cargo".into()];

    assert_denied(
        decide(&policy, "wipe", json!({}), Some(shell_info("rm"))),
        "not on this profile's allowed shell commands",
    );
}

/// The point of matching the resolved program rather than the command string:
/// argument text must not be able to satisfy the allow-list.
#[test]
fn argument_text_cannot_smuggle_a_match_past_the_shell_allow_list() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_shell_commands = vec!["cargo".into()];

    // `sh -c "cargo build"` — the word cargo appears, but the program is sh.
    assert_denied(
        decide(&policy, "sneaky", json!({}), Some(shell_info("sh"))),
        "runs 'sh'",
    );
}

#[test]
fn a_bare_allow_list_entry_matches_the_programs_file_name() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_shell_commands = vec!["git".into()];

    let program = if cfg!(windows) { "C:\\Program Files\\Git\\bin\\git.exe" } else { "/usr/bin/git" };
    assert_eq!(
        decide(&policy, "commit", json!({}), Some(shell_info(program))),
        PolicyDecision::Allow
    );
}

/// An operator who wrote out a full path meant that binary, not any binary of
/// the same name found somewhere else.
#[test]
fn an_allow_list_entry_naming_a_directory_must_match_the_whole_path() {
    let mut policy = PermissionPolicy::allow_all();
    let allowed = if cfg!(windows) { "C:\\tools\\git.exe" } else { "/usr/bin/git" };
    let other = if cfg!(windows) { "C:\\evil\\git.exe" } else { "/tmp/evil/git" };
    policy.allow_shell_commands = vec![allowed.into()];

    assert_eq!(
        decide(&policy, "commit", json!({}), Some(shell_info(allowed))),
        PolicyDecision::Allow
    );
    assert_denied(
        decide(&policy, "commit", json!({}), Some(shell_info(other))),
        "not on this profile's allowed shell commands",
    );
}

#[cfg(windows)]
#[test]
fn windows_shell_matching_ignores_case_and_the_exe_suffix() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_shell_commands = vec!["Cargo".into()];

    assert_eq!(
        decide(&policy, "build", json!({}), Some(shell_info("cargo.exe"))),
        PolicyDecision::Allow
    );
}

#[test]
fn the_shell_allow_list_does_not_touch_non_shell_tools() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_shell_commands = vec!["cargo".into()];

    assert_eq!(
        decide(&policy, "file_read", json!({ "path": "a.md" }), Some(read_info())),
        PolicyDecision::Allow
    );
}

// ---------------------------------------------------------------------------
// Custom shell tools vs. the path allow-lists
// ---------------------------------------------------------------------------

/// A subprocess can write anywhere and offers no path to check, so a profile
/// that restricts write paths cannot honestly let one through.
#[test]
fn a_custom_shell_tool_is_refused_when_write_paths_are_restricted() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_denied(
        decide(&policy, "run_tests", json!({}), Some(shell_info("cargo"))),
        "no path that could be checked",
    );
}

/// ...and being on the shell allow-list does not buy an exemption from it.
#[test]
fn the_shell_allow_list_does_not_override_a_write_path_restriction() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_shell_commands = vec!["cargo".into()];
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_denied(
        decide(&policy, "run_tests", json!({}), Some(shell_info("cargo"))),
        "no path that could be checked",
    );
}

#[test]
fn a_custom_shell_tool_is_refused_when_read_paths_are_restricted() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_read_paths = vec!["/ws/docs".into()];

    assert_denied(
        decide(&policy, "run_tests", json!({}), Some(shell_info("cargo"))),
        "no path that could be checked",
    );
}

// ---------------------------------------------------------------------------
// require_approval_on_write
// ---------------------------------------------------------------------------

#[test]
fn require_approval_on_write_returns_requires_approval() {
    let mut policy = PermissionPolicy::allow_all();
    policy.require_approval_on_write = true;

    assert_eq!(
        decide(&policy, "file_write", json!({ "path": "src/main.rs" }), Some(write_info())),
        PolicyDecision::RequiresApproval
    );
}

#[test]
fn require_approval_does_not_affect_non_write_tools() {
    let mut policy = PermissionPolicy::allow_all();
    policy.require_approval_on_write = true;

    assert_eq!(
        decide(&policy, "get_story", json!({}), Some(inert_info())),
        PolicyDecision::Allow
    );
    assert_eq!(
        decide(&policy, "file_read", json!({ "path": "a.md" }), Some(read_info())),
        PolicyDecision::Allow
    );
}

/// `file_edit` arrived with the patch-based edit story and mutates the
/// filesystem exactly as `file_write` does, so it has to clear the same two
/// gates. Ported from that story's tests when the two branches met here —
/// without them, the newer write tool would be a hole straight through both.
#[test]
fn file_edit_requires_approval_when_writes_need_approval() {
    let mut policy = PermissionPolicy::allow_all();
    policy.require_approval_on_write = true;

    assert_eq!(
        decide(&policy, "file_edit", json!({ "path": "src/main.rs" }), Some(write_info())),
        PolicyDecision::RequiresApproval
    );
}

#[test]
fn file_edit_is_bound_by_allow_file_write_paths() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["src/".into()];

    assert!(matches!(
        decide(&policy, "file_edit", json!({ "path": "docs/secret.md" }), Some(write_info())),
        PolicyDecision::Deny(_)
    ));
    assert_eq!(
        decide(&policy, "file_edit", json!({ "path": "src/main.rs" }), Some(write_info())),
        PolicyDecision::Allow
    );
}

/// The gap this story closes: a shell command was never classified as a write,
/// so `require_approval_on_write` never saw it.
#[test]
fn require_approval_on_write_covers_custom_shell_tools() {
    let mut policy = PermissionPolicy::allow_all();
    policy.require_approval_on_write = true;

    assert_eq!(
        decide(&policy, "run_tests", json!({}), Some(shell_info("cargo"))),
        PolicyDecision::RequiresApproval
    );
}

/// A denial outranks an approval prompt — there is no point asking a human to
/// approve something the policy has already ruled out.
#[test]
fn a_path_denial_takes_precedence_over_the_approval_prompt() {
    let mut policy = PermissionPolicy::allow_all();
    policy.require_approval_on_write = true;
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];

    assert_denied(
        decide(&policy, "file_write", json!({ "path": "other/x" }), Some(write_info())),
        "outside",
    );
}

// ---------------------------------------------------------------------------
// Classification by name, and failing closed
// ---------------------------------------------------------------------------

/// `WRITE_TOOLS` still classifies by name for callers that have no registry to
/// consult, so the approval gate keeps working for them.
#[test]
fn write_classification_by_name_works_without_a_tool_declaration() {
    let mut policy = PermissionPolicy::allow_all();
    policy.require_approval_on_write = true;
    let inputs = json!({ "path": "a.txt" });

    assert_eq!(
        policy.check_tool(&ToolRequest::new("file_write", &inputs)),
        PolicyDecision::RequiresApproval
    );
    assert_eq!(
        policy.check_tool(&ToolRequest::new("file_edit", &inputs)),
        PolicyDecision::RequiresApproval
    );
}

/// The six frontend/Tauri command names that used to sit in `WRITE_TOOLS` are
/// gone. No model can call them, so classifying them gated nothing — but a
/// stale list invites the belief that it did.
#[test]
fn the_retired_frontend_command_names_are_no_longer_classified_as_writes() {
    let mut policy = PermissionPolicy::allow_all();
    policy.require_approval_on_write = true;
    let inputs = json!({ "path": "a.txt" });

    for stale in [
        "write_file_text",
        "create_empty_file",
        "create_dir_fs",
        "delete_path",
        "rename_path",
        "duplicate_file",
    ] {
        assert_eq!(
            policy.check_tool(&ToolRequest::new(stale, &inputs)),
            PolicyDecision::Allow,
            "{stale} is not an agent tool and should not be classified"
        );
    }
}

/// The name lists and the tools' own declarations must not drift apart: a name
/// classified one way here and the other way there is a hole that only shows up
/// on whichever path the caller happens to take.
#[tokio::test]
async fn the_name_lists_agree_with_what_the_registered_builtins_declare() {
    let db = db::testing::make_test_pool().await;
    let mut registry = tools::ToolRegistry::new();
    tools::builtin::register_builtins(&mut registry, db);

    let registered: Vec<String> =
        registry.all_definitions().into_iter().map(|d| d.name).collect();
    assert!(!registered.is_empty(), "register_builtins registered nothing");

    for name in &registered {
        let info = registry.permission_info(name).expect("registered tool");
        assert_eq!(
            info.writes_files,
            WRITE_TOOLS.contains(&name.as_str()),
            "'{name}' disagrees about being a write tool"
        );
        assert_eq!(
            info.reads_files,
            READ_TOOLS.contains(&name.as_str()),
            "'{name}' disagrees about being a read tool"
        );
    }

    // Nothing in READ_TOOLS is a name no model can call.
    for name in READ_TOOLS {
        assert!(
            registered.iter().any(|r| r == name),
            "READ_TOOLS names '{name}', which register_builtins does not register"
        );
    }

    // The same holds for WRITE_TOOLS, with one deliberate exception:
    // `file_edit` is registered by the patch-based edit tool, which is landing
    // on its own branch. Keeping its entry here is what stops whichever of the
    // two stories merges second from silently dropping the other's gate. When
    // that tool lands, this exception disappears.
    let unregistered: Vec<&&str> = WRITE_TOOLS
        .iter()
        .filter(|name| !registered.iter().any(|r| r == *name))
        .collect();
    assert_eq!(
        unregistered,
        vec![&"file_edit"],
        "WRITE_TOOLS should contain only registered tools, plus the known file_edit exception"
    );
}

#[test]
fn an_unclassifiable_tool_is_refused_when_a_path_restriction_is_configured() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];
    let inputs = json!({ "path": "allowed/x" });

    assert_denied(
        policy.check_tool(&ToolRequest::new("mystery_tool", &inputs)),
        "could not be classified",
    );
}

#[test]
fn an_unclassifiable_tool_is_permitted_when_nothing_is_restricted() {
    let policy = PermissionPolicy::allow_all();
    let inputs = json!({});

    assert_eq!(
        policy.check_tool(&ToolRequest::new("mystery_tool", &inputs)),
        PolicyDecision::Allow
    );
}

// ---------------------------------------------------------------------------
// from_db_permissions / from_db_json
// ---------------------------------------------------------------------------

#[test]
fn from_db_permissions_parses_every_retained_list() {
    let policy = PermissionPolicy::from_db_permissions(
        r#"["file_read"]"#,
        r#"["/docs"]"#,
        r#"["/out"]"#,
        r#"["git"]"#,
        true,
    );

    assert_eq!(policy.allowed_tools, vec!["file_read".to_string()]);
    assert_eq!(policy.allow_file_read_paths, vec!["/docs".to_string()]);
    assert_eq!(policy.allow_file_write_paths, vec!["/out".to_string()]);
    assert_eq!(policy.allow_shell_commands, vec!["git".to_string()]);
    assert!(policy.require_approval_on_write);
}

#[test]
fn from_db_permissions_treats_unparseable_json_as_unrestricted() {
    let policy = PermissionPolicy::from_db_permissions("not json", "", "null", "{}", false);

    assert!(policy.allowed_tools.is_empty());
    assert!(policy.allow_file_read_paths.is_empty());
    assert!(policy.allow_file_write_paths.is_empty());
    assert!(policy.allow_shell_commands.is_empty());
}

/// The legacy array shape still produces a restricted policy — dropping
/// `allow_network_hosts` must not turn one into allow-all.
#[test]
fn from_db_json_still_restricts_on_the_legacy_array_shape() {
    let policy = PermissionPolicy::from_db_json(Some(json!(["get_story"])));

    assert_eq!(policy.allowed_tools, vec!["get_story".to_string()]);
    assert!(!policy.check("memory_write"));
    assert!(policy.check("get_story"));
}

#[test]
fn from_db_json_treats_null_and_absent_as_allow_all() {
    assert!(PermissionPolicy::from_db_json(None).allowed_tools.is_empty());
    assert!(PermissionPolicy::from_db_json(Some(serde_json::Value::Null))
        .allowed_tools
        .is_empty());
}

// ---------------------------------------------------------------------------
// Workspace-root resolution
// ---------------------------------------------------------------------------

/// Relative tool inputs are resolved against the same root the tool will use,
/// so an allow-list written in absolute terms lines up with what the agent
/// actually asks for.
#[test]
fn a_relative_input_is_resolved_against_the_workspace_root() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/ws/allowed".into()];
    let inputs = json!({ "path": "allowed/x" });

    // Same input, a different root: now outside.
    let elsewhere = PathBuf::from("/other");
    assert_denied(
        policy.check_tool(
            &ToolRequest::new("file_write", &inputs)
                .with_info(Some(write_info()))
                .with_workspace_root(Some(elsewhere.as_path())),
        ),
        "outside",
    );
}

#[test]
fn without_a_workspace_root_paths_are_compared_as_written() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_write_paths = vec!["/workspace/".into()];

    let inside = json!({ "path": "/workspace/src/main.rs" });
    assert_eq!(
        policy.check_tool(
            &ToolRequest::new("file_write", &inside).with_info(Some(write_info()))
        ),
        PolicyDecision::Allow
    );

    let outside = json!({ "path": "/etc/passwd" });
    assert_denied(
        policy.check_tool(
            &ToolRequest::new("file_write", &outside).with_info(Some(write_info()))
        ),
        "outside",
    );
}

#[test]
fn the_workspace_root_itself_is_inside_an_allow_list_naming_it() {
    let mut policy = PermissionPolicy::allow_all();
    policy.allow_file_read_paths = vec![".".into()];
    let inputs = json!({ "path": "." });

    assert_eq!(
        policy.check_tool(
            &ToolRequest::new("file_list", &inputs)
                .with_info(Some(read_info()))
                .with_workspace_root(Some(Path::new("/ws"))),
        ),
        PolicyDecision::Allow
    );
}
