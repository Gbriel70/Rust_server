//! Compile-fail examples live in tests/fixtures, never in the production modules.
use std::{fs, path::PathBuf, process::Command};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fails(file: &str, expected: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = Temp(std::env::temp_dir().join(format!(
        "rust-serv-compile-{}-{}",
        std::process::id(),
        file
    )));
    fs::create_dir(&temp.0).unwrap();
    let result = Command::new("rustc")
        .arg("--edition=2024")
        .arg(root.join("tests/fixtures").join(file))
        .arg("--out-dir")
        .arg(&temp.0)
        .output()
        .unwrap();
    assert!(!result.status.success(), "sample unexpectedly compiled");
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(
        error.contains("E0277") && error.contains(expected),
        "{error}"
    );
}
#[test]
fn rc_cannot_cross_thread_boundary() {
    fails("rc_is_not_send.rs", "Send");
}
#[test]
fn arc_does_not_make_cell_thread_safe() {
    fails("cell_is_not_sync.rs", "Sync");
}
