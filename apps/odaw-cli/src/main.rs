use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use odaw_control::{RenameProject, Session, SessionError};
use odaw_core::{NodeRole, Project, ProjectError};
use odaw_format::{Container, ContainerError, ProjectFile};
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
    #[error("refusing to overwrite {path}; pass --force")]
    Exists { path: PathBuf },
    #[error("failed to encode JSON: {0}")]
    Json(#[from] serde_json::Error),
}
