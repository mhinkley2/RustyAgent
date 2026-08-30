//! A migration that has shipped is immutable.
//!
//! sqlx records a SHA-384 of every migration at the moment it is applied, and
//! refuses to run when the file later differs from what it recorded. That is
//! the right behaviour — a changed migration means the database and the code
//! disagree about history — but the failure lands on the *user*: their app
//! stops starting, with no forward path except editing `_sqlx_migrations` by
//! hand or discarding the database.
//!
//! Nothing else in this suite can catch it. `make_test_pool` opens a fresh
//! in-memory database and applies every migration from scratch, so a file's
//! checksum always matches what is on disk; CI has no database with history.
//! An edit to a shipped migration passes every test and every CI job, and
//! breaks every existing install.
//!
//! It happened: a status comment in `20260409000001_initial.sql` was updated
//! to point at `story_status::STORY_STATUSES`, which changed the file, which
//! stopped the app starting for anyone with an existing database. The comment
//! was accurate and the change was still wrong.
//!
//! `.gitattributes` already pins `*.sql` to LF for the same reason — line
//! endings changing a checksum is the other way in. This covers the way a
//! person or an agent gets there: by editing the file on purpose.

use std::collections::BTreeMap;
use std::path::PathBuf;

use sha2::{Digest, Sha384};

/// Checksums of every migration, as shipped.
///
/// Regenerate **only** when adding a new migration, never to make a failing
/// test pass:
///
/// ```text
/// cd src-tauri/crates/db/migrations
/// for f in *.sql; do printf "%s  %s\n" "$(sha384sum "$f" | cut -d' ' -f1)" "$f"; done > ../migrations.sha384
/// ```
const MANIFEST: &str = include_str!("../migrations.sha384");

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// `filename -> checksum`, as recorded.
fn recorded() -> BTreeMap<String, String> {
    MANIFEST
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (sum, name) = line
                .split_once("  ")
                .unwrap_or_else(|| panic!("malformed manifest line: {line}"));
            (name.trim().to_string(), sum.trim().to_string())
        })
        .collect()
}

/// `filename -> checksum`, as the files actually are.
fn on_disk() -> BTreeMap<String, String> {
    std::fs::read_dir(migrations_dir())
        .expect("read the migrations directory")
        .filter_map(|entry| {
            let path = entry.expect("read a directory entry").path();
            if path.extension()?.to_str()? != "sql" {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            let bytes = std::fs::read(&path).expect("read a migration");
            let digest = Sha384::digest(&bytes);
            let hex = digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
            Some((name, hex))
        })
        .collect()
}

#[test]
fn no_shipped_migration_has_been_edited() {
    let recorded = recorded();
    let actual = on_disk();

    let mut edited = Vec::new();
    for (name, expected) in &recorded {
        match actual.get(name) {
            Some(found) if found != expected => edited.push(name.clone()),
            Some(_) => {}
            None => panic!(
                "Migration '{name}' is in the manifest but missing from disk.\n\n\
                 Deleting a migration that has been applied breaks every existing \
                 install exactly as editing one does: sqlx finds a version recorded \
                 in the database that the binary does not have, and refuses to start."
            ),
        }
    }

    assert!(
        edited.is_empty(),
        "These migrations have been edited after shipping: {edited:?}\n\n\
         sqlx records a SHA-384 of each migration when it is applied and refuses to \
         run when the file later differs, so every existing database — including \
         every user's — stops the app at startup. The fresh in-memory database this \
         suite uses will never show it.\n\n\
         An applied migration is immutable, comments included. Revert the change; if \
         it is a schema change, add a new migration instead. Regenerate the manifest \
         only when adding a file, never to silence this."
    );
}

#[test]
fn every_migration_on_disk_is_recorded() {
    let recorded = recorded();
    let unrecorded: Vec<_> = on_disk()
        .keys()
        .filter(|name| !recorded.contains_key(*name))
        .cloned()
        .collect();

    assert!(
        unrecorded.is_empty(),
        "New migrations are not in the manifest: {unrecorded:?}\n\n\
         Add them by regenerating `crates/db/migrations.sha384` — see the comment on \
         MANIFEST. The step is deliberate: it is the moment to be sure a new file is \
         what you meant, rather than an edit to an existing one wearing a new name."
    );
}

/// The manifest is only worth anything if it hashes what sqlx hashes.
#[test]
fn the_manifest_uses_the_same_digest_sqlx_records() {
    // sqlx stores SHA-384 of the file's bytes. This is the checksum
    // `_sqlx_migrations` holds for the initial schema in a real database, read
    // out of a live install at the time this test was written.
    const INITIAL_SCHEMA_AS_APPLIED: &str = "dd68fd61829f3388fb573e1532b99ea82098d9abe30\
                                             6094071d98b25072110829faa9bb3ab3025bccae40a13cd8b4636";

    let actual = on_disk();
    let initial = actual
        .get("20260409000001_initial.sql")
        .expect("the initial schema is on disk");

    assert_eq!(
        initial, INITIAL_SCHEMA_AS_APPLIED,
        "the initial schema no longer hashes to what a real database recorded — \
         either the file changed, or this test is hashing something sqlx does not"
    );
}
