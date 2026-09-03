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

    let contents: Vec<&str> = structured["events"]
        .as_array()
        .expect("array")
        .iter()
        .map(|event| event["content"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(contents, vec!["first", "second", "third"]);
    assert_eq!(structured["total"], json!(3));
    assert_eq!(structured["complete"], json!(true));
    assert_eq!(structured["next_offset"], serde_json::Value::Null);
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

    assert_eq!(messages["total"], json!(1));
    assert_eq!(messages["complete"], json!(true));
    let messages = messages["messages"].as_array().expect("array");
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

    let names: Vec<&str> = structured["entries"]
        .as_array()
        .expect("array")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(names, vec!["src", "README.md"]);
    assert_eq!(structured["complete"], json!(true));
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

// -- response caps -----------------------------------------------------------
//
// The MCP surface answers into an *external* agent's context — Claude Code, an
// editor, anything that speaks the protocol. These tests pin down that a large
// answer arrives clipped, that the client can tell it was clipped, and that it
// can ask for the rest.

/// Seed a file whose lines are exactly 64 bytes each, so the 32 KB cap lands on
/// a line boundary at a number the assertions can name.
fn seed_padded_lines(root: &std::path::Path, name: &str, count: usize) -> String {
    let content: String = (0..count)
        .map(|i| format!("line {i:04} {}\n", "-".repeat(53)))
        .collect();
    assert_eq!(content.len(), count * 64, "line width assumption broke");
    std::fs::write(root.join(name), &content).expect("seed file");
    content
}

/// The cap these assertions do arithmetic against.
///
/// Taken from `tools::read_cap`, not restated. A second copy of the number
/// would keep passing after the real one changed — which is the exact drift
/// this whole change exists to remove, so it would be a poor place to
/// reintroduce it.
const CAP: usize = tools::read_cap::MAX_READ_BYTES;

/// The file bytes of a reply, with the trailing marker removed.
///
/// The marker is introduced by its own newline, so the split must keep the
/// body's own terminator — dropping it would turn a CRLF file's last returned
/// line into a bare `\r` and make this helper, not the code, the thing under
/// test.
fn strip_marker(text: &str) -> &str {
    match text.rfind("\n[read_file ") {
        Some(i) => &text[..i],
        None => text,
    }
}

#[tokio::test]
async fn read_file_under_the_cap_returns_the_file_verbatim_and_says_it_is_complete() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let content = seed_padded_lines(dir.path(), "small.txt", 100);

    let structured = call_ok(
        &ctx,
        &registry,
        "read_file",
        json!({ "path": dir.path().join("small.txt").to_string_lossy() }),
    )
    .await;

    assert_eq!(structured["content"], json!(content));
    assert_eq!(structured["truncated"], json!(false));
    assert_eq!(structured["complete"], json!(true));
    assert_eq!(structured["next_offset"], serde_json::Value::Null);
}

#[tokio::test]
async fn read_file_over_the_cap_truncates_and_states_the_real_size_and_resume_line() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    // 600 * 64 = 38400 bytes, comfortably past the 32 KB cap.
    let content = seed_padded_lines(dir.path(), "big.txt", 600);
    let path = dir.path().join("big.txt").to_string_lossy().into_owned();

    let structured = call_ok(&ctx, &registry, "read_file", json!({ "path": path })).await;

    let text = structured["content"].as_str().expect("content");
    // The head is a byte-exact prefix of the file, cut on a line boundary.
    assert!(text.starts_with(&content[..CAP]), "the head was not returned verbatim");
    // ...and the external agent is told, in terms it cannot read as file
    // content, that this is not the whole file.
    assert!(text.contains("[read_file TRUNCATED:"), "no marker in {text:?}");
    assert!(text.contains("is NOT the complete file"));
    assert!(text.contains("38400 bytes / 600 lines"), "got {text:?}");
    assert!(text.contains("lines 1-512"), "got {text:?}");
    assert!(text.contains("5632 bytes remain after it"), "got {text:?}");
    // ...and how to ask for the rest, both in prose and in a field.
    assert!(text.contains("Call read_file again with \"offset\": 513"), "got {text:?}");
    assert_eq!(structured["truncated"], json!(true));
    assert_eq!(structured["complete"], json!(false));
    assert_eq!(structured["next_offset"], json!(513));
    assert_eq!(structured["total_bytes"], json!(38400));
    assert_eq!(structured["total_lines"], json!(600));
}

#[tokio::test]
async fn an_external_client_can_page_a_large_file_to_the_end_from_next_offset() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let content = seed_padded_lines(dir.path(), "big.txt", 1500);
    let path = dir.path().join("big.txt").to_string_lossy().into_owned();

    // Exactly what an external client would do: follow next_offset until it is
    // null, concatenating the body of each reply.
    let mut assembled = String::new();
    let mut offset = json!(1);
    let mut pages = 0;
    loop {
        let structured =
            call_ok(&ctx, &registry, "read_file", json!({ "path": path, "offset": offset })).await;
        let text = structured["content"].as_str().expect("content").to_string();
        assembled.push_str(strip_marker(&text));
        pages += 1;
        assert!(pages < 20, "paging did not converge");
        match structured["next_offset"].as_u64() {
            Some(next) => offset = json!(next),
            None => break,
        }
    }

    assert!(pages > 1, "the file should have needed more than one page");
    assert_eq!(assembled, content, "paging must reassemble the file exactly");
}

#[tokio::test]
async fn read_file_offset_and_limit_return_exactly_that_line_range() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    std::fs::write(dir.path().join("five.txt"), "alpha\nbravo\ncharlie\ndelta\necho\n")
        .expect("seed");
    let path = dir.path().join("five.txt").to_string_lossy().into_owned();

    let structured = call_ok(
        &ctx,
        &registry,
        "read_file",
        json!({ "path": path, "offset": 2, "limit": 2 }),
    )
    .await;

    let text = structured["content"].as_str().expect("content");
    assert!(text.starts_with("bravo\ncharlie\n"), "got {text:?}");
    // Short of the whole file but not cut by the cap: PARTIAL, not TRUNCATED.
    assert!(text.contains("[read_file PARTIAL: the text above is lines 2-3 of 5"), "got {text:?}");
    assert_eq!(structured["truncated"], json!(false));
    assert_eq!(structured["complete"], json!(false));
    assert_eq!(structured["next_offset"], json!(4));
}

#[tokio::test]
async fn read_file_rejects_an_offset_past_the_end_of_the_file() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    std::fs::write(dir.path().join("two.txt"), "one\ntwo\n").expect("seed");
    let path = dir.path().join("two.txt").to_string_lossy().into_owned();

    let message =
        call_err(&ctx, &registry, "read_file", json!({ "path": path, "offset": 40 })).await;

    assert!(message.contains("has 2 line(s)"), "got {message}");
}

#[tokio::test]
async fn read_file_rejects_a_zero_offset_rather_than_reading_it_as_one() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    std::fs::write(dir.path().join("two.txt"), "one\ntwo\n").expect("seed");
    let path = dir.path().join("two.txt").to_string_lossy().into_owned();

    let message =
        call_err(&ctx, &registry, "read_file", json!({ "path": path, "offset": 0 })).await;

    assert!(message.contains("positive integer"), "got {message}");
}

#[tokio::test]
async fn read_file_truncation_does_not_split_a_codepoint_in_a_file_with_no_newlines() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    // A minified-bundle shape: no newline anywhere, and a 3-byte codepoint
    // straddling the 32768-byte mark. Slicing there would panic.
    let content = "\u{20AC}".repeat(20_000);
    std::fs::write(dir.path().join("bundle.min.js"), &content).expect("seed");
    let path = dir.path().join("bundle.min.js").to_string_lossy().into_owned();

    let structured = call_ok(&ctx, &registry, "read_file", json!({ "path": path })).await;

    let text = structured["content"].as_str().expect("content");
    assert!(text.contains("[read_file TRUNCATED:"), "got no marker");
    let cut = (CAP / 3) * 3;
    assert!(cut < CAP, "the straddle assumption broke");
    assert!(text.starts_with(&content[..cut]));
    assert!(!text.starts_with(&content[..cut + 3]), "the cut kept a whole extra codepoint");
}

#[tokio::test]
async fn read_file_truncation_keeps_crlf_terminators_intact() {
    // CI is Linux and development is Windows, so a CRLF file must survive the
    // byte cut on both. Cutting between the "\r" and the "\n" would hand the
    // agent a line ending that is not on disk.
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    // 63 bytes of text plus CRLF = 65 bytes per line; 600 lines = 39000 bytes.
    let content: String = (0..600)
        .map(|i| format!("line {i:04} {}\r\n", "-".repeat(53)))
        .collect();
    assert_eq!(content.len(), 600 * 65, "line width assumption broke");
    std::fs::write(dir.path().join("crlf.txt"), &content).expect("seed");
    let path = dir.path().join("crlf.txt").to_string_lossy().into_owned();

    let structured = call_ok(&ctx, &registry, "read_file", json!({ "path": path })).await;

    let text = structured["content"].as_str().expect("content");
    let whole_lines = CAP / 65;
    let cut = whole_lines * 65;
    assert!(text.starts_with(&content[..cut]), "the head was not returned verbatim");
    assert!(!text.starts_with(&content[..cut + 1]), "the cut landed inside a line");
    let body = strip_marker(text);
    assert_eq!(body, &content[..cut], "the body is not byte-identical to the file");
    assert!(body.ends_with("\r\n"), "a CRLF pair was split");
    assert!(!body.contains("-\n"), "a bare LF appeared in a CRLF file");
}

#[tokio::test]
async fn get_run_events_pages_a_long_log_instead_of_returning_all_of_it() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    db::testing::seed_profile(&ctx.db, "agent-1", "Agent").await;
    db::testing::seed_story(&ctx.db, "story-1", "Story", "ready").await;
    db::testing::seed_run(&ctx.db, "run-1", "story-1", "agent-1").await;
    for seq in 0..120 {
        sqlx::query(
            "INSERT INTO run_events (id, run_id, event_type, content, sequence_num)
             VALUES (?, 'run-1', 'token', ?, ?)",
        )
        .bind(format!("e{seq:03}"))
        .bind(format!("event {seq}"))
        .bind(seq)
        .execute(&ctx.db)
        .await
        .expect("seed event");
    }

    let first = call_ok(&ctx, &registry, "get_run_events", json!({ "run_id": "run-1" })).await;

    // The default limit bounds an unasked-for read.
    assert_eq!(first["returned"], json!(50));
    assert_eq!(first["total"], json!(120));
    assert_eq!(first["complete"], json!(false));
    assert_eq!(first["next_offset"], json!(51));
    assert!(first["notice"].as_str().expect("notice").contains("\"offset\": 51"));

    // Following next_offset walks the log to its end without gaps or repeats.
    let mut seen: Vec<String> = Vec::new();
    let mut offset = json!(1);
    loop {
        let page = call_ok(
            &ctx,
            &registry,
            "get_run_events",
            json!({ "run_id": "run-1", "offset": offset }),
        )
        .await;
        for event in page["events"].as_array().expect("events") {
            seen.push(event["content"].as_str().unwrap_or_default().to_string());
        }
        match page["next_offset"].as_u64() {
            Some(next) => offset = json!(next),
            None => break,
        }
    }
    assert_eq!(seen.len(), 120);
    assert_eq!(seen[0], "event 0");
    assert_eq!(seen[119], "event 119");
}

#[tokio::test]
async fn get_run_events_caps_one_enormous_tool_output_in_place() {
    // The failure this whole story is about: a single tool result can be
    // megabytes, so bounding the event count alone bounds nothing.
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    db::testing::seed_profile(&ctx.db, "agent-1", "Agent").await;
    db::testing::seed_story(&ctx.db, "story-1", "Story", "ready").await;
    db::testing::seed_run(&ctx.db, "run-1", "story-1", "agent-1").await;
    let huge = "z".repeat(200_000);
    sqlx::query(
        "INSERT INTO run_events (id, run_id, event_type, tool_name, tool_output, sequence_num)
         VALUES ('e1', 'run-1', 'tool_result', 'shell', ?, 0)",
    )
    .bind(&huge)
    .execute(&ctx.db)
    .await
    .expect("seed event");

    let structured = call_ok(&ctx, &registry, "get_run_events", json!({ "run_id": "run-1" })).await;

    let output = structured["events"][0]["tool_output"].as_str().expect("tool_output");
    assert!(output.len() < 10_000, "the field was not capped: {} bytes", output.len());
    assert!(
        output.contains("[get_run_events FIELD TRUNCATED: 'tool_output' is 200000 bytes"),
        "the cap must name the field it cut"
    );
    // The row is still returned rather than dropped, so the caller can see
    // that the call happened at all.
    assert_eq!(structured["returned"], json!(1));
    assert_eq!(structured["events"][0]["tool_name"], json!("shell"));
}

#[tokio::test]
async fn get_chat_session_messages_pages_and_caps_a_long_conversation() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let session = call_ok(&ctx, &registry, "create_chat_session", json!({})).await;
    let session_id = session["id"].as_str().expect("session id").to_string();
    for i in 0..60 {
        call_ok(
            &ctx,
            &registry,
            "append_chat_message",
            json!({ "session_id": session_id, "role": "user", "content": format!("msg {i}") }),
        )
        .await;
    }
    // One message carrying a pasted file, which a row count would not bound.
    call_ok(
        &ctx,
        &registry,
        "append_chat_message",
        json!({ "session_id": session_id, "role": "user", "content": "p".repeat(50_000) }),
    )
    .await;

    let first = call_ok(
        &ctx,
        &registry,
        "get_chat_session_messages",
        json!({ "session_id": session_id }),
    )
    .await;

    assert_eq!(first["total"], json!(61));
    assert_eq!(first["returned"], json!(50));
    assert_eq!(first["complete"], json!(false));
    assert_eq!(first["next_offset"], json!(51));

    // The oversized message is found by its content, not by assuming a
    // position. `chat_session_messages` orders by `created_at ASC, id ASC`,
    // `created_at` is millisecond-resolution, and `id` is a random UUID — so
    // messages appended inside one millisecond tie on the timestamp and then
    // sort by a random string. Asserting the pasted file lands at offset 61
    // made this test a coin flip that only lost under load.
    let last = call_ok(
        &ctx,
        &registry,
        "get_chat_session_messages",
        json!({ "session_id": session_id, "offset": 51 }),
    )
    .await;
    assert_eq!(last["next_offset"], serde_json::Value::Null);

    let capped = last["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|m| m["content"].as_str())
        .find(|c| c.contains("[get_chat_session_messages FIELD TRUNCATED: 'content' is 50000 bytes"))
        .expect("the pasted file should appear capped somewhere in the final page");

    assert!(capped.len() < 10_000, "the pasted file was not capped");
}

#[tokio::test]
async fn list_directory_pages_a_large_directory() {
    let (dir, ctx) = workspace_ctx().await;
    let registry = registry();
    for i in 0..250 {
        std::fs::write(dir.path().join(format!("f{i:04}.txt")), "x").expect("write");
    }
    let path = dir.path().to_string_lossy().into_owned();

    let first = call_ok(&ctx, &registry, "list_directory", json!({ "path": path })).await;

    assert_eq!(first["total"], json!(250));
    assert_eq!(first["returned"], json!(200), "the default limit must bound an unasked-for read");
    assert_eq!(first["next_offset"], json!(201));
    assert_eq!(first["complete"], json!(false));

    let rest = call_ok(
        &ctx,
        &registry,
        "list_directory",
        json!({ "path": path, "offset": 201 }),
    )
    .await;
    assert_eq!(rest["returned"], json!(50));
    assert_eq!(rest["next_offset"], serde_json::Value::Null);
    assert_eq!(rest["entries"][0]["name"], json!("f0200.txt"));
}

#[tokio::test]
async fn the_read_tool_descriptions_advertise_their_caps_and_paging() {
    // The description string is the only documentation an external agent gets:
    // if it does not mention the cap, the agent has no reason to expect one.
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();

    let response = send(&ctx, &registry, request(json!(1), "tools/list", json!({}))).await;
    let listed = response["result"]["tools"].as_array().expect("tools").clone();
    let describe = |name: &str| -> String {
        listed
            .iter()
            .find(|tool| tool["name"] == json!(name))
            .unwrap_or_else(|| panic!("{name} not listed"))["description"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };

    let read_file = describe("read_file");
    assert!(read_file.contains("10 MB"), "the memory guard is still advertised");
    assert!(read_file.contains("32 KB"), "the context cap must be advertised too: {read_file}");
    assert!(read_file.contains("TRUNCATED"), "got {read_file}");
    assert!(read_file.contains("offset") && read_file.contains("limit"), "got {read_file}");

    // The diff read carries one thing the file read does not: a diff that was
    // cut is no longer a patch, and an agent that does not know that will try
    // to apply it.
    let run_diff = describe("get_run_diff");
    assert!(run_diff.contains("32 KB"), "the cap must be advertised: {run_diff}");
    assert!(run_diff.contains("TRUNCATED"), "got {run_diff}");
    assert!(run_diff.contains("git apply"), "the patch warning must be advertised: {run_diff}");

    for name in
        ["get_run_events", "get_chat_session_messages", "list_directory", "get_run_diff"]
    {
        let description = describe(name);
        assert!(
            description.contains("offset") && description.contains("limit"),
            "{name} must document its paging: {description}"
        );
    }

    for name in ["read_file", "get_run_events", "get_chat_session_messages", "list_directory"] {
        let schema = listed
            .iter()
            .find(|tool| tool["name"] == json!(name))
            .expect("listed")["inputSchema"]["properties"]
            .clone();
        assert!(schema.get("offset").is_some(), "{name} has no offset parameter");
        assert!(schema.get("limit").is_some(), "{name} has no limit parameter");
    }
}

// -- get_run_diff cap -------------------------------------------------------
//
// The last uncapped read on the MCP surface. Run isolation means every run now
// records a diff, and a large refactor's diff went straight into an external
// agent's context window. These reuse `tools::read_cap`, so what they pin is
// this tool's wiring into it, plus the one thing a diff needs that a file read
// does not: that a truncated diff is not a patch.

/// Seed a run carrying `diff` as its captured output.
async fn seed_run_with_diff(ctx: &board_mcp::McpCtx, run_id: &str, diff: Option<&str>) {
    db::testing::seed_profile(&ctx.db, "agent-1", "Agent").await;
    db::testing::seed_story(&ctx.db, "story-1", "Story", "done").await;
    sqlx::query(
        "INSERT INTO story_runs (id, story_id, agent_profile_id, status, before_sha, diff_output)
         VALUES (?, 'story-1', 'agent-1', 'done', 'abc123', ?)",
    )
    .bind(run_id)
    .bind(diff)
    .execute(&ctx.db)
    .await
    .expect("seed a run with a diff");
}

/// A diff of `count` lines, each exactly 64 bytes, so the cap lands where the
/// assertions can name it.
fn padded_diff(count: usize) -> String {
    (0..count).map(|i| format!("+line {i:04} {}\n", "-".repeat(52))).collect()
}

#[tokio::test]
async fn get_run_diff_under_the_cap_returns_the_diff_verbatim_and_says_it_is_complete() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let diff = padded_diff(100);
    seed_run_with_diff(&ctx, "run-small", Some(&diff)).await;

    let structured =
        call_ok(&ctx, &registry, "get_run_diff", json!({ "run_id": "run-small" })).await;

    assert_eq!(structured["diff_output"], json!(diff));
    assert_eq!(structured["truncated"], json!(false));
    assert_eq!(structured["complete"], json!(true));
    assert_eq!(structured["next_offset"], serde_json::Value::Null);
    assert_eq!(structured["before_sha"], json!("abc123"));
}

#[tokio::test]
async fn get_run_diff_over_the_cap_truncates_and_states_the_real_size_and_resume_line() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    // 600 * 64 = 38400 bytes, comfortably past the 32 KB cap.
    let diff = padded_diff(600);
    seed_run_with_diff(&ctx, "run-big", Some(&diff)).await;

    let structured =
        call_ok(&ctx, &registry, "get_run_diff", json!({ "run_id": "run-big" })).await;

    let text = structured["diff_output"].as_str().expect("diff_output");
    assert!(text.starts_with(&diff[..CAP]), "the head was not returned verbatim");
    assert!(text.contains("[get_run_diff TRUNCATED:"), "no marker in {text:?}");
    assert!(text.contains("38400 bytes / 600 lines"), "got {text:?}");
    assert!(text.contains("lines 1-512"), "got {text:?}");
    assert!(text.contains("Call get_run_diff again with \"offset\": 513"), "got {text:?}");
    assert_eq!(structured["truncated"], json!(true));
    assert_eq!(structured["complete"], json!(false));
    assert_eq!(structured["next_offset"], json!(513));
    assert_eq!(structured["total_bytes"], json!(38400));
    assert_eq!(structured["total_lines"], json!(600));
}

#[tokio::test]
async fn a_truncated_diff_says_it_is_not_an_applicable_patch() {
    // The one warning this tool does not inherit from the file read. Half a
    // file is still quotable; half a unified diff fed to `git apply` fails or,
    // worse, applies a subset of the change.
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    seed_run_with_diff(&ctx, "run-big", Some(&padded_diff(600))).await;

    let structured =
        call_ok(&ctx, &registry, "get_run_diff", json!({ "run_id": "run-big" })).await;
    let text = structured["diff_output"].as_str().expect("diff_output");

    assert!(text.contains("NOT an applicable"), "got {text:?}");
    assert!(text.contains("git apply"), "got {text:?}");
}

#[tokio::test]
async fn paging_with_next_offset_retrieves_the_rest_of_the_diff() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let diff = padded_diff(600);
    seed_run_with_diff(&ctx, "run-big", Some(&diff)).await;

    let first = call_ok(&ctx, &registry, "get_run_diff", json!({ "run_id": "run-big" })).await;
    let next = first["next_offset"].as_u64().expect("a next_offset");

    let second = call_ok(
        &ctx,
        &registry,
        "get_run_diff",
        json!({ "run_id": "run-big", "offset": next }),
    )
    .await;

    let tail = second["diff_output"].as_str().expect("diff_output");
    // Line 513 onwards, byte-exact, and this time it is the end.
    assert!(tail.starts_with(&diff[CAP..]), "the tail was not returned verbatim");
    assert_eq!(second["truncated"], json!(false));
    assert_eq!(second["next_offset"], serde_json::Value::Null);
    assert_eq!(second["before_sha"], json!("abc123"));
}

#[tokio::test]
async fn a_run_that_changed_nothing_returns_cleanly_rather_than_reporting_truncation() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    seed_run_with_diff(&ctx, "run-empty", None).await;

    let structured =
        call_ok(&ctx, &registry, "get_run_diff", json!({ "run_id": "run-empty" })).await;

    assert_eq!(structured["diff_output"], serde_json::Value::Null);
    assert_eq!(structured["truncated"], json!(false));
    assert_eq!(structured["complete"], json!(true));
    assert_eq!(structured["before_sha"], json!("abc123"));
}

#[tokio::test]
async fn truncation_is_char_boundary_safe_on_a_diff_of_multibyte_text() {
    // A diff of a file with non-ASCII content. Slicing at a fixed byte offset
    // would panic mid-codepoint; the cut must retreat to a boundary.
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    let diff: String = (0..800).map(|_| format!("+{}\n", "\u{20AC}".repeat(20))).collect();
    seed_run_with_diff(&ctx, "run-utf8", Some(&diff)).await;

    let structured =
        call_ok(&ctx, &registry, "get_run_diff", json!({ "run_id": "run-utf8" })).await;

    let text = structured["diff_output"].as_str().expect("diff_output");
    assert_eq!(structured["truncated"], json!(true));
    // The head is a byte-exact prefix, so every character survived intact.
    let body = text.split("\n[get_run_diff").next().expect("body");
    assert!(diff.starts_with(body), "the head was not a verbatim prefix");
}

#[tokio::test]
async fn get_run_diff_rejects_a_zero_offset_the_way_every_other_paged_read_does() {
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    seed_run_with_diff(&ctx, "run-small", Some(&padded_diff(10))).await;

    let response = send(
        &ctx,
        &registry,
        call("get_run_diff", json!({ "run_id": "run-small", "offset": 0 })),
    )
    .await;

    assert_eq!(response["result"]["isError"], json!(true), "got {response:?}");
}

#[tokio::test]
async fn before_sha_survives_truncation() {
    // It is what lets a caller fetch the real diff by other means, so it is the
    // one field the cap must not take with it.
    let (_dir, ctx) = workspace_ctx().await;
    let registry = registry();
    seed_run_with_diff(&ctx, "run-big", Some(&padded_diff(600))).await;

    let structured =
        call_ok(&ctx, &registry, "get_run_diff", json!({ "run_id": "run-big" })).await;

    assert_eq!(structured["truncated"], json!(true));
    assert_eq!(structured["before_sha"], json!("abc123"));
}
