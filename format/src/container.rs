use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use odaw_core::Project;
use thiserror::Error;

use crate::manifest::{ASSETS_DIR, CACHE_DIR, FORMAT_NAME, MANIFEST_FILE, Manifest, PROJECT_FILE};
use crate::project_file::{FORMAT_VERSION, FormatError, ProjectFile};

#[derive(Clone, Debug)]
pub struct Container {
    root: PathBuf,
}

impl Container {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read(&self) -> Result<ProjectFile, ContainerError> {
        self.recover()?;
        self.require_directory()?;

        let manifest = self.read_manifest()?;
        let project_path = self.root.join(&manifest.project);
        let bytes = read(&project_path)?;
        let (project_version, rest) = postcard::take_from_bytes::<u16>(&bytes)?;
        if project_version != manifest.format_version {
            return Err(ContainerError::VersionMismatch {
                manifest: manifest.format_version,
                project: project_version,
            });
        }
        let project = match project_version {
            1 => {
                let project: crate::v1::ProjectDtoV1 = postcard::from_bytes(rest)?;
                ProjectFile {
                    format_version: FORMAT_VERSION,
                    project: project.into(),
                }
            }
            2 => {
                let project: crate::v2::ProjectDtoV2 = postcard::from_bytes(rest)?;
                ProjectFile {
                    format_version: FORMAT_VERSION,
                    project: project.into(),
                }
            }
            3 => {
                let project: crate::v3::ProjectDtoV3 = postcard::from_bytes(rest)?;
                ProjectFile {
                    format_version: FORMAT_VERSION,
                    project: project.into(),
                }
            }
            4 => {
                let project: crate::v4::ProjectDtoV4 = postcard::from_bytes(rest)?;
                ProjectFile {
                    format_version: FORMAT_VERSION,
                    project: project.into(),
                }
            }
            FORMAT_VERSION => {
                let project: crate::dto::ProjectDto = postcard::from_bytes(rest)?;
                ProjectFile {
                    format_version: FORMAT_VERSION,
                    project,
                }
            }
            found => {
                return Err(ContainerError::UnsupportedVersion {
                    found,
                    supported: FORMAT_VERSION,
                });
            }
        };
        Ok(project)
    }

    pub fn load(&self) -> Result<Project, ContainerError> {
        Ok(self.read()?.into_project()?)
    }

    pub fn save(&self, project: &Project) -> Result<(), ContainerError> {
        self.recover()?;

        if let Some(parent) = self.root.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| ContainerError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }

        if self.root.exists() && !self.root.is_dir() {
            return Err(ContainerError::NotADirectory {
                path: self.root.clone(),
            });
        }
        self.ensure_replaceable()?;

        let temp = self.temp_path()?;
        let backup = self.backup_path()?;
        remove_path(&temp)?;
        remove_path(&backup)?;

        write_container(&temp, project)?;
        if self.root.is_dir() {
            copy_directory(&self.root.join(ASSETS_DIR), &temp.join(ASSETS_DIR))?;
        }

        let had_previous = self.root.exists();
        if had_previous {
            fs::rename(&self.root, &backup).map_err(|source| ContainerError::Io {
                path: self.root.clone(),
                source,
            })?;
        }

        if let Err(source) = fs::rename(&temp, &self.root) {
            if had_previous {
                fs::rename(&backup, &self.root).map_err(|source| ContainerError::Restore {
                    backup: backup.clone(),
                    source,
                })?;
            }
            return Err(ContainerError::Io {
                path: self.root.clone(),
                source,
            });
        }

        remove_path(&backup)?;
        Ok(())
    }

    fn ensure_replaceable(&self) -> Result<(), ContainerError> {
        if !self.root.is_dir() {
            return Ok(());
        }

        let mut entries = fs::read_dir(&self.root).map_err(|source| ContainerError::Io {
            path: self.root.clone(),
            source,
        })?;
        match entries.next() {
            None => Ok(()),
            Some(Ok(_)) if self.root.join(MANIFEST_FILE).is_file() => Ok(()),
            Some(Ok(_)) => Err(ContainerError::NotAContainer {
                path: self.root.clone(),
            }),
            Some(Err(source)) => Err(ContainerError::Io {
                path: self.root.clone(),
                source,
            }),
        }
    }

    fn require_directory(&self) -> Result<(), ContainerError> {
        match fs::metadata(&self.root) {
            Ok(metadata) if metadata.is_dir() => Ok(()),
            Ok(_) => Err(ContainerError::NotADirectory {
                path: self.root.clone(),
            }),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                Err(ContainerError::Missing {
                    path: self.root.clone(),
                })
            }
            Err(source) => Err(ContainerError::Io {
                path: self.root.clone(),
                source,
            }),
        }
    }

    fn read_manifest(&self) -> Result<Manifest, ContainerError> {
        let path = self.root.join(MANIFEST_FILE);
        let bytes = read(&path)?;
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|source| ContainerError::Manifest {
                path: path.clone(),
                source,
            })?;

        if manifest.format != FORMAT_NAME {
            return Err(ContainerError::UnknownFormat { path });
        }
        if manifest.format_version == 0 || manifest.format_version > FORMAT_VERSION {
            return Err(ContainerError::UnsupportedVersion {
                found: manifest.format_version,
                supported: FORMAT_VERSION,
            });
        }
        if manifest.project != PROJECT_FILE {
            return Err(ContainerError::UnsupportedProjectPath {
                path,
                project: manifest.project,
            });
        }
        Ok(manifest)
    }

    fn recover(&self) -> Result<(), ContainerError> {
        let temp = self.temp_path()?;
        let backup = self.backup_path()?;

        if !self.root.exists() && backup.exists() {
            fs::rename(&backup, &self.root).map_err(|source| ContainerError::Io {
                path: self.root.clone(),
                source,
            })?;
        } else if self.root.exists() && backup.exists() {
            remove_path(&backup)?;
        }

        remove_path(&temp)
    }

    fn temp_path(&self) -> Result<PathBuf, ContainerError> {
        self.sibling_path(".tmp")
    }

    fn backup_path(&self) -> Result<PathBuf, ContainerError> {
        self.sibling_path(".bak")
    }

    fn sibling_path(&self, suffix: &str) -> Result<PathBuf, ContainerError> {
        let name = self.root.file_name().ok_or(ContainerError::InvalidPath)?;
        let mut sibling = OsString::from(".");
        sibling.push(name);
        sibling.push(suffix);
        Ok(self.root.with_file_name(sibling))
    }
}

fn write_container(root: &Path, project: &Project) -> Result<(), ContainerError> {
    create_directory(root)?;
    create_directory(&root.join(ASSETS_DIR))?;
    create_directory(&root.join(CACHE_DIR))?;

    let file = ProjectFile::from_project(project);
    let bytes = postcard::to_stdvec(&file)?;
    write_synced(&root.join(PROJECT_FILE), &bytes)?;

    let manifest = serde_json::to_vec_pretty(&Manifest::new())?;
    write_synced(&root.join(MANIFEST_FILE), &manifest)?;
    Ok(())
}

fn create_directory(path: &Path) -> Result<(), ContainerError> {
    fs::create_dir_all(path).map_err(|source| ContainerError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), ContainerError> {
    if !source.is_dir() {
        return Ok(());
    }
    create_directory(destination)?;
    for entry in fs::read_dir(source).map_err(|error| ContainerError::Io {
        path: source.to_path_buf(),
        source: error,
    })? {
        let entry = entry.map_err(|error| ContainerError::Io {
            path: source.to_path_buf(),
            source: error,
        })?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(|source| ContainerError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            copy_directory(&path, &target)?;
        } else if file_type.is_file() {
            fs::copy(&path, &target).map_err(|source| ContainerError::Io {
                path: path.clone(),
                source,
            })?;
        } else {
            return Err(ContainerError::UnsupportedAssetEntry { path });
        }
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), ContainerError> {
    let mut file = File::create(path).map_err(|source| ContainerError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(bytes).map_err(|source| ContainerError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.sync_all().map_err(|source| ContainerError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read(path: &Path) -> Result<Vec<u8>, ContainerError> {
    fs::read(path).map_err(|source| ContainerError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn remove_path(path: &Path) -> Result<(), ContainerError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            fs::remove_dir_all(path).map_err(|source| ContainerError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
        Ok(_) => fs::remove_file(path).map_err(|source| ContainerError::Io {
            path: path.to_path_buf(),
            source,
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ContainerError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[derive(Debug, Error)]
pub enum ContainerError {
    #[error("container directory {path} does not exist")]
    Missing { path: PathBuf },
    #[error("container path {path} is not a directory")]
    NotADirectory { path: PathBuf },
    #[error("refusing to replace {path}: it is not an odaw container")]
    NotAContainer { path: PathBuf },
    #[error("failed to access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid manifest at {path}: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("manifest at {path} is not an odaw container")]
    UnknownFormat { path: PathBuf },
    #[error("unsupported container format version {found} (this build supports {supported})")]
    UnsupportedVersion { found: u16, supported: u16 },
    #[error("manifest at {path} points at unsupported project file {project:?}")]
    UnsupportedProjectPath { path: PathBuf, project: String },
    #[error("manifest format version {manifest} does not match project file version {project}")]
    VersionMismatch { manifest: u16, project: u16 },
    #[error("failed to encode or decode the project: {0}")]
    Codec(#[from] postcard::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Format(#[from] FormatError),
    #[error("container path has no file name")]
    InvalidPath,
    #[error("asset path {path} is not a regular file or directory")]
    UnsupportedAssetEntry { path: PathBuf },
    #[error("failed to restore the previous container from {backup}: {source}")]
    Restore {
        backup: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use odaw_core::{
        Asset, AssetId, AssetKind, AutomationLane, AutomationMode, ChannelLayout, Clip,
        ClipContent, ClipId, Curve, Edge, EdgeId, Frames, Keyframe, LoopRegion, Marker, MarkerId,
        ModulationLink, Node, NodeId, NodeKind, NodePort, NodeRole, ParamId, ParamValue, Port,
        PortId, Section, SectionId, SectionRef, Tick, TimeSignature,
    };
    use tempfile::TempDir;

    use super::*;

    fn sample_project() -> Project {
        let mut project = Project::empty("demo", 48_000, 42).unwrap();

        let track = NodeId::from_raw(1);
        let gain = NodeId::from_raw(2);
        let master = NodeId::from_raw(3);
        let modulator = NodeId::from_raw(4);
        let clip = ClipId::from_raw(5);
        let section = SectionId::from_raw(6);
        let asset = AssetId::from_bytes([3; 32]);

        project.nodes.insert(
            track,
            Node::new(NodeRole::Track)
                .with_name("Lead")
                .with_color(0x12_34_56)
                .soloed(true)
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
                .with_rack([gain])
                .with_param(ParamId::new("gain"), ParamValue::Float(0.8))
                .with_clips([clip]),
        );
        project.nodes.insert(
            gain,
            Node::device(NodeKind::from("builtin.gain")).with_state(vec![1, 2, 3]),
        );
        project.nodes.insert(
            master,
            Node::new(NodeRole::Master)
                .with_ports([Port::input(PortId::from_raw(0), ChannelLayout::Stereo)]),
        );
        project.nodes.insert(
            modulator,
            Node::new(NodeRole::Modulator)
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Mono)]),
        );

        project.track_order.push_back(track);
        project.nodes.get_mut(&track).unwrap().automation.insert(
            ParamId::new("gain"),
            AutomationLane {
                mode: AutomationMode::Trim,
                enabled: true,
                keyframes: [
                    Keyframe {
                        tick: Tick::ZERO,
                        value: ParamValue::Float(0.0),
                        curve: Curve::Hold,
                    },
                    Keyframe {
                        tick: Tick::from_raw(2 * odaw_core::PPQ),
                        value: ParamValue::Float(0.5),
                        curve: Curve::Linear,
                    },
                ]
                .into_iter()
                .collect(),
            },
        );
        project.nodes.get_mut(&track).unwrap().modulations.insert(
            ParamId::new("gain"),
            [ModulationLink {
                source: modulator,
                amount: 0.2,
            }]
            .into_iter()
            .collect(),
        );

        project.edges.insert(
            EdgeId::from_raw(1),
            Edge {
                from: NodePort {
                    node: track,
                    port: PortId::from_raw(0),
                },
                to: NodePort {
                    node: master,
                    port: PortId::from_raw(0),
                },
            },
        );

        project.assets.insert(
            asset,
            Asset {
                name: "kick.wav".into(),
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
                    source_length: Frames::from_raw(48_000),
                },
            },
        );
        project.sections.insert(
            section,
            Section {
                name: "intro".into(),
                length: Tick::from_raw(4 * odaw_core::PPQ),
                clips: [clip].into_iter().collect(),
            },
        );
        project.arrangement.push_back(SectionRef {
            section,
            repeat: 2,
            active: true,
        });
        project.markers.insert(
            MarkerId::from_raw(7),
            Marker {
                name: "start".into(),
                tick: Tick::ZERO,
            },
        );
        project.loops.push_back(LoopRegion {
            start: Tick::ZERO,
            end: Tick::from_raw(4 * odaw_core::PPQ),
        });

        project.signatures.push_back(TimeSignature {
            tick: Tick::from_raw(4 * odaw_core::PPQ),
            numerator: 3,
            denominator: 4,
        });

        project
    }

    fn temp_container(directory: &TempDir) -> Container {
        Container::new(directory.path().join("song.odaw"))
    }

    #[test]
    fn save_and_load_round_trip() {
        let directory = TempDir::new().unwrap();
        let container = temp_container(&directory);
        let project = sample_project();

        container.save(&project).unwrap();
        let loaded = container.load().unwrap();

        assert_eq!(loaded, project);
        assert!(container.root().join(ASSETS_DIR).is_dir());
        assert!(container.root().join(CACHE_DIR).is_dir());
        assert!(container.root().join(MANIFEST_FILE).is_file());
        assert!(container.root().join(PROJECT_FILE).is_file());
    }

    #[test]
    fn save_replaces_an_existing_container() {
        let directory = TempDir::new().unwrap();
        let container = temp_container(&directory);
        let mut project = Project::empty("first", 48_000, 0).unwrap();

        container.save(&project).unwrap();
        project.meta.name = "second".into();
        container.save(&project).unwrap();

        assert_eq!(container.load().unwrap().meta.name, "second");
        assert!(!container.backup_path().unwrap().exists());
    }

    #[test]
    fn save_preserves_asset_blobs() {
        let directory = TempDir::new().unwrap();
        let container = temp_container(&directory);
        let project = Project::empty("demo", 48_000, 0).unwrap();
        container.save(&project).unwrap();

        let blob = container.root().join(ASSETS_DIR).join("blob.bin");
        fs::write(&blob, b"asset bytes").unwrap();
        container.save(&project).unwrap();

        assert_eq!(fs::read(&blob).unwrap(), b"asset bytes");
    }

    #[test]
    fn load_recovers_an_interrupted_save() {
        let directory = TempDir::new().unwrap();
        let container = temp_container(&directory);
        let project = sample_project();
        container.save(&project).unwrap();

        fs::rename(container.root(), container.backup_path().unwrap()).unwrap();
        assert!(!container.root().exists());

        let loaded = container.load().unwrap();
        assert_eq!(loaded, project);
        assert!(container.root().exists());
        assert!(!container.backup_path().unwrap().exists());
    }

    #[test]
    fn missing_container_is_reported() {
        let directory = TempDir::new().unwrap();
        let container = temp_container(&directory);
        assert!(matches!(
            container.load(),
            Err(ContainerError::Missing { .. })
        ));
    }

    #[test]
    fn unsupported_manifest_version_is_rejected() {
        let directory = TempDir::new().unwrap();
        let container = temp_container(&directory);
        container
            .save(&Project::empty("demo", 48_000, 0).unwrap())
            .unwrap();

        let manifest = Manifest {
            format_version: FORMAT_VERSION + 1,
            ..Manifest::new()
        };
        fs::write(
            container.root().join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        assert!(matches!(
            container.load(),
            Err(ContainerError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn corrupt_project_file_is_rejected() {
        let directory = TempDir::new().unwrap();
        let container = temp_container(&directory);
        container
            .save(&Project::empty("demo", 48_000, 0).unwrap())
            .unwrap();
        fs::write(container.root().join(PROJECT_FILE), b"").unwrap();

        assert!(matches!(container.load(), Err(ContainerError::Codec(_))));
    }

    #[test]
    fn save_rejects_a_file_target() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("song.odaw");
        fs::write(&path, b"not a directory").unwrap();
        let container = Container::new(path);

        assert!(matches!(
            container.save(&Project::empty("demo", 48_000, 0).unwrap()),
            Err(ContainerError::NotADirectory { .. })
        ));
    }

    #[test]
    fn save_refuses_to_clobber_a_non_container_directory() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("song.odaw");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("notes.txt"), b"do not delete").unwrap();
        let container = Container::new(path);

        assert!(matches!(
            container.save(&Project::empty("demo", 48_000, 0).unwrap()),
            Err(ContainerError::NotAContainer { .. })
        ));
        assert!(container.root().join("notes.txt").is_file());
    }

    #[test]
    fn manifest_and_project_versions_must_match() {
        let directory = TempDir::new().unwrap();
        let container = temp_container(&directory);
        container
            .save(&Project::empty("demo", 48_000, 0).unwrap())
            .unwrap();

        let bytes = fs::read(container.root().join(PROJECT_FILE)).unwrap();
        let mut file: ProjectFile = postcard::from_bytes(&bytes).unwrap();
        file.format_version = FORMAT_VERSION + 1;
        fs::write(
            container.root().join(PROJECT_FILE),
            postcard::to_stdvec(&file).unwrap(),
        )
        .unwrap();

        assert!(matches!(
            container.load(),
            Err(ContainerError::VersionMismatch { .. })
        ));
    }
}
