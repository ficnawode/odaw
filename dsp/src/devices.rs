use std::f64::consts::TAU;

use odaw_core::{NodeId, ParamAddress, ParamId};
use odaw_engine::{Block, DeviceProcessor};

pub const SINE_KIND: &str = "builtin.sine";
pub const GAIN_KIND: &str = "builtin.gain";
pub const PASS_KIND: &str = "builtin.pass";

#[derive(Debug)]
pub struct Sine {
    frequency: ParamAddress,
    gain: ParamAddress,
}

impl Sine {
    pub fn new(node: NodeId) -> Self {
        Self {
            frequency: ParamAddress {
                node,
                param: ParamId::new("frequency"),
            },
            gain: ParamAddress {
                node,
                param: ParamId::new("gain"),
            },
        }
    }
}

impl DeviceProcessor for Sine {
    fn process(&mut self, block: &mut Block<'_>) {
        let sample_rate = block.sample_rate as f64;
        let frequency = finite_or(block.params.float(&self.frequency, 440.0), 440.0)
            .clamp(0.0, sample_rate * 0.5);
        let gain = finite_or(block.params.float(&self.gain, 0.2), 0.0);

        for frame in 0..block.frames {
            let position = (block.start_frame + frame as u64) as f64 / sample_rate;
            let sample = (TAU * frequency * position).sin() * gain;
            let value = sample as f32;
            for channel in 0..block.channels {
                block.output[frame * block.channels + channel] = value;
            }
        }
    }
}

#[derive(Debug)]
pub struct Gain {
    gain: ParamAddress,
}

impl Gain {
    pub fn new(node: NodeId) -> Self {
        Self {
            gain: ParamAddress {
                node,
                param: ParamId::new("gain"),
            },
        }
    }
}

impl DeviceProcessor for Gain {
    fn process(&mut self, block: &mut Block<'_>) {
        let gain = finite_or(block.params.float(&self.gain, 1.0), 1.0) as f32;
        for (destination, input) in block.output.iter_mut().zip(block.input) {
            *destination = *input * gain;
        }
    }
}

#[derive(Debug, Default)]
pub struct PassThrough;

impl PassThrough {
    pub fn new() -> Self {
        Self
    }
}

impl DeviceProcessor for PassThrough {
    fn process(&mut self, block: &mut Block<'_>) {
        block.output.copy_from_slice(block.input);
    }
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;
    use odaw_engine::ParamBuffer;

    fn output_buffer(frames: usize, channels: usize) -> Vec<f32> {
        vec![0.0; frames * channels]
    }

    #[test]
    fn sine_uses_absolute_frame_position() {
        let node = NodeId::from_raw(1);
        let mut params = ParamBuffer::new();
        params.ensure(
            ParamAddress {
                node,
                param: ParamId::new("frequency"),
            },
            odaw_core::ParamValue::Float(12_000.0),
        );
        params.ensure(
            ParamAddress {
                node,
                param: ParamId::new("gain"),
            },
            odaw_core::ParamValue::Float(1.0),
        );

        let mut sine = Sine::new(node);
        let input = vec![0.0; 4];
        let mut output = output_buffer(2, 2);
        let mut block = Block {
            node,
            sample_rate: 48_000,
            channels: 2,
            start_frame: 0,
            frames: 2,
            input: &input,
            output: &mut output,
            params: &params,
        };
        sine.process(&mut block);

        assert_eq!(output, vec![0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn sine_continues_from_the_block_start_frame() {
        let node = NodeId::from_raw(1);
        let mut params = ParamBuffer::new();
        params.ensure(
            ParamAddress {
                node,
                param: ParamId::new("frequency"),
            },
            odaw_core::ParamValue::Float(12_000.0),
        );
        params.ensure(
            ParamAddress {
                node,
                param: ParamId::new("gain"),
            },
            odaw_core::ParamValue::Float(1.0),
        );

        let mut sine = Sine::new(node);
        let input = vec![0.0; 2];
        let mut output = output_buffer(1, 2);
        let mut block = Block {
            node,
            sample_rate: 48_000,
            channels: 2,
            start_frame: 3,
            frames: 1,
            input: &input,
            output: &mut output,
            params: &params,
        };
        sine.process(&mut block);

        assert_eq!(output, vec![-1.0, -1.0]);
    }

    #[test]
    fn gain_multiplies_every_channel() {
        let node = NodeId::from_raw(1);
        let mut params = ParamBuffer::new();
        params.ensure(
            ParamAddress {
                node,
                param: ParamId::new("gain"),
            },
            odaw_core::ParamValue::Float(0.5),
        );

        let mut gain = Gain::new(node);
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let mut output = output_buffer(2, 2);
        let mut block = Block {
            node,
            sample_rate: 48_000,
            channels: 2,
            start_frame: 0,
            frames: 2,
            input: &input,
            output: &mut output,
            params: &params,
        };
        gain.process(&mut block);

        assert_eq!(output, vec![0.5, 1.0, 1.5, 2.0]);
    }

    #[test]
    fn pass_through_copies_the_input() {
        let node = NodeId::from_raw(1);
        let params = ParamBuffer::new();
        let input = vec![0.1, 0.2, 0.3, 0.4];
        let mut output = output_buffer(2, 2);
        let mut block = Block {
            node,
            sample_rate: 48_000,
            channels: 2,
            start_frame: 0,
            frames: 2,
            input: &input,
            output: &mut output,
            params: &params,
        };
        PassThrough::new().process(&mut block);

        assert_eq!(output, input);
    }
}
