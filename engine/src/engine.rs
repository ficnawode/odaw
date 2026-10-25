use std::num::NonZeroUsize;

use thiserror::Error;

use crate::param::ParamBuffer;
use crate::plan::{Block, BufferId, CompiledProject, RenderPlan};

#[derive(Debug)]
pub struct Engine {
    plan: RenderPlan,
    params: ParamBuffer,
    buffers: Vec<Vec<f32>>,
    scratch: Vec<f32>,
    block_size: usize,
    start_frame: u64,
}

impl Engine {
    pub fn new(compiled: CompiledProject, block_size: NonZeroUsize) -> Result<Self, EngineError> {
        let CompiledProject { plan, params } = compiled;
        if plan.sample_rate == 0 {
            return Err(EngineError::ZeroSampleRate);
        }
        if plan.channels == 0 {
            return Err(EngineError::ZeroChannels);
        }

        let buffer_count = plan.steps.len();
        let mut produced = vec![false; buffer_count];
        for (step_index, step) in plan.steps.iter().enumerate() {
            if step.output >= buffer_count {
                return Err(EngineError::OutputOutOfRange {
                    step: step_index,
                    output: step.output,
                });
            }
            if produced[step.output] {
                return Err(EngineError::DuplicateOutput {
                    buffer: step.output,
                });
            }
            for input in &step.inputs {
                if *input >= buffer_count {
                    return Err(EngineError::InputOutOfRange {
                        step: step_index,
                        input: *input,
                    });
                }
                if !produced[*input] {
                    return Err(EngineError::InputNotProduced {
                        step: step_index,
                        input: *input,
                    });
                }
            }
            produced[step.output] = true;
        }

        let master = plan.master.ok_or(EngineError::MissingMaster)?;
        if master >= buffer_count {
            return Err(EngineError::MasterOutOfRange { buffer: master });
        }
        if !produced[master] {
            return Err(EngineError::MasterNotProduced { buffer: master });
        }

        let buffer_size = block_size
            .get()
            .checked_mul(plan.channels)
            .ok_or(EngineError::TooLarge)?;
        let buffers = (0..buffer_count).map(|_| vec![0.0; buffer_size]).collect();

        Ok(Self {
            plan,
            params,
            buffers,
            scratch: vec![0.0; buffer_size],
            block_size: block_size.get(),
            start_frame: 0,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.plan.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.plan.channels
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    pub fn start_frame(&self) -> u64 {
        self.start_frame
    }

    pub fn params(&self) -> &ParamBuffer {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut ParamBuffer {
        &mut self.params
    }

    pub fn process_block(&mut self, frames: usize, output: &mut [f32]) -> Result<(), EngineError> {
        if frames > self.block_size {
            return Err(EngineError::BlockTooLarge {
                requested: frames,
                capacity: self.block_size,
            });
        }
        if frames == 0 {
            return Ok(());
        }

        let Engine {
            plan,
            params,
            buffers,
            scratch,
            start_frame,
            ..
        } = self;
        let channels = plan.channels;
        let sample_rate = plan.sample_rate;
        let master = plan.master;
        let required = frames.checked_mul(channels).ok_or(EngineError::TooLarge)?;

        if output.len() < required {
            return Err(EngineError::OutputTooSmall {
                provided: output.len(),
                required,
            });
        }

        for step in &mut plan.steps {
            scratch[..required].fill(0.0);
            for input in &step.inputs {
                let source = &buffers[*input][..required];
                for (destination, value) in scratch[..required].iter_mut().zip(source) {
                    *destination += *value;
                }
            }

            let destination = &mut buffers[step.output][..required];
            destination.fill(0.0);
            let mut block = Block {
                node: step.node,
                sample_rate,
                channels,
                start_frame: *start_frame,
                frames,
                input: &scratch[..required],
                output: destination,
                params,
            };
            step.processor.process(&mut block);
        }

        output[..required].fill(0.0);
        if let Some(master) = master {
            output[..required].copy_from_slice(&buffers[master][..required]);
        }

        *start_frame = start_frame
            .checked_add(frames as u64)
            .ok_or(EngineError::TooLarge)?;
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("sample rate must be greater than zero")]
    ZeroSampleRate,
    #[error("channel count must be greater than zero")]
    ZeroChannels,
    #[error("the requested size is too large")]
    TooLarge,
    #[error("step {step} writes output buffer {output}, which is out of range")]
    OutputOutOfRange { step: usize, output: BufferId },
    #[error("output buffer {buffer} is written by more than one step")]
    DuplicateOutput { buffer: BufferId },
    #[error("step {step} reads input buffer {input}, which is out of range")]
    InputOutOfRange { step: usize, input: BufferId },
    #[error("step {step} reads input buffer {input} before it is produced")]
    InputNotProduced { step: usize, input: BufferId },
    #[error("the render plan has no master output")]
    MissingMaster,
    #[error("master output buffer {buffer} is out of range")]
    MasterOutOfRange { buffer: BufferId },
    #[error("master output buffer {buffer} is not produced")]
    MasterNotProduced { buffer: BufferId },
    #[error("requested {requested} frames but the engine block size is {capacity}")]
    BlockTooLarge { requested: usize, capacity: usize },
    #[error("output has {provided} samples but {required} are required")]
    OutputTooSmall { provided: usize, required: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{DeviceProcessor, PlanStep};
    use odaw_core::NodeId;

    #[derive(Debug)]
    struct Constant(f32);

    impl DeviceProcessor for Constant {
        fn process(&mut self, block: &mut Block<'_>) {
            block.output.fill(self.0);
        }
    }

    #[derive(Debug)]
    struct Double;

    impl DeviceProcessor for Double {
        fn process(&mut self, block: &mut Block<'_>) {
            for (destination, value) in block.output.iter_mut().zip(block.input) {
                *destination = value * 2.0;
            }
        }
    }

    #[derive(Debug)]
    struct FrameIndex;

    impl DeviceProcessor for FrameIndex {
        fn process(&mut self, block: &mut Block<'_>) {
            for frame in 0..block.frames {
                let value = (block.start_frame + frame as u64) as f32;
                for channel in 0..block.channels {
                    block.output[frame * block.channels + channel] = value;
                }
            }
        }
    }

    #[derive(Debug)]
    struct ParamConstant;

    impl DeviceProcessor for ParamConstant {
        fn process(&mut self, block: &mut Block<'_>) {
            let address = odaw_core::ParamAddress {
                node: block.node,
                param: odaw_core::ParamId::new("value"),
            };
            block.output.fill(block.params.float(&address, 0.0) as f32);
        }
    }

    fn step(
        node: NodeId,
        inputs: Vec<BufferId>,
        output: BufferId,
        processor: impl DeviceProcessor + 'static,
    ) -> PlanStep {
        PlanStep {
            node,
            inputs,
            output,
            processor: Box::new(processor),
        }
    }

    fn compiled(steps: Vec<PlanStep>, master: Option<BufferId>) -> CompiledProject {
        CompiledProject {
            plan: RenderPlan {
                sample_rate: 48_000,
                channels: 2,
                steps,
                master,
            },
            params: ParamBuffer::new(),
        }
    }

    fn engine(steps: Vec<PlanStep>, master: Option<BufferId>, block_size: usize) -> Engine {
        Engine::new(
            compiled(steps, master),
            NonZeroUsize::new(block_size).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn source_is_copied_to_the_master_output() {
        let mut engine = engine(
            vec![step(NodeId::from_raw(1), vec![], 0, Constant(0.5))],
            Some(0),
            4,
        );
        let mut output = [0.0; 8];
        engine.process_block(4, &mut output).unwrap();
        assert_eq!(output, [0.5; 8]);
        assert_eq!(engine.start_frame(), 4);
    }

    #[test]
    fn inputs_are_summed_in_declared_order() {
        let mut engine = engine(
            vec![
                step(NodeId::from_raw(1), vec![], 0, Constant(0.25)),
                step(NodeId::from_raw(2), vec![], 1, Constant(0.5)),
                step(NodeId::from_raw(3), vec![0, 1], 2, Double),
            ],
            Some(2),
            2,
        );
        let mut output = [0.0; 4];
        engine.process_block(2, &mut output).unwrap();
        assert_eq!(output, [1.5, 1.5, 1.5, 1.5]);
    }

    #[test]
    fn frame_positions_advance_across_blocks() {
        let mut engine = engine(
            vec![step(NodeId::from_raw(1), vec![], 0, FrameIndex)],
            Some(0),
            2,
        );
        let mut first = [0.0; 4];
        let mut second = [0.0; 4];
        engine.process_block(2, &mut first).unwrap();
        engine.process_block(2, &mut second).unwrap();
        assert_eq!(first, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(second, [2.0, 2.0, 3.0, 3.0]);
    }

    #[test]
    fn out_of_order_inputs_are_rejected() {
        let result = Engine::new(
            compiled(vec![step(NodeId::from_raw(1), vec![0], 0, Double)], Some(0)),
            NonZeroUsize::new(4).unwrap(),
        );
        assert_eq!(
            result.err(),
            Some(EngineError::InputNotProduced { step: 0, input: 0 })
        );
    }

    #[test]
    fn duplicate_outputs_are_rejected() {
        let result = Engine::new(
            compiled(
                vec![
                    step(NodeId::from_raw(1), vec![], 0, Constant(0.0)),
                    step(NodeId::from_raw(2), vec![], 0, Constant(0.0)),
                ],
                Some(0),
            ),
            NonZeroUsize::new(4).unwrap(),
        );
        assert_eq!(
            result.err(),
            Some(EngineError::DuplicateOutput { buffer: 0 })
        );
    }

    #[test]
    fn missing_master_is_rejected() {
        let result = Engine::new(
            compiled(
                vec![step(NodeId::from_raw(1), vec![], 0, Constant(0.0))],
                None,
            ),
            NonZeroUsize::new(4).unwrap(),
        );
        assert_eq!(result.err(), Some(EngineError::MissingMaster));
    }

    #[test]
    fn oversized_blocks_are_rejected() {
        let mut engine = engine(
            vec![step(NodeId::from_raw(1), vec![], 0, Constant(0.0))],
            Some(0),
            2,
        );
        assert_eq!(
            engine.process_block(3, &mut [0.0; 6]).err(),
            Some(EngineError::BlockTooLarge {
                requested: 3,
                capacity: 2,
            })
        );
    }

    #[test]
    fn small_output_slices_are_rejected() {
        let mut engine = engine(
            vec![step(NodeId::from_raw(1), vec![], 0, Constant(0.0))],
            Some(0),
            4,
        );
        assert_eq!(
            engine.process_block(4, &mut [0.0; 2]).err(),
            Some(EngineError::OutputTooSmall {
                provided: 2,
                required: 8,
            })
        );
    }

    #[test]
    fn parameter_values_can_change_between_blocks() {
        let node = NodeId::from_raw(1);
        let address = odaw_core::ParamAddress {
            node,
            param: odaw_core::ParamId::new("value"),
        };
        let mut compiled = compiled(vec![step(node, vec![], 0, ParamConstant)], Some(0));
        compiled
            .params
            .ensure(address.clone(), odaw_core::ParamValue::Float(0.25));
        let mut engine = Engine::new(compiled, NonZeroUsize::new(2).unwrap()).unwrap();

        let mut first = [0.0; 4];
        engine.process_block(2, &mut first).unwrap();
        assert_eq!(first, [0.25; 4]);

        engine
            .params_mut()
            .set(&address, odaw_core::ParamValue::Float(0.5));
        let mut second = [0.0; 4];
        engine.process_block(2, &mut second).unwrap();
        assert_eq!(second, [0.5; 4]);
    }
}
