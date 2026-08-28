//! Tool behaviour: workspace scoping, the confinement guard, and round-trips.

mod common;

use common::*;
use serde_json::json;
use tempfile::TempDir;

/// Register a real temp directory as a workspace and scope the context to it.
async fn workspace_ctx() -> (TempDir, board_mcp::McpCtx) {
    let dir = TempDir::new().expect("temp dir");
    let canonical = std::fs::canonicalize(dir.path()).expect("canonicalize");
    let path = canonical
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());

    let mut ctx = ctx().await;
    db::testing::seed_workspace(&ctx.db, "ws-1", &path).await;
    ctx.refresh_workspace().await;
    (dir, ctx)
}

// -- use_workspace confinement ----------------------------------------------

#[tokio::test]
async fn use_workspace_accepts_a_directory_already_registered_as_a_workspace() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();

    let structured = call_ok(
        &ctx,
        &registry,
        "use_workspace",
        json!({ "path": dir.path().to_string_lossy() }),
    )
    .await;

    assert_eq!(structured["workspace"]["id"], json!("ws-1"));
}

#[tokio::test]
async fn use_workspace_refuses_a_real_directory_that_is_not_a_known_workspace() {
    // The security regression test. Without confinement an MCP client could
    // point the workspace at any directory on the machine and then read it
    // through read_file — a whole-disk read primitive.
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let outside = TempDir::new().expect("outside dir");

    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces")
        .fetch_one(&ctx.db)
        .await
        .expect("count");

    let message = call_err(
        &ctx,
        &registry,
        "use_workspace",
        json!({ "path": outside.path().to_string_lossy() }),
    )
    .await;

    assert!(message.contains("Unknown workspace"), "got {message}");
    assert!(
        message.contains("Open it in the RustyAgent app first"),
        "the message should say how to fix it: {message}"
    );

    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces")
        .fetch_one(&ctx.db)
        .await
        .expect("count");
    assert_eq!(before, after, "no workspace row may be created");
}

#[tokio::test]
async fn use_workspace_rejects_a_path_that_does_not_exist() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();

    let message = call_err(
        &ctx,
        &registry,
        "use_workspace",
        json!({ "path": "/definitely/not/here/at/all" }),
    )
    .await;

    assert!(message.contains("does not exist"), "got {message}");
}

#[tokio::test]
async fn use_workspace_notifies_the_host_exactly_once() {
    let (host_ctx, host) = ctx_with_host().await;
    let registry = registry();
    let dir = TempDir::new().expect("temp dir");
    let canonical = std::fs::canonicalize(dir.path()).expect("canonicalize");
    let path = canonical
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .map(str::to_string)
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());
    db::testing::seed_workspace(&host_ctx.db, "ws-1", &path).await;

    call_ok(
        &host_ctx,
        &registry,
        "use_workspace",
        json!({ "path": dir.path().to_string_lossy() }),
    )
    .await;

    assert_eq!(host.workspace_change_count(), 1);
}

#[tokio::test]
async fn list_workspaces_marks_the_active_one() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    db::testing::seed_workspace(&ctx.db, "ws-2", "/somewhere/else").await;

    let structured = call_ok(&ctx, &registry, "list_workspaces", json!({})).await;

    let workspaces = structured["workspaces"].as_array().expect("array");
    assert_eq!(workspaces.len(), 2);
    let active: Vec<_> = workspaces
        .iter()
        .filter(|w| w["is_active"] == json!(true))
        .collect();
    assert_eq!(active.len(), 1, "exactly one workspace should be active");
    assert_eq!(active[0]["id"], json!("ws-1"));
}

// -- board -------------------------------------------------------------------

#[tokio::test]
async fn a_story_created_over_mcp_comes_back_from_list_stories() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();

    call_ok(
        &ctx,
        &registry,
        "create_story",
        json!({ "title": "Wire up MCP", "description": "end to end" }),
    )
    .await;

    let structured = call_ok(&ctx, &registry, "list_stories", json!({})).await;
    let text = serde_json::to_string(&structured).expect("serialize");
    assert!(text.contains("Wire up MCP"), "got {text}");
}

#[tokio::test]
async fn reorder_stories_applies_every_update() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    for id in ["s1", "s2", "s3"] {
        db::testing::seed_story(&ctx.db, id, id, "backlog").await;
    }

    let structured = call_ok(
        &ctx,
        &registry,
        "reorder_stories",
        json!({ "updates": [
            { "id": "s1", "sort_order": 30 },
            { "id": "s2", "sort_order": 20 },
            { "id": "s3", "sort_order": 10 }
        ]}),
    )
    .await;

    assert_eq!(structured["updated"], json!(3));
    let order: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, sort_order FROM stories ORDER BY sort_order ASC")
            .fetch_all(&ctx.db)
            .await
            .expect("query");
    assert_eq!(
        order,
        vec![
            ("s3".to_string(), 10),
            ("s2".to_string(), 20),
            ("s1".to_string(), 30)
        ]
    );
}

#[tokio::test]
async fn reorder_stories_rejects_a_malformed_update_entry() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();

    let message = call_err(
        &ctx,
        &registry,
        "reorder_stories",
        json!({ "updates": [{ "id": "s1" }] }),
    )
    .await;

    assert!(message.contains("sort_order"), "got {message}");
}

// -- runs --------------------------------------------------------------------

#[tokio::test]
async fn run_events_come_back_in_sequence_order() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    db::testing::seed_profile(&ctx.db, "agent-1", "Agent").await;
    db::testing::seed_story(&ctx.db, "story-1", "Story", "ready").await;
    db::testing::seed_run(&ctx.db, "run-1", "story-1", "agent-1").await;

    // Insert out of order on purpose.
    for (id, seq, content) in [("e3", 2, "third"), ("e1", 0, "first"), ("e2", 1, "second")] {
        sqlx::query(
            "INSERT INTO run_events (id, run_id, event_type, content, sequence_num)
             VALUES (?, 'run-1', 'token', ?, ?)",
        )
        .bind(id)
        .bind(content)
        .bind(seq)
        .execute(&ctx.db)
        .await
        .expect("seed event");
    }

    let structured = call_ok(&ctx, &registry, "get_run_events", json!({ "run_id": "run-1" })).await;

    let contents: Vec<&str> = structured
        .as_array()
        .expect("array")
        .iter()
        .map(|event| event["content"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(contents, vec!["first", "second", "third"]);
}

#[tokio::test]
async fn get_run_on_a_missing_id_is_a_tool_error_not_a_protocol_error() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();

    let message = call_err(&ctx, &registry, "get_run", json!({ "run_id": "nope" })).await;

    assert!(message.to_lowercase().contains("not found"), "got {message}");
}

// -- chat --------------------------------------------------------------------

#[tokio::test]
async fn a_chat_session_round_trips_through_create_append_and_read() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();

    let session = call_ok(
        &ctx,
        &registry,
        "create_chat_session",
        json!({ "title": "Planning" }),
    )
    .await;
    let session_id = session["id"].as_str().expect("session id").to_string();

    call_ok(
        &ctx,
        &registry,
        "append_chat_message",
        json!({ "session_id": session_id, "role": "user", "content": "hello" }),
    )
    .await;

    let messages = call_ok(
        &ctx,
        &registry,
        "get_chat_session_messages",
        json!({ "session_id": session_id }),
    )
    .await;

    let messages = messages.as_array().expect("array");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], json!("user"));
    assert_eq!(messages[0]["content"], json!("hello"));
}

// -- filesystem --------------------------------------------------------------

#[tokio::test]
async fn read_file_reads_a_file_inside_the_workspace() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let file = dir.path().join("notes.md");
    std::fs::write(&file, "hello from the workspace").expect("write");

    let structured = call_ok(
        &ctx,
        &registry,
        "read_file",
        json!({ "path": file.to_string_lossy() }),
    )
    .await;

    assert_eq!(structured["content"], json!("hello from the workspace"));
}

#[tokio::test]
async fn read_file_refuses_a_path_outside_the_workspace() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let outside = TempDir::new().expect("outside");
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "do not read me").expect("write");

    let message = call_err(
        &ctx,
        &registry,
        "read_file",
        json!({ "path": secret.to_string_lossy() }),
    )
    .await;

    assert!(
        message.contains("outside the workspace"),
        "got {message}"
    );
}

#[tokio::test]
async fn list_directory_skips_build_and_vcs_directories() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    for skipped in [".git", "node_modules", "target"] {
        std::fs::create_dir_all(dir.path().join(skipped)).expect("mkdir");
    }
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("README.md"), "x").expect("write");

    let structured = call_ok(
        &ctx,
        &registry,
        "list_directory",
        json!({ "path": dir.path().to_string_lossy() }),
    )
    .await;

    let names: Vec<&str> = structured
        .as_array()
        .expect("array")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(names, vec!["src", "README.md"]);
}

// -- agents and settings -----------------------------------------------------

#[tokio::test]
async fn agent_permissions_default_to_allow_all_when_no_row_exists() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    db::testing::seed_profile(&ctx.db, "agent-1", "Agent").await;

    let structured = call_ok(
        &ctx,
        &registry,
        "get_agent_permissions",
        json!({ "profile_id": "agent-1" }),
    )
    .await;

    assert_eq!(structured["profileId"], json!("agent-1"));
    assert_eq!(structured["allowedTools"], json!([]));
}

#[tokio::test]
async fn workspace_settings_round_trip() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();

    call_ok(
        &ctx,
        &registry,
        "save_workspace_settings",
        json!({ "overrides": { "theme": "dark" } }),
    )
    .await;

    let structured = call_ok(&ctx, &registry, "get_workspace_settings", json!({})).await;

    assert_eq!(structured["theme"], json!("dark"));
}

#[tokio::test]
async fn workspace_scoped_tools_report_clearly_when_no_workspace_is_active() {
    // No seeded workspace at all.
    let ctx = ctx().await;
    let registry = registry();

    let message = call_err(&ctx, &registry, "get_workspace_settings", json!({})).await;

    assert!(message.contains("No active workspace"), "got {message}");
}

// -- pipeline validation -----------------------------------------------------

#[tokio::test]
async fn validate_pipeline_rejects_an_empty_step_list() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    sqlx::query(
        "INSERT INTO stories (id, title, status, story_type, pipeline_config)
         VALUES ('p1', 'Pipeline', 'ready', 'pipeline', ?)",
    )
    .bind(r#"{"mode":"sequential","steps":[]}"#)
    .execute(&ctx.db)
    .await
    .expect("seed pipeline story");

    let message = call_err(&ctx, &registry, "validate_pipeline", json!({ "story_id": "p1" })).await;

    assert!(message.contains("at least one step"), "got {message}");
}

#[tokio::test]
async fn validate_pipeline_rejects_a_step_referencing_the_pipeline_itself() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    sqlx::query(
        "INSERT INTO stories (id, title, status, story_type, pipeline_config)
         VALUES ('p1', 'Pipeline', 'ready', 'pipeline', ?)",
    )
    .bind(r#"{"mode":"sequential","steps":[{"label":"Self","storyId":"p1","agentId":"a1"}]}"#)
    .execute(&ctx.db)
    .await
    .expect("seed pipeline story");

    let message = call_err(&ctx, &registry, "validate_pipeline", json!({ "story_id": "p1" })).await;

    assert!(message.to_lowercase().contains("cycle"), "got {message}");
}
