use std::f64::consts::TAU;
use std::sync::Arc;

use odaw_core::{NodeId, ParamAddress, ParamId, ParamValue};
use odaw_engine::{Block, DeviceParameter, DeviceProcessor, Meter, NoteEventKind};

pub const SINE_KIND: &str = "builtin.sine";
pub const POLY_KIND: &str = "builtin.poly";
pub const GAIN_KIND: &str = "builtin.gain";
pub const PASS_KIND: &str = "builtin.pass";
pub const STRIP_GAIN_PARAM: &str = "gain_db";
pub const STRIP_PAN_PARAM: &str = "pan";

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
    fn parameter_info(&self) -> Vec<DeviceParameter> {
        vec![
            DeviceParameter {
                id: ParamId::new("frequency"),
                label: "Frequency".to_owned(),
                default: ParamValue::Float(440.0),
                min: 0.0,
                max: 20_000.0,
                step_count: 0,
                unit: "Hz".to_owned(),
                read_only: false,
            },
            DeviceParameter {
                id: ParamId::new("gain"),
                label: "Gain".to_owned(),
                default: ParamValue::Float(0.2),
                min: 0.0,
                max: 1.0,
                step_count: 0,
                unit: String::new(),
                read_only: false,
            },
        ]
    }

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
    fn parameter_info(&self) -> Vec<DeviceParameter> {
        vec![DeviceParameter {
            id: ParamId::new("gain"),
            label: "Gain".to_owned(),
            default: ParamValue::Float(1.0),
            min: 0.0,
            max: 4.0,
            step_count: 0,
            unit: String::new(),
            read_only: false,
        }]
    }

    fn process(&mut self, block: &mut Block<'_>) {
        let gain = finite_or(block.params.float(&self.gain, 1.0), 1.0) as f32;
        for (destination, input) in block.output.iter_mut().zip(block.input) {
            *destination = *input * gain;
        }
    }
}

/// Track/master output stage: decibel gain and constant-power-free stereo pan.
///
/// Panning is linear so that a centre pan leaves both channels at unity; hard
/// left or right mutes the opposite channel instead of boosting the other one.
#[derive(Debug)]
pub struct ChannelStrip {
    gain: ParamAddress,
    pan: ParamAddress,
    meter: Option<Arc<Meter>>,
}

impl ChannelStrip {
    pub fn new(node: NodeId) -> Self {
        Self {
            gain: ParamAddress {
                node,
                param: ParamId::new(STRIP_GAIN_PARAM),
            },
            pan: ParamAddress {
                node,
                param: ParamId::new(STRIP_PAN_PARAM),
            },
            meter: None,
        }
    }

    pub fn with_meter(mut self, meter: Arc<Meter>) -> Self {
        self.meter = Some(meter);
        self
    }
}

impl DeviceProcessor for ChannelStrip {
    fn process(&mut self, block: &mut Block<'_>) {
        let smooth =
            block.params.ramp(&self.gain).is_some() || block.params.ramp(&self.pan).is_some();
        if !smooth {
            let gain_db = finite_or(block.params.float(&self.gain, 0.0), 0.0).clamp(-96.0, 24.0);
            let gain = 10f32.powf(gain_db as f32 / 20.0);
            let pan = finite_or(block.params.float(&self.pan, 0.0), 0.0).clamp(-1.0, 1.0) as f32;
            apply_gain(block, gain, pan);
        } else {
            let channels = block.channels;
            for frame in 0..block.frames {
                let gain_db = finite_or(
                    block
                        .params
                        .float_at(&self.gain, frame, block.frames)
                        .unwrap_or(0.0),
                    0.0,
                )
                .clamp(-96.0, 24.0);
                let gain = 10f32.powf(gain_db as f32 / 20.0);
                let pan = finite_or(
                    block
                        .params
                        .float_at(&self.pan, frame, block.frames)
                        .unwrap_or(0.0),
                    0.0,
                )
                .clamp(-1.0, 1.0) as f32;
                let start = frame * channels;
                match channels {
                    0 => {}
                    1 => {
                        block.output[start] = block.input[start] * gain;
                    }
                    _ => {
                        let left = gain * (1.0 - pan).clamp(0.0, 1.0);
                        let right = gain * (1.0 + pan).clamp(0.0, 1.0);
                        block.output[start] = block.input[start] * left;
                        block.output[start + 1] = block.input[start + 1] * right;
                        for channel in 2..channels {
                            block.output[start + channel] = block.input[start + channel] * gain;
                        }
                    }
                }
            }
        }

        if let Some(meter) = &self.meter {
            meter.store_block(block.output, block.channels);
        }
    }
}

fn apply_gain(block: &mut Block<'_>, gain: f32, pan: f32) {
    if pan == 0.0 {
        for (destination, input) in block.output.iter_mut().zip(block.input) {
            *destination = *input * gain;
        }
        return;
    }
    let left = gain * (1.0 - pan).clamp(0.0, 1.0);
    let right = gain * (1.0 + pan).clamp(0.0, 1.0);
    match block.channels {
        0 => {}
        1 => {
            for (destination, input) in block.output.iter_mut().zip(block.input) {
                *destination = *input * gain;
            }
        }
        _ => {
            for (input, output) in block
                .input
                .chunks_exact(block.channels)
                .zip(block.output.chunks_exact_mut(block.channels))
            {
                output[0] = input[0] * left;
                output[1] = input[1] * right;
                for channel in 2..block.channels {
                    output[channel] = input[channel] * gain;
                }
            }
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

/// Compensating delay used by the compiler's latency equalisation.
#[derive(Debug)]
pub struct Delay {
    frames: usize,
    channels: usize,
    buffer: Vec<f32>,
    write: usize,
    primed: usize,
}

impl Delay {
    pub fn new(frames: usize, channels: usize) -> Self {
        let frames = frames.max(1);
        let channels = channels.max(1);
        Self {
            frames,
            channels,
            buffer: vec![0.0; frames * channels],
            write: 0,
            primed: 0,
        }
    }
}

impl DeviceProcessor for Delay {
    fn latency_frames(&self) -> u32 {
        u32::try_from(self.frames).unwrap_or(u32::MAX)
    }

    fn process(&mut self, block: &mut Block<'_>) {
        let channels = block.channels;
        if channels == 0 || self.channels != channels {
            block.output.fill(0.0);
            return;
        }
        for frame in 0..block.frames {
            let read = self.write * channels;
            let write = read;
            for channel in 0..channels {
                let input = block.input[frame * channels + channel];
                let delayed = self.buffer[read + channel];
                self.buffer[write + channel] = input;
                block.output[frame * channels + channel] = if self.primed >= self.frames {
                    delayed
                } else {
                    0.0
                };
            }
            self.write += 1;
            if self.write == self.frames {
                self.write = 0;
            }
            self.primed = self.primed.saturating_add(1);
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
    fn parameter_info(&self) -> Vec<DeviceParameter> {
        vec![DeviceParameter {
            id: ParamId::new("gain"),
            label: "Gain".to_owned(),
            default: ParamValue::Float(0.25),
            min: 0.0,
            max: 1.0,
            step_count: 0,
            unit: String::new(),
            read_only: false,
        }]
    }

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
    fn channel_strip_applies_db_gain_and_linear_pan() {
        let node = NodeId::from_raw(1);
        let mut params = ParamBuffer::new();
        params.ensure(
            ParamAddress {
                node,
                param: ParamId::new(STRIP_GAIN_PARAM),
            },
            odaw_core::ParamValue::Float(-6.0),
        );
        params.ensure(
            ParamAddress {
                node,
                param: ParamId::new(STRIP_PAN_PARAM),
            },
            odaw_core::ParamValue::Float(1.0),
        );

        let mut strip = ChannelStrip::new(node);
        let input = vec![1.0, 1.0, 0.5, 0.5];
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
        strip.process(&mut block);

        let gain = 10f32.powf(-6.0 / 20.0);
        assert!((output[0] - 0.0).abs() < 1e-6);
        assert!((output[1] - gain).abs() < 1e-6);
        assert!((output[2] - 0.0).abs() < 1e-6);
        assert!((output[3] - 0.5 * gain).abs() < 1e-6);
    }

    #[test]
    fn channel_strip_reports_post_fader_levels() {
        let node = NodeId::from_raw(1);
        let params = ParamBuffer::new();
        let meter = Arc::new(Meter::new());
        let input = vec![0.4, -0.8, 0.2, -0.1];
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
        ChannelStrip::new(node)
            .with_meter(Arc::clone(&meter))
            .process(&mut block);
        assert_eq!(meter.levels(), (0.4, 0.8));
    }

    #[test]
    fn channel_strip_at_unity_is_transparent() {
        let node = NodeId::from_raw(1);
        let params = ParamBuffer::new();
        let input = vec![0.25, -0.25, 0.5, -0.5];
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
        ChannelStrip::new(node).process(&mut block);
        assert_eq!(output, input);
    }

    #[test]
    fn delay_holds_samples_for_its_latency() {
        let node = NodeId::from_raw(1);
        let params = ParamBuffer::new();
        let mut delay = Delay::new(2, 1);
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let mut output = output_buffer(4, 1);
        let mut block = Block {
            node,
            sample_rate: 48_000,
            channels: 1,
            start_frame: 0,
            frames: 4,
            input: &input,
            output: &mut output,
            params: &params,
            event_target: None,
            events: &[],
        };
        delay.process(&mut block);
        assert_eq!(output, vec![0.0, 0.0, 1.0, 2.0]);
        assert_eq!(delay.latency_frames(), 2);
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
