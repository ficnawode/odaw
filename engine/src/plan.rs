use std::fmt;

use odaw_core::NodeId;

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
    pub processor: Box<dyn DeviceProcessor>,
}

#[derive(Debug)]
pub struct CompiledProject {
    pub plan: RenderPlan,
    pub params: ParamBuffer,
}

pub trait DeviceProcessor: fmt::Debug + Send {
    fn latency_frames(&self) -> u32 {
        0
    }

    fn process(&mut self, block: &mut Block<'_>);
}

pub struct Block<'a> {
    pub node: NodeId,
    pub sample_rate: u32,
    pub channels: usize,
    pub start_frame: u64,
    pub frames: usize,
    pub input: &'a [f32],
    pub output: &'a mut [f32],
    pub params: &'a ParamBuffer,
}
