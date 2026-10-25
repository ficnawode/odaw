use std::process::Command;

use tempfile::TempDir;

fn odaw() -> Command {
    Command::new(env!("CARGO_BIN_EXE_odaw"))
}

#[test]
fn new_inspect_and_validate_round_trip() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("song.odaw");
    let path = path.to_str().unwrap();

    let created = odaw()
        .args(["new", path, "--name", "demo", "--seed", "9"])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );

    let inspected = odaw().args(["inspect", path, "--json"]).output().unwrap();
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(json["format_version"], 1);
    assert_eq!(json["project"]["meta"]["name"], "demo");
    assert_eq!(json["project"]["meta"]["seed"], 9);
    assert_eq!(json["project"]["meta"]["sample_rate"], 48_000);

    let validated = odaw().args(["validate", path]).output().unwrap();
    assert!(
        validated.status.success(),
        "{}",
        String::from_utf8_lossy(&validated.stderr)
    );
}

#[test]
fn new_refuses_to_overwrite_without_force() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("song.odaw");
    let path = path.to_str().unwrap();

    assert!(
        odaw()
            .args(["new", path])
            .output()
            .unwrap()
            .status
            .success()
    );
    let second = odaw().args(["new", path]).output().unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("--force"));

    let forced = odaw()
        .args(["new", path, "--force", "--name", "replacement"])
        .output()
        .unwrap();
    assert!(forced.status.success());

    let inspected = odaw().args(["inspect", path, "--json"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(json["project"]["meta"]["name"], "replacement");
}

#[test]
fn inspect_reports_a_missing_container() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("missing.odaw");
    let output = odaw()
        .args(["inspect", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));
}
