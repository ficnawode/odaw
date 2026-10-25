use std::collections::BTreeMap;
use std::sync::Arc;

use odaw_core::AssetId;
use odaw_engine::{Block, DeviceProcessor};

/// Decoded PCM for one asset. The samples stay interleaved at the source
/// sample rate; the clip player resamples on the fly.
#[derive(Clone, Debug)]
pub struct AudioAsset {
    pub sample_rate: u32,
    pub channels: usize,
    pub frames: usize,
    samples: Arc<[f32]>,
}

impl AudioAsset {
    pub fn new(sample_rate: u32, channels: usize, samples: Arc<[f32]>) -> Option<Self> {
        if sample_rate == 0 || channels == 0 || samples.is_empty() {
            return None;
        }
        let frames = samples.len() / channels;
        (frames > 0).then_some(Self {
            sample_rate,
            channels,
            frames,
            samples,
        })
    }

    pub fn from_samples(sample_rate: u32, channels: usize, samples: Vec<f32>) -> Option<Self> {
        Self::new(sample_rate, channels, samples.into())
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frames as f64 / f64::from(self.sample_rate)
    }

    fn channel(&self, frame: usize, channel: usize) -> f32 {
        let channel = if self.channels == 1 {
            0
        } else {
            channel.min(self.channels - 1)
        };
        self.samples[frame * self.channels + channel]
    }

    fn sample_at(&self, position: f64) -> (f32, f32) {
        if !position.is_finite() || position < 0.0 {
            return (0.0, 0.0);
        }
        let frame = position.floor() as usize;
        if frame >= self.frames {
            return (0.0, 0.0);
        }
        let next = (frame + 1).min(self.frames - 1);
        let fraction = (position - frame as f64) as f32;
        let left = self.channel(frame, 0);
        let right = self.channel(frame, 1);
        let next_left = self.channel(next, 0);
        let next_right = self.channel(next, 1);
        (
            left + (next_left - left) * fraction,
            right + (next_right - right) * fraction,
        )
    }
}

pub type AudioSources = BTreeMap<AssetId, Arc<AudioAsset>>;

/// One audio clip as it appears in the compiled plan: an asset plus a timeline
/// range and the source frame range that should sound within it.
#[derive(Clone, Debug)]
pub struct PlayableClip {
    pub asset: Arc<AudioAsset>,
    pub start_frame: u64,
    pub end_frame: u64,
    pub source_start: u64,
    pub source_length: u64,
}

impl PlayableClip {
    pub fn new(
        asset: Arc<AudioAsset>,
        start_frame: u64,
        end_frame: u64,
        source_start: u64,
        source_length: u64,
    ) -> Self {
        Self {
            asset,
            start_frame,
            end_frame,
            source_start,
            source_length,
        }
    }
}

/// Sums every audio clip on one track into a stereo source signal. The track's
/// channel strip is still responsible for gain, pan, mute, and metering.
#[derive(Debug, Default)]
pub struct AudioClipPlayer {
    project_sample_rate: u32,
    clips: Vec<PlayableClip>,
}

impl AudioClipPlayer {
    pub fn new(project_sample_rate: u32, clips: Vec<PlayableClip>) -> Self {
        Self {
            project_sample_rate,
            clips,
        }
    }

    pub fn clips(&self) -> &[PlayableClip] {
        &self.clips
    }
}

impl DeviceProcessor for AudioClipPlayer {
    fn process(&mut self, block: &mut Block<'_>) {
        block.output.fill(0.0);
        if self.clips.is_empty() || block.channels == 0 || self.project_sample_rate == 0 {
            return;
        }

        let block_start = block.start_frame;
        let block_end = block_start.saturating_add(block.frames as u64);
        let project_rate = f64::from(self.project_sample_rate);
        let channels = block.channels;

        for clip in &self.clips {
            let from = clip.start_frame.max(block_start);
            let to = clip.end_frame.min(block_end);
            if from >= to {
                continue;
            }
            let step = f64::from(clip.asset.sample_rate) / project_rate;
            let available = clip
                .source_length
                .min((clip.asset.frames as u64).saturating_sub(clip.source_start));

            for position in from..to {
                let elapsed = position - clip.start_frame;
                let source = clip.source_start as f64 + elapsed as f64 * step;
                let relative = source - clip.source_start as f64;
                if relative >= available as f64 {
                    break;
                }
                let (left, right) = clip.asset.sample_at(source);
                let frame = (position - block_start) as usize;
                let offset = frame * channels;
                if channels == 1 {
                    block.output[offset] += (left + right) * 0.5;
                } else {
                    block.output[offset] += left;
                    block.output[offset + 1] += right;
                    for channel in 2..channels {
                        block.output[offset + channel] += (left + right) * 0.5;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_ramp(rate: u32, frames: usize) -> Arc<AudioAsset> {
        let mut samples = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            let value = frame as f32 / frames as f32;
            samples.push(value);
            samples.push(-value);
        }
        Arc::new(AudioAsset::from_samples(rate, 2, samples).unwrap())
    }

    fn player(asset: Arc<AudioAsset>, start: u64, end: u64, source_start: u64) -> AudioClipPlayer {
        AudioClipPlayer::new(
            48_000,
            vec![PlayableClip::new(asset, start, end, source_start, 100)],
        )
    }

    fn process(player: &mut AudioClipPlayer, start_frame: u64, frames: usize) -> Vec<f32> {
        let input = vec![0.0; frames * 2];
        let mut output = vec![0.0; frames * 2];
        let params = odaw_engine::ParamBuffer::new();
        let mut block = Block {
            node: odaw_core::NodeId::from_raw(1),
            event_target: None,
            sample_rate: 48_000,
            channels: 2,
            start_frame,
            frames,
            input: &input,
            output: &mut output,
            params: &params,
            events: &[],
        };
        player.process(&mut block);
        output
    }

    #[test]
    fn audio_clips_play_inside_their_timeline_range() {
        let asset = stereo_ramp(48_000, 100);
        let mut player = player(asset, 10, 20, 0);
        let before = process(&mut player, 0, 8);
        assert!(before.iter().all(|sample| *sample == 0.0));

        let during = process(&mut player, 10, 4);
        assert!(during.iter().any(|sample| *sample != 0.0));
        assert!((during[0] - 0.0).abs() < 1e-6);
        assert!((during[2] - 0.01).abs() < 1e-6);
        assert!((during[3] + 0.01).abs() < 1e-6);
    }

    #[test]
    fn seeking_into_a_clip_reads_the_matching_source_frame() {
        let asset = stereo_ramp(48_000, 100);
        let mut player = player(asset, 10, 20, 0);
        let output = process(&mut player, 12, 2);
        assert!((output[0] - 0.02).abs() < 1e-6);
        assert!((output[2] - 0.03).abs() < 1e-6);
    }

    #[test]
    fn source_offsets_and_resampling_are_honoured() {
        let asset = stereo_ramp(24_000, 100);
        let mut player = player(asset, 0, 20, 10);
        let output = process(&mut player, 0, 4);
        assert!((output[0] - 0.10).abs() < 1e-5);
        assert!((output[2] - 0.105).abs() < 1e-5);
        assert!((output[4] - 0.11).abs() < 1e-5);
    }

    #[test]
    fn mono_assets_are_duplicated_to_both_channels() {
        let asset = Arc::new(AudioAsset::from_samples(48_000, 1, vec![0.25, 0.5, 0.75]).unwrap());
        let mut player = player(asset, 0, 3, 0);
        let output = process(&mut player, 0, 3);
        assert_eq!(output, vec![0.25, 0.25, 0.5, 0.5, 0.75, 0.75]);
    }

    #[test]
    fn overlapping_clips_are_summed() {
        let asset = Arc::new(AudioAsset::from_samples(48_000, 2, vec![0.25; 200]).unwrap());
        let mut player = AudioClipPlayer::new(
            48_000,
            vec![
                PlayableClip::new(Arc::clone(&asset), 0, 10, 0, 10),
                PlayableClip::new(asset, 0, 10, 0, 10),
            ],
        );
        let output = process(&mut player, 0, 2);
        assert_eq!(output, vec![0.5, 0.5, 0.5, 0.5]);
    }
}
