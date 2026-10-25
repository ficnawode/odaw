use std::sync::Arc;

use odaw_core::{
    Asset, AssetId, AutomationLane, Clip, ClipId, Edge, EdgeId, LoopRegion, Marker, MarkerId,
    ModulationLink, Node, NodeId, NodeKind, ParamAddress, ParamId, ParamValue, Port, Section,
    SectionId, SectionRef, TempoPoint, TimeSignature,
};

use crate::operator::OperatorRef;

pub const COMMAND_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub version: u16,
    pub operator: Option<OperatorRef>,
    pub kind: CommandKind,
}

impl Command {
    pub fn new(kind: CommandKind) -> Self {
        Self {
            version: COMMAND_VERSION,
            operator: None,
            kind,
        }
    }

    pub fn with_operator(mut self, operator: OperatorRef) -> Self {
        self.operator = Some(operator);
        self
    }

    pub fn label(&self) -> &'static str {
        self.kind.label()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CommandKind {
    Meta(MetaCommand),
    Asset(AssetCommand),
    Node(NodeCommand),
    Device(DeviceCommand),
    Edge(EdgeCommand),
    Clip(ClipCommand),
    Section(SectionCommand),
    Arrangement(ArrangementCommand),
    Loop(LoopCommand),
    Param(ParamCommand),
    Automation(AutomationCommand),
    Modulation(ModulationCommand),
    Tempo(TempoCommand),
    Marker(MarkerCommand),
}

impl CommandKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Meta(_) => "Edit project metadata",
            Self::Asset(_) => "Edit asset",
            Self::Node(_) => "Edit node",
            Self::Device(_) => "Edit device",
            Self::Edge(_) => "Edit connection",
            Self::Clip(_) => "Edit clip",
            Self::Section(_) => "Edit section",
            Self::Arrangement(_) => "Edit arrangement",
            Self::Loop(_) => "Edit loop",
            Self::Param(_) => "Set parameter",
            Self::Automation(_) => "Edit automation",
            Self::Modulation(_) => "Edit modulation",
            Self::Tempo(_) => "Edit tempo",
            Self::Marker(_) => "Edit marker",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetaCommand {
    SetName(String),
    SetSeed(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetCommand {
    Set {
        asset: AssetId,
        value: Option<Asset>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeCommand {
    Add { node: NodeId, value: Node },
    Remove { node: NodeId },
    Replace { node: NodeId, value: Node },
    SetRack { node: NodeId, rack: Vec<NodeId> },
    SetPorts { node: NodeId, ports: Vec<Port> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeviceCommand {
    SetKind {
        node: NodeId,
        kind: NodeKind,
    },
    SetState {
        node: NodeId,
        state: Option<Arc<[u8]>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeCommand {
    Add { edge: EdgeId, value: Edge },
    Remove { edge: EdgeId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipCommand {
    Add { clip: ClipId, value: Clip },
    Remove { clip: ClipId },
    Replace { clip: ClipId, value: Clip },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionCommand {
    Add { section: SectionId, value: Section },
    Remove { section: SectionId },
    Replace { section: SectionId, value: Section },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArrangementCommand {
    Set(Vec<SectionRef>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoopCommand {
    Set(Vec<LoopRegion>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamChange {
    Value,
    Structural,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamCommand {
    Set {
        address: ParamAddress,
        value: ParamValue,
        change: ParamChange,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum AutomationCommand {
    Set {
        node: NodeId,
        param: ParamId,
        lane: Option<AutomationLane>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModulationCommand {
    Set {
        node: NodeId,
        param: ParamId,
        links: Vec<ModulationLink>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TempoCommand {
    SetTempoMap { points: Vec<TempoPoint> },
    SetSignatures { signatures: Vec<TimeSignature> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerCommand {
    Set {
        marker: MarkerId,
        value: Option<Marker>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_start_at_the_current_version() {
        let command = Command::new(CommandKind::Meta(MetaCommand::SetSeed(7)));
        assert_eq!(command.version, COMMAND_VERSION);
        assert!(command.operator.is_none());
    }

    #[test]
    fn command_labels_are_human_readable() {
        let command = Command::new(CommandKind::Param(ParamCommand::Set {
            address: ParamAddress {
                node: NodeId::from_raw(1),
                param: "gain".into(),
            },
            value: ParamValue::Float(1.0),
            change: ParamChange::Value,
        }));
        assert_eq!(command.label(), "Set parameter");
    }
}
