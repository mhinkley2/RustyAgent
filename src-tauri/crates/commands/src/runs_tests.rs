//! Tests for accepting and reverting an isolated run.
//!
//! These are the two operations that touch a user's real repository, so each
//! case builds a throwaway one in a temp directory and checks not only that the
//! happy path works but that the refusals refuse — a revert must never be able
//! to reach past the run's own worktree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use db::testing::{make_test_pool, seed_profile, seed_run, seed_story, seed_workspace};
use db::DbPool;
use sqlx::Row;
use tempfile::TempDir;

use crate::runs::{accept_run, revert_run, sweep_orphaned_worktrees};

const STORY_ID: &str = "story-1";
const PROFILE_ID: &str = "agent-1";

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    _tmp: TempDir,
    db: DbPool,
    repo: PathBuf,
    worktrees: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        let tmp = TempDir::new().expect("temp dir");
        let repo = tmp.path().join("repo");
        let worktrees = tmp.path().join("worktrees");
        std::fs::create_dir_all(&repo).expect("mkdir");

        git(&repo, &["-c", "init.defaultBranch=main", "init", "-q"]);
        // Pinned so a Windows machine with core.autocrlf on behaves like CI.
        git(&repo, &["config", "core.autocrlf", "false"]);
        git(&repo, &["config", "core.eol", "lf"]);
        std::fs::write(repo.join("tracked.txt"), "original\n").expect("write");
        git(&repo, &["add", "-A"]);
        git(
            &repo,
            &[
                "-c", "user.name=Fixture",
                "-c", "user.email=fixture@localhost",
                "commit", "-q", "--no-verify", "-m", "initial",
            ],
        );

        let db = make_test_pool().await;
        seed_profile(&db, PROFILE_ID, "Test Agent").await;
        seed_story(&db, STORY_ID, "Test Story", "ready").await;

        Self { _tmp: tmp, db, repo, worktrees }
    }

    /// Create a run row that has been isolated, run, and committed — the state
    /// accept and revert actually meet.
    async fn finished_isolated_run(&self, run_id: &str, writes: &[(&str, &str)]) -> String {
        let outcome = runtime::worktree::create(&self.repo, &self.worktrees, run_id);
        let wt = outcome.worktree().expect("isolation should succeed").clone();

        for (rel, contents) in writes {
            let path = wt.path.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(path, contents).expect("write");
        }
        runtime::worktree::commit_all(&wt.path, "run output").expect("commit");

        seed_run(&self.db, run_id, STORY_ID, PROFILE_ID).await;
        sqlx::query(
            "UPDATE story_runs SET status = 'done', worktree_path = ?, branch_name = ?, \
             isolation_status = 'isolated' WHERE id = ?",
        )
        .bind(wt.path.to_string_lossy().to_string())
        .bind(&wt.branch)
        .bind(run_id)
        .execute(&self.db)
        .await
        .expect("update run row");

        wt.branch
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.repo.join(rel)).expect("read")
    }

    fn status(&self) -> String {
        git(&self.repo, &["status", "--porcelain"])
    }

    fn branch_exists(&self, branch: &str) -> bool {
        Command::new("git")
            .args(["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")])
            .current_dir(&self.repo)
            .output()
            .expect("git")
            .status
            .success()
    }

    async fn isolation_status(&self, run_id: &str) -> Option<String> {
        sqlx::query("SELECT isolation_status FROM story_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&self.db)
            .await
            .expect("fetch run")
            .try_get("isolation_status")
            .ok()
            .flatten()
    }

    async fn worktree_path(&self, run_id: &str) -> Option<String> {
        sqlx::query("SELECT worktree_path FROM story_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&self.db)
            .await
            .expect("fetch run")
            .try_get("worktree_path")
            .ok()
            .flatten()
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// ---------------------------------------------------------------------------
// Accept
// ---------------------------------------------------------------------------

#[tokio::test]
async fn accepting_a_run_brings_its_files_into_the_users_tree() {
    let fx = Fixture::new().await;
    let branch = fx
        .finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;

    accept_run("run-1".into(), &fx.db).await.expect("accept");

    assert_eq!(fx.read("created.txt"), "by the agent\n");
    assert_eq!(fx.isolation_status("run-1").await.as_deref(), Some("accepted"));
    assert!(!fx.branch_exists(&branch), "the run's branch should be cleaned up");
}

#[tokio::test]
async fn accepting_removes_the_runs_worktree() {
    let fx = Fixture::new().await;
    fx.finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;
    let path = PathBuf::from(fx.worktree_path("run-1").await.expect("path"));

    accept_run("run-1".into(), &fx.db).await.expect("accept");

    assert!(!path.exists(), "the worktree should be gone after accepting");
}

#[tokio::test]
async fn a_failed_accept_leaves_the_run_intact_to_retry_or_revert() {
    // git refuses to merge over an uncommitted local edit. The run's worktree
    // and branch must survive that refusal, or the user loses both their own
    // work and the agent's.
    let fx = Fixture::new().await;
    let branch = fx
        .finished_isolated_run("run-1", &[("tracked.txt", "the agent's version\n")])
        .await;
    std::fs::write(fx.repo.join("tracked.txt"), "the user's unsaved work\n").expect("write");
    let path = PathBuf::from(fx.worktree_path("run-1").await.expect("path"));

    let err = accept_run("run-1".into(), &fx.db).await.expect_err("must fail");

    assert!(!err.is_empty());
    assert_eq!(fx.read("tracked.txt"), "the user's unsaved work\n");
    assert!(path.exists(), "the worktree must survive a failed accept");
    assert!(fx.branch_exists(&branch), "the branch must survive a failed accept");
    assert_eq!(fx.isolation_status("run-1").await.as_deref(), Some("isolated"));
}

#[tokio::test]
async fn accepting_the_same_run_twice_is_refused_with_an_explanation() {
    let fx = Fixture::new().await;
    fx.finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;
    accept_run("run-1".into(), &fx.db).await.expect("accept");

    let err = accept_run("run-1".into(), &fx.db).await.expect_err("must refuse");

    assert!(err.contains("already been accepted"), "unhelpful error: {err}");
}

// ---------------------------------------------------------------------------
// Revert
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reverting_leaves_the_users_tree_byte_identical_to_its_pre_run_state() {
    let fx = Fixture::new().await;
    // Including work the user had not committed when the run started.
    std::fs::write(fx.repo.join("tracked.txt"), "half-finished\n").expect("write");
    std::fs::write(fx.repo.join("scratch.txt"), "notes\n").expect("write");
    let before_status = fx.status();
    let before_head = git(&fx.repo, &["rev-parse", "HEAD"]);

    let branch = fx
        .finished_isolated_run(
            "run-1",
            &[("created.txt", "by the agent\n"), ("tracked.txt", "clobbered\n")],
        )
        .await;

    revert_run("run-1".into(), &fx.db).await.expect("revert");

    assert_eq!(fx.read("tracked.txt"), "half-finished\n");
    assert_eq!(fx.read("scratch.txt"), "notes\n");
    assert!(!fx.repo.join("created.txt").exists());
    assert_eq!(fx.status(), before_status);
    assert_eq!(git(&fx.repo, &["rev-parse", "HEAD"]), before_head);
    assert!(!fx.branch_exists(&branch));
    assert_eq!(fx.isolation_status("run-1").await.as_deref(), Some("reverted"));
}

#[tokio::test]
async fn reverting_removes_the_worktree_directory() {
    let fx = Fixture::new().await;
    fx.finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;
    let path = PathBuf::from(fx.worktree_path("run-1").await.expect("path"));

    revert_run("run-1".into(), &fx.db).await.expect("revert");

    assert!(!path.exists());
}

#[tokio::test]
async fn reverting_a_run_that_was_never_isolated_is_refused_rather_than_guessed_at() {
    // There is no safe automatic undo for changes already sitting in the user's
    // tree, and a `git checkout` or `reset` here would destroy their work.
    let fx = Fixture::new().await;
    seed_run(&fx.db, "run-1", STORY_ID, PROFILE_ID).await;
    sqlx::query(
        "UPDATE story_runs SET status = 'done', isolation_status = 'not_a_git_repo' WHERE id = ?",
    )
    .bind("run-1")
    .execute(&fx.db)
    .await
    .expect("update");

    let err = revert_run("run-1".into(), &fx.db).await.expect_err("must refuse");

    assert!(err.contains("was not isolated"), "unhelpful error: {err}");
    assert!(err.contains("yours to do with git"), "unhelpful error: {err}");
}

#[tokio::test]
async fn a_run_from_before_isolation_existed_is_refused_too() {
    let fx = Fixture::new().await;
    seed_run(&fx.db, "run-1", STORY_ID, PROFILE_ID).await;
    sqlx::query("UPDATE story_runs SET status = 'done' WHERE id = ?")
        .bind("run-1")
        .execute(&fx.db)
        .await
        .expect("update");

    let err = revert_run("run-1".into(), &fx.db).await.expect_err("must refuse");

    assert!(err.contains("was not isolated"), "unhelpful error: {err}");
}

#[tokio::test]
async fn a_run_still_in_flight_can_be_neither_accepted_nor_reverted() {
    // Its changes are not committed yet, so both operations would act on a
    // half-written worktree.
    let fx = Fixture::new().await;
    fx.finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;
    sqlx::query("UPDATE story_runs SET status = 'running' WHERE id = ?")
        .bind("run-1")
        .execute(&fx.db)
        .await
        .expect("update");

    let accept_err = accept_run("run-1".into(), &fx.db).await.expect_err("must refuse");
    let revert_err = revert_run("run-1".into(), &fx.db).await.expect_err("must refuse");

    assert!(accept_err.contains("still going"), "unhelpful error: {accept_err}");
    assert!(revert_err.contains("still going"), "unhelpful error: {revert_err}");
    assert_eq!(fx.isolation_status("run-1").await.as_deref(), Some("isolated"));
}

#[tokio::test]
async fn an_unknown_run_is_reported_as_missing() {
    let fx = Fixture::new().await;

    let err = revert_run("nope".into(), &fx.db).await.expect_err("must fail");

    assert!(err.contains("not found"), "unhelpful error: {err}");
}

// ---------------------------------------------------------------------------
// Startup sweep
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_sweep_keeps_worktrees_of_runs_still_awaiting_a_decision() {
    let fx = Fixture::new().await;
    fx.finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;
    let kept = PathBuf::from(fx.worktree_path("run-1").await.expect("path"));
    // A directory no run row claims: the crash case.
    let orphan = fx.worktrees.join("left-behind");
    std::fs::create_dir_all(&orphan).expect("mkdir");

    let removed = sweep_orphaned_worktrees(&fx.worktrees, &fx.db)
        .await
        .expect("sweep");

    assert_eq!(removed, 1);
    assert!(kept.exists(), "an undecided run's worktree was swept away");
    assert!(!orphan.exists());
}

#[tokio::test]
async fn a_reverted_runs_directory_no_longer_claims_protection() {
    let fx = Fixture::new().await;
    fx.finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;
    let path = PathBuf::from(fx.worktree_path("run-1").await.expect("path"));
    revert_run("run-1".into(), &fx.db).await.expect("revert");
    // Simulate cleanup having half-failed: the directory is back, unclaimed.
    std::fs::create_dir_all(&path).expect("mkdir");

    sweep_orphaned_worktrees(&fx.worktrees, &fx.db).await.expect("sweep");

    assert!(!path.exists());
    // The row keeps the path for the record even though the directory is gone.
    assert!(fx.worktree_path("run-1").await.is_some());
}

#[tokio::test]
async fn the_sweep_never_touches_the_users_repository() {
    let fx = Fixture::new().await;
    fx.finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;

    sweep_orphaned_worktrees(&fx.worktrees, &fx.db).await.expect("sweep");

    assert_eq!(fx.read("tracked.txt"), "original\n");
    assert_eq!(fx.status(), "");
}

#[tokio::test]
async fn sweeping_before_any_run_has_been_isolated_is_a_no_op() {
    let fx = Fixture::new().await;

    assert_eq!(
        sweep_orphaned_worktrees(&fx.worktrees, &fx.db).await.expect("sweep"),
        0
    );
    assert!(runtime::worktree::sweep_orphans(&fx.worktrees, &HashSet::new()).is_empty());
}

/// Accept must refuse rather than guess when the owning repository is unknown.
///
/// `main_repo_for` derives the repo from the run's worktree, then from the
/// story's workspace. It used to fall back to whatever workspace was *active*,
/// which is simply wherever the user happens to be pointed — not necessarily
/// the repository the run was created from. Since accept runs
/// `git merge --squash` in whatever path comes back, guessing pointed a git
/// write at a repository nobody named.
#[tokio::test]
async fn accept_refuses_when_the_owning_repository_cannot_be_determined() {
    let fx = Fixture::new().await;
    let branch = fx
        .finished_isolated_run("run-1", &[("created.txt", "by the agent\n")])
        .await;

    // A different repository, registered and most-recently-opened — this is
    // what `get_active_workspace_path` would have handed back.
    let elsewhere = fx._tmp.path().join("unrelated-repo");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    git(&elsewhere, &["-c", "init.defaultBranch=main", "init", "-q"]);
    seed_workspace(
        &fx.db,
        "ws-unrelated",
        &elsewhere.to_string_lossy(),
    )
    .await;

    // Remove the worktree directory so the first resolution path fails and the
    // fallback is the only thing left.
    let worktree = PathBuf::from(fx.worktree_path("run-1").await.expect("path"));
    std::fs::remove_dir_all(&worktree).expect("remove worktree");

    let err = accept_run("run-1".into(), &fx.db).await.expect_err("must refuse");

    assert!(
        err.contains("which repository this run belongs to"),
        "unhelpful error: {err}"
    );
    // The unrelated repository was not touched, and the run stays decidable.
    assert!(!elsewhere.join("created.txt").exists());
    assert_eq!(fx.isolation_status("run-1").await.as_deref(), Some("isolated"));
    assert!(fx.branch_exists(&branch), "the branch must survive a refusal");
}


// ---------------------------------------------------------------------------
// The filter wire format
// ---------------------------------------------------------------------------

/// `RunFilters` carries no `rename_all`, so the keys on the wire are
/// snake_case. This is pinned because getting it wrong is *silent*: serde
/// ignores unrecognised fields, so a camelCase key does not error — it leaves
/// the filter unset and every run comes back.
///
/// The TypeScript `RunFilters` promised camelCase for a long time without
/// anyone noticing, because every caller passed `null` until a story panel
/// needed one story's runs.
#[test]
fn a_filter_arrives_under_its_snake_case_key() {
    let filters: crate::runs::RunFilters =
        serde_json::from_value(serde_json::json!({ "story_id": "s1" })).expect("parse");

    assert_eq!(filters.story_id.as_deref(), Some("s1"));
}

#[test]
fn a_camel_case_key_is_dropped_rather_than_refused() {
    let filters: crate::runs::RunFilters =
        serde_json::from_value(serde_json::json!({ "storyId": "s1" })).expect("parse");

    assert_eq!(
        filters.story_id, None,
        "this is the failure mode: no error, no filter, and every run returned"
    );
}

#[test]
fn every_filter_field_uses_the_same_convention() {
    let filters: crate::runs::RunFilters = serde_json::from_value(serde_json::json!({
        "story_id": "s1",
        "agent_profile_id": "a1",
        "status": "done",
    }))
    .expect("parse");

    assert_eq!(filters.story_id.as_deref(), Some("s1"));
    assert_eq!(filters.agent_profile_id.as_deref(), Some("a1"));
    assert_eq!(filters.status.as_deref(), Some("done"));
}

// ---------------------------------------------------------------------------
// Durations
// ---------------------------------------------------------------------------
//
// A duration is only ever as good as the format the timestamps were stored in.
// `story_runs` has held two — the schema's RFC 3339 defaults and the
// `CURRENT_TIMESTAMP` the writers used to emit — and a run stored the second
// way parsed as nothing, so the column rendered `—` for every finished run.
// The writers now agree with the schema and a migration normalises what was
// already written; these pin both halves.

/// Seed a finished run with timestamps spelled exactly as given.
async fn seed_finished_run(db: &DbPool, id: &str, started: &str, finished: &str) {
    sqlx::query(
        "INSERT INTO story_runs (id, story_id, agent_profile_id, status, started_at, finished_at)
         VALUES (?, ?, ?, 'done', ?, ?)",
    )
    .bind(id)
    .bind(STORY_ID)
    .bind(PROFILE_ID)
    .bind(started)
    .bind(finished)
    .execute(db)
    .await
    .expect("seed a finished run");
}

#[tokio::test]
async fn a_finished_run_reports_a_duration() {
    let f = Fixture::new().await;
    seed_finished_run(
        &f.db,
        "run-duration",
        "2026-05-22T20:58:30.000Z",
        "2026-05-22T20:59:12.000Z",
    )
    .await;

    let runs = crate::runs::get_runs(None, None, &f.db).await.expect("list runs");
    let run = runs.iter().find(|r| r.id == "run-duration").expect("the seeded run");

    assert_eq!(run.duration_secs, Some(42.0));
}

/// The writers wrote this shape for the whole life of the app before the fix.
/// The read boundary normalises it and the migration rewrites it in place, so
/// history keeps its durations rather than going permanently blank.
#[tokio::test]
async fn a_run_stored_the_old_way_still_reports_a_duration() {
    let f = Fixture::new().await;
    seed_finished_run(
        &f.db,
        "run-legacy",
        "2026-05-22 20:58:30",
        "2026-05-22 20:59:12",
    )
    .await;

    let runs = crate::runs::get_runs(None, None, &f.db).await.expect("list runs");
    let run = runs.iter().find(|r| r.id == "run-legacy").expect("the seeded run");

    assert_eq!(run.duration_secs, Some(42.0));
}

/// The guard the database cannot provide. What the writers actually emit has
/// to parse with the function that reads it back, and a `CURRENT_TIMESTAMP`
/// creeping back into a writer fails here rather than in a blank column.
#[tokio::test]
async fn what_the_writers_emit_parses_where_the_reader_parses_it() {
    let f = Fixture::new().await;
    sqlx::query(&format!(
        "INSERT INTO story_runs (id, story_id, agent_profile_id, status, started_at, finished_at)
         VALUES ('run-written', ?, ?, 'done', {now}, {now})",
        now = db::timestamps::NOW_ISO8601
    ))
    .bind(STORY_ID)
    .bind(PROFILE_ID)
    .execute(&f.db)
    .await
    .expect("write timestamps the way the writers do");

    let stored: (String, String) =
        sqlx::query_as("SELECT started_at, finished_at FROM story_runs WHERE id = 'run-written'")
            .fetch_one(&f.db)
            .await
            .expect("read them back");

    assert!(
        db::timestamps::parses_as_rfc3339(&stored.0),
        "started_at was stored as {:?}",
        stored.0
    );
    assert!(
        db::timestamps::parses_as_rfc3339(&stored.1),
        "finished_at was stored as {:?}",
        stored.1
    );
}

/// Runs sort by `started_at`, which is a lexicographic sort over text. Mixed
/// formats sort inconsistently against each other because `T` and a space have
/// different byte values; one format means the order is the real one.
#[tokio::test]
async fn runs_sort_by_start_time_across_the_format_boundary() {
    let f = Fixture::new().await;
    seed_finished_run(&f.db, "older", "2026-05-22T20:58:30.000Z", "2026-05-22T20:59:12.000Z").await;
    seed_finished_run(&f.db, "newer", "2026-05-23T09:00:00.000Z", "2026-05-23T09:01:00.000Z").await;

    let runs = crate::runs::get_runs(None, None, &f.db).await.expect("list runs");
    let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();

    let newer = ids.iter().position(|id| *id == "newer").expect("newer run");
    let older = ids.iter().position(|id| *id == "older").expect("older run");
    assert!(newer < older, "expected newest first, got {ids:?}");
}
