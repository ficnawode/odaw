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

#[test]
fn rename_goes_through_the_session_and_persists() {
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
    let renamed = odaw().args(["rename", path, "renamed"]).output().unwrap();
    assert!(
        renamed.status.success(),
        "{}",
        String::from_utf8_lossy(&renamed.stderr)
    );

    let inspected = odaw().args(["inspect", path, "--json"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(json["project"]["meta"]["name"], "renamed");
}

#[test]
fn demo_renders_a_wav_file() {
    let directory = TempDir::new().unwrap();
    let project = directory.path().join("demo.odaw");
    let output = directory.path().join("demo.wav");

    let created = odaw()
        .args(["demo", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );

    let rendered = odaw()
        .args([
            "render",
            project.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
            "--duration",
            "0.1",
            "--block-size",
            "64",
        ])
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );

    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(&bytes[36..40], b"data");
    assert_eq!(
        u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        48_000
    );
    assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
    assert_eq!(
        u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize,
        bytes.len() - 44
    );
}

#[test]
fn demo_render_uses_the_loop_length_by_default() {
    let directory = TempDir::new().unwrap();
    let project = directory.path().join("demo.odaw");
    let output = directory.path().join("demo.wav");

    assert!(
        odaw()
            .args(["demo", project.to_str().unwrap()])
            .output()
            .unwrap()
            .status
            .success()
    );
    let rendered = odaw()
        .args([
            "render",
            project.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );

    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.len() > 1_000_000);
    assert_eq!(
        u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize,
        bytes.len() - 44
    );
}

#[test]
fn render_rejects_an_invalid_duration() {
    let directory = TempDir::new().unwrap();
    let project = directory.path().join("demo.odaw");
    let output = directory.path().join("demo.wav");

    assert!(
        odaw()
            .args(["demo", project.to_str().unwrap()])
            .output()
            .unwrap()
            .status
            .success()
    );
    let rendered = odaw()
        .args([
            "render",
            project.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
            "--duration",
            "-1",
        ])
        .output()
        .unwrap();
    assert!(!rendered.status.success());
    assert!(String::from_utf8_lossy(&rendered.stderr).contains("duration"));
}
