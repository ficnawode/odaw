use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use odaw_control::{RenameProject, Session, SessionError};
use odaw_core::{NodeRole, Project, ProjectError, TempoError, Tick};
use odaw_dsp::demo;
use odaw_format::{Container, ContainerError, ProjectFile};
use odaw_io::{WavError, WavSpec, write_wav};
use odaw_render::{RenderError, RenderOptions, render_project};
use thiserror::Error;

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
    /// Check that a project container loads and validates.
    Validate { path: PathBuf },
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
        Command::Render {
            path,
            output,
            block_size,
            duration,
        } => render_project_file(&path, &output, block_size, duration),
        Command::Validate { path } => validate_project(&path),
    }
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

fn render_project_file(
    path: &Path,
    output: &Path,
    block_size: std::num::NonZeroUsize,
    duration: Option<f64>,
) -> Result<(), CliError> {
    let project = Container::new(path).load()?;
    let frames = render_frames(&project, duration)?;
    let audio = render_project(&project, RenderOptions { block_size, frames })?;
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

fn render_frames(project: &Project, duration: Option<f64>) -> Result<u64, CliError> {
    if let Some(seconds) = duration {
        if !seconds.is_finite() || seconds < 0.0 || seconds > u64::MAX as f64 {
            return Err(CliError::InvalidDuration(seconds));
        }
        return Ok((seconds * f64::from(project.meta.sample_rate)).round() as u64);
    }

    if let Some(tick) = project_end_tick(project) {
        let frames = project.tempo.tick_to_frames(tick)?;
        return Ok(u64::try_from(frames.raw().max(0)).expect("frames are non-negative"));
    }

    Ok(u64::from(project.meta.sample_rate) * 5)
}

fn project_end_tick(project: &Project) -> Option<Tick> {
    let mut end: Option<Tick> = None;
    for loop_region in &project.loops {
        end = Some(end.map_or(loop_region.end, |current| current.max(loop_region.end)));
    }
    for clip in project.clips.values() {
        let clip_end = Tick::from_raw(clip.start.raw().saturating_add(clip.length.raw()));
        end = Some(end.map_or(clip_end, |current| current.max(clip_end)));
    }
    end.filter(|tick| *tick > Tick::ZERO)
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
    Wav(#[from] WavError),
    #[error(transparent)]
    Time(#[from] TempoError),
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
}
