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

/// The body of one TOML section, up to the next section header.
///
/// Scoping matters: a manifest can carry `version = "…"` in a
/// `[dependencies.foo]` table as legitimately as in `[package]`, and a check
/// that searched the whole file would call the first one a drifting version.
fn section<'a>(source: &'a str, header: &str) -> Option<&'a str> {
    let start = source.find(header)?;
    let rest = &source[start..];
    let end = rest[1..].find("\n[").map(|i| i + 1).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// A literal `version = "…"` declared directly in `body`.
///
/// `None` for `version.workspace = true`, which is inheritance rather than a
/// copy — that is the whole distinction this file is drawing.
fn literal_version(body: &str) -> Option<String> {
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("version") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(value) = rest.strip_prefix('=') else {
            // `version.workspace = true` — inherited, not declared.
            continue;
        };
        let value = value.trim();
        if value.starts_with('"') {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// The `version` under `[workspace.package]` in the workspace manifest.
fn workspace_package_version(source: &str) -> Option<String> {
    literal_version(section(source, "[workspace.package]")?)
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

/// A dependency's version is not the crate's own, and a check that searched
/// the whole manifest would have said otherwise the first time somebody wrote
/// a dependency in table form.
#[test]
fn a_dependency_table_is_not_mistaken_for_the_crates_own_version() {
    let manifest = r#"
[package]
name = "example"
version.workspace = true
edition = "2021"

[dependencies.serde]
version = "1.0"
features = ["derive"]
"#;

    assert_eq!(
        section(manifest, "[package]").and_then(literal_version),
        None,
        "the package inherits its version; only the dependency declares one"
    );
    assert_eq!(
        section(manifest, "[dependencies.serde]").and_then(literal_version),
        Some("1.0".to_string()),
        "and the dependency's version is still readable, so the scoping works"
    );
}

#[test]
fn a_crate_declaring_its_own_version_is_caught() {
    let manifest = "[package]\nname = \"example\"\nversion = \"0.1.0\"\n";

    assert_eq!(
        section(manifest, "[package]").and_then(literal_version),
        Some("0.1.0".to_string())
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
        // Only `[package]`, and only a literal. `version.workspace = true` is
        // inheritance, and a `version = "…"` inside a `[dependencies.foo]`
        // table is somebody else's version — neither is a copy that can drift.
        let declares_its_own = section(&source, "[package]")
            .and_then(literal_version)
            .is_some();
        if declares_its_own {
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
