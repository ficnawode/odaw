use std::collections::BTreeMap;

use serde::Deserialize;

use crate::dto::{
    AssetDto, ClipDto, EdgeDto, LoopRegionDto, MarkerDto, MetaDto, SectionDto, SectionRefDto,
    TempoMapDto, TimeSignatureDto,
};
use crate::v4::NodeDtoV4;

/// Version 3 had no explicit track order. It is otherwise identical to the
/// version 4 DTO, so reuse the sub-DTOs and only supply the display order.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ProjectDtoV3 {
    pub meta: MetaDto,
    pub tempo: TempoMapDto,
    pub signatures: Vec<TimeSignatureDto>,
    pub markers: BTreeMap<u64, MarkerDto>,
    pub sections: BTreeMap<u64, SectionDto>,
    pub arrangement: Vec<SectionRefDto>,
    pub loops: Vec<LoopRegionDto>,
    pub nodes: BTreeMap<u64, NodeDtoV4>,
    pub edges: BTreeMap<u64, EdgeDto>,
    pub clips: BTreeMap<u64, ClipDto>,
    pub assets: BTreeMap<String, AssetDto>,
}

impl From<ProjectDtoV3> for crate::dto::ProjectDto {
    fn from(project: ProjectDtoV3) -> Self {
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
