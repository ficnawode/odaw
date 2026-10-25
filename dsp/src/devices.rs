use std::f64::consts::TAU;

use odaw_core::{NodeId, ParamAddress, ParamId};
use odaw_engine::{Block, DeviceProcessor, NoteEventKind};

pub const SINE_KIND: &str = "builtin.sine";
pub const POLY_KIND: &str = "builtin.poly";
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
pub struct Silence;

impl Silence {
    pub fn new() -> Self {
        Self
    }
}

impl DeviceProcessor for Silence {
    fn process(&mut self, block: &mut Block<'_>) {
        block.output.fill(0.0);
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

const VOICES: usize = 16;
const RELEASE_SECONDS: f64 = 0.005;

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    active: bool,
    pitch: u8,
    phase: f64,
    increment: f64,
    gain: f32,
    release_step: f32,
    releasing: bool,
    age: u64,
}

#[derive(Debug)]
pub struct PolySine {
    gain: ParamAddress,
    voices: [Voice; VOICES],
    next_age: u64,
}

impl PolySine {
    pub fn new(node: NodeId) -> Self {
        Self {
            gain: ParamAddress {
                node,
                param: ParamId::new("gain"),
            },
            voices: [Voice::default(); VOICES],
            next_age: 0,
        }
    }

    fn start(&mut self, pitch: u8, velocity: u8, sample_rate: u32) -> usize {
        let index = self
            .voices
            .iter()
            .position(|voice| !voice.active)
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.age)
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });
        let frequency = 440.0 * 2f64.powf((f64::from(pitch) - 69.0) / 12.0);
        self.next_age = self.next_age.wrapping_add(1);
        self.voices[index] = Voice {
            active: true,
            pitch,
            phase: 0.0,
            increment: TAU * frequency / f64::from(sample_rate),
            gain: f32::from(velocity) / 127.0,
            release_step: 0.0,
            releasing: false,
            age: self.next_age,
        };
        index
    }

    fn release(&mut self, pitch: u8, sample_rate: u32) {
        let release_frames = (RELEASE_SECONDS * f64::from(sample_rate)).max(1.0) as f32;
        for voice in &mut self.voices {
            if voice.active && !voice.releasing && voice.pitch == pitch {
                voice.releasing = true;
                voice.release_step = voice.gain / release_frames;
            }
        }
    }
}

impl DeviceProcessor for PolySine {
    fn process(&mut self, block: &mut Block<'_>) {
        let instrument_gain = finite_or(block.params.float(&self.gain, 0.25), 0.0);
        let channels = block.channels;
        let frames = block.frames;
        let mut event_index = 0;
        block.output.fill(0.0);

        for frame in 0..frames {
            while event_index < block.events.len() {
                let event = block.events[event_index];
                if Some(event.target) != block.event_target {
                    event_index += 1;
                    continue;
                }
                if event.frame as usize > frame {
                    break;
                }
                match event.kind {
                    NoteEventKind::On => {
                        self.start(event.pitch, event.velocity, block.sample_rate);
                    }
                    NoteEventKind::Off => {
                        self.release(event.pitch, block.sample_rate);
                    }
                }
                event_index += 1;
            }

            let mut sample = 0.0f32;
            for voice in &mut self.voices {
                if !voice.active {
                    continue;
                }
                sample += (voice.phase.sin() * f64::from(voice.gain) * instrument_gain) as f32;
                voice.phase += voice.increment;
                if voice.phase >= TAU {
                    voice.phase -= TAU;
                }
                if voice.releasing {
                    voice.gain -= voice.release_step;
                    if voice.gain <= 0.0 {
                        voice.gain = 0.0;
                        voice.active = false;
                        voice.releasing = false;
                    }
                }
            }

            for channel in 0..channels {
                block.output[frame * channels + channel] += sample;
            }
        }
    }
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

#[cfg(test)]
mod tests {
    use super::*;
    use odaw_engine::{NoteEvent, ParamBuffer};

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
            event_target: None,
            events: &[],
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
            event_target: None,
            events: &[],
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
            event_target: None,
            events: &[],
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
            event_target: None,
            events: &[],
        };
        PassThrough::new().process(&mut block);

        assert_eq!(output, input);
    }

    #[test]
    fn poly_sine_starts_on_a_note_event() {
        let instrument = NodeId::from_raw(1);
        let target = NodeId::from_raw(2);
        let mut params = ParamBuffer::new();
        params.ensure(
            ParamAddress {
                node: instrument,
                param: ParamId::new("gain"),
            },
            odaw_core::ParamValue::Float(1.0),
        );
        let events = [NoteEvent {
            target,
            frame: 0,
            kind: NoteEventKind::On,
            pitch: 69,
            velocity: 127,
        }];
        let input = vec![0.0; 3];
        let mut output = output_buffer(3, 1);
        let mut block = Block {
            node: instrument,
            event_target: Some(target),
            sample_rate: 48_000,
            channels: 1,
            start_frame: 0,
            frames: 3,
            input: &input,
            output: &mut output,
            params: &params,
            events: &events,
        };
        PolySine::new(instrument).process(&mut block);

        let step = (TAU * 440.0 / 48_000.0).sin() as f32;
        let second = (2.0 * TAU * 440.0 / 48_000.0).sin() as f32;
        assert_eq!(output[0], 0.0);
        assert!((output[1] - step).abs() < 1e-6);
        assert!((output[2] - second).abs() < 1e-6);
    }

    #[test]
    fn poly_sine_releases_and_then_goes_silent() {
        let instrument = NodeId::from_raw(1);
        let target = NodeId::from_raw(2);
        let mut params = ParamBuffer::new();
        params.ensure(
            ParamAddress {
                node: instrument,
                param: ParamId::new("gain"),
            },
            odaw_core::ParamValue::Float(1.0),
        );
        let events = [
            NoteEvent {
                target,
                frame: 0,
                kind: NoteEventKind::On,
                pitch: 69,
                velocity: 127,
            },
            NoteEvent {
                target,
                frame: 2,
                kind: NoteEventKind::Off,
                pitch: 69,
                velocity: 0,
            },
        ];
        let input = vec![0.0; 300];
        let mut output = output_buffer(300, 1);
        let mut block = Block {
            node: instrument,
            event_target: Some(target),
            sample_rate: 48_000,
            channels: 1,
            start_frame: 0,
            frames: 300,
            input: &input,
            output: &mut output,
            params: &params,
            events: &events,
        };
        PolySine::new(instrument).process(&mut block);

        assert!(output[..2].iter().any(|sample| *sample != 0.0));
        assert!(output[280..].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn poly_sine_ignores_events_for_other_targets() {
        let instrument = NodeId::from_raw(1);
        let events = [NoteEvent {
            target: NodeId::from_raw(2),
            frame: 0,
            kind: NoteEventKind::On,
            pitch: 69,
            velocity: 127,
        }];
        let params = ParamBuffer::new();
        let input = vec![0.0; 4];
        let mut output = output_buffer(4, 1);
        let mut block = Block {
            node: instrument,
            event_target: Some(NodeId::from_raw(1)),
            sample_rate: 48_000,
            channels: 1,
            start_frame: 0,
            frames: 4,
            input: &input,
            output: &mut output,
            params: &params,
            events: &events,
        };
        PolySine::new(instrument).process(&mut block);

        assert!(output.iter().all(|sample| *sample == 0.0));
    }
}
