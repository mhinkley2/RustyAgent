// Filesystem commands — list directories and read/write files.
// All paths are validated to be within the active workspace root to prevent
// path traversal attacks.

use serde::{Deserialize, Serialize};
use db::DbPool;

use crate::workspace::get_active_workspace_path;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Display name (last path component).
    pub name: String,
    /// Absolute path to the entry.
    pub path: String,
    /// True if this entry is a directory.
    pub is_dir: bool,
    /// Size in bytes (0 for directories).
    pub size: u64,
}

// ---------------------------------------------------------------------------
// Path security helpers
// ---------------------------------------------------------------------------

/// Canonicalize `path` and verify it is within `workspace_root`.
/// Returns the canonical PathBuf, or an error if the path escapes the workspace.
fn safe_path(
    path: &str,
    workspace_root: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let candidate = std::path::PathBuf::from(path);
    // canonicalize resolves symlinks and ".." components
    let canonical = std::fs::canonicalize(&candidate)
        .map_err(|e| format!("Invalid path '{path}': {e}"))?;

    let workspace_canonical = std::fs::canonicalize(workspace_root)
        .map_err(|e| format!("Cannot resolve workspace root: {e}"))?;

    if !canonical.starts_with(&workspace_canonical) {
        return Err(format!(
            "Access denied: path '{}' is outside the workspace",
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// Validate that the *parent* of a not-yet-existing path is within the workspace.
/// Used for create/rename operations where the target doesn't exist yet.
fn safe_path_for_new(
    path: &str,
    workspace_root: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let candidate = std::path::PathBuf::from(path);
    let parent = candidate
        .parent()
        .ok_or("Path has no parent directory")?;
    let parent_canonical = std::fs::canonicalize(parent)
        .map_err(|e| format!("Cannot resolve parent directory: {e}"))?;
    let workspace_canonical = std::fs::canonicalize(workspace_root)
        .map_err(|e| format!("Cannot resolve workspace root: {e}"))?;
    if !parent_canonical.starts_with(&workspace_canonical) {
        return Err("Access denied: path is outside the workspace".into());
    }
    let file_name = candidate.file_name().ok_or("Path has no file name")?;
    Ok(parent_canonical.join(file_name))
}

fn workspace_canonical(workspace_root: &std::path::Path) -> Result<std::path::PathBuf, String> {
    std::fs::canonicalize(workspace_root)
        .map_err(|e| format!("Cannot resolve workspace root: {e}"))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List the immediate children of `path`.
/// Skips common non-essential entries (`.git`, `node_modules`, `target`).
pub async fn list_directory(
    path: String,
    db: &DbPool,
) -> Result<Vec<FileEntry>, String> {
    let workspace_root = get_active_workspace_path(db)
        .await
        .ok_or("No workspace is open")?;

    let dir = safe_path(&path, &workspace_root)?;
    list_directory_at(&dir, &path)
}

/// Directory listing itself, split out from the command so it can be exercised
/// without a Tauri `State`.
fn list_directory_at(
    dir: &std::path::Path,
    display_path: &str,
) -> Result<Vec<FileEntry>, String> {
    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", display_path));
    }

    const SKIP: &[&str] = &[
        ".git", "node_modules", "target", ".next", "dist", "__pycache__",
        ".cache", ".venv", "venv", ".tox", "build", ".svn",
    ];

    let mut entries: Vec<FileEntry> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read directory: {e}"))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // Skip hidden entries that start with '.' except common project files
            // Also skip common build/cache directories
            if SKIP.contains(&name.as_str()) {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let is_dir = meta.is_dir();
            let size = if is_dir { 0 } else { meta.len() };
            let raw = entry.path().to_string_lossy().into_owned();
            // Strip Windows extended-length path prefix (\\?\) added by canonicalize.
            let path = raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string();
            Some(FileEntry {
                name,
                path,
                is_dir,
                size,
            })
        })
        .collect();

    // Dirs first, then files; both alphabetical
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
    });

    Ok(entries)
}

/// Read a file as UTF-8 text, in full.
///
/// **Deliberately uncapped**, and it must stay that way: this backs the Tauri
/// command behind the editor UI, where a human who opens a file wants the file.
/// Truncating here would silently shorten an editor buffer, and the next save
/// would write the truncation to disk.
///
/// The board-mcp `read_file` tool also calls this, and applies the 32 KB
/// context cap from `tools::read_cap` to the *result*. That is the right place
/// for it: only that caller is answering into a model's context window.
///
/// The 10 MB refusal below is a different limit from that cap and is not
/// redundant with it. This one is a **memory** guard — it stops the process
/// allocating half a gigabyte for one editor tab. The context cap is a
/// **context** guard — it stops a 9 MB file, which passes this check
/// untouched, from flooding an external agent's window. Remove either and a
/// real hole opens: without the memory guard a 500 MB file is read whole
/// before anything truncates it; without the context cap every file under
/// 10 MB is handed to the model intact.
pub async fn read_file_text(
    path: String,
    db: &DbPool,
) -> Result<String, String> {
    let workspace_root = get_active_workspace_path(db)
        .await
        .ok_or("No workspace is open")?;

    let file_path = safe_path(&path, &workspace_root)?;

    if file_path.is_dir() {
        return Err("Path is a directory, not a file".into());
    }

    // The memory guard. Not the context cap — see this function's doc comment
    // for why both exist and why neither is redundant.
    let size = file_path.metadata().map(|m| m.len()).unwrap_or(0);
    if size > 10 * 1024 * 1024 {
        return Err(format!(
            "File is too large to open ({} MB). Maximum is 10 MB.",
            size / 1024 / 1024
        ));
    }

    std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Cannot read file: {e}"))
}

/// Write text content to a file (creates parent directories if needed).
pub async fn write_file_text(
    path: String,
    content: String,
    db: &DbPool,
) -> Result<(), String> {
    let workspace_root = get_active_workspace_path(db)
        .await
        .ok_or("No workspace is open")?;

    let file_path = safe_path_for_new(&path, &workspace_root)?;

    std::fs::write(&file_path, content)
        .map_err(|e| format!("Cannot write file: {e}"))?;

    Ok(())
}

/// Rename a file or directory within the workspace. `new_name` is the new
/// file/folder name only (no path separators allowed). Returns the new path.
pub async fn rename_path(
    old_path: String,
    new_name: String,
    db: &DbPool,
) -> Result<String, String> {
    if new_name.is_empty() {
        return Err("Name cannot be empty".into());
    }
    if new_name.contains('/') || new_name.contains('\\') {
        return Err("Name cannot contain path separators".into());
    }

    let workspace_root = get_active_workspace_path(db)
        .await
        .ok_or("No workspace is open")?;

    let ws_canonical = workspace_canonical(&workspace_root)?;
    let src = safe_path(&old_path, &workspace_root)?;
    let parent = src.parent().ok_or("Path has no parent")?;

    // Validate parent is in workspace (defensive; src already is)
    if !parent.starts_with(&ws_canonical) {
        return Err("Access denied".into());
    }

    let dst = parent.join(&new_name);
    if dst.exists() {
        return Err(format!("'{}' already exists", new_name));
    }

    std::fs::rename(&src, &dst)
        .map_err(|e| format!("Cannot rename: {e}"))?;

    Ok(dst.to_string_lossy().into_owned())
}

/// Duplicate a file to the same directory with a " copy" suffix.
/// Returns the path of the new file.
pub async fn duplicate_file(
    path: String,
    db: &DbPool,
) -> Result<String, String> {
    let workspace_root = get_active_workspace_path(db)
        .await
        .ok_or("No workspace is open")?;

    let src = safe_path(&path, &workspace_root)?;
    if src.is_dir() {
        return Err("Cannot duplicate a directory".into());
    }

    let parent = src.parent().ok_or("No parent directory")?;
    let stem = src.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ext = src
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    // Find an available name: "foo copy.rs", "foo copy 2.rs", …
    let mut dst = parent.join(format!("{stem} copy{ext}"));
    let mut counter = 2u32;
    while dst.exists() {
        dst = parent.join(format!("{stem} copy {counter}{ext}"));
        counter += 1;
    }

    std::fs::copy(&src, &dst)
        .map_err(|e| format!("Cannot duplicate file: {e}"))?;

    Ok(dst.to_string_lossy().into_owned())
}

/// Delete a file or directory (recursively) within the workspace.
pub async fn delete_path(
    path: String,
    db: &DbPool,
) -> Result<(), String> {
    let workspace_root = get_active_workspace_path(db)
        .await
        .ok_or("No workspace is open")?;

    let p = safe_path(&path, &workspace_root)?;

    // Prevent deleting the workspace root itself
    let ws_canonical = workspace_canonical(&workspace_root)?;
    if p == ws_canonical {
        return Err("Cannot delete the workspace root".into());
    }

    if p.is_dir() {
        std::fs::remove_dir_all(&p)
            .map_err(|e| format!("Cannot delete directory: {e}"))?;
    } else {
        std::fs::remove_file(&p)
            .map_err(|e| format!("Cannot delete file: {e}"))?;
    }

    Ok(())
}

/// Create an empty file at `path`. The parent directory must already exist.
/// Returns the canonical path of the created file.
pub async fn create_empty_file(
    path: String,
    db: &DbPool,
) -> Result<String, String> {
    let workspace_root = get_active_workspace_path(db)
        .await
        .ok_or("No workspace is open")?;

    let file_path = safe_path_for_new(&path, &workspace_root)?;
    if file_path.exists() {
        return Err(format!(
            "'{}' already exists",
            file_path.file_name().unwrap_or_default().to_string_lossy()
        ));
    }

    std::fs::write(&file_path, "")
        .map_err(|e| format!("Cannot create file: {e}"))?;

    Ok(file_path.to_string_lossy().into_owned())
}

/// Create a directory at `path`. The parent must already exist.
/// Returns the canonical path of the created directory.
pub async fn create_dir_fs(
    path: String,
    db: &DbPool,
) -> Result<String, String> {
    let workspace_root = get_active_workspace_path(db)
        .await
        .ok_or("No workspace is open")?;

    let dir_path = safe_path_for_new(&path, &workspace_root)?;
    if dir_path.exists() {
        return Err(format!(
            "'{}' already exists",
            dir_path.file_name().unwrap_or_default().to_string_lossy()
        ));
    }

    std::fs::create_dir(&dir_path)
        .map_err(|e| format!("Cannot create directory: {e}"))?;

    Ok(dir_path.to_string_lossy().into_owned())
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// A workspace rooted at a real temp directory, canonicalised so assertions
    /// compare like with like (Windows temp can hand back an 8.3 short name).
    fn workspace() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("temp dir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
        (dir, root)
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    // -- safe_path -----------------------------------------------------------

    #[test]
    fn safe_path_accepts_a_file_inside_the_workspace() {
        let (_dir, root) = workspace();
        let file = root.join("notes.md");
        std::fs::write(&file, "x").expect("write");

        let resolved = safe_path(&s(&file), &root).expect("should resolve");

        assert!(resolved.ends_with("notes.md"));
        assert!(resolved.starts_with(std::fs::canonicalize(&root).unwrap()));
    }

    #[test]
    fn safe_path_accepts_the_workspace_root_itself() {
        let (_dir, root) = workspace();

        assert!(safe_path(&s(&root), &root).is_ok());
    }

    #[test]
    fn safe_path_rejects_a_path_outside_the_workspace() {
        let (_dir, root) = workspace();
        let outside = TempDir::new().expect("outside");
        let file = outside.path().join("secrets.txt");
        std::fs::write(&file, "x").expect("write");

        let err = safe_path(&s(&file), &root).expect_err("should be rejected");

        assert!(err.contains("outside the workspace"), "got {err}");
    }

    #[test]
    fn safe_path_rejects_dotdot_traversal_out_of_the_workspace() {
        let (_dir, root) = workspace();
        let parent = root.parent().expect("parent").to_path_buf();
        let planted = parent.join("rustyagent-escape-probe.txt");
        std::fs::write(&planted, "x").expect("write");

        let attempt = root.join("..").join("rustyagent-escape-probe.txt");
        let result = safe_path(&s(&attempt), &root);
        let _ = std::fs::remove_file(&planted);

        let err = result.expect_err("should be rejected");
        assert!(err.contains("outside the workspace"), "got {err}");
    }

    #[test]
    fn safe_path_rejects_a_nonexistent_path_as_invalid() {
        // canonicalize fails before the containment check, so a missing file is
        // reported as an invalid path rather than an access denial.
        let (_dir, root) = workspace();
        let missing = root.join("nope.md");

        let err = safe_path(&s(&missing), &root).expect_err("should be rejected");

        assert!(err.contains("Invalid path"), "got {err}");
    }

    #[test]
    fn safe_path_rejects_a_sibling_directory_sharing_the_root_prefix() {
        let dir = TempDir::new().expect("temp dir");
        let base = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let root = base.join("proj");
        let sibling = base.join("proj-evil");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::create_dir_all(&sibling).expect("mkdir");
        let loot = sibling.join("loot.txt");
        std::fs::write(&loot, "x").expect("write");

        let err = safe_path(&s(&loot), &root).expect_err("should be rejected");

        assert!(err.contains("outside the workspace"), "got {err}");
    }

    #[test]
    #[cfg(windows)]
    fn safe_path_resolves_a_junction_before_the_containment_check() {
        let (_dir, root) = workspace();
        let outside = TempDir::new().expect("outside");
        std::fs::write(outside.path().join("loot.txt"), "x").expect("write");

        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(root.join("escape"))
            .arg(outside.path())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !made {
            eprintln!("skipping: could not create a directory junction here");
            return;
        }

        let attempt = root.join("escape").join("loot.txt");
        let err = safe_path(&s(&attempt), &root).expect_err("should be rejected");

        assert!(err.contains("outside the workspace"), "got {err}");
    }

    #[test]
    #[cfg(unix)]
    fn safe_path_resolves_a_symlink_before_the_containment_check() {
        let (_dir, root) = workspace();
        let outside = TempDir::new().expect("outside");
        std::fs::write(outside.path().join("loot.txt"), "x").expect("write");
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).expect("symlink");

        let attempt = root.join("escape").join("loot.txt");
        let err = safe_path(&s(&attempt), &root).expect_err("should be rejected");

        assert!(err.contains("outside the workspace"), "got {err}");
    }

    #[test]
    fn safe_path_reports_an_unresolvable_workspace_root() {
        let (_dir, root) = workspace();
        let file = root.join("notes.md");
        std::fs::write(&file, "x").expect("write");
        let bogus_root = root.join("does-not-exist");

        let err = safe_path(&s(&file), &bogus_root).expect_err("should be rejected");

        assert!(err.contains("Cannot resolve workspace root"), "got {err}");
    }

    // -- safe_path_for_new ---------------------------------------------------

    #[test]
    fn safe_path_for_new_accepts_a_target_whose_parent_is_inside() {
        let (_dir, root) = workspace();
        let target = root.join("new-file.md");

        let resolved = safe_path_for_new(&s(&target), &root).expect("should resolve");

        assert!(resolved.ends_with("new-file.md"));
        assert!(!target.exists(), "the helper must not create anything");
    }

    #[test]
    fn safe_path_for_new_accepts_a_target_in_an_existing_subdirectory() {
        let (_dir, root) = workspace();
        std::fs::create_dir_all(root.join("docs")).expect("mkdir");
        let target = root.join("docs").join("new.md");

        let resolved = safe_path_for_new(&s(&target), &root).expect("should resolve");

        assert!(resolved.ends_with("new.md"));
    }

    #[test]
    fn safe_path_for_new_rejects_a_target_whose_parent_is_outside() {
        let (_dir, root) = workspace();
        let outside = TempDir::new().expect("outside");
        let target = outside.path().join("planted.txt");

        let err = safe_path_for_new(&s(&target), &root).expect_err("should be rejected");

        assert!(err.contains("outside the workspace"), "got {err}");
    }

    #[test]
    fn safe_path_for_new_rejects_a_dotdot_escape() {
        let (_dir, root) = workspace();
        let target = root.join("..").join("planted.txt");

        let err = safe_path_for_new(&s(&target), &root).expect_err("should be rejected");

        assert!(err.contains("outside the workspace"), "got {err}");
    }

    #[test]
    fn safe_path_for_new_rejects_a_target_whose_parent_does_not_exist() {
        let (_dir, root) = workspace();
        let target = root.join("missing-dir").join("new.md");

        let err = safe_path_for_new(&s(&target), &root).expect_err("should be rejected");

        assert!(err.contains("Cannot resolve parent directory"), "got {err}");
    }

    #[test]
    fn safe_path_for_new_rejects_a_path_with_no_parent() {
        let (_dir, root) = workspace();
        let filesystem_root = if cfg!(windows) { "C:\\" } else { "/" };

        let err = safe_path_for_new(filesystem_root, &root).expect_err("should be rejected");

        assert!(err.contains("no parent directory"), "got {err}");
    }

    #[test]
    fn safe_path_for_new_rejects_a_bare_relative_name() {
        // Path::parent on "notes.md" yields an empty path rather than None, so
        // this fails at canonicalize instead of the no-parent branch.
        let (_dir, root) = workspace();

        let err = safe_path_for_new("notes.md", &root).expect_err("should be rejected");

        assert!(err.contains("Cannot resolve parent directory"), "got {err}");
    }

    // -- list_directory_at ---------------------------------------------------

    #[test]
    fn list_directory_skips_build_and_vcs_directories() {
        let (_dir, root) = workspace();
        for skipped in [".git", "node_modules", "target", "dist", "__pycache__"] {
            std::fs::create_dir_all(root.join(skipped)).expect("mkdir");
        }
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("README.md"), "x").expect("write");

        let entries = list_directory_at(&root, ".").expect("should list");

        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);
    }

    #[test]
    fn list_directory_sorts_directories_first_then_alphabetically() {
        let (_dir, root) = workspace();
        std::fs::create_dir_all(root.join("zeta")).expect("mkdir");
        std::fs::create_dir_all(root.join("alpha")).expect("mkdir");
        std::fs::write(root.join("b.txt"), "x").expect("write");
        std::fs::write(root.join("a.txt"), "x").expect("write");

        let entries = list_directory_at(&root, ".").expect("should list");

        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta", "a.txt", "b.txt"]);
    }

    #[test]
    fn list_directory_reports_size_for_files_and_zero_for_directories() {
        let (_dir, root) = workspace();
        std::fs::create_dir_all(root.join("adir")).expect("mkdir");
        std::fs::write(root.join("five.txt"), "12345").expect("write");

        let entries = list_directory_at(&root, ".").expect("should list");

        let dir_entry = entries.iter().find(|e| e.name == "adir").expect("adir");
        let file_entry = entries.iter().find(|e| e.name == "five.txt").expect("file");
        assert!(dir_entry.is_dir);
        assert_eq!(dir_entry.size, 0);
        assert!(!file_entry.is_dir);
        assert_eq!(file_entry.size, 5);
    }

    #[test]
    fn list_directory_on_a_file_reports_not_a_directory() {
        let (_dir, root) = workspace();
        let file = root.join("notes.md");
        std::fs::write(&file, "x").expect("write");

        let err = list_directory_at(&file, "notes.md").expect_err("should be rejected");

        assert!(err.contains("is not a directory"), "got {err}");
    }

    #[test]
    fn list_directory_strips_the_windows_extended_length_prefix() {
        let (_dir, root) = workspace();
        std::fs::write(root.join("a.txt"), "x").expect("write");
        let canonical = std::fs::canonicalize(&root).expect("canonicalize");

        let entries = list_directory_at(&canonical, ".").expect("should list");

        assert!(
            !entries[0].path.starts_with(r"\\?\"),
            "path leaked an extended-length prefix: {}",
            entries[0].path
        );
    }

    #[test]
    fn list_directory_on_an_empty_directory_returns_no_entries() {
        let (_dir, root) = workspace();

        let entries = list_directory_at(&root, ".").expect("should list");

        assert!(entries.is_empty());
    }

    // -- read_file_text: the editor path -------------------------------------
    //
    // This function serves both the Tauri command behind the editor UI and the
    // board-mcp `read_file` tool. Only the MCP side applies the 32 KB context
    // cap: a human who opens a file in the editor wants the file, and a
    // truncated editor buffer would be a data-loss bug the moment they saved.
    // These tests hold that line from the other side, so a future change that
    // moves the cap down into this function fails here.

    /// A pool whose single workspace row points at `root`, which
    /// `get_active_workspace_path` resolves as the active workspace.
    async fn workspace_pool(root: &Path) -> db::DbPool {
        let pool = db::testing::make_test_pool().await;
        db::testing::seed_workspace(&pool, "ws-1", &s(root)).await;
        pool
    }

    #[tokio::test]
    async fn read_file_text_returns_a_file_far_larger_than_the_context_cap_in_full() {
        let (_dir, root) = workspace();
        let pool = workspace_pool(&root).await;
        // 4x the 32 KB cap the MCP read path applies, with no line long enough
        // to be mistaken for the cap doing something else.
        let content: String = (0..2048).map(|i| format!("line {i:04} {}\n", "-".repeat(53))).collect();
        assert_eq!(content.len(), 2048 * 64);
        let file = root.join("big.txt");
        std::fs::write(&file, &content).expect("write");

        let read = read_file_text(s(&file), &pool).await.expect("should read");

        assert_eq!(read, content, "the editor must receive the whole file");
        assert!(!read.contains("TRUNCATED"), "the editor path must carry no marker");
    }

    #[tokio::test]
    async fn read_file_text_still_refuses_a_directory() {
        let (_dir, root) = workspace();
        let pool = workspace_pool(&root).await;
        std::fs::create_dir(root.join("src")).expect("mkdir");

        let err = read_file_text(s(&root.join("src")), &pool)
            .await
            .expect_err("should be rejected");

        assert!(err.contains("is a directory"), "got {err}");
    }

    #[tokio::test]
    async fn read_file_text_refuses_a_path_outside_the_workspace() {
        let (_dir, root) = workspace();
        let pool = workspace_pool(&root).await;
        let outside = TempDir::new().expect("outside");
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "x").expect("write");

        let err = read_file_text(s(&secret), &pool)
            .await
            .expect_err("should be rejected");

        assert!(err.contains("outside the workspace"), "got {err}");
    }
}
