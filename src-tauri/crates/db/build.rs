fn main() {
    // Re-run this build script (and recompile the crate) whenever any migration
    // file changes. Without this, adding a new .sql file would not trigger a
    // Rust rebuild, meaning sqlx::migrate!() wouldn't embed the new file.
    println!("cargo:rerun-if-changed=migrations");
}
