use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dto::{
    AssetDto, AutomationLaneDtoV4, ClipDto, EdgeDto, LoopRegionDto, MarkerDto, MetaDto,
    ModulationLinkDto, NodeDto, NodeRoleDto, ParamValueDto, PortDto, ProjectDto, SectionDto,
    SectionRefDto, TempoMapDto, TimeSignatureDto,
};

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFileV2 {
    pub format_version: u16,
    pub project: ProjectDtoV2,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectDtoV2 {
    pub meta: MetaDto,
    pub tempo: TempoMapDto,
    pub signatures: Vec<TimeSignatureDto>,
    pub markers: BTreeMap<u64, MarkerDto>,
    pub sections: BTreeMap<u64, SectionDto>,
    pub arrangement: Vec<SectionRefDto>,
    pub loops: Vec<LoopRegionDto>,
    pub nodes: BTreeMap<u64, NodeDtoV2>,
    pub edges: BTreeMap<u64, EdgeDto>,
    pub clips: BTreeMap<u64, ClipDto>,
    pub assets: BTreeMap<String, AssetDto>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeDtoV2 {
    pub role: NodeRoleDto,
    pub kind: Option<String>,
    pub rack: Vec<u64>,
    pub ports: Vec<PortDto>,
    pub params: BTreeMap<String, ParamValueDto>,
    pub automation: BTreeMap<String, AutomationLaneDtoV4>,
    pub modulations: BTreeMap<String, Vec<ModulationLinkDto>>,
    pub clips: Vec<u64>,
    pub state: Option<Vec<u8>>,
}

#[cfg(test)]
impl ProjectFileV2 {
    pub fn into_current(self) -> crate::project_file::ProjectFile {
        crate::project_file::ProjectFile {
            format_version: crate::project_file::FORMAT_VERSION,
            project: self.project.into(),
        }
    }
}

impl From<ProjectDtoV2> for ProjectDto {
    fn from(project: ProjectDtoV2) -> Self {
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
            clips: project.clips,
            assets: project.assets,
        }
    }
}

impl From<NodeDtoV2> for NodeDto {
    fn from(node: NodeDtoV2) -> Self {
        Self {
            role: node.role,
            kind: node.kind,
            rack: node.rack,
            ports: node.ports,
            params: node.params,
            automation: node
                .automation
                .into_iter()
                .map(|(id, lane)| (id, lane.into()))
                .collect(),
            modulations: node.modulations,
            clips: node.clips,
            state: node.state,
            name: None,
            color: None,
            muted: false,
            soloed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Container;
    use crate::dto::{ChannelLayoutDto, TempoPointDto, TimeSignatureDto};
    use crate::manifest::Manifest;
    use odaw_core::Project;
    use std::fs;
    use tempfile::TempDir;

    fn empty_project() -> ProjectDtoV2 {
        ProjectDtoV2 {
            meta: MetaDto {
                name: "v2".into(),
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
    fn v2_files_migrate_to_the_current_version() {
        let current = ProjectFileV2 {
            format_version: 2,
            project: empty_project(),
        }
        .into_current();
        assert_eq!(current.format_version, crate::project_file::FORMAT_VERSION);
        assert_eq!(current.project.meta.name, "v2");
    }

    #[test]
    fn v2_nodes_gain_default_track_metadata() {
        let node = NodeDtoV2 {
            role: NodeRoleDto::Track,
            kind: None,
            rack: Vec::new(),
            ports: vec![PortDto {
                id: 0,
                direction: crate::dto::PortDirectionDto::Output,
                layout: ChannelLayoutDto::Stereo,
            }],
            params: BTreeMap::new(),
            automation: BTreeMap::new(),
            modulations: BTreeMap::new(),
            clips: Vec::new(),
            state: None,
        };
        let current: NodeDto = node.into();
        assert_eq!(current.name, None);
        assert_eq!(current.color, None);
        assert!(!current.muted);
        assert!(!current.soloed);
    }

    #[test]
    fn a_v2_container_loads_and_migrates() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("legacy.odaw");
        let container = Container::new(&path);
        container
            .save(&Project::empty("current", 48_000, 0).unwrap())
            .unwrap();

        let mut manifest = Manifest::new();
        manifest.format_version = 2;
        fs::write(
            path.join(crate::manifest::MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let legacy = ProjectFileV2 {
            format_version: 2,
            project: empty_project(),
        };
        fs::write(
            path.join(crate::manifest::PROJECT_FILE),
            postcard::to_stdvec(&legacy).unwrap(),
        )
        .unwrap();

        assert_eq!(container.load().unwrap().meta.name, "v2");
    }
}
