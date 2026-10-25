use std::collections::BTreeMap;

use odaw_core::{
    Asset, AssetId, AssetKind, AutomationLane, AutomationMode, ChannelLayout, Clip, ClipContent,
    ClipId, Curve, Edge, EdgeId, Frames, Keyframe, LoopRegion, Marker, MarkerId, Meta,
    ModulationLink, Node, NodeId, NodeKind, NodePort, NodeRole, Note, NoteId, ParamId, ParamValue,
    Port, PortDirection, PortId, Project, Section, SectionId, SectionRef, TempoMap, TempoPoint,
    Tick, TimeSignature,
};
use odaw_core::{PersistentMap, PersistentVector};
use serde::{Deserialize, Serialize};

use crate::project_file::FormatError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectDto {
    pub meta: MetaDto,
    pub tempo: TempoMapDto,
    pub signatures: Vec<TimeSignatureDto>,
    pub markers: BTreeMap<u64, MarkerDto>,
    pub sections: BTreeMap<u64, SectionDto>,
    pub arrangement: Vec<SectionRefDto>,
    pub loops: Vec<LoopRegionDto>,
    pub nodes: BTreeMap<u64, NodeDto>,
    pub edges: BTreeMap<u64, EdgeDto>,
    pub clips: BTreeMap<u64, ClipDto>,
    pub assets: BTreeMap<String, AssetDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaDto {
    pub name: String,
    pub sample_rate: u32,
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempoMapDto {
    pub sample_rate: u32,
    pub points: Vec<TempoPointDto>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempoPointDto {
    pub tick: i64,
    pub micros_per_quarter: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSignatureDto {
    pub tick: i64,
    pub numerator: u16,
    pub denominator: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerDto {
    pub name: String,
    pub tick: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRegionDto {
    pub start: i64,
    pub end: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionRefDto {
    pub section: u64,
    pub repeat: u32,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionDto {
    pub name: String,
    pub length: i64,
    pub clips: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeDto {
    pub role: NodeRoleDto,
    pub kind: Option<String>,
    pub rack: Vec<u64>,
    pub ports: Vec<PortDto>,
    pub params: BTreeMap<String, ParamValueDto>,
    pub automation: BTreeMap<String, AutomationLaneDto>,
    pub modulations: BTreeMap<String, Vec<ModulationLinkDto>>,
    pub clips: Vec<u64>,
    pub state: Option<Vec<u8>>,
    pub name: Option<String>,
    pub color: Option<u32>,
    pub muted: bool,
    pub soloed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRoleDto {
    Track,
    Bus,
    Master,
    Send,
    Folder,
    Input,
    Output,
    Modulator,
    Device,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortDto {
    pub id: u32,
    pub direction: PortDirectionDto,
    pub layout: ChannelLayoutDto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortDirectionDto {
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelLayoutDto {
    Mono,
    Stereo,
    Channels(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamValueDto {
    Float(f64),
    Int(i64),
    Bool(bool),
    Enum(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutomationModeDto {
    Absolute,
    Trim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurveDto {
    Hold,
    Linear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyframeDto {
    pub tick: i64,
    pub value: ParamValueDto,
    pub curve: CurveDto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationLaneDto {
    pub mode: AutomationModeDto,
    pub keyframes: Vec<KeyframeDto>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModulationLinkDto {
    pub source: u64,
    pub amount: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeDto {
    pub from_node: u64,
    pub from_port: u32,
    pub to_node: u64,
    pub to_port: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipDto {
    pub node: u64,
    pub start: i64,
    pub length: i64,
    pub content: ClipContentDto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipContentDto {
    Audio {
        asset: String,
        source_start: i64,
        source_length: i64,
    },
    Midi {
        source_start: i64,
        source_length: i64,
        notes: Vec<NoteDto>,
    },
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteDto {
    pub id: u64,
    pub pitch: u8,
    pub start: i64,
    pub length: i64,
    pub velocity: u8,
    pub channel: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDto {
    pub name: String,
    pub kind: AssetKindDto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKindDto {
    Audio,
    Midi,
    ImpulseResponse,
}

impl From<&Project> for ProjectDto {
    fn from(project: &Project) -> Self {
        Self {
            meta: MetaDto::from(&project.meta),
            tempo: TempoMapDto::from(&project.tempo),
            signatures: project
                .signatures
                .iter()
                .copied()
                .map(TimeSignatureDto::from)
                .collect(),
            markers: project
                .markers
                .iter()
                .map(|(id, marker)| (id.raw(), MarkerDto::from(marker)))
                .collect(),
            sections: project
                .sections
                .iter()
                .map(|(id, section)| (id.raw(), SectionDto::from(section)))
                .collect(),
            arrangement: project
                .arrangement
                .iter()
                .copied()
                .map(SectionRefDto::from)
                .collect(),
            loops: project
                .loops
                .iter()
                .copied()
                .map(LoopRegionDto::from)
                .collect(),
            nodes: project
                .nodes
                .iter()
                .map(|(id, node)| (id.raw(), NodeDto::from(node)))
                .collect(),
            edges: project
                .edges
                .iter()
                .map(|(id, edge)| (id.raw(), EdgeDto::from(edge)))
                .collect(),
            clips: project
                .clips
                .iter()
                .map(|(id, clip)| (id.raw(), ClipDto::from(clip)))
                .collect(),
            assets: project
                .assets
                .iter()
                .map(|(id, asset)| (id.to_hex(), AssetDto::from(asset)))
                .collect(),
        }
    }
}

impl TryFrom<ProjectDto> for Project {
    type Error = FormatError;

    fn try_from(dto: ProjectDto) -> Result<Self, Self::Error> {
        let tempo = TempoMap::new(
            dto.tempo.sample_rate,
            dto.tempo.points.into_iter().map(TempoPoint::from),
        )?;

        let mut assets = PersistentMap::new();
        for (id, asset) in dto.assets {
            let id = AssetId::from_hex(&id).map_err(|source| FormatError::InvalidAssetId {
                value: id.clone(),
                source,
            })?;
            assets.insert(id, Asset::from(asset));
        }

        let mut clips = PersistentMap::new();
        for (id, clip) in dto.clips {
            clips.insert(ClipId::from_raw(id), Clip::try_from(clip)?);
        }

        let project = Self {
            meta: Meta::from(dto.meta),
            tempo,
            signatures: dto
                .signatures
                .into_iter()
                .map(TimeSignature::from)
                .collect(),
            markers: dto
                .markers
                .into_iter()
                .map(|(id, marker)| (MarkerId::from_raw(id), Marker::from(marker)))
                .collect(),
            sections: dto
                .sections
                .into_iter()
                .map(|(id, section)| (SectionId::from_raw(id), Section::from(section)))
                .collect(),
            arrangement: dto.arrangement.into_iter().map(SectionRef::from).collect(),
            loops: dto.loops.into_iter().map(LoopRegion::from).collect(),
            nodes: dto
                .nodes
                .into_iter()
                .map(|(id, node)| (NodeId::from_raw(id), Node::from(node)))
                .collect(),
            edges: dto
                .edges
                .into_iter()
                .map(|(id, edge)| (EdgeId::from_raw(id), Edge::from(edge)))
                .collect(),
            clips,
            assets,
        };

        project.validate()?;
        Ok(project)
    }
}

impl From<&Meta> for MetaDto {
    fn from(meta: &Meta) -> Self {
        Self {
            name: meta.name.clone(),
            sample_rate: meta.sample_rate,
            seed: meta.seed,
        }
    }
}

impl From<MetaDto> for Meta {
    fn from(dto: MetaDto) -> Self {
        Self {
            name: dto.name,
            sample_rate: dto.sample_rate,
            seed: dto.seed,
        }
    }
}

impl From<&TempoMap> for TempoMapDto {
    fn from(tempo: &TempoMap) -> Self {
        Self {
            sample_rate: tempo.sample_rate(),
            points: tempo.points().map(TempoPointDto::from).collect(),
        }
    }
}

impl From<TempoPoint> for TempoPointDto {
    fn from(point: TempoPoint) -> Self {
        Self {
            tick: point.tick.raw(),
            micros_per_quarter: point.micros_per_quarter,
        }
    }
}

impl From<TempoPointDto> for TempoPoint {
    fn from(dto: TempoPointDto) -> Self {
        Self {
            tick: Tick::from_raw(dto.tick),
            micros_per_quarter: dto.micros_per_quarter,
        }
    }
}

impl From<TimeSignature> for TimeSignatureDto {
    fn from(signature: TimeSignature) -> Self {
        Self {
            tick: signature.tick.raw(),
            numerator: signature.numerator,
            denominator: signature.denominator,
        }
    }
}

impl From<TimeSignatureDto> for TimeSignature {
    fn from(dto: TimeSignatureDto) -> Self {
        Self {
            tick: Tick::from_raw(dto.tick),
            numerator: dto.numerator,
            denominator: dto.denominator,
        }
    }
}

impl From<&Marker> for MarkerDto {
    fn from(marker: &Marker) -> Self {
        Self {
            name: marker.name.clone(),
            tick: marker.tick.raw(),
        }
    }
}

impl From<MarkerDto> for Marker {
    fn from(dto: MarkerDto) -> Self {
        Self {
            name: dto.name,
            tick: Tick::from_raw(dto.tick),
        }
    }
}

impl From<&Section> for SectionDto {
    fn from(section: &Section) -> Self {
        Self {
            name: section.name.clone(),
            length: section.length.raw(),
            clips: section.clips.iter().map(|clip| clip.raw()).collect(),
        }
    }
}

impl From<SectionDto> for Section {
    fn from(dto: SectionDto) -> Self {
        Self {
            name: dto.name,
            length: Tick::from_raw(dto.length),
            clips: dto.clips.into_iter().map(ClipId::from_raw).collect(),
        }
    }
}

impl From<SectionRef> for SectionRefDto {
    fn from(reference: SectionRef) -> Self {
        Self {
            section: reference.section.raw(),
            repeat: reference.repeat,
            active: reference.active,
        }
    }
}

impl From<SectionRefDto> for SectionRef {
    fn from(dto: SectionRefDto) -> Self {
        Self {
            section: SectionId::from_raw(dto.section),
            repeat: dto.repeat,
            active: dto.active,
        }
    }
}

impl From<LoopRegion> for LoopRegionDto {
    fn from(region: LoopRegion) -> Self {
        Self {
            start: region.start.raw(),
            end: region.end.raw(),
        }
    }
}

impl From<LoopRegionDto> for LoopRegion {
    fn from(dto: LoopRegionDto) -> Self {
        Self {
            start: Tick::from_raw(dto.start),
            end: Tick::from_raw(dto.end),
        }
    }
}

impl From<&Node> for NodeDto {
    fn from(node: &Node) -> Self {
        Self {
            role: NodeRoleDto::from(node.role),
            kind: node.kind.as_ref().map(|kind| kind.as_str().to_owned()),
            rack: node.rack.iter().map(|id| id.raw()).collect(),
            ports: node.ports.iter().map(PortDto::from).collect(),
            params: node
                .params
                .iter()
                .map(|(id, value)| (id.as_str().to_owned(), ParamValueDto::from(*value)))
                .collect(),
            automation: node
                .automation
                .iter()
                .map(|(id, lane)| (id.as_str().to_owned(), AutomationLaneDto::from(lane)))
                .collect(),
            modulations: node
                .modulations
                .iter()
                .map(|(id, links)| {
                    (
                        id.as_str().to_owned(),
                        links.iter().copied().map(ModulationLinkDto::from).collect(),
                    )
                })
                .collect(),
            clips: node.clips.iter().map(|id| id.raw()).collect(),
            state: node.state.as_ref().map(|state| state.to_vec()),
            name: node.name.clone(),
            color: node.color,
            muted: node.muted,
            soloed: node.soloed,
        }
    }
}

impl From<NodeDto> for Node {
    fn from(dto: NodeDto) -> Self {
        Self {
            role: NodeRole::from(dto.role),
            kind: dto.kind.map(NodeKind::from),
            name: dto.name,
            color: dto.color,
            muted: dto.muted,
            soloed: dto.soloed,
            rack: dto.rack.into_iter().map(NodeId::from_raw).collect(),
            ports: dto.ports.into_iter().map(Port::from).collect(),
            params: dto
                .params
                .into_iter()
                .map(|(id, value)| (ParamId::new(id), ParamValue::from(value)))
                .collect(),
            automation: dto
                .automation
                .into_iter()
                .map(|(id, lane)| (ParamId::new(id), AutomationLane::from(lane)))
                .collect(),
            modulations: dto
                .modulations
                .into_iter()
                .map(|(id, links)| {
                    (
                        ParamId::new(id),
                        links
                            .into_iter()
                            .map(ModulationLink::from)
                            .collect::<PersistentVector<ModulationLink>>(),
                    )
                })
                .collect(),
            clips: dto.clips.into_iter().map(ClipId::from_raw).collect(),
            state: dto.state.map(Into::into),
        }
    }
}

impl From<NodeRole> for NodeRoleDto {
    fn from(role: NodeRole) -> Self {
        match role {
            NodeRole::Track => Self::Track,
            NodeRole::Bus => Self::Bus,
            NodeRole::Master => Self::Master,
            NodeRole::Send => Self::Send,
            NodeRole::Folder => Self::Folder,
            NodeRole::Input => Self::Input,
            NodeRole::Output => Self::Output,
            NodeRole::Modulator => Self::Modulator,
            NodeRole::Device => Self::Device,
        }
    }
}

impl From<NodeRoleDto> for NodeRole {
    fn from(role: NodeRoleDto) -> Self {
        match role {
            NodeRoleDto::Track => Self::Track,
            NodeRoleDto::Bus => Self::Bus,
            NodeRoleDto::Master => Self::Master,
            NodeRoleDto::Send => Self::Send,
            NodeRoleDto::Folder => Self::Folder,
            NodeRoleDto::Input => Self::Input,
            NodeRoleDto::Output => Self::Output,
            NodeRoleDto::Modulator => Self::Modulator,
            NodeRoleDto::Device => Self::Device,
        }
    }
}

impl From<&Port> for PortDto {
    fn from(port: &Port) -> Self {
        Self {
            id: port.id.raw(),
            direction: PortDirectionDto::from(port.direction),
            layout: ChannelLayoutDto::from(port.layout),
        }
    }
}

impl From<PortDto> for Port {
    fn from(dto: PortDto) -> Self {
        Self {
            id: PortId::from_raw(dto.id),
            direction: PortDirection::from(dto.direction),
            layout: ChannelLayout::from(dto.layout),
        }
    }
}

impl From<PortDirection> for PortDirectionDto {
    fn from(direction: PortDirection) -> Self {
        match direction {
            PortDirection::Input => Self::Input,
            PortDirection::Output => Self::Output,
        }
    }
}

impl From<PortDirectionDto> for PortDirection {
    fn from(direction: PortDirectionDto) -> Self {
        match direction {
            PortDirectionDto::Input => Self::Input,
            PortDirectionDto::Output => Self::Output,
        }
    }
}

impl From<ChannelLayout> for ChannelLayoutDto {
    fn from(layout: ChannelLayout) -> Self {
        match layout {
            ChannelLayout::Mono => Self::Mono,
            ChannelLayout::Stereo => Self::Stereo,
            ChannelLayout::Channels(count) => Self::Channels(count),
        }
    }
}

impl From<ChannelLayoutDto> for ChannelLayout {
    fn from(layout: ChannelLayoutDto) -> Self {
        match layout {
            ChannelLayoutDto::Mono => Self::Mono,
            ChannelLayoutDto::Stereo => Self::Stereo,
            ChannelLayoutDto::Channels(count) => Self::Channels(count),
        }
    }
}

impl From<ParamValue> for ParamValueDto {
    fn from(value: ParamValue) -> Self {
        match value {
            ParamValue::Float(value) => Self::Float(value),
            ParamValue::Int(value) => Self::Int(value),
            ParamValue::Bool(value) => Self::Bool(value),
            ParamValue::Enum(value) => Self::Enum(value),
        }
    }
}

impl From<ParamValueDto> for ParamValue {
    fn from(value: ParamValueDto) -> Self {
        match value {
            ParamValueDto::Float(value) => Self::Float(value),
            ParamValueDto::Int(value) => Self::Int(value),
            ParamValueDto::Bool(value) => Self::Bool(value),
            ParamValueDto::Enum(value) => Self::Enum(value),
        }
    }
}

impl From<&AutomationLane> for AutomationLaneDto {
    fn from(lane: &AutomationLane) -> Self {
        Self {
            mode: AutomationModeDto::from(lane.mode),
            keyframes: lane.keyframes.iter().map(KeyframeDto::from).collect(),
        }
    }
}

impl From<AutomationLaneDto> for AutomationLane {
    fn from(dto: AutomationLaneDto) -> Self {
        Self {
            mode: AutomationMode::from(dto.mode),
            keyframes: dto.keyframes.into_iter().map(Keyframe::from).collect(),
        }
    }
}

impl From<AutomationMode> for AutomationModeDto {
    fn from(mode: AutomationMode) -> Self {
        match mode {
            AutomationMode::Absolute => Self::Absolute,
            AutomationMode::Trim => Self::Trim,
        }
    }
}

impl From<AutomationModeDto> for AutomationMode {
    fn from(mode: AutomationModeDto) -> Self {
        match mode {
            AutomationModeDto::Absolute => Self::Absolute,
            AutomationModeDto::Trim => Self::Trim,
        }
    }
}

impl From<&Keyframe> for KeyframeDto {
    fn from(keyframe: &Keyframe) -> Self {
        Self {
            tick: keyframe.tick.raw(),
            value: ParamValueDto::from(keyframe.value),
            curve: CurveDto::from(keyframe.curve),
        }
    }
}

impl From<KeyframeDto> for Keyframe {
    fn from(dto: KeyframeDto) -> Self {
        Self {
            tick: Tick::from_raw(dto.tick),
            value: ParamValue::from(dto.value),
            curve: Curve::from(dto.curve),
        }
    }
}

impl From<Curve> for CurveDto {
    fn from(curve: Curve) -> Self {
        match curve {
            Curve::Hold => Self::Hold,
            Curve::Linear => Self::Linear,
        }
    }
}

impl From<CurveDto> for Curve {
    fn from(curve: CurveDto) -> Self {
        match curve {
            CurveDto::Hold => Self::Hold,
            CurveDto::Linear => Self::Linear,
        }
    }
}

impl From<ModulationLink> for ModulationLinkDto {
    fn from(link: ModulationLink) -> Self {
        Self {
            source: link.source.raw(),
            amount: link.amount,
        }
    }
}

impl From<ModulationLinkDto> for ModulationLink {
    fn from(dto: ModulationLinkDto) -> Self {
        Self {
            source: NodeId::from_raw(dto.source),
            amount: dto.amount,
        }
    }
}

impl From<&Edge> for EdgeDto {
    fn from(edge: &Edge) -> Self {
        Self {
            from_node: edge.from.node.raw(),
            from_port: edge.from.port.raw(),
            to_node: edge.to.node.raw(),
            to_port: edge.to.port.raw(),
        }
    }
}

impl From<EdgeDto> for Edge {
    fn from(dto: EdgeDto) -> Self {
        Self {
            from: NodePort {
                node: NodeId::from_raw(dto.from_node),
                port: PortId::from_raw(dto.from_port),
            },
            to: NodePort {
                node: NodeId::from_raw(dto.to_node),
                port: PortId::from_raw(dto.to_port),
            },
        }
    }
}

impl From<&Clip> for ClipDto {
    fn from(clip: &Clip) -> Self {
        Self {
            node: clip.node.raw(),
            start: clip.start.raw(),
            length: clip.length.raw(),
            content: ClipContentDto::from(&clip.content),
        }
    }
}

impl TryFrom<ClipDto> for Clip {
    type Error = FormatError;

    fn try_from(dto: ClipDto) -> Result<Self, Self::Error> {
        Ok(Self {
            node: NodeId::from_raw(dto.node),
            start: Tick::from_raw(dto.start),
            length: Tick::from_raw(dto.length),
            content: ClipContent::try_from(dto.content)?,
        })
    }
}

impl From<&ClipContent> for ClipContentDto {
    fn from(content: &ClipContent) -> Self {
        match content {
            ClipContent::Audio {
                asset,
                source_start,
                source_length,
            } => Self::Audio {
                asset: asset.to_hex(),
                source_start: source_start.raw(),
                source_length: source_length.raw(),
            },
            ClipContent::Midi {
                source_start,
                source_length,
                notes,
            } => Self::Midi {
                source_start: source_start.raw(),
                source_length: source_length.raw(),
                notes: notes.iter().copied().map(NoteDto::from).collect(),
            },
            ClipContent::Empty => Self::Empty,
        }
    }
}

impl TryFrom<ClipContentDto> for ClipContent {
    type Error = FormatError;

    fn try_from(dto: ClipContentDto) -> Result<Self, Self::Error> {
        Ok(match dto {
            ClipContentDto::Audio {
                asset,
                source_start,
                source_length,
            } => Self::Audio {
                asset: AssetId::from_hex(&asset).map_err(|source| FormatError::InvalidAssetId {
                    value: asset,
                    source,
                })?,
                source_start: Frames::from_raw(source_start),
                source_length: Frames::from_raw(source_length),
            },
            ClipContentDto::Midi {
                source_start,
                source_length,
                notes,
            } => Self::Midi {
                source_start: Tick::from_raw(source_start),
                source_length: Tick::from_raw(source_length),
                notes: notes.into_iter().map(Note::from).collect(),
            },
            ClipContentDto::Empty => Self::Empty,
        })
    }
}

impl From<Note> for NoteDto {
    fn from(note: Note) -> Self {
        Self {
            id: note.id.raw(),
            pitch: note.pitch,
            start: note.start.raw(),
            length: note.length.raw(),
            velocity: note.velocity,
            channel: note.channel,
        }
    }
}

impl From<NoteDto> for Note {
    fn from(dto: NoteDto) -> Self {
        Self {
            id: NoteId::from_raw(dto.id),
            pitch: dto.pitch,
            start: Tick::from_raw(dto.start),
            length: Tick::from_raw(dto.length),
            velocity: dto.velocity,
            channel: dto.channel,
        }
    }
}

impl From<&Asset> for AssetDto {
    fn from(asset: &Asset) -> Self {
        Self {
            name: asset.name.clone(),
            kind: AssetKindDto::from(asset.kind),
        }
    }
}

impl From<AssetDto> for Asset {
    fn from(dto: AssetDto) -> Self {
        Self {
            name: dto.name,
            kind: AssetKind::from(dto.kind),
        }
    }
}

impl From<AssetKind> for AssetKindDto {
    fn from(kind: AssetKind) -> Self {
        match kind {
            AssetKind::Audio => Self::Audio,
            AssetKind::Midi => Self::Midi,
            AssetKind::ImpulseResponse => Self::ImpulseResponse,
        }
    }
}

impl From<AssetKindDto> for AssetKind {
    fn from(kind: AssetKindDto) -> Self {
        match kind {
            AssetKindDto::Audio => Self::Audio,
            AssetKindDto::Midi => Self::Midi,
            AssetKindDto::ImpulseResponse => Self::ImpulseResponse,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_lanes_round_trip() {
        let lane = AutomationLane {
            mode: AutomationMode::Trim,
            keyframes: [
                Keyframe {
                    tick: Tick::ZERO,
                    value: ParamValue::Float(0.25),
                    curve: Curve::Hold,
                },
                Keyframe {
                    tick: Tick::from_raw(10),
                    value: ParamValue::Float(0.75),
                    curve: Curve::Linear,
                },
            ]
            .into_iter()
            .collect(),
        };
        let dto = AutomationLaneDto::from(&lane);
        assert_eq!(AutomationLane::from(dto), lane);
    }

    #[test]
    fn invalid_asset_ids_are_rejected_when_decoding() {
        let content = ClipContentDto::Audio {
            asset: "not-a-hash".into(),
            source_start: 0,
            source_length: 1,
        };
        assert!(matches!(
            ClipContent::try_from(content),
            Err(FormatError::InvalidAssetId { .. })
        ));
    }

    #[test]
    fn midi_notes_round_trip() {
        let content = ClipContent::Midi {
            source_start: Tick::ZERO,
            source_length: Tick::from_raw(1_000),
            notes: [Note {
                id: NoteId::from_raw(3),
                pitch: 60,
                start: Tick::from_raw(120),
                length: Tick::from_raw(240),
                velocity: 100,
                channel: 0,
            }]
            .into_iter()
            .collect(),
        };
        let dto = ClipContentDto::from(&content);
        assert_eq!(ClipContent::try_from(dto).unwrap(), content);
    }
}
