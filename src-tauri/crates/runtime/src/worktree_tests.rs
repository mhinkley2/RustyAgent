//! Tests for per-run worktree isolation and diff fidelity.
//!
//! Every case builds a throwaway repository in a temp directory. Nothing here
//! ever touches the checkout the suite is running from — a test that shells out
//! to `git` in the wrong directory is exactly the failure mode this feature
//! exists to prevent.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use crate::git::get_diff_since;
use crate::worktree::{
    self, Isolation, BRANCH_PREFIX, STATUS_ISOLATED, STATUS_NOT_A_REPO, STATUS_UNAVAILABLE,
};

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// A disposable git repository plus a directory to hang worktrees off.
struct Fixture {
    _root: TempDir,
    repo: PathBuf,
    worktrees: PathBuf,
}

impl Fixture {
    /// An initialised repository with one commit containing `tracked.txt`.
    fn new() -> Self {
        let fx = Self::empty();
        fx.write("tracked.txt", "original\n");
        fx.write(".gitignore", "ignored/\n");
        fx.git(&["add", "-A"]);
        fx.commit("initial");
        fx
    }

    /// An initialised repository with no commits at all.
    fn empty() -> Self {
        let root = TempDir::new().expect("temp dir");
        let repo = root.path().join("repo");
        let worktrees = root.path().join("worktrees");
        std::fs::create_dir_all(&repo).expect("create repo dir");
        let fx = Self { _root: root, repo, worktrees };
        fx.git(&["-c", "init.defaultBranch=main", "init", "-q"]);
        // Pin line-ending handling in the fixture's own config. Without it the
        // suite passes on Linux and fails on a Windows machine whose global
        // `core.autocrlf` is `true`, which is the Git for Windows default.
        fx.git(&["config", "core.autocrlf", "false"]);
        fx.git(&["config", "core.eol", "lf"]);
        fx
    }

    /// A plain directory that is not a repository.
    fn not_a_repo() -> Self {
        let root = TempDir::new().expect("temp dir");
        let repo = root.path().join("plain");
        let worktrees = root.path().join("worktrees");
        std::fs::create_dir_all(&repo).expect("create plain dir");
        Self { _root: root, repo, worktrees }
    }

    fn git(&self, args: &[&str]) -> String {
        run_git(&self.repo, args)
    }

    fn commit(&self, message: &str) {
        run_git(
            &self.repo,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "-q",
                "--no-verify",
                "-m",
                message,
            ],
        );
    }

    fn write(&self, rel: &str, contents: &str) {
        write_file(&self.repo, rel, contents);
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.repo.join(rel)).expect("read file")
    }

    fn isolate(&self, run_id: &str) -> Isolation {
        worktree::create(&self.repo, &self.worktrees, run_id)
    }

    /// Isolate, asserting success, and hand back the worktree.
    fn worktree_for(&self, run_id: &str) -> worktree::RunWorktree {
        match self.isolate(run_id) {
            Isolation::Isolated(wt) => wt,
            other => panic!("expected isolation to succeed, got {other:?}"),
        }
    }

    /// Everything `git status --porcelain` reports in the user's checkout.
    fn status(&self) -> String {
        self.git(&["status", "--porcelain"])
    }
}

fn run_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} in {}: {e}", dir.display()));
    assert!(
        out.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_file(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(path, contents).expect("write file");
}

// ---------------------------------------------------------------------------
// Creating a worktree
// ---------------------------------------------------------------------------

#[test]
fn a_run_gets_its_own_checkout_on_its_own_branch() {
    let fx = Fixture::new();

    let wt = fx.worktree_for("run-a");

    assert!(wt.path.is_dir(), "worktree directory should exist");
    assert_eq!(wt.branch, format!("{BRANCH_PREFIX}run-a"));
    assert_eq!(wt.base_sha, fx.git(&["rev-parse", "HEAD"]));
    // The branch is checked out *there*, not in the user's tree.
    assert_eq!(run_git(&wt.path, &["rev-parse", "--abbrev-ref", "HEAD"]), wt.branch);
    assert_ne!(fx.git(&["rev-parse", "--abbrev-ref", "HEAD"]), wt.branch);
    // And it starts from the base commit's content.
    assert_eq!(
        std::fs::read_to_string(wt.path.join("tracked.txt")).unwrap(),
        "original\n"
    );
}

#[test]
fn files_an_agent_writes_never_reach_the_users_checkout() {
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");

    write_file(&wt.path, "new_file.rs", "fn main() {}\n");
    write_file(&wt.path, "tracked.txt", "rewritten by the agent\n");

    assert!(
        !fx.repo.join("new_file.rs").exists(),
        "the agent's new file leaked into the user's tree"
    );
    assert_eq!(fx.read("tracked.txt"), "original\n");
    assert_eq!(fx.status(), "", "the user's tree should be untouched");
}

#[test]
fn two_concurrent_runs_writing_the_same_file_do_not_clobber_each_other() {
    let fx = Fixture::new();
    let a = fx.worktree_for("run-a");
    let b = fx.worktree_for("run-b");

    write_file(&a.path, "tracked.txt", "written by A\n");
    write_file(&b.path, "tracked.txt", "written by B\n");

    assert_ne!(a.path, b.path);
    assert_ne!(a.branch, b.branch);
    assert_eq!(
        std::fs::read_to_string(a.path.join("tracked.txt")).unwrap(),
        "written by A\n"
    );
    assert_eq!(
        std::fs::read_to_string(b.path.join("tracked.txt")).unwrap(),
        "written by B\n"
    );
    assert_eq!(fx.read("tracked.txt"), "original\n");
}

#[test]
fn a_second_run_with_a_colliding_id_still_gets_a_worktree() {
    // `git worktree add` fails outright if the branch already exists, so the
    // run id is treated as a hint rather than a guarantee.
    let fx = Fixture::new();
    let first = fx.worktree_for("same-id");

    let second = fx.worktree_for("same-id");

    assert_ne!(first.branch, second.branch);
    assert_ne!(first.path, second.path);
    assert!(second.branch.starts_with(BRANCH_PREFIX));
    assert!(second.path.is_dir());
}

#[test]
fn a_worktree_starts_from_the_committed_state_not_the_dirty_one() {
    let fx = Fixture::new();
    fx.write("tracked.txt", "uncommitted edit\n");
    fx.write("untracked.txt", "not staged\n");

    let outcome = fx.isolate("run-a");
    let wt = outcome.worktree().expect("isolated").clone();

    assert!(wt.base_was_dirty);
    assert_eq!(
        std::fs::read_to_string(wt.path.join("tracked.txt")).unwrap(),
        "original\n"
    );
    assert!(!wt.path.join("untracked.txt").exists());
    // And the operator is told, because it changes how the run behaves.
    let note = outcome.note().expect("a dirty base deserves a note");
    assert!(note.contains("uncommitted changes"), "unhelpful note: {note}");
    // The user's own edits survive untouched.
    assert_eq!(fx.read("tracked.txt"), "uncommitted edit\n");
}

#[test]
fn a_clean_isolation_needs_no_warning() {
    let fx = Fixture::new();

    let outcome = fx.isolate("run-a");

    assert_eq!(outcome.status(), STATUS_ISOLATED);
    assert_eq!(outcome.note(), None);
}

#[test]
fn ignored_files_are_not_carried_into_the_worktree() {
    // Documented behaviour, not an accident: copying ignored files would hand
    // every unattended run whatever secrets live in a .env.
    let fx = Fixture::new();
    write_file(&fx.repo, "ignored/secret.env", "TOKEN=hunter2\n");

    let wt = fx.worktree_for("run-a");

    assert!(!wt.path.join("ignored/secret.env").exists());
}

// ---------------------------------------------------------------------------
// Degrading safely
// ---------------------------------------------------------------------------

#[test]
fn a_non_git_workspace_says_so_instead_of_isolating_silently() {
    let fx = Fixture::not_a_repo();

    let outcome = fx.isolate("run-a");

    assert_eq!(outcome.status(), STATUS_NOT_A_REPO);
    assert!(outcome.worktree().is_none());
    let note = outcome.note().expect("a refusal must explain itself");
    assert!(note.contains("not a git repository"), "unhelpful note: {note}");
    assert!(note.contains("cannot be reverted"), "unhelpful note: {note}");
}

#[test]
fn a_repository_with_no_commits_degrades_with_a_reason() {
    let fx = Fixture::empty();

    let outcome = fx.isolate("run-a");

    assert_eq!(outcome.status(), STATUS_UNAVAILABLE);
    assert!(outcome.worktree().is_none());
    assert!(outcome.note().unwrap().contains("no commits"));
}

#[test]
fn a_missing_workspace_directory_degrades_with_a_reason() {
    let fx = Fixture::new();
    let missing = fx.repo.join("does-not-exist");

    let outcome = worktree::create(&missing, &fx.worktrees, "run-a");

    assert_eq!(outcome.status(), STATUS_UNAVAILABLE);
    assert!(outcome.note().unwrap().contains("does not exist"));
}

// ---------------------------------------------------------------------------
// Committing the run's output
// ---------------------------------------------------------------------------

#[test]
fn a_finished_run_leaves_a_commit_on_its_own_branch() {
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    write_file(&wt.path, "created.txt", "by the agent\n");

    let sha = worktree::commit_all(&wt.path, "run output")
        .expect("commit should succeed")
        .expect("a changed worktree produces a commit");

    assert_eq!(run_git(&wt.path, &["rev-parse", "HEAD"]), sha);
    assert_ne!(sha, wt.base_sha);
    // The branch moved; the user's branch did not.
    assert_eq!(
        run_git(&fx.repo, &["rev-parse", &wt.branch]),
        sha,
        "the branch should point at the run's commit"
    );
    assert_eq!(fx.git(&["rev-parse", "HEAD"]), wt.base_sha);
}

#[test]
fn a_run_that_changed_nothing_produces_no_commit() {
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");

    let sha = worktree::commit_all(&wt.path, "run output").expect("commit call should succeed");

    assert_eq!(sha, None);
    assert_eq!(run_git(&wt.path, &["rev-parse", "HEAD"]), wt.base_sha);
}

#[test]
fn committing_works_without_a_configured_git_identity() {
    // `commit_all` supplies its own author, so a user who has never run
    // `git config user.email` still gets a rollback point.
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    run_git(&wt.path, &["config", "user.email", ""]);
    write_file(&wt.path, "created.txt", "by the agent\n");

    let sha = worktree::commit_all(&wt.path, "run output").expect("commit should succeed");

    assert!(sha.is_some());
}

// ---------------------------------------------------------------------------
// Diff fidelity
// ---------------------------------------------------------------------------

#[test]
fn a_run_whose_only_action_is_creating_a_file_produces_a_diff_containing_it() {
    // The bug this closes: `git diff <sha>` omits untracked files, so the most
    // common thing an agent does was missing from every recorded diff.
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    write_file(&wt.path, "src/brand_new.rs", "fn added_by_the_agent() {}\n");

    let diff = get_diff_since(&wt.path, &wt.base_sha).expect("a created file is a change");

    assert!(diff.contains("src/brand_new.rs"), "diff omits the new file:\n{diff}");
    assert!(
        diff.contains("added_by_the_agent"),
        "diff omits the new file's content:\n{diff}"
    );
}

#[test]
fn the_diff_also_records_modifications_and_deletions() {
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    write_file(&wt.path, "tracked.txt", "changed\n");
    std::fs::remove_file(wt.path.join(".gitignore")).expect("delete file");

    let diff = get_diff_since(&wt.path, &wt.base_sha).expect("changes were made");

    assert!(diff.contains("tracked.txt"));
    assert!(diff.contains("+changed"));
    assert!(diff.contains(".gitignore"));
}

#[test]
fn an_unchanged_tree_records_no_diff() {
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");

    assert_eq!(get_diff_since(&wt.path, &wt.base_sha), None);
}

#[test]
fn taking_a_diff_leaves_the_repositorys_own_index_alone() {
    // `git add -N .` would have been the obvious way to include untracked
    // files, and it would have left every one of them staged in the user's
    // index. Observing a workspace must not modify it.
    let fx = Fixture::new();
    fx.write("untracked_by_the_user.txt", "mine\n");
    let before = fx.status();
    let head = fx.git(&["rev-parse", "HEAD"]);

    let diff = get_diff_since(&fx.repo, &head).expect("the untracked file is a change");

    assert!(diff.contains("untracked_by_the_user.txt"));
    assert_eq!(fx.status(), before, "the user's index or tree was modified");
    assert!(before.contains("??"), "sanity: the file should still be untracked");
}

#[test]
fn a_non_git_directory_yields_no_diff_rather_than_an_error() {
    let fx = Fixture::not_a_repo();

    assert_eq!(get_diff_since(&fx.repo, "deadbeef"), None);
}

// ---------------------------------------------------------------------------
// Accept
// ---------------------------------------------------------------------------

#[test]
fn accepting_a_run_brings_its_changes_into_the_users_tree() {
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    write_file(&wt.path, "created.txt", "by the agent\n");
    write_file(&wt.path, "tracked.txt", "edited by the agent\n");
    worktree::commit_all(&wt.path, "run output").expect("commit");

    worktree::apply_to_main(&fx.repo, &wt.branch).expect("accept should succeed");

    assert_eq!(fx.read("created.txt"), "by the agent\n");
    assert_eq!(fx.read("tracked.txt"), "edited by the agent\n");
}

#[test]
fn accepting_leaves_the_changes_staged_for_the_user_to_review() {
    // `merge --squash` stops short of committing on purpose: the user decides
    // what the commit says, and can still back out with a plain `git restore`.
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    write_file(&wt.path, "created.txt", "by the agent\n");
    worktree::commit_all(&wt.path, "run output").expect("commit");

    worktree::apply_to_main(&fx.repo, &wt.branch).expect("accept");

    assert_eq!(fx.git(&["rev-parse", "HEAD"]), wt.base_sha, "no commit was made");
    assert!(fx.status().contains("created.txt"));
}

#[test]
fn accepting_refuses_a_branch_rustyagent_did_not_create() {
    let fx = Fixture::new();

    let err = worktree::apply_to_main(&fx.repo, "main").expect_err("must refuse");

    assert!(err.contains("Refusing to merge"), "unhelpful error: {err}");
}

#[test]
fn accepting_refuses_rather_than_overwriting_the_users_uncommitted_work() {
    // The property that matters: git aborts the merge instead of clobbering,
    // and the user's edit is still there afterwards.
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    write_file(&wt.path, "tracked.txt", "the agent's version\n");
    worktree::commit_all(&wt.path, "run output").expect("commit");
    fx.write("tracked.txt", "the user's unsaved work\n");

    let err = worktree::apply_to_main(&fx.repo, &wt.branch)
        .expect_err("merging over an uncommitted edit must fail");

    assert!(!err.is_empty());
    assert_eq!(fx.read("tracked.txt"), "the user's unsaved work\n");
}

// ---------------------------------------------------------------------------
// Revert and removal
// ---------------------------------------------------------------------------

#[test]
fn reverting_a_run_leaves_the_users_tree_byte_identical() {
    let fx = Fixture::new();
    // Whatever the user had before the run, including uncommitted work.
    fx.write("tracked.txt", "half-finished work\n");
    fx.write("scratch.txt", "notes\n");
    let before_status = fx.status();
    let before_head = fx.git(&["rev-parse", "HEAD"]);

    let wt = fx.worktree_for("run-a");
    write_file(&wt.path, "created.txt", "by the agent\n");
    write_file(&wt.path, "tracked.txt", "the agent's version\n");
    worktree::commit_all(&wt.path, "run output").expect("commit");

    worktree::remove(&fx.repo, &wt.path).expect("remove worktree");
    worktree::delete_branch(&fx.repo, &wt.branch).expect("delete branch");

    assert!(!wt.path.exists(), "the worktree should be gone");
    assert_eq!(fx.read("tracked.txt"), "half-finished work\n");
    assert_eq!(fx.read("scratch.txt"), "notes\n");
    assert!(!fx.repo.join("created.txt").exists());
    assert_eq!(fx.status(), before_status);
    assert_eq!(fx.git(&["rev-parse", "HEAD"]), before_head);
}

#[test]
fn removing_refuses_the_users_main_checkout() {
    // The one guard that stands between a cleanup path and someone's work.
    let fx = Fixture::new();

    let err = worktree::remove(&fx.repo, &fx.repo).expect_err("must refuse the main worktree");

    assert!(err.contains("Refusing to remove"), "unhelpful error: {err}");
    assert!(fx.repo.join("tracked.txt").exists());
}

#[test]
fn removing_refuses_a_directory_that_is_not_a_worktree_of_this_repository() {
    let fx = Fixture::new();
    let stranger = fx.worktrees.join("somebody-elses-directory");
    std::fs::create_dir_all(&stranger).expect("create dir");
    write_file(&stranger, "important.txt", "not ours\n");

    let err = worktree::remove(&fx.repo, &stranger).expect_err("must refuse");

    assert!(err.contains("Refusing to remove"), "unhelpful error: {err}");
    assert!(stranger.join("important.txt").exists());
}

#[test]
fn removing_an_already_deleted_worktree_succeeds_quietly() {
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    worktree::remove(&fx.repo, &wt.path).expect("first removal");

    worktree::remove(&fx.repo, &wt.path).expect("removing twice must not error");
}

#[test]
fn deleting_a_branch_refuses_anything_outside_rustyagents_namespace() {
    let fx = Fixture::new();
    let head = fx.git(&["rev-parse", "HEAD"]);

    let err = worktree::delete_branch(&fx.repo, "main").expect_err("must refuse");

    assert!(err.contains("Refusing to delete"), "unhelpful error: {err}");
    assert_eq!(fx.git(&["rev-parse", "main"]), head, "main must still exist");
}

// ---------------------------------------------------------------------------
// Startup sweep
// ---------------------------------------------------------------------------

#[test]
fn the_sweep_removes_worktrees_no_run_claims() {
    let fx = Fixture::new();
    let orphan = fx.worktree_for("orphaned-run");

    let swept = worktree::sweep_orphans(&fx.worktrees, &HashSet::new());

    assert_eq!(swept.len(), 1);
    assert_eq!(swept[0].error, None, "sweep reported: {:?}", swept[0].error);
    assert!(!orphan.path.exists());
}

#[test]
fn the_sweep_leaves_a_finished_runs_worktree_alone_until_it_is_decided() {
    // A run that finished but has not been accepted or reverted still claims
    // its worktree — deleting it at startup would throw away the very changes
    // the user has not looked at yet.
    let fx = Fixture::new();
    let kept = fx.worktree_for("undecided-run");
    let orphan = fx.worktree_for("orphaned-run");
    write_file(&kept.path, "created.txt", "by the agent\n");

    let claimed: HashSet<String> =
        [kept.path.to_string_lossy().to_string()].into_iter().collect();
    let swept = worktree::sweep_orphans(&fx.worktrees, &claimed);

    assert_eq!(swept.len(), 1);
    assert_eq!(swept[0].path, orphan.path);
    assert!(kept.path.join("created.txt").exists());
    assert!(!orphan.path.exists());
}

#[test]
fn the_sweep_never_reaches_outside_its_own_directory() {
    let fx = Fixture::new();
    let _wt = fx.worktree_for("run-a");

    worktree::sweep_orphans(&fx.worktrees, &HashSet::new());

    assert!(fx.repo.join("tracked.txt").exists(), "the user's tree was swept");
    assert_eq!(fx.status(), "");
}

#[test]
fn sweeping_a_directory_that_does_not_exist_is_a_no_op() {
    let fx = Fixture::new();

    assert!(worktree::sweep_orphans(&fx.worktrees.join("nope"), &HashSet::new()).is_empty());
}

#[test]
fn the_sweep_deletes_leftovers_whose_repository_is_gone() {
    // The crash case: a worktree directory survives but the repository that
    // knew about it does not. git can do nothing with it; it is still garbage.
    let fx = Fixture::new();
    let stray = fx.worktrees.join("stray");
    write_file(&stray, "leftover.txt", "junk\n");

    let swept = worktree::sweep_orphans(&fx.worktrees, &HashSet::new());

    assert_eq!(swept.len(), 1);
    assert_eq!(swept[0].error, None, "sweep reported: {:?}", swept[0].error);
    assert!(!stray.exists());
}

// ---------------------------------------------------------------------------
// Chaining worktrees for a sequential pipeline
// ---------------------------------------------------------------------------

#[test]
fn a_chained_worktree_starts_from_the_previous_steps_output() {
    // Isolating each step of a sequential pipeline must not break the handoff:
    // step two has to be able to read the file step one wrote.
    let fx = Fixture::new();
    let first = fx.worktree_for("step-1");
    write_file(&first.path, "from_step_one.txt", "handed over\n");
    let tip = worktree::commit_all(&first.path, "step one")
        .expect("commit")
        .expect("step one changed something");

    let second = match worktree::create_from(&fx.repo, &fx.worktrees, "step-2", Some(&tip)) {
        Isolation::Isolated(wt) => wt,
        other => panic!("expected isolation, got {other:?}"),
    };

    assert_eq!(second.base_sha, tip);
    assert_eq!(
        std::fs::read_to_string(second.path.join("from_step_one.txt")).unwrap(),
        "handed over\n"
    );
    // And still nothing has reached the user's tree.
    assert!(!fx.repo.join("from_step_one.txt").exists());
}

#[test]
fn a_chained_step_records_only_its_own_changes_in_its_diff() {
    let fx = Fixture::new();
    let first = fx.worktree_for("step-1");
    write_file(&first.path, "from_step_one.txt", "handed over\n");
    let tip = worktree::commit_all(&first.path, "step one").expect("commit").expect("sha");

    let second = match worktree::create_from(&fx.repo, &fx.worktrees, "step-2", Some(&tip)) {
        Isolation::Isolated(wt) => wt,
        other => panic!("expected isolation, got {other:?}"),
    };
    write_file(&second.path, "from_step_two.txt", "mine\n");
    let diff = get_diff_since(&second.path, &second.base_sha).expect("step two changed something");

    assert!(diff.contains("from_step_two.txt"));
    assert!(
        !diff.contains("from_step_one.txt"),
        "step two's diff should not re-report step one's work:\n{diff}"
    );
}

#[test]
fn accepting_the_last_step_brings_the_whole_chain_in() {
    let fx = Fixture::new();
    let first = fx.worktree_for("step-1");
    write_file(&first.path, "from_step_one.txt", "one\n");
    let tip = worktree::commit_all(&first.path, "step one").expect("commit").expect("sha");
    let second = match worktree::create_from(&fx.repo, &fx.worktrees, "step-2", Some(&tip)) {
        Isolation::Isolated(wt) => wt,
        other => panic!("expected isolation, got {other:?}"),
    };
    write_file(&second.path, "from_step_two.txt", "two\n");
    worktree::commit_all(&second.path, "step two").expect("commit");

    worktree::apply_to_main(&fx.repo, &second.branch).expect("accept the last step");

    assert_eq!(fx.read("from_step_one.txt"), "one\n");
    assert_eq!(fx.read("from_step_two.txt"), "two\n");
}

#[test]
fn an_unresolvable_base_is_refused_rather_than_silently_falling_back_to_head() {
    // Falling back to HEAD here would drop the previous step's work without
    // saying anything, which is the exact failure mode isolation exists to end.
    let fx = Fixture::new();

    let outcome = worktree::create_from(
        &fx.repo,
        &fx.worktrees,
        "step-2",
        Some("0000000000000000000000000000000000000000"),
    );

    assert_eq!(outcome.status(), STATUS_UNAVAILABLE);
    assert!(outcome.note().unwrap().contains("Could not resolve"));
}

#[test]
fn the_sweep_refuses_to_delete_a_repository_that_ended_up_under_its_directory() {
    // The last-resort `remove_dir_all` must not be reachable for anything that
    // is a repository in its own right, however it got there.
    let fx = Fixture::new();
    let stray = fx.worktrees.join("someones-repo");
    std::fs::create_dir_all(&stray).expect("mkdir");
    run_git(&stray, &["-c", "init.defaultBranch=main", "init", "-q"]);
    write_file(&stray, "important.txt", "not ours\n");

    let swept = worktree::sweep_orphans(&fx.worktrees, &HashSet::new());

    assert_eq!(swept.len(), 1);
    assert!(
        swept[0].error.as_deref().unwrap_or("").contains("Refusing to delete"),
        "unexpected sweep result: {:?}",
        swept[0]
    );
    assert!(stray.join("important.txt").exists());
}

#[test]
fn the_diff_leaves_ignored_files_out() {
    // Staging the whole tree into a scratch index must not turn a diff into a
    // dump of node_modules. `git add -A` honours .gitignore; this pins it.
    let fx = Fixture::new();
    let wt = fx.worktree_for("run-a");
    write_file(&wt.path, "ignored/build-output.bin", "junk\n");
    write_file(&wt.path, "kept.txt", "real work\n");

    let diff = get_diff_since(&wt.path, &wt.base_sha).expect("changes were made");

    assert!(diff.contains("kept.txt"));
    assert!(!diff.contains("build-output.bin"), "ignored files leaked into the diff:\n{diff}");
}
