use std::fmt;

use odaw_core::{NodeId, ParamId, ParamValue};

use crate::meter::MeterBank;
use crate::param::ParamBuffer;

pub type BufferId = usize;

#[derive(Debug)]
pub struct RenderPlan {
    pub sample_rate: u32,
    pub channels: usize,
    pub steps: Vec<PlanStep>,
    pub master: Option<BufferId>,
}

#[derive(Debug)]
pub struct PlanStep {
    pub node: NodeId,
    pub inputs: Vec<BufferId>,
    pub output: BufferId,
    pub event_target: Option<NodeId>,
    pub processor: Box<dyn DeviceProcessor>,
}

#[derive(Debug)]
pub struct CompiledProject {
    pub plan: RenderPlan,
    pub params: ParamBuffer,
    pub meters: MeterBank,
}

pub trait DeviceProcessor: fmt::Debug + Send {
    fn latency_frames(&self) -> u32 {
        0
    }

    /// Parameters this device wants present in the parameter buffer, including
    /// their initial values. Document values are inserted first and win.
    fn parameter_defaults(&self) -> Vec<(ParamId, ParamValue)> {
        Vec::new()
    }

    fn process(&mut self, block: &mut Block<'_>);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteEventKind {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteEvent {
    pub target: NodeId,
    pub frame: u32,
    pub kind: NoteEventKind,
    pub pitch: u8,
    pub velocity: u8,
}

pub struct Block<'a> {
    pub node: NodeId,
    pub event_target: Option<NodeId>,
    pub sample_rate: u32,
    pub channels: usize,
    pub start_frame: u64,
    pub frames: usize,
    pub input: &'a [f32],
    pub output: &'a mut [f32],
    pub params: &'a ParamBuffer,
    pub events: &'a [NoteEvent],
}
