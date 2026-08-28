//! Path containment helpers shared by the built-in file tools, the custom
//! shell tool, and the runtime's permission policy.
//!
//! Every one of those three has to answer the same question — "does this path
//! stay inside that directory?" — and getting it wrong in any of them is a
//! containment hole. String comparison is not an answer:
//!
//! * `"/allowed-other".starts_with("/allowed")` is `true`, but
//!   `/allowed-other` is not inside `/allowed`.
//! * `..` has to be resolved before comparing, and resolving it with
//!   `canonicalize` fails outright for a file that does not exist yet.
//! * Windows spells the same directory several ways (`C:` vs `c:`, `\\?\`
//!   prefixed or not), and compares case-insensitively.
//!
//! So containment is decided on *path components*, after resolving symlinks as
//! far as the path actually exists.

use std::path::{Component, Path, PathBuf};

/// Lexically normalise a path (resolves `.` and `..` without touching the FS).
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// On Windows `canonicalize` returns a `\\?\`-prefixed path; strip it so both
/// sides of a containment check are in the same form.
pub fn strip_unc(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
}

/// Canonicalise the deepest existing ancestor of `path`, then re-append the
/// components that do not exist yet.
///
/// `canonicalize` alone fails outright for a file being created for the first
/// time, but skipping it entirely (a purely lexical normalisation) would let a
/// symlink inside the workspace resolve to a target outside it.
pub fn resolve_existing_prefix(path: &Path) -> PathBuf {
    let normalised = normalize_path(path);
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = normalised.as_path();

    loop {
        if let Ok(real) = std::fs::canonicalize(probe) {
            let mut out = strip_unc(&real);
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (probe.parent(), probe.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                probe = parent;
            }
            // Reached the root without finding anything that exists.
            _ => return normalised,
        }
    }
}

/// Compare two path components for identity, honouring the host filesystem's
/// case rules.
fn component_eq(a: &Component, b: &Component) -> bool {
    if cfg!(windows) {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    } else {
        a.as_os_str() == b.as_os_str()
    }
}

/// Is `candidate` `prefix` itself, or somewhere beneath it?
///
/// Compares component by component, so `/allowed-other` is *not* inside
/// `/allowed` — the trap a `starts_with` on the string form walks straight
/// into. Both arguments are expected to have been through
/// [`resolve_existing_prefix`] already; this function does no I/O.
///
/// A prefix with no components (`""`) matches nothing. An empty allow-list
/// entry is a configuration mistake, and reading it as "everything is allowed"
/// would silently disable the very restriction it appears in.
pub fn is_within(candidate: &Path, prefix: &Path) -> bool {
    let mut prefix_components = prefix.components().peekable();
    if prefix_components.peek().is_none() {
        return false;
    }

    let mut candidate_components = candidate.components();
    for p in prefix_components {
        match candidate_components.next() {
            Some(actual) if component_eq(&actual, &p) => {}
            _ => return false,
        }
    }
    true
}

/// Resolve a path as written by an agent (or by an operator into an allow-list)
/// into the form containment checks compare.
///
/// `root` is joined on when set. `Path::join` already replaces the base when
/// the argument is rooted, which is what we want: an allow-list entry of
/// `/workspace/` and a tool input of `/workspace/x` must land in the same
/// coordinate system as each other, whatever the platform thinks "absolute"
/// means. The only rule that matters is that *both* sides of a comparison go
/// through this function.
pub fn resolve_for_containment(raw: &str, root: Option<&Path>) -> PathBuf {
    let requested = Path::new(raw);
    let joined = match root {
        Some(r) => r.join(requested),
        None => requested.to_path_buf(),
    };
    resolve_existing_prefix(&joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn normalize_resolves_dot_and_parent_components() {
        assert_eq!(normalize_path(Path::new("a/./b/../c")), p("a/c"));
    }

    #[test]
    fn a_path_is_within_itself() {
        assert!(is_within(&p("/allowed"), &p("/allowed")));
    }

    #[test]
    fn a_child_is_within_its_parent() {
        assert!(is_within(&p("/allowed/deep/file.txt"), &p("/allowed")));
    }

    /// The whole reason this is not `starts_with` on the string.
    #[test]
    fn a_sibling_sharing_a_name_prefix_is_not_within() {
        assert!(!is_within(&p("/allowed-other/file.txt"), &p("/allowed")));
        assert!(!is_within(&p("/data-private/x"), &p("/data")));
    }

    #[test]
    fn a_parent_is_not_within_its_child() {
        assert!(!is_within(&p("/allowed"), &p("/allowed/deep")));
    }

    #[test]
    fn a_trailing_separator_on_the_prefix_makes_no_difference() {
        assert!(is_within(&p("/allowed/file.txt"), &p("/allowed/")));
    }

    #[test]
    fn an_empty_prefix_matches_nothing() {
        assert!(!is_within(&p("/anything"), &p("")));
        assert!(!is_within(&p(""), &p("")));
    }

    #[test]
    fn an_unrelated_path_is_not_within() {
        assert!(!is_within(&p("/etc/passwd"), &p("/allowed")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_containment_ignores_case_and_drive_spelling() {
        assert!(is_within(&p(r"C:\Work\Proj\src"), &p(r"c:\work\proj")));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_containment_is_case_sensitive() {
        assert!(!is_within(&p("/Allowed/x"), &p("/allowed")));
    }

    #[test]
    fn a_traversal_that_escapes_the_prefix_is_resolved_before_comparison() {
        let candidate = resolve_for_containment("allowed/../../etc/passwd", Some(Path::new("/ws")));
        assert!(!is_within(&candidate, &resolve_for_containment("allowed", Some(Path::new("/ws")))));
    }

    #[test]
    fn a_traversal_that_stays_inside_the_prefix_still_matches() {
        let root = Path::new("/ws");
        let candidate = resolve_for_containment("allowed/deep/../file.txt", Some(root));
        assert!(is_within(&candidate, &resolve_for_containment("allowed", Some(root))));
    }

    /// The resolved form carries no `.` or `..` left to reinterpret, and still
    /// sits under the prefix it started from. Asserting the exact string would
    /// be a Windows trap: a rooted-but-driveless path like `/workspace` really
    /// does mean `C:\workspace` there, and resolution says so.
    #[test]
    fn resolve_without_a_root_resolves_dot_and_parent_components() {
        let resolved = resolve_for_containment("/workspace/./src/../main.rs", None);

        assert!(resolved.ends_with("main.rs"), "got {resolved:?}");
        assert!(
            !resolved
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
            "got {resolved:?}"
        );
        assert!(is_within(&resolved, &resolve_for_containment("/workspace", None)));
        assert!(!is_within(&resolved, &resolve_for_containment("/workspace/src", None)));
    }

    /// A real directory on disk is canonicalised, so a symlink cannot smuggle a
    /// path out of the prefix it appears to be under.
    #[test]
    fn an_existing_directory_is_canonicalised() {
        let dir = tempfile::tempdir().expect("tempdir");
        let resolved = resolve_for_containment("child/file.txt", Some(dir.path()));
        let root = resolve_for_containment(".", Some(dir.path()));
        assert!(is_within(&resolved, &root), "{resolved:?} not within {root:?}");
    }
}
