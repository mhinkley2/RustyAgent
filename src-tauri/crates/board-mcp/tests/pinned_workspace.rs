//! Two clients, one database, two projects.
//!
//! The board is workspace-scoped throughout, but "which workspace" had exactly
//! one answer per database: the most recently opened row, re-read on every
//! JSON-RPC message. Two editor windows on two projects therefore shared a
//! scope, and whichever activated its workspace last silently became the scope
//! for both — no error, no warning, the other one just reading and writing the
//! wrong project's board.
//!
//! These run two `McpCtx` values against one pool, which is exactly what two
//! stdio processes are. No editor and no subprocess required: `McpCtx` is
//! deliberately free of Tauri types so the whole tool surface is reachable from
//! a test.

use std::path::{Path, PathBuf};

use board_mcp::{build_registry, McpCtx};
use serde_json::{json, Value};

/// Two registered projects on one board, with directories that really exist.
///
/// `use_workspace` refuses a path that is not on disk, so the fixture has to be
/// real — which is right: a workspace the user cannot open is not one an agent
/// should be able to select either.
struct TwoProjects {
    db: db::DbPool,
    a: PathBuf,
    b: PathBuf,
    _root: tempfile::TempDir,
}

async fn two_projects() -> TwoProjects {
    let root = tempfile::tempdir().expect("temp dir");
    let a = root.path().join("project-a");
    let b = root.path().join("project-b");
    std::fs::create_dir_all(&a).expect("create project-a");
    std::fs::create_dir_all(&b).expect("create project-b");

    let db = db::testing::make_test_pool().await;
    db::testing::seed_workspace(&db, "ws-a", &a.to_string_lossy()).await;
    db::testing::seed_workspace(&db, "ws-b", &b.to_string_lossy()).await;

    TwoProjects { db, a, b, _root: root }
}

async fn seed_story_in(db: &db::DbPool, id: &str, title: &str, workspace_id: &str) {
    sqlx::query(
        "INSERT INTO stories (id, title, status, workspace_id) VALUES (?, ?, 'backlog', ?)",
    )
    .bind(id)
    .bind(title)
    .bind(workspace_id)
    .execute(db)
    .await
    .expect("seed a story");
}

/// A client confined to one project, as the stdio binary builds one.
fn pinned(db: &db::DbPool, path: &Path, id: &str) -> McpCtx {
    McpCtx::new(db.clone()).pinned_to(path.to_path_buf(), Some(id.to_string()))
}

/// Dispatch one tool call and return its structured payload.
async fn call(ctx: &McpCtx, name: &str, args: Value) -> Value {
    let mut ctx = ctx.clone();
    ctx.refresh_workspace().await;

    let message = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": args },
    });
    let response = board_mcp::handle_message(&ctx, &build_registry(), &message)
        .await
        .expect("a response");

    let result = &response["result"];
    assert_ne!(
        result["isError"],
        json!(true),
        "{name} failed: {}",
        result["content"][0]["text"]
    );
    result["structuredContent"].clone()
}

/// As [`call`], but for a call expected to be refused.
async fn call_err(ctx: &McpCtx, name: &str, args: Value) -> String {
    let mut ctx = ctx.clone();
    ctx.refresh_workspace().await;

    let message = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": args },
    });
    let response = board_mcp::handle_message(&ctx, &build_registry(), &message)
        .await
        .expect("a response");

    assert_eq!(
        response["result"]["isError"],
        json!(true),
        "{name} should have been refused"
    );
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The payload of a tool that answers with a bare array rather than an object.
async fn call_array(ctx: &McpCtx, name: &str, args: Value) -> Vec<Value> {
    let mut ctx = ctx.clone();
    ctx.refresh_workspace().await;

    let message = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": args },
    });
    let response = board_mcp::handle_message(&ctx, &build_registry(), &message)
        .await
        .expect("a response");

    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    serde_json::from_str(text).expect("an array")
}

fn story_titles(payload: &Value) -> Vec<String> {
    payload["stories"]
        .as_array()
        .expect("stories")
        .iter()
        .map(|s| s["title"].as_str().unwrap_or_default().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// The defect
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_pinned_clients_read_their_own_boards() {
    let p = two_projects().await;
    seed_story_in(&p.db, "a1", "Project A work", "ws-a").await;
    seed_story_in(&p.db, "b1", "Project B work", "ws-b").await;

    let a = pinned(&p.db, &p.a, "ws-a");
    let b = pinned(&p.db, &p.b, "ws-b");

    assert_eq!(story_titles(&call(&a, "list_stories", json!({})).await), ["Project A work"]);
    assert_eq!(story_titles(&call(&b, "list_stories", json!({})).await), ["Project B work"]);
}

#[tokio::test]
async fn one_client_activating_its_workspace_does_not_move_another() {
    // The failure exactly: B opens its project, and A's *next* call returns B's
    // board. Nothing errors — A simply reads the wrong one.
    let p = two_projects().await;
    seed_story_in(&p.db, "a1", "Project A work", "ws-a").await;
    seed_story_in(&p.db, "b1", "Project B work", "ws-b").await;

    let a = pinned(&p.db, &p.a, "ws-a");

    db::touch_workspace(&p.db, &p.b)
        .await
        .expect("activate project B, as the app or another client would");

    assert_eq!(
        story_titles(&call(&a, "list_stories", json!({})).await),
        ["Project A work"],
        "the pinned client followed another client's workspace switch",
    );
}

#[tokio::test]
async fn an_unpinned_client_still_follows_the_active_workspace() {
    // The behaviour every existing setup has, unchanged. This feature is
    // additive, and a client launched outside any project depends on it.
    let p = two_projects().await;
    seed_story_in(&p.db, "a1", "Project A work", "ws-a").await;
    seed_story_in(&p.db, "b1", "Project B work", "ws-b").await;

    let shared = McpCtx::new(p.db.clone());
    db::touch_workspace(&p.db, &p.b)
        .await
        .expect("activate project B");

    assert_eq!(
        story_titles(&call(&shared, "list_stories", json!({})).await),
        ["Project B work"],
    );
}

// ---------------------------------------------------------------------------
// "Only", not "by default"
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pinned_client_cannot_switch_workspaces() {
    let p = two_projects().await;
    let a = pinned(&p.db, &p.a, "ws-a");

    let error = call_err(&a, "use_workspace", json!({ "path": p.b.to_string_lossy() })).await;

    assert!(
        error.contains(&p.a.to_string_lossy().to_string()),
        "the refusal should name the pin: {error}",
    );
    assert!(error.contains("RUSTYAGENT_WORKSPACE"), "and how to lift it: {error}");
}

#[tokio::test]
async fn a_pinned_client_cannot_even_reselect_its_own_workspace() {
    // Accepting this one would be a no-op that reads as a success, and the
    // model would have no way to learn the rule from it.
    let p = two_projects().await;
    let a = pinned(&p.db, &p.a, "ws-a");

    let error = call_err(&a, "use_workspace", json!({ "path": p.a.to_string_lossy() })).await;

    assert!(error.contains("cannot switch workspaces"), "got {error}");
}

#[tokio::test]
async fn an_unpinned_client_can_still_switch() {
    let p = two_projects().await;
    let shared = McpCtx::new(p.db.clone());

    let payload = call(&shared, "use_workspace", json!({ "path": p.b.to_string_lossy() })).await;

    assert_eq!(payload["workspace"]["id"], json!("ws-b"));
}

#[tokio::test]
async fn a_pinned_client_sees_only_its_own_workspace() {
    // A client that cannot switch has no use for the others, and listing a
    // user's repositories to an agent confined away from them is disclosure
    // without benefit.
    let p = two_projects().await;
    let a = pinned(&p.db, &p.a, "ws-a");

    let payload = call(&a, "list_workspaces", json!({})).await;

    let ids: Vec<&str> = payload["workspaces"]
        .as_array()
        .expect("workspaces")
        .iter()
        .map(|w| w["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, ["ws-a"]);
    assert_eq!(payload["workspaces"][0]["is_active"], json!(true));
}

#[tokio::test]
async fn an_unpinned_client_sees_every_workspace() {
    let p = two_projects().await;
    let shared = McpCtx::new(p.db.clone());

    let payload = call(&shared, "list_workspaces", json!({})).await;

    assert_eq!(payload["workspaces"].as_array().expect("workspaces").len(), 2);
}

// ---------------------------------------------------------------------------
// What else the scope decides
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pinned_client_writes_to_its_own_board() {
    // Reading the wrong board is bad; writing to it is worse, and it is the
    // same pointer that decides both.
    let p = two_projects().await;
    let a = pinned(&p.db, &p.a, "ws-a");

    db::touch_workspace(&p.db, &p.b)
        .await
        .expect("activate project B underneath it");
    call(&a, "create_story", json!({ "title": "Written by A" })).await;

    let b = pinned(&p.db, &p.b, "ws-b");
    assert_eq!(
        story_titles(&call(&a, "list_stories", json!({})).await),
        ["Written by A"],
    );
    assert!(
        story_titles(&call(&b, "list_stories", json!({})).await).is_empty(),
        "A's story landed on B's board",
    );
}

#[tokio::test]
async fn a_pinned_client_lists_its_own_workspaces_agent_profiles() {
    // `get_profiles` filters `scope = 'global' OR workspace_id = ?`, so the
    // shared pointer leaked another project's agents too — including any loaded
    // from that project's `.rusty/agents/*.toml`.
    let p = two_projects().await;
    for (id, name, ws) in [("p-a", "A's agent", "ws-a"), ("p-b", "B's agent", "ws-b")] {
        sqlx::query(
            "INSERT INTO agent_profiles
                 (id, name, provider, model, system_prompt, scope, workspace_id)
             VALUES (?, ?, 'mock', 'mock-model', '', 'workspace', ?)",
        )
        .bind(id)
        .bind(name)
        .bind(ws)
        .execute(&p.db)
        .await
        .expect("seed a profile");
    }

    let a = pinned(&p.db, &p.a, "ws-a");
    db::touch_workspace(&p.db, &p.b)
        .await
        .expect("activate project B underneath it");

    let profiles = call_array(&a, "list_agent_profiles", json!({})).await;
    let names: Vec<&str> = profiles
        .iter()
        .map(|profile| profile["name"].as_str().unwrap_or_default())
        .collect();

    assert_eq!(names, ["A's agent"]);
}
