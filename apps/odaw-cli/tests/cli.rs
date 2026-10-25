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
    assert_eq!(json["format_version"], 3);
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

#[test]
fn render_includes_audio_clip_assets() {
    use std::fs::File;
    use std::io::BufWriter;

    use odaw_core::{
        Asset, AssetId, AssetKind, ChannelLayout, Clip, ClipContent, ClipId, Edge, EdgeId, Frames,
        Node, NodeId, NodePort, NodeRole, Port, PortId, Project, Tick,
    };
    use odaw_format::Container;
    use odaw_io::{WavSpec, hash_file, write_wav};

    let directory = TempDir::new().unwrap();
    let source = directory.path().join("tone.wav");
    let frames = 24_000;
    let mut samples = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        let value = (frame as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.4;
        samples.push(value);
        samples.push(value);
    }
    write_wav(
        BufWriter::new(File::create(&source).unwrap()),
        WavSpec {
            sample_rate: 48_000,
            channels: 2,
        },
        &samples,
    )
    .unwrap();

    let asset = AssetId::from_bytes(hash_file(&source).unwrap());
    let mut project = Project::empty("audio", 48_000, 0).unwrap();
    let master = NodeId::from_raw(1);
    let track = NodeId::from_raw(2);
    let clip = ClipId::from_raw(3);
    project.nodes.insert(
        master,
        Node::new(NodeRole::Master).with_ports([
            Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
            Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
        ]),
    );
    project.nodes.insert(
        track,
        Node::new(NodeRole::Track)
            .with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ])
            .with_clips([clip]),
    );
    project.assets.insert(
        asset,
        Asset {
            name: "tone.wav".into(),
            kind: AssetKind::Audio,
        },
    );
    project.clips.insert(
        clip,
        Clip {
            node: track,
            start: Tick::ZERO,
            length: Tick::from_raw(odaw_core::PPQ),
            content: ClipContent::Audio {
                asset,
                source_start: Frames::ZERO,
                source_length: Frames::from_raw(frames as i64),
            },
        },
    );
    project.edges.insert(
        EdgeId::from_raw(4),
        Edge {
            from: NodePort {
                node: track,
                port: PortId::from_raw(1),
            },
            to: NodePort {
                node: master,
                port: PortId::from_raw(0),
            },
        },
    );

    let container = directory.path().join("audio.odaw");
    Container::new(&container).save(&project).unwrap();
    std::fs::copy(
        &source,
        container
            .join("assets")
            .join(format!("{}.wav", asset.to_hex())),
    )
    .unwrap();

    let output = directory.path().join("audio.wav");
    let rendered = odaw()
        .args([
            "render",
            container.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
            "--duration",
            "0.05",
        ])
        .output()
        .unwrap();
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let bytes = std::fs::read(output).unwrap();
    assert!(bytes.len() > 44);
    assert!(bytes[44..].iter().any(|byte| *byte != 0));
}
