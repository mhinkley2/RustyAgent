//! The three files that describe the shipped artifact must agree.
//!
//! This repository has been bitten three times by one value living in several
//! copies with no check: the model catalogues drifted until the profile editor
//! offered retired model ids, the app and `rustyagent-board-mcp` disagreed
//! about where the database lived, and every build ever produced was `0.1.0`
//! regardless of what had merged. The bump is the small half of fixing the
//! third; this is the durable half.
//!
//! **`tauri.conf.json` is the source of truth.** It is the version Tauri
//! stamps onto a bundle, so it is the only one a user can observe from an
//! installed artifact — a bug report naming a version is naming that one.
//! `package.json` and the Cargo workspace must match it.
//!
//! Only three files are checked. The ten member crates take
//! `version.workspace = true`, so there is nothing left in them to drift;
//! extending this to thirteen paths would trade a drift problem for a churn
//! problem.
//!
//! This lives in a test rather than a CI script deliberately: it fails on
//! `cargo test` before a push, not after one.

use std::path::{Path, PathBuf};

/// `src-tauri/`.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The repository root, one level above `src-tauri/`.
fn repo_root() -> PathBuf {
    crate_dir()
        .parent()
        .expect("src-tauri has a parent")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// The first `"version": "..."` in a JSON file.
///
/// Both files put it at the top level within the first few lines, before any
/// dependency block that might also carry the key.
fn json_version(source: &str) -> Option<String> {
    let at = source.find("\"version\"")?;
    let rest = &source[at + "\"version\"".len()..];
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    Some(rest[start..end].to_string())
}

/// The `version` under `[workspace.package]` in the workspace manifest.
fn workspace_package_version(source: &str) -> Option<String> {
    let section = source.find("[workspace.package]")?;
    let rest = &source[section..];
    // Stop at the next section header so a later `version = …` cannot be
    // mistaken for this one.
    let end = rest[1..].find("\n[").map(|i| i + 1).unwrap_or(rest.len());
    for line in rest[..end].lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("version") {
            let value = value.trim_start().strip_prefix('=')?.trim();
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

struct Declared {
    label: &'static str,
    path: PathBuf,
    version: String,
}

fn declared_versions() -> Vec<Declared> {
    let tauri_conf = crate_dir().join("tauri.conf.json");
    let package_json = repo_root().join("package.json");
    let cargo_toml = crate_dir().join("Cargo.toml");

    vec![
        Declared {
            label: "tauri.conf.json (source of truth — stamped onto bundles)",
            version: json_version(&read(&tauri_conf))
                .expect("tauri.conf.json declares a version"),
            path: tauri_conf,
        },
        Declared {
            label: "package.json",
            version: json_version(&read(&package_json)).expect("package.json declares a version"),
            path: package_json,
        },
        Declared {
            label: "Cargo.toml [workspace.package]",
            version: workspace_package_version(&read(&cargo_toml))
                .expect("the workspace declares a version"),
            path: cargo_toml,
        },
    ]
}

#[test]
fn the_three_version_files_agree() {
    let declared = declared_versions();
    let truth = &declared[0];

    let disagreeing: Vec<&Declared> = declared
        .iter()
        .filter(|d| d.version != truth.version)
        .collect();

    if !disagreeing.is_empty() {
        let mut message = String::from(
            "The files describing the shipped artifact declare different versions.\n\n\
             `tauri.conf.json` is the source of truth: it is the version Tauri stamps \
             onto the installer, so it is the one a user can see. Bring the others to \
             match it.\n\n",
        );
        for d in &declared {
            message.push_str(&format!("  {:<12} {}\n               {}\n", d.version, d.label, d.path.display()));
        }
        panic!("{message}");
    }
}

/// A bump that leaves the number where it started is not a bump. This is the
/// state the repository shipped four local releases in.
#[test]
fn the_version_has_moved_past_the_scaffolded_default() {
    let declared = declared_versions();
    assert_ne!(
        declared[0].version, "0.1.0",
        "still on the scaffolded default: every build would again be indistinguishable \
         from every other by name"
    );
}

/// Tauri requires a semver-shaped version, and the MSI bundler rejects
/// pre-release suffixes in some configurations — a bad value here fails at
/// bundle time, long after the change that caused it.
#[test]
fn the_version_is_a_plain_semver_triple() {
    let version = declared_versions().remove(0).version;
    let parts: Vec<&str> = version.split('.').collect();

    assert_eq!(
        parts.len(),
        3,
        "expected MAJOR.MINOR.PATCH, found '{version}'"
    );
    for part in parts {
        assert!(
            !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()),
            "'{version}' is not a plain semver triple; the MSI bundler rejects suffixes"
        );
    }
}

/// The version the running app reports has to be the one on the installer, or
/// a bug report naming it sends the reader to the wrong build.
#[test]
fn the_compiled_in_version_matches_the_bundled_one() {
    assert_eq!(
        env!("CARGO_PKG_VERSION"),
        declared_versions()[0].version,
        "the crate compiled at a different version than tauri.conf.json will stamp"
    );
}

#[test]
fn no_member_crate_pins_its_own_version() {
    let crates_dir = crate_dir().join("crates");
    let mut pinned = Vec::new();

    for entry in std::fs::read_dir(&crates_dir).expect("read crates/") {
        let manifest = entry.expect("a crates/ entry").path().join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let source = read(&manifest);
        // `version.workspace = true` is inheritance; a literal `version = "…"`
        // in `[package]` is a copy that can drift.
        if source.contains("\nversion = \"") || source.starts_with("version = \"") {
            pinned.push(manifest.display().to_string());
        }
    }

    assert!(
        pinned.is_empty(),
        "these crates pin their own version instead of inheriting it: {pinned:#?}\n\n\
         Use `version.workspace = true`. Ten copies of one number is the defect this \
         guard exists to prevent, and adding an eleventh reintroduces it."
    );
}
