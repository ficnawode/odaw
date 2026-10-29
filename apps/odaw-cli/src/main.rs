use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::num::NonZeroU32;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use odaw_compile::{CompileError, Compiler};
use odaw_control::{
    AutomationCommand as AutomationEdit, Command as ControlCommand,
    CommandKind as ControlCommandKind, RenameProject, Session, SessionError,
};
use odaw_core::{
    AssetId, AssetKind, AutomationLane, AutomationMode, Curve, Keyframe, NodeRole, ParamId,
    ParamValue, Project, ProjectError, TempoError, Tick,
};
use odaw_dsp::{AudioAsset, AudioSources, demo};
use odaw_engine::{RealtimeEngine, RealtimeError, RealtimeState, RealtimeStateError, realtime};
use odaw_format::{Container, ContainerError, ProjectFile};
use odaw_io::{WavError, WavSpec, write_wav};
use odaw_platform::{AudioBackend, AudioCallback, AudioError};
use odaw_render::{RenderError, RenderOptions, project_end_frame, render_project_with_compiler};
use odaw_vst3::{Catalog, Vst3Backend};
use thiserror::Error;

mod dexed_demo;

#[derive(Debug, Parser)]
#[command(
    name = "odaw",
    version,
    about = "A deterministic, scriptable digital audio workstation"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new project container.
    New {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "48000")]
        sample_rate: NonZeroU32,
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long)]
        force: bool,
    },
    /// Print a summary of a project container.
    Inspect {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Rename a project through the operator/session path.
    Rename { path: PathBuf, name: String },
    /// Create a demo project with a sine instrument and gain stage.
    Demo {
        path: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Create a layered Dexed project playing Ode to Joy, with reverb sends.
    DemoDexed {
        path: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Create the Dexed arrangement with automation on synth, reverb, pan, and master parameters.
    DemoAutomation {
        path: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Render a project to a 16-bit PCM WAV file.
    Render {
        path: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value = "512")]
        block_size: std::num::NonZeroUsize,
        /// Render length in seconds. Defaults to the project's loop or clip end.
        #[arg(long, allow_hyphen_values = true)]
        duration: Option<f64>,
    },
    /// Play a project through the default audio output device.
    Play {
        path: PathBuf,
        /// Play length in seconds. Defaults to the project's loop or clip end.
        #[arg(long, allow_hyphen_values = true)]
        duration: Option<f64>,
    },
    /// Check that a project container loads and validates.
    Validate { path: PathBuf },
    /// List installed VST3 plugins, scanning again with --refresh.
    Plugins {
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// Inspect or edit parameter automation.
    Automation {
        #[command(subcommand)]
        command: AutomationCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AutomationCommand {
    /// Print every automation lane in a project.
    List { path: PathBuf },
    /// Add a keyframe, creating the lane when necessary.
    Set {
        path: PathBuf,
        #[arg(long)]
        node: u64,
        #[arg(long)]
        param: String,
        #[arg(long)]
        tick: i64,
        #[arg(long, allow_hyphen_values = true)]
        value: f64,
        #[arg(long)]
        trim: bool,
        #[arg(long)]
        hold: bool,
    },
    /// Remove a lane.
    Clear {
        path: PathBuf,
        #[arg(long)]
        node: u64,
        #[arg(long)]
        param: String,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("odaw: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), CliError> {
    match Cli::parse().command {
        Command::New {
            path,
            name,
            sample_rate,
            seed,
            force,
        } => create_project(&path, name, sample_rate.get(), seed, force),
        Command::Inspect { path, json } => inspect_project(&path, json),
        Command::Rename { path, name } => rename_project(&path, &name),
        Command::Demo { path, force } => create_demo(&path, force),
        Command::DemoDexed { path, force } => {
            dexed_demo::create(&path, force).map_err(CliError::Demo)
        }
        Command::DemoAutomation { path, force } => {
            dexed_demo::create_automation(&path, force).map_err(CliError::Demo)
        }
        Command::Render {
            path,
            output,
            block_size,
            duration,
        } => render_project_file(&path, &output, block_size, duration),
        Command::Play { path, duration } => play_project(&path, duration),
        Command::Validate { path } => validate_project(&path),
        Command::Plugins { refresh, json } => list_plugins(refresh, json),
        Command::Automation { command } => automation(command),
    }
}

fn automation(command: AutomationCommand) -> Result<(), CliError> {
    match command {
        AutomationCommand::List { path } => {
            let project = Container::new(&path).load()?;
            for (id, node) in &project.nodes {
                for (param, lane) in &node.automation {
                    let name = node
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("node {}", id.raw()));
                    println!(
                        "{name} ({}) {param} [{}]{}",
                        id.raw(),
                        match lane.mode {
                            AutomationMode::Absolute => "absolute",
                            AutomationMode::Trim => "trim",
                        },
                        if lane.enabled { "" } else { " disabled" }
                    );
                    for point in &lane.keyframes {
                        println!(
                            "  tick {}  {:?}  {:?}",
                            point.tick.raw(),
                            point.value,
                            point.curve
                        );
                    }
                }
            }
            Ok(())
        }
        AutomationCommand::Set {
            path,
            node,
            param,
            tick,
            value,
            trim,
            hold,
        } => {
            let project = Container::new(&path).load()?;
            let node_id = odaw_core::NodeId::from_raw(node);
            let param = ParamId::new(param);
            let existing = project
                .nodes
                .get(&node_id)
                .and_then(|node| node.automation.get(&param).cloned());
            let value = existing
                .as_ref()
                .and_then(|lane| lane.keyframes.front())
                .map_or(ParamValue::Float(value), |point| match point.value {
                    ParamValue::Float(_) => ParamValue::Float(value),
                    ParamValue::Int(_) => ParamValue::Int(value.round() as i64),
                    ParamValue::Bool(_) => ParamValue::Bool(value >= 0.5),
                    ParamValue::Enum(_) => ParamValue::Enum(value.max(0.0).round() as u32),
                });
            let mut lane = existing.unwrap_or(AutomationLane {
                mode: if trim {
                    AutomationMode::Trim
                } else {
                    AutomationMode::Absolute
                },
                enabled: true,
                keyframes: [].into_iter().collect(),
            });
            lane.mode = if trim {
                AutomationMode::Trim
            } else {
                AutomationMode::Absolute
            };
            lane.keyframes.push_back(Keyframe {
                tick: Tick::from_raw(tick),
                value,
                curve: if hold { Curve::Hold } else { Curve::Linear },
            });
            let mut session = Session::new(project)?;
            session.apply_command(ControlCommand::new(ControlCommandKind::Automation(
                AutomationEdit::Set {
                    node: node_id,
                    param,
                    lane: Some(lane),
                },
            )))?;
            Container::new(&path).save(session.project())?;
            println!("updated automation on {}", path.display());
            Ok(())
        }
        AutomationCommand::Clear { path, node, param } => {
            let project = Container::new(&path).load()?;
            let mut session = Session::new(project)?;
            session.apply_command(ControlCommand::new(ControlCommandKind::Automation(
                AutomationEdit::Set {
                    node: odaw_core::NodeId::from_raw(node),
                    param: ParamId::new(param),
                    lane: None,
                },
            )))?;
            Container::new(&path).save(session.project())?;
            println!("cleared automation on {}", path.display());
            Ok(())
        }
    }
}

fn list_plugins(refresh: bool, json: bool) -> Result<(), CliError> {
    let catalog = if refresh {
        let catalog = Catalog::scan();
        catalog.save()?;
        catalog
    } else {
        Catalog::load_or_empty()
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&catalog.entries)?);
        return Ok(());
    }
    if catalog.entries.is_empty() {
        println!("no VST3 plugins found; pass --refresh to scan");
        return Ok(());
    }
    for entry in &catalog.entries {
        println!(
            "{}  {}  {}  [{}]  {}",
            entry.kind(),
            entry.name,
            entry.vendor,
            if entry.is_instrument() {
                "instrument"
            } else {
                "effect"
            },
            entry.path.display()
        );
    }
    Ok(())
}

fn create_project(
    path: &Path,
    name: Option<String>,
    sample_rate: u32,
    seed: u64,
    force: bool,
) -> Result<(), CliError> {
    if path.exists() && !force {
        return Err(CliError::Exists {
            path: path.to_path_buf(),
        });
    }

    let name = name.unwrap_or_else(|| project_name(path));
    let project = Project::empty(name, sample_rate, seed)?;
    Container::new(path).save(&project)?;
    println!("created {}", path.display());
    Ok(())
}

fn inspect_project(path: &Path, json: bool) -> Result<(), CliError> {
    let project = Container::new(path).load()?;

    if json {
        let file = ProjectFile::from_project(&project);
        println!("{}", serde_json::to_string_pretty(&file)?);
        return Ok(());
    }

    let tracks = project
        .nodes
        .iter()
        .filter(|(_, node)| node.role == NodeRole::Track)
        .count();

    println!("name: {}", project.meta.name);
    println!("format: {}", odaw_format::FORMAT_VERSION);
    println!("sample rate: {}", project.meta.sample_rate);
    println!("seed: {}", project.meta.seed);
    println!("tempo points: {}", project.tempo.len());
    println!("signatures: {}", project.signatures.len());
    println!("tracks: {tracks}");
    println!("nodes: {}", project.nodes.len());
    println!("edges: {}", project.edges.len());
    println!("clips: {}", project.clips.len());
    println!("assets: {}", project.assets.len());
    println!("sections: {}", project.sections.len());
    println!("markers: {}", project.markers.len());
    println!("loops: {}", project.loops.len());
    Ok(())
}

fn validate_project(path: &Path) -> Result<(), CliError> {
    Container::new(path).load()?;
    println!("ok {}", path.display());
    Ok(())
}

fn rename_project(path: &Path, name: &str) -> Result<(), CliError> {
    let project = Container::new(path).load()?;
    let mut session = Session::new(project)?;
    session.apply(&RenameProject::new(name))?;
    Container::new(path).save(session.project())?;
    println!("renamed {} to {name}", path.display());
    Ok(())
}

fn create_demo(path: &Path, force: bool) -> Result<(), CliError> {
    if path.exists() && !force {
        return Err(CliError::Exists {
            path: path.to_path_buf(),
        });
    }
    let project = demo::project();
    Container::new(path).save(&project)?;
    println!("created demo {}", path.display());
    Ok(())
}

struct Player {
    engine: RealtimeEngine,
}

impl AudioCallback for Player {
    fn process(&mut self, output: &mut [f32], channels: usize, sample_rate: u32) {
        self.engine.process(output, channels, sample_rate);
    }
}

fn play_project(path: &Path, duration: Option<f64>) -> Result<(), CliError> {
    let container = Container::new(path);
    let project = container.load()?;
    let sources = load_audio_sources(container.root(), &project)?;
    let frames = render_frames(&project, duration)?;
    let compiled = plugin_compiler(&project, 8192).compile_with_audio(&project, &sources)?;
    let sample_rate = compiled.plan.sample_rate;
    let channels = u16::try_from(compiled.plan.channels)
        .map_err(|_| CliError::UnsupportedChannels(compiled.plan.channels))?;
    let block_size = NonZeroUsize::new(8192).expect("the realtime block size is non-zero");
    let state = RealtimeState::new(compiled, &project, block_size, Some(frames))?;
    let (engine, mut handle) = realtime(state, 8);
    handle.play()?;
    let backend = AudioBackend::start(Player { engine }, sample_rate, channels)?;

    while !handle.finished() {
        if !backend.is_healthy() {
            return Err(CliError::AudioStreamFailed);
        }
        handle.collect_retired();
        std::thread::sleep(Duration::from_millis(20));
    }

    println!(
        "played {} frames at {} Hz through {:?}",
        handle.position(),
        sample_rate,
        backend.sample_format()
    );
    Ok(())
}

fn render_project_file(
    path: &Path,
    output: &Path,
    block_size: std::num::NonZeroUsize,
    duration: Option<f64>,
) -> Result<(), CliError> {
    let container = Container::new(path);
    let project = container.load()?;
    let sources = load_audio_sources(container.root(), &project)?;
    let frames = render_frames(&project, duration)?;
    let audio = render_project_with_compiler(
        &project,
        RenderOptions { block_size, frames },
        &sources,
        &plugin_compiler(&project, block_size.get()),
    )?;
    let channels =
        u16::try_from(audio.channels).map_err(|_| CliError::UnsupportedChannels(audio.channels))?;

    let file = File::create(output).map_err(|source| CliError::Io {
        path: output.to_path_buf(),
        source,
    })?;
    let mut writer = BufWriter::new(file);
    write_wav(
        &mut writer,
        WavSpec {
            sample_rate: audio.sample_rate,
            channels,
        },
        &audio.samples,
    )?;
    writer.flush().map_err(|source| CliError::Io {
        path: output.to_path_buf(),
        source,
    })?;

    println!(
        "rendered {} frames at {} Hz to {}",
        frames,
        audio.sample_rate,
        output.display()
    );
    Ok(())
}

fn plugin_compiler(project: &Project, block_size: usize) -> Compiler {
    let mut backend = Vst3Backend::load_cached();
    let needs_scan = backend.catalog().entries.is_empty()
        && project.nodes.values().any(|node| {
            node.kind
                .as_ref()
                .is_some_and(|kind| kind.as_str().starts_with(odaw_vst3::KIND_PREFIX))
        });
    if needs_scan {
        let catalog = Catalog::scan();
        let _ = catalog.save();
        backend.set_catalog(catalog);
    }
    Compiler::builtin()
        .with_factory(backend.factory())
        .with_max_block_size(block_size)
}

fn load_audio_sources(root: &Path, project: &Project) -> Result<AudioSources, CliError> {
    let mut sources = AudioSources::new();
    let directory = root.join("assets");
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Ok(sources);
    };
    let files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    let assets: Vec<AssetId> = project
        .assets
        .iter()
        .filter(|(_, asset)| asset.kind == AssetKind::Audio)
        .map(|(id, _)| *id)
        .collect();
    for id in assets {
        let hex = id.to_hex();
        let Some(path) = files
            .iter()
            .find(|path| path.file_stem().is_some_and(|stem| stem == hex.as_str()))
        else {
            continue;
        };
        let decoded = odaw_io::decode_peaks(path, 1).map_err(|source| CliError::Decode {
            path: path.clone(),
            source,
        })?;
        if let Some(asset) = AudioAsset::new(
            decoded.sample_rate,
            decoded.channels,
            decoded.samples.into(),
        ) {
            sources.insert(id, std::sync::Arc::new(asset));
        }
    }
    Ok(sources)
}

fn render_frames(project: &Project, duration: Option<f64>) -> Result<u64, CliError> {
    if let Some(seconds) = duration {
        if !seconds.is_finite() || seconds < 0.0 || seconds > u64::MAX as f64 {
            return Err(CliError::InvalidDuration(seconds));
        }
        return Ok((seconds * f64::from(project.meta.sample_rate)).round() as u64);
    }

    if let Some(frames) = project_end_frame(project)? {
        return Ok(frames);
    }

    Ok(u64::from(project.meta.sample_rate) * 5)
}

fn project_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("untitled")
        .to_owned()
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Container(#[from] ContainerError),
    #[error(transparent)]
    Control(#[from] SessionError),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Realtime(#[from] RealtimeError),
    #[error(transparent)]
    RealtimeState(#[from] RealtimeStateError),
    #[error("the audio output stream failed")]
    AudioStreamFailed,
    #[error(transparent)]
    Wav(#[from] WavError),
    #[error(transparent)]
    Time(#[from] TempoError),
    #[error("failed to decode {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: odaw_io::DecodeError,
    },
    #[error("refusing to overwrite {path}; pass --force")]
    Exists { path: PathBuf },
    #[error("invalid render duration {0}")]
    InvalidDuration(f64),
    #[error("unsupported channel count {0}")]
    UnsupportedChannels(usize),
    #[error("failed to write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to encode JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Catalog(#[from] odaw_vst3::catalog::CatalogError),
    #[error("{0}")]
    Demo(String),
}
