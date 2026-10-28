use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dto::{
    AssetDto, ClipContentDto, ClipDto, EdgeDto, LoopRegionDto, MarkerDto, MetaDto, ProjectDto,
    SectionDto, SectionRefDto, TempoMapDto, TimeSignatureDto,
};
use crate::v2::NodeDtoV2;

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFileV1 {
    pub format_version: u16,
    pub project: ProjectDtoV1,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectDtoV1 {
    pub meta: MetaDto,
    pub tempo: TempoMapDto,
    pub signatures: Vec<TimeSignatureDto>,
    pub markers: BTreeMap<u64, MarkerDto>,
    pub sections: BTreeMap<u64, SectionDto>,
    pub arrangement: Vec<SectionRefDto>,
    pub loops: Vec<LoopRegionDto>,
    pub nodes: BTreeMap<u64, NodeDtoV2>,
    pub edges: BTreeMap<u64, EdgeDto>,
    pub clips: BTreeMap<u64, ClipDtoV1>,
    pub assets: BTreeMap<String, AssetDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipDtoV1 {
    pub node: u64,
    pub start: i64,
    pub length: i64,
    pub content: ClipContentDtoV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipContentDtoV1 {
    Audio {
        asset: String,
        source_start: i64,
        source_length: i64,
    },
    Midi {
        source_start: i64,
        source_length: i64,
    },
    Empty,
}

#[cfg(test)]
impl ProjectFileV1 {
    pub fn into_current(self) -> crate::project_file::ProjectFile {
        crate::project_file::ProjectFile {
            format_version: crate::project_file::FORMAT_VERSION,
            project: self.project.into(),
        }
    }
}

impl From<ProjectDtoV1> for ProjectDto {
    fn from(project: ProjectDtoV1) -> Self {
        Self {
            meta: project.meta,
            tempo: project.tempo,
            signatures: project.signatures,
            markers: project.markers,
            sections: project.sections,
            arrangement: project.arrangement,
            loops: project.loops,
            nodes: project
                .nodes
                .into_iter()
                .map(|(id, node)| (id, node.into()))
                .collect(),
            track_order: Vec::new(),
            edges: project.edges,
            clips: project
                .clips
                .into_iter()
                .map(|(id, clip)| (id, clip.into()))
                .collect(),
            assets: project.assets,
        }
    }
}

impl From<ClipDtoV1> for ClipDto {
    fn from(clip: ClipDtoV1) -> Self {
        Self {
            node: clip.node,
            start: clip.start,
            length: clip.length,
            content: match clip.content {
                ClipContentDtoV1::Audio {
                    asset,
                    source_start,
                    source_length,
                } => ClipContentDto::Audio {
                    asset,
                    source_start,
                    source_length,
                },
                ClipContentDtoV1::Midi {
                    source_start,
                    source_length,
                } => ClipContentDto::Midi {
                    source_start,
                    source_length,
                    notes: Vec::new(),
                },
                ClipContentDtoV1::Empty => ClipContentDto::Empty,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{
        ChannelLayoutDto, NodeRoleDto, PortDirectionDto, PortDto, TempoPointDto, TimeSignatureDto,
    };

    fn empty_project() -> ProjectDtoV1 {
        ProjectDtoV1 {
            meta: MetaDto {
                name: "v1".into(),
                sample_rate: 48_000,
                seed: 0,
            },
            tempo: TempoMapDto {
                sample_rate: 48_000,
                points: vec![TempoPointDto {
                    tick: 0,
                    micros_per_quarter: 500_000,
                }],
            },
            signatures: vec![TimeSignatureDto {
                tick: 0,
                numerator: 4,
                denominator: 4,
            }],
            markers: BTreeMap::new(),
            sections: BTreeMap::new(),
            arrangement: Vec::new(),
            loops: Vec::new(),
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            clips: BTreeMap::new(),
            assets: BTreeMap::new(),
        }
    }

    #[test]
    fn v1_files_migrate_to_the_current_version() {
        let file = ProjectFileV1 {
            format_version: 1,
            project: empty_project(),
        };
        let current = file.into_current();
        assert_eq!(current.format_version, crate::project_file::FORMAT_VERSION);
        assert_eq!(current.project.meta.name, "v1");
    }

    #[test]
    fn v1_midi_clips_gain_an_empty_note_list() {
        let mut project = empty_project();
        project.nodes.insert(
            1,
            NodeDtoV2 {
                role: NodeRoleDto::Track,
                kind: None,
                rack: Vec::new(),
                ports: vec![PortDto {
                    id: 0,
                    direction: PortDirectionDto::Output,
                    layout: ChannelLayoutDto::Stereo,
                }],
                params: BTreeMap::new(),
                automation: BTreeMap::new(),
                modulations: BTreeMap::new(),
                clips: vec![2],
                state: None,
            },
        );
        project.clips.insert(
            2,
            ClipDtoV1 {
                node: 1,
                start: 0,
                length: 1_000,
                content: ClipContentDtoV1::Midi {
                    source_start: 0,
                    source_length: 1_000,
                },
            },
        );

        let current: ProjectDto = project.into();
        assert_eq!(
            current.clips[&2].content,
            ClipContentDto::Midi {
                source_start: 0,
                source_length: 1_000,
                notes: Vec::new(),
            }
        );
        assert_eq!(current.clips[&2].node, 1);
    }

    #[test]
    fn a_v1_container_loads_and_migrates() {
        use std::fs;

        use tempfile::TempDir;

        use crate::Container;
        use crate::manifest::{MANIFEST_FILE, Manifest, PROJECT_FILE};
        use odaw_core::Project;

        let directory = TempDir::new().unwrap();
        let path = directory.path().join("legacy.odaw");
        let container = Container::new(&path);
        container
            .save(&Project::empty("current", 48_000, 0).unwrap())
            .unwrap();

        let mut manifest = Manifest::new();
        manifest.format_version = 1;
        fs::write(
            path.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let legacy = ProjectFileV1 {
            format_version: 1,
            project: empty_project(),
        };
        fs::write(
            path.join(PROJECT_FILE),
            postcard::to_stdvec(&legacy).unwrap(),
        )
        .unwrap();

        let file = container.read().unwrap();
        assert_eq!(file.format_version, crate::project_file::FORMAT_VERSION);
        assert_eq!(file.project.meta.name, "v1");
        assert_eq!(container.load().unwrap().meta.name, "v1");
    }
}
