//! Per-run isolation: every run gets its own git worktree on its own branch.
//!
//! # Why
//!
//! Agents write files. Without isolation they write them into the developer's
//! live checkout, several runs at once, with no way to undo. A linked git
//! worktree gives each run a private working directory that shares the
//! repository's object store but has its own index and its own HEAD, so two
//! runs cannot see — let alone clobber — each other's edits, and neither can
//! touch the user's tree.
//!
//! # Safety rules this module holds to
//!
//! Everything here is **additive** with respect to the user's repository:
//!
//! * Nothing ever runs `reset`, `clean`, `checkout`, or `stash` against the
//!   user's checkout. There is no code path, not even an error path, that can
//!   discard uncommitted work.
//! * The only removal is [`remove`], and it refuses any path git does not
//!   report as a *linked* worktree — the main working tree is rejected by an
//!   explicit check as well as by `git worktree remove` itself.
//! * The only branch deletion is [`delete_branch`], and it refuses any name
//!   that does not start with [`BRANCH_PREFIX`], so it can only ever delete a
//!   branch this module created.
//! * [`apply_to_main`] uses `git merge --squash`, which git aborts when it
//!   would overwrite local modifications. The failure is reported, not forced.
//!
//! # Ignored files
//!
//! A fresh worktree contains the tracked contents of the base commit and
//! nothing else: no `node_modules`, no `.env`, no build output. Ignored files
//! are deliberately **not** copied in — copying a `.env` would hand every
//! unattended run the user's secrets, and copying build artefacts would make
//! the disk cost of a run unbounded. A run that needs them has to create them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Every branch this module creates starts with this. [`delete_branch`] will
/// not touch anything else.
pub const BRANCH_PREFIX: &str = "rustyagent/run-";

/// Identity used for the bookkeeping commit. A user who has never configured
/// `user.email` would otherwise make `git commit` fail outright.
const COMMIT_NAME: &str = "RustyAgent";
const COMMIT_EMAIL: &str = "rustyagent@localhost";

/// Value stored in `story_runs.isolation_status` when a worktree was created.
pub const STATUS_ISOLATED: &str = "isolated";
/// Stored when the workspace is not a git repository.
pub const STATUS_NOT_A_REPO: &str = "not_a_git_repo";
/// Stored when it is a git repository but isolation could not be set up.
pub const STATUS_UNAVAILABLE: &str = "unavailable";
/// Stored when the run has no workspace root at all.
pub const STATUS_NO_WORKSPACE: &str = "no_workspace";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A worktree created for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunWorktree {
    /// Root of the isolated checkout. This becomes the run's workspace root.
    pub path: PathBuf,
    /// Branch the worktree has checked out, always [`BRANCH_PREFIX`]-prefixed.
    pub branch: String,
    /// Commit the worktree was branched from.
    pub base_sha: String,
    /// Whether the user's checkout had uncommitted changes when the run began.
    ///
    /// Those changes are *not* in the worktree — it is a clean checkout of
    /// `base_sha` — so a run can behave differently here than it would have in
    /// the user's tree. Worth telling the operator about; not worth refusing
    /// over.
    pub base_was_dirty: bool,
}

/// The outcome of trying to isolate a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    /// A worktree was created; the run executes there.
    Isolated(RunWorktree),
    /// The workspace is not a git repository, so there is nothing to isolate
    /// against. The run falls back to the user's directory — loudly.
    NotAGitRepo(String),
    /// It is a git repository, but a worktree could not be created.
    Unavailable(String),
}

impl Isolation {
    /// The value to store in `story_runs.isolation_status`.
    pub fn status(&self) -> &'static str {
        match self {
            Isolation::Isolated(_) => STATUS_ISOLATED,
            Isolation::NotAGitRepo(_) => STATUS_NOT_A_REPO,
            Isolation::Unavailable(_) => STATUS_UNAVAILABLE,
        }
    }

    /// Operator-facing explanation. `None` for a plain successful isolation
    /// with nothing surprising about it.
    pub fn note(&self) -> Option<String> {
        match self {
            Isolation::Isolated(wt) if wt.base_was_dirty => Some(format!(
                "Running in an isolated worktree on branch '{}'. Your checkout had \
                 uncommitted changes at run start; the worktree is a clean checkout of \
                 {} and does not contain them, so this run may behave differently than \
                 it would in your tree.",
                wt.branch,
                short(&wt.base_sha),
            )),
            Isolation::Isolated(_) => None,
            Isolation::NotAGitRepo(note) | Isolation::Unavailable(note) => Some(note.clone()),
        }
    }

    pub fn worktree(&self) -> Option<&RunWorktree> {
        match self {
            Isolation::Isolated(wt) => Some(wt),
            _ => None,
        }
    }
}

fn short(sha: &str) -> &str {
    if sha.len() > 12 { &sha[..12] } else { sha }
}

// ---------------------------------------------------------------------------
// git invocation
// ---------------------------------------------------------------------------

/// Run `git` in `dir`.
///
/// `core.longpaths=true` is passed per-invocation rather than written to the
/// user's config: worktree paths live under the app data directory, which on
/// Windows is already deep, and a repository with long paths of its own would
/// otherwise fail to check out. Setting it with `-c` changes nothing on disk.
fn git(dir: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git")
        .arg("-c")
        .arg("core.longpaths=true")
        .args(args)
        .current_dir(dir)
        .output()
}

fn stdout_trimmed(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn stderr_trimmed(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if text.is_empty() {
        "git reported no error output".to_string()
    } else {
        text
    }
}

/// Where per-run worktrees live: `<app data>/worktrees`.
///
/// `None` when the app data directory cannot be resolved, in which case a run
/// falls back to un-isolated execution and records that it did.
pub fn dir_for<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    use tauri::Manager;
    app.path().app_data_dir().ok().map(|d| d.join("worktrees"))
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// Whether `root` is inside a git working tree.
pub fn is_git_repo(root: &Path) -> bool {
    match git(root, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(out) => out.status.success() && stdout_trimmed(&out) == "true",
        Err(_) => false,
    }
}

/// Whether `root` has uncommitted changes (staged, unstaged, or untracked).
pub fn is_dirty(root: &Path) -> bool {
    match git(root, &["status", "--porcelain"]) {
        Ok(out) => out.status.success() && !stdout_trimmed(&out).is_empty(),
        Err(_) => false,
    }
}

/// `git rev-parse HEAD`, or `None` in a repository with no commits yet.
pub fn head_sha(root: &Path) -> Option<String> {
    let out = git(root, &["rev-parse", "HEAD"]).ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = stdout_trimmed(&out);
    if sha.is_empty() { None } else { Some(sha) }
}

/// Resolve a revision to a full commit SHA, or `None` if git does not know it.
fn resolve_commit(root: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    let out = git(root, &["rev-parse", "--verify", "--quiet", &spec]).ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = stdout_trimmed(&out);
    if sha.is_empty() { None } else { Some(sha) }
}

fn branch_exists(root: &Path, branch: &str) -> bool {
    let refname = format!("refs/heads/{branch}");
    match git(root, &["show-ref", "--verify", "--quiet", &refname]) {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

/// Absolute path of the main working tree that `dir` belongs to.
///
/// `git worktree list --porcelain` always lists the main worktree first, and it
/// is available in every git version this app could plausibly meet — unlike
/// `rev-parse --path-format=absolute`, which needs git 2.31.
pub fn main_worktree_root(dir: &Path) -> Option<PathBuf> {
    let out = git(dir, &["worktree", "list", "--porcelain"]).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().find_map(|l| l.strip_prefix("worktree "))?;
    Some(PathBuf::from(first.trim()))
}

/// Whether git reports `path` as a *linked* worktree of the repository that
/// `dir` belongs to — that is, a worktree that is not the main one.
///
/// This is the guard that stands between [`remove`] and a user's checkout.
pub fn is_linked_worktree(dir: &Path, path: &Path) -> bool {
    let out = match git(dir, &["worktree", "list", "--porcelain"]) {
        Ok(out) if out.status.success() => out,
        _ => return false,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut listed = text.lines().filter_map(|l| l.strip_prefix("worktree "));
    // The first entry is the main worktree; it must never match.
    let main = listed.next().map(|m| PathBuf::from(m.trim()));
    if main.as_deref().is_some_and(|m| same_path(m, path)) {
        return false;
    }
    listed.any(|w| same_path(Path::new(w.trim()), path))
}

/// Compare two paths for pointing at the same place.
///
/// `canonicalize` where possible — git reports forward slashes on Windows and
/// the caller holds a `PathBuf` built with backslashes, so a textual comparison
/// would say two spellings of one directory are different. Falls back to a
/// lexical comparison when the path no longer exists.
fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => normalize_lexical(a) == normalize_lexical(b),
    }
}

fn normalize_lexical(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

/// Create an isolated worktree for `run_id` under `worktrees_dir`, branched
/// from `HEAD`.
///
/// Never fails the caller: a workspace that cannot be isolated comes back as
/// [`Isolation::NotAGitRepo`] or [`Isolation::Unavailable`] carrying the reason,
/// which the run records and shows rather than silently proceeding.
pub fn create(workspace_root: &Path, worktrees_dir: &Path, run_id: &str) -> Isolation {
    create_from(workspace_root, worktrees_dir, run_id, None)
}

/// As [`create`], but branched from `base` instead of `HEAD`.
///
/// The steps of a sequential pipeline chain this way: each branches from the
/// commit the previous step left behind, so the handoff still works — step two
/// can read the file step one wrote — while every step keeps a worktree, a
/// branch, and a diff of its own. Accepting the last step's branch brings the
/// whole chain in, because it contains every earlier commit.
pub fn create_from(
    workspace_root: &Path,
    worktrees_dir: &Path,
    run_id: &str,
    base: Option<&str>,
) -> Isolation {
    if !workspace_root.is_dir() {
        return Isolation::Unavailable(format!(
            "Workspace directory '{}' does not exist, so this run could not be isolated.",
            workspace_root.display()
        ));
    }
    if !is_git_repo(workspace_root) {
        return Isolation::NotAGitRepo(format!(
            "'{}' is not a git repository, so this run could not be isolated. It writes \
             directly into that directory and its changes cannot be reverted from here.",
            workspace_root.display()
        ));
    }

    // A base handed in by the caller is a commit an earlier step of this
    // pipeline just made. It has to still be resolvable — falling back to HEAD
    // would silently drop that step's work out from under the next one.
    let base_sha = match base {
        Some(base) => match resolve_commit(workspace_root, base) {
            Some(sha) => sha,
            None => {
                return Isolation::Unavailable(format!(
                    "Could not resolve '{base}' to branch this run's worktree from."
                ))
            }
        },
        None => match head_sha(workspace_root) {
            Some(sha) => sha,
            None => {
                return Isolation::Unavailable(format!(
                    "'{}' is a git repository with no commits yet, so there is nothing to \
                     branch a worktree from. This run writes directly into that directory.",
                    workspace_root.display()
                ))
            }
        },
    };

    if let Err(e) = std::fs::create_dir_all(worktrees_dir) {
        return Isolation::Unavailable(format!(
            "Could not create the worktree directory '{}': {e}",
            worktrees_dir.display()
        ));
    }

    let base_was_dirty = is_dirty(workspace_root);

    // `git worktree add` fails outright if the branch or the directory already
    // exists, so neither is assumed to be free. A run id is a UUID, so a
    // collision means a leftover from a previous life rather than a real clash;
    // stepping to the next suffix is cheaper than deciding whether the leftover
    // is safe to delete.
    let (path, branch) = match free_slot(workspace_root, worktrees_dir, run_id) {
        Some(pair) => pair,
        None => {
            return Isolation::Unavailable(
                "Could not find a free worktree directory and branch name for this run."
                    .to_string(),
            )
        }
    };

    let path_arg = path.to_string_lossy().to_string();
    let out = match git(
        workspace_root,
        &["worktree", "add", "-b", &branch, &path_arg, &base_sha],
    ) {
        Ok(out) => out,
        Err(e) => {
            return Isolation::Unavailable(format!(
                "Could not run git to create a worktree for this run: {e}"
            ))
        }
    };

    if !out.status.success() {
        return Isolation::Unavailable(format!(
            "git could not create a worktree for this run: {}",
            stderr_trimmed(&out)
        ));
    }

    Isolation::Isolated(RunWorktree {
        path,
        branch,
        base_sha,
        base_was_dirty,
    })
}

/// First `(directory, branch)` pair where neither the directory nor the branch
/// is already taken.
fn free_slot(repo: &Path, worktrees_dir: &Path, run_id: &str) -> Option<(PathBuf, String)> {
    for attempt in 0..16u32 {
        let suffix = if attempt == 0 {
            run_id.to_string()
        } else {
            format!("{run_id}-{attempt}")
        };
        let path = worktrees_dir.join(&suffix);
        let branch = format!("{BRANCH_PREFIX}{suffix}");
        if !path.exists() && !branch_exists(repo, &branch) {
            return Some((path, branch));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Committing
// ---------------------------------------------------------------------------

/// Stage and commit everything in `worktree` on its own branch.
///
/// Returns the new commit's SHA, or `Ok(None)` when the run changed nothing.
///
/// Hooks are skipped (`--no-verify`). This commit is bookkeeping on a throwaway
/// branch inside a scratch checkout, made without anyone watching; running a
/// user's `pre-commit` unattended there is exactly the sort of surprise this
/// module exists to prevent.
pub fn commit_all(worktree: &Path, message: &str) -> Result<Option<String>, String> {
    let add = git(worktree, &["add", "-A", "--", "."])
        .map_err(|e| format!("Could not run git add: {e}"))?;
    if !add.status.success() {
        return Err(format!("git add failed: {}", stderr_trimmed(&add)));
    }

    let staged = git(worktree, &["diff", "--cached", "--quiet"])
        .map_err(|e| format!("Could not run git diff: {e}"))?;
    // Exit 0 means "no differences staged" — the run wrote nothing.
    if staged.status.success() {
        return Ok(None);
    }

    let out = git(
        worktree,
        &[
            "-c",
            &format!("user.name={COMMIT_NAME}"),
            "-c",
            &format!("user.email={COMMIT_EMAIL}"),
            "commit",
            "--no-verify",
            "-m",
            message,
        ],
    )
    .map_err(|e| format!("Could not run git commit: {e}"))?;

    if !out.status.success() {
        return Err(format!("git commit failed: {}", stderr_trimmed(&out)));
    }

    Ok(head_sha(worktree))
}

// ---------------------------------------------------------------------------
// Accept / revert
// ---------------------------------------------------------------------------

/// Bring `branch` into the working tree at `main_root`, staged but uncommitted.
///
/// `merge --squash` leaves the result in the index and working tree for the
/// user to inspect and commit themselves, and — the reason it is used here —
/// git refuses it outright when it would overwrite local modifications. A dirty
/// checkout gets an error, never a silent overwrite.
pub fn apply_to_main(main_root: &Path, branch: &str) -> Result<String, String> {
    if !branch.starts_with(BRANCH_PREFIX) {
        return Err(format!(
            "Refusing to merge '{branch}': not a branch RustyAgent created."
        ));
    }
    let out = git(main_root, &["merge", "--squash", "--", branch])
        .map_err(|e| format!("Could not run git merge: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "Could not apply this run to your working tree: {}\n{}",
            stderr_trimmed(&out),
            stdout_trimmed(&out)
        )
        .trim()
        .to_string());
    }
    Ok(stdout_trimmed(&out))
}

/// Remove a linked worktree.
///
/// Refuses anything git does not list as a *linked* worktree of the repository
/// `dir` belongs to. The user's main checkout can never reach the `git worktree
/// remove` call below, and that call refuses the main worktree in any case.
pub fn remove(dir: &Path, worktree_path: &Path) -> Result<(), String> {
    if !worktree_path.exists() {
        // Already gone. Drop the stale administrative record and call it done.
        let _ = git(dir, &["worktree", "prune"]);
        return Ok(());
    }
    if !is_linked_worktree(dir, worktree_path) {
        return Err(format!(
            "Refusing to remove '{}': git does not report it as a linked worktree of this \
             repository.",
            worktree_path.display()
        ));
    }
    let path_arg = worktree_path.to_string_lossy().to_string();
    // `--force` here means "remove even though the worktree has modifications".
    // Those modifications are the run's own output, which has already been
    // committed on the run's branch by `commit_all`.
    let out = git(dir, &["worktree", "remove", "--force", &path_arg])
        .map_err(|e| format!("Could not run git worktree remove: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git could not remove the worktree: {}",
            stderr_trimmed(&out)
        ));
    }
    Ok(())
}

/// Delete a branch this module created.
///
/// Refuses any name outside the [`BRANCH_PREFIX`] namespace, so a corrupt or
/// hand-edited `story_runs.branch_name` cannot be turned into a way to delete
/// the user's `main`.
pub fn delete_branch(dir: &Path, branch: &str) -> Result<(), String> {
    if !branch.starts_with(BRANCH_PREFIX) {
        return Err(format!(
            "Refusing to delete branch '{branch}': not a branch RustyAgent created."
        ));
    }
    let out = git(dir, &["branch", "-D", "--", branch])
        .map_err(|e| format!("Could not run git branch -D: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git could not delete branch '{branch}': {}",
            stderr_trimmed(&out)
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Startup sweep
// ---------------------------------------------------------------------------

/// Whether `path` is the root of a repository in its own right.
///
/// A linked worktree's `.git` is a *file* pointing back at the repository it
/// belongs to; a main working tree's is a *directory*. The sweep uses the
/// distinction to make sure its last-resort `remove_dir_all` can never land on
/// somebody's actual repository, however it ended up under the worktrees
/// directory.
fn is_main_worktree_dir(path: &Path) -> bool {
    path.join(".git").is_dir()
}

/// One worktree directory found during a sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweptWorktree {
    pub path: PathBuf,
    /// `None` when it was removed cleanly.
    pub error: Option<String>,
}

/// Delete worktree directories under `worktrees_dir` that no run claims.
///
/// `claimed` holds the `worktree_path` of every run still recorded in the
/// database, in the spelling stored there. A directory nobody claims is
/// garbage: its run row was deleted, or the process died between creating the
/// directory and writing the row. A run that finished but has not been accepted
/// or reverted still claims its worktree and is left completely alone — the
/// whole point of keeping it is that the user has not decided yet.
///
/// Only direct children of `worktrees_dir` are ever considered, and each is
/// handed to [`remove`], which requires git to confirm it is a linked worktree.
pub fn sweep_orphans(worktrees_dir: &Path, claimed: &HashSet<String>) -> Vec<SweptWorktree> {
    let claimed: HashSet<String> = claimed.iter().map(|c| normalize_lexical(Path::new(c))).collect();

    let entries = match std::fs::read_dir(worktrees_dir) {
        Ok(entries) => entries,
        // No worktrees directory yet is the normal state on a fresh install.
        Err(_) => return Vec::new(),
    };

    let mut swept = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || claimed.contains(&normalize_lexical(&path)) {
            continue;
        }
        // Run git from the repository the orphan belongs to, not from inside
        // the orphan: on Windows a process cannot delete its own directory.
        let dir = main_worktree_root(&path).unwrap_or_else(|| path.clone());
        let error = match remove(&dir, &path) {
            Ok(()) => None,
            // git declined — most often because the repository the leftover
            // belonged to is gone, so it is not a worktree of anything any
            // more. It is still a direct child of our own worktrees directory
            // and still unclaimed, so it is still garbage.
            // A repository of its own, not a leftover worktree. Somebody put it
            // here deliberately, or a path went badly wrong; either way it is
            // not this function's to delete.
            Err(e) if is_main_worktree_dir(&path) => Some(format!(
                "Refusing to delete '{}': it looks like a repository of its own, not a \
                 leftover worktree ({e})",
                path.display()
            )),
            Err(_) if path.exists() => std::fs::remove_dir_all(&path)
                .err()
                .map(|e| format!("Could not delete '{}': {e}", path.display())),
            Err(_) => None,
        };
        swept.push(SweptWorktree { path, error });
    }
    swept
}
