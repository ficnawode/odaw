use crate::collections::PersistentVector;
use crate::ids::{AssetId, ClipId, NodeId, SectionId};
use crate::time::{Frames, Tick};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Audio,
    Midi,
    ImpulseResponse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub kind: AssetKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipContent {
    Audio {
        asset: AssetId,
        source_start: Frames,
        source_length: Frames,
    },
    Midi {
        source_start: Tick,
        source_length: Tick,
    },
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub node: NodeId,
    pub start: Tick,
    pub length: Tick,
    pub content: ClipContent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub length: Tick,
    pub clips: PersistentVector<ClipId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionRef {
    pub section: SectionId,
    pub repeat: u32,
    pub active: bool,
}
