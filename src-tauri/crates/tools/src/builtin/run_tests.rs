//! Tests for [`crate::builtin::run`] and the honesty of `spawn_subtask`.
//!
//! `spawn_subtask` returned a `run_id` that none of the thirteen registered
//! tools could accept, and told the model to poll `get_story` instead — a card
//! that only moves if the child agent chooses to move it. These pin the reply
//! an orchestrator now gets, and the two claims the tool makes about itself.

use serde_json::json;

use crate::builtin::run::GetRunTool;
use crate::builtin::subtask::SpawnSubtaskTool;
use crate::test_support::{make_ctx, make_test_pool};
use crate::{Tool, ToolContext, ToolOutput};
use db::DbPool;

const PROFILE: &str = "agent-1";

async fn pool() -> DbPool {
    let db = make_test_pool().await;
    sqlx::query(
        "INSERT INTO agent_profiles (id, name, provider, model, system_prompt)
         VALUES (?, 'An agent', 'mock', 'mock-model', 'You are a test agent.')",
    )
    .bind(PROFILE)
    .execute(&db)
    .await
    .expect("seed a profile");
    db
}

async fn seed_story(db: &DbPool, id: &str, status: &str) {
    sqlx::query("INSERT INTO stories (id, title, status) VALUES (?, 'Child work', ?)")
        .bind(id)
        .bind(status)
        .execute(db)
        .await
        .expect("seed a story");
}

async fn seed_run(db: &DbPool, id: &str, story_id: &str, status: &str, finished: bool) {
    sqlx::query(
        "INSERT INTO story_runs (id, story_id, agent_profile_id, status, iteration_count,
                                 started_at, finished_at)
         VALUES (?, ?, ?, ?, 3, '2026-08-31T00:00:00.000Z', ?)",
    )
    .bind(id)
    .bind(story_id)
    .bind(PROFILE)
    .bind(status)
    .bind(finished.then_some("2026-08-31T00:01:30.000Z"))
    .execute(db)
    .await
    .expect("seed a run");
}

async fn seed_error_event(db: &DbPool, run_id: &str, sequence: i64, content: &str) {
    sqlx::query(
        "INSERT INTO run_events (id, run_id, event_type, content, sequence_num)
         VALUES (?, ?, 'error', ?, ?)",
    )
    .bind(format!("{run_id}-e{sequence}"))
    .bind(run_id)
    .bind(content)
    .bind(sequence)
    .execute(db)
    .await
    .expect("seed an error event");
}

async fn get_run(ctx: &ToolContext, run_id: &str) -> serde_json::Value {
    let out = GetRunTool.execute(json!({ "run_id": run_id }), ctx).await;
    assert!(!out.is_error, "{}", out.content);
    serde_json::from_str(&out.content).expect("the reply is JSON")
}

// ---------------------------------------------------------------------------
// get_run
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_running_subtask_is_reported_as_not_terminal() {
    let db = pool().await;
    seed_story(&db, "s1", "in_progress").await;
    seed_run(&db, "run-1", "s1", "running", false).await;
    let ctx = make_ctx(db.clone());

    let reply = get_run(&ctx, "run-1").await;

    assert_eq!(reply["status"], json!("running"));
    assert_eq!(reply["is_terminal"], json!(false));
    assert_eq!(reply["finished_at"], serde_json::Value::Null);
    assert_eq!(reply["story_id"], json!("s1"));
}

#[tokio::test]
async fn each_way_a_run_can_stop_reads_as_terminal() {
    // The three an orchestrator has to stop waiting on. A child that failed or
    // was cancelled is exactly the case polling `get_story` never resolved.
    for status in ["done", "failed", "cancelled"] {
        let db = pool().await;
        seed_story(&db, "s1", "in_progress").await;
        seed_run(&db, "run-1", "s1", status, true).await;
        let ctx = make_ctx(db.clone());

        let reply = get_run(&ctx, "run-1").await;

        assert_eq!(reply["status"], json!(status));
        assert_eq!(reply["is_terminal"], json!(true), "{status} should be terminal");
        assert_eq!(reply["finished_at"], json!("2026-08-31T00:01:30.000Z"));
    }
}

#[tokio::test]
async fn a_failed_run_reports_why() {
    let db = pool().await;
    seed_story(&db, "s1", "in_progress").await;
    seed_run(&db, "run-1", "s1", "failed", true).await;
    seed_error_event(&db, "run-1", 1, "connection reset").await;
    seed_error_event(&db, "run-1", 2, "429 rate limited").await;
    let ctx = make_ctx(db.clone());

    let reply = get_run(&ctx, "run-1").await;

    // The newest, not the first: a run that retried and then stopped for a
    // different reason should report the one it actually stopped for.
    assert_eq!(reply["error"], json!("429 rate limited"));
}

#[tokio::test]
async fn a_run_that_succeeded_reports_no_error_even_with_one_on_its_timeline() {
    // A recovered tool failure is on the timeline of plenty of successful runs.
    // Reporting it as "the run's error" would tell an orchestrator its child
    // failed when it did not.
    let db = pool().await;
    seed_story(&db, "s1", "in_progress").await;
    seed_run(&db, "run-1", "s1", "done", true).await;
    seed_error_event(&db, "run-1", 1, "a tool call went wrong and was retried").await;
    let ctx = make_ctx(db.clone());

    let reply = get_run(&ctx, "run-1").await;

    assert_eq!(reply["error"], serde_json::Value::Null);
}

#[tokio::test]
async fn an_enormous_error_is_capped_rather_than_flooding_the_poller() {
    use crate::paging::MAX_FIELD_BYTES;

    let db = pool().await;
    seed_story(&db, "s1", "in_progress").await;
    seed_run(&db, "run-1", "s1", "failed", true).await;
    seed_error_event(&db, "run-1", 1, &"x".repeat(MAX_FIELD_BYTES * 3)).await;
    let ctx = make_ctx(db.clone());

    let reply = get_run(&ctx, "run-1").await;

    let error = reply["error"].as_str().expect("error");
    assert!(error.len() < MAX_FIELD_BYTES * 2, "the cap did not bite: {}", error.len());
    assert!(error.contains("[get_run FIELD TRUNCATED"), "got {error}");
}

#[tokio::test]
async fn the_card_comes_back_with_the_run() {
    // Both in one call: the run says whether the work stopped, the card says
    // what a human would see.
    let db = pool().await;
    seed_story(&db, "s1", "review").await;
    seed_run(&db, "run-1", "s1", "done", true).await;
    let ctx = make_ctx(db.clone());

    let reply = get_run(&ctx, "run-1").await;

    assert_eq!(reply["story_status"], json!("review"));
    assert_eq!(reply["story_title"], json!("Child work"));
}

#[tokio::test]
async fn an_unknown_run_is_an_error_naming_the_id() {
    let db = pool().await;
    let ctx = make_ctx(db.clone());

    let out = GetRunTool.execute(json!({ "run_id": "nope" }), &ctx).await;

    assert!(out.is_error);
    assert!(out.content.contains("nope"), "got {}", out.content);
}

#[tokio::test]
async fn get_run_without_a_run_id_says_so() {
    let db = pool().await;
    let ctx = make_ctx(db.clone());

    let out = GetRunTool.execute(json!({}), &ctx).await;

    assert!(out.is_error);
    assert!(out.content.contains("run_id"), "got {}", out.content);
}

// ---------------------------------------------------------------------------
// spawn_subtask's claims about itself
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spawn_subtask_points_at_a_tool_that_exists() {
    // The old description told the model to poll `get_story`, whose card only
    // moves if the child agent chooses to move it. A tool description is prompt
    // text the model reasons from, so a wrong one actively misleads.
    let described = SpawnSubtaskTool.description();

    assert!(described.contains("get_run"), "got {described}");
    assert!(
        !described.contains("poll get_story"),
        "the description still names the mechanism that does not work: {described}",
    );

    let mut registry = crate::ToolRegistry::new();
    crate::builtin::register_builtins(&mut registry, make_test_pool().await);
    let names: Vec<String> = registry.all_definitions().into_iter().map(|d| d.name).collect();
    assert!(
        names.contains(&"get_run".to_string()),
        "spawn_subtask names a tool the agent was never given: {names:?}",
    );
}

#[tokio::test]
async fn the_depth_guard_fires_at_the_depth_it_advertises() {
    // It never fired: the limit was 5 and a child can only ever reach 1,
    // because a spawned child is built with no spawning callback at all.
    let db = pool().await;
    seed_story(&db, "s1", "ready").await;
    let mut ctx = make_ctx(db.clone());
    ctx.pipeline_depth = 1;

    let out = SpawnSubtaskTool
        .execute(json!({ "story_id": "s1", "agent_id": PROFILE }), &ctx)
        .await;

    assert!(out.is_error, "a subtask must not be able to spawn subtasks");
    assert!(out.content.contains("depth limit (1)"), "got {}", out.content);
}

#[tokio::test]
async fn a_root_run_is_still_allowed_to_spawn() {
    // The guard has to bite one level down, not at the root — otherwise the fix
    // for a decorative limit would be to disable the feature.
    let db = pool().await;
    seed_story(&db, "s1", "ready").await;
    let ctx = make_ctx(db.clone());
    assert_eq!(ctx.pipeline_depth, 0);

    let out: ToolOutput = SpawnSubtaskTool
        .execute(json!({ "story_id": "s1", "agent_id": PROFILE }), &ctx)
        .await;

    // No spawn callback is injected in tests, so it stops at that — but it must
    // stop *there*, past the depth guard, not at the guard.
    assert!(out.is_error);
    assert!(
        out.content.contains("not available in this context"),
        "the depth guard refused the root run: {}",
        out.content,
    );
}
