use std::{fs, path::PathBuf, process::Command};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fails(file: &str, codes: &[&str]) {
    let temp = Temp(
        std::env::temp_dir().join(format!("rust-serv-lifetime-{}-{file}", std::process::id())),
    );
    fs::create_dir(&temp.0).unwrap();
    let output = Command::new("rustc")
        .arg("--edition=2024")
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        )
        .arg("--out-dir")
        .arg(&temp.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    for code in codes {
        assert!(error.contains(code), "{error}");
    }
}
#[test]
fn self_referential_connection_cannot_move_its_borrowed_buffer() {
    fails("self_reference.rs", &["E0505", "E0515"]);
}
#[test]
fn a_request_cannot_escape_its_local_buffer() {
    fails("borrow_escapes_buffer.rs", &["E0515"]);
}
