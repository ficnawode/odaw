use std::fmt;
use std::sync::Arc;

use crate::collections::{PersistentMap, PersistentVector};
use crate::ids::{NodeId, PortId};
use crate::params::{ParamId, ParamValue};
use crate::time::Tick;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeRole {
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

impl NodeRole {
    pub const fn is_device(self) -> bool {
        matches!(self, Self::Device)
    }

    pub const fn holds_rack(self) -> bool {
        matches!(self, Self::Track | Self::Bus | Self::Folder)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeKind(String);

impl NodeKind {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for NodeKind {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for NodeKind {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for NodeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelLayout {
    Mono,
    Stereo,
    Channels(u16),
}

impl ChannelLayout {
    pub const fn channels(self) -> u16 {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Channels(count) => count,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Port {
    pub id: PortId,
    pub direction: PortDirection,
    pub layout: ChannelLayout,
}

impl Port {
    pub const fn input(id: PortId, layout: ChannelLayout) -> Self {
        Self {
            id,
            direction: PortDirection::Input,
            layout,
        }
    }

    pub const fn output(id: PortId, layout: ChannelLayout) -> Self {
        Self {
            id,
            direction: PortDirection::Output,
            layout,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodePort {
    pub node: NodeId,
    pub port: PortId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: NodePort,
    pub to: NodePort,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Hold,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationMode {
    Absolute,
    Trim,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub tick: Tick,
    pub value: ParamValue,
    pub curve: Curve,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutomationLane {
    pub mode: AutomationMode,
    pub keyframes: PersistentVector<Keyframe>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModulationLink {
    pub source: NodeId,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub role: NodeRole,
    pub kind: Option<NodeKind>,
    pub name: Option<String>,
    pub color: Option<u32>,
    pub muted: bool,
    pub soloed: bool,
    pub rack: PersistentVector<NodeId>,
    pub ports: PersistentVector<Port>,
    pub params: PersistentMap<ParamId, ParamValue>,
    pub automation: PersistentMap<ParamId, AutomationLane>,
    pub modulations: PersistentMap<ParamId, PersistentVector<ModulationLink>>,
    pub clips: PersistentVector<crate::ids::ClipId>,
    pub state: Option<Arc<[u8]>>,
}

impl Node {
    pub fn new(role: NodeRole) -> Self {
        Self {
            role,
            kind: None,
            name: None,
            color: None,
            muted: false,
            soloed: false,
            rack: PersistentVector::new(),
            ports: PersistentVector::new(),
            params: PersistentMap::new(),
            automation: PersistentMap::new(),
            modulations: PersistentMap::new(),
            clips: PersistentVector::new(),
            state: None,
        }
    }

    pub fn device(kind: NodeKind) -> Self {
        let mut node = Self::new(NodeRole::Device);
        node.kind = Some(kind);
        node
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_color(mut self, color: u32) -> Self {
        self.color = Some(color & 0x00ff_ffff);
        self
    }

    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    pub fn soloed(mut self, soloed: bool) -> Self {
        self.soloed = soloed;
        self
    }

    pub fn with_ports(mut self, ports: impl IntoIterator<Item = Port>) -> Self {
        self.ports = ports.into_iter().collect();
        self
    }

    pub fn with_rack(mut self, rack: impl IntoIterator<Item = NodeId>) -> Self {
        self.rack = rack.into_iter().collect();
        self
    }

    pub fn with_param(mut self, id: ParamId, value: ParamValue) -> Self {
        self.params.insert(id, value);
        self
    }

    pub fn with_clips(mut self, clips: impl IntoIterator<Item = crate::ids::ClipId>) -> Self {
        self.clips = clips.into_iter().collect();
        self
    }

    pub fn with_state(mut self, state: impl Into<Arc<[u8]>>) -> Self {
        self.state = Some(state.into());
        self
    }

    pub fn port(&self, id: PortId) -> Option<&Port> {
        self.ports.iter().find(|port| port.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_node_has_a_kind_and_no_rack() {
        let node = Node::device(NodeKind::from("builtin.gain"));
        assert_eq!(node.role, NodeRole::Device);
        assert_eq!(
            node.kind.as_ref().map(NodeKind::as_str),
            Some("builtin.gain")
        );
        assert!(node.rack.is_empty());
    }

    #[test]
    fn node_finds_ports_by_id() {
        let node = Node::new(NodeRole::Track)
            .with_ports([Port::output(PortId::from_raw(3), ChannelLayout::Stereo)]);
        assert_eq!(
            node.port(PortId::from_raw(3)).map(|port| port.layout),
            Some(ChannelLayout::Stereo)
        );
        assert_eq!(node.port(PortId::from_raw(4)), None);
    }
}
