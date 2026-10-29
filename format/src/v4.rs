use std::collections::BTreeMap;

use serde::Deserialize;

use crate::dto::{
    AssetDto, AutomationLaneDtoV4, ClipDto, EdgeDto, LoopRegionDto, MarkerDto, MetaDto, NodeDto,
    NodeRoleDto, ParamValueDto, PortDto, ProjectDto, SectionDto, SectionRefDto, TempoMapDto,
    TimeSignatureDto,
};

/// Version 4 is the current DTO without the automation `enabled` flag.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ProjectDtoV4 {
    pub meta: MetaDto,
    pub tempo: TempoMapDto,
    pub signatures: Vec<TimeSignatureDto>,
    pub markers: BTreeMap<u64, MarkerDto>,
    pub sections: BTreeMap<u64, SectionDto>,
    pub arrangement: Vec<SectionRefDto>,
    pub loops: Vec<LoopRegionDto>,
    pub nodes: BTreeMap<u64, NodeDtoV4>,
    pub track_order: Vec<u64>,
    pub edges: BTreeMap<u64, EdgeDto>,
    pub clips: BTreeMap<u64, ClipDto>,
    pub assets: BTreeMap<String, AssetDto>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct NodeDtoV4 {
    pub role: NodeRoleDto,
    pub kind: Option<String>,
    pub rack: Vec<u64>,
    pub ports: Vec<PortDto>,
    pub params: BTreeMap<String, ParamValueDto>,
    pub automation: BTreeMap<String, AutomationLaneDtoV4>,
    pub modulations: BTreeMap<String, Vec<crate::dto::ModulationLinkDto>>,
    pub clips: Vec<u64>,
    pub state: Option<Vec<u8>>,
    pub name: Option<String>,
    pub color: Option<u32>,
    pub muted: bool,
    pub soloed: bool,
}

impl From<ProjectDtoV4> for ProjectDto {
    fn from(project: ProjectDtoV4) -> Self {
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
            track_order: project.track_order,
            edges: project.edges,
            clips: project.clips,
            assets: project.assets,
        }
    }
}

impl From<NodeDtoV4> for NodeDto {
    fn from(node: NodeDtoV4) -> Self {
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
            name: node.name,
            color: node.color,
            muted: node.muted,
            soloed: node.soloed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{AutomationModeDto, CurveDto, KeyframeDto, ParamValueDto};
    use odaw_core::Tick;

    #[test]
    fn version_four_lanes_gain_enabled_by_default() {
        let node = NodeDtoV4 {
            role: NodeRoleDto::Track,
            kind: None,
            rack: Vec::new(),
            ports: Vec::new(),
            params: BTreeMap::new(),
            automation: BTreeMap::from([(
                "gain_db".to_owned(),
                AutomationLaneDtoV4 {
                    mode: AutomationModeDto::Absolute,
                    keyframes: vec![KeyframeDto {
                        tick: Tick::ZERO.raw(),
                        value: ParamValueDto::Float(0.0),
                        curve: CurveDto::Linear,
                    }],
                },
            )]),
            modulations: BTreeMap::new(),
            clips: Vec::new(),
            state: None,
            name: None,
            color: None,
            muted: false,
            soloed: false,
        };
        let current: NodeDto = node.into();
        assert!(current.automation["gain_db"].enabled);
    }
}
