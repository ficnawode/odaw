use std::num::NonZeroUsize;

use odaw_compile::{CompileError, Compiler};
use odaw_core::{Project, TempoError, Tick};
use odaw_engine::{Engine, EngineError, Scheduler, SchedulerError};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub block_size: NonZeroUsize,
    pub frames: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedAudio {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

pub fn render_project(
    project: &Project,
    options: RenderOptions,
) -> Result<RenderedAudio, RenderError> {
    let compiled = Compiler::builtin().compile(project)?;
    let sample_rate = compiled.plan.sample_rate;
    let channels = compiled.plan.channels;
    let mut engine = Engine::new(compiled, options.block_size)?;
    let scheduler = Scheduler::new(project)?;

    let total =
        usize::try_from(options.frames).map_err(|_| RenderError::TooManyFrames(options.frames))?;
    let sample_count = total
        .checked_mul(channels)
        .ok_or(RenderError::TooManyFrames(options.frames))?;
    let mut samples = vec![0.0; sample_count];

    let block_size = options.block_size.get();
    let mut events = Vec::new();
    let mut offset = 0usize;
    while offset < total {
        let frames = (total - offset).min(block_size);
        scheduler.events_for_block(offset as u64, frames, &mut events);
        let start = offset * channels;
        let end = start + frames * channels;
        engine.process_block(frames, &events, &mut samples[start..end])?;
        offset += frames;
    }

    Ok(RenderedAudio {
        sample_rate,
        channels,
        samples,
    })
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error(transparent)]
    Compile(#[from] CompileError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Schedule(#[from] SchedulerError),
    #[error("{0} frames do not fit in memory")]
    TooManyFrames(u64),
}

pub fn project_end_tick(project: &Project) -> Option<Tick> {
    let mut end: Option<Tick> = None;
    for loop_region in &project.loops {
        end = Some(end.map_or(loop_region.end, |current| current.max(loop_region.end)));
    }
    for clip in project.clips.values() {
        let clip_end = Tick::from_raw(clip.start.raw().saturating_add(clip.length.raw()));
        end = Some(end.map_or(clip_end, |current| current.max(clip_end)));
    }
    end.filter(|tick| *tick > Tick::ZERO)
}

pub fn project_end_frame(project: &Project) -> Result<Option<u64>, TempoError> {
    project_end_tick(project)
        .map(|tick| {
            project
                .tempo
                .tick_to_frames(tick)
                .map(|frames| frames.raw().max(0) as u64)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use odaw_dsp::demo;

    fn options(block_size: usize, frames: u64) -> RenderOptions {
        RenderOptions {
            block_size: NonZeroUsize::new(block_size).unwrap(),
            frames,
        }
    }

    #[test]
    fn demo_renders_audio() {
        let audio = render_project(&demo::project(), options(64, 1_000)).unwrap();
        assert_eq!(audio.sample_rate, 48_000);
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.samples.len(), 2_000);
        assert!(audio.samples.iter().any(|sample| *sample != 0.0));
    }

    #[test]
    fn rendering_is_deterministic() {
        let first = render_project(&demo::project(), options(128, 2_000)).unwrap();
        let second = render_project(&demo::project(), options(128, 2_000)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn output_is_block_size_independent() {
        let first = render_project(&demo::project(), options(32, 1_500)).unwrap();
        let second = render_project(&demo::project(), options(64, 1_500)).unwrap();
        let third = render_project(&demo::project(), options(512, 1_500)).unwrap();
        assert_eq!(first.samples, second.samples);
        assert_eq!(second.samples, third.samples);
    }

    #[test]
    fn first_samples_follow_the_expected_voice_gain() {
        let audio = render_project(&demo::project(), options(16, 4)).unwrap();
        let frequency = 440.0 * 2f64.powf((60.0 - 69.0) / 12.0);
        let expected = (2.0 * std::f64::consts::PI * frequency / 48_000.0).sin()
            * 0.25
            * 0.5
            * (100.0 / 127.0);
        assert_eq!(audio.samples[0], 0.0);
        assert_eq!(audio.samples[1], 0.0);
        assert!((f64::from(audio.samples[2]) - expected).abs() < 1e-6);
        assert!((f64::from(audio.samples[3]) - expected).abs() < 1e-6);
    }

    #[test]
    fn zero_frames_render_an_empty_buffer() {
        let audio = render_project(&demo::project(), options(64, 0)).unwrap();
        assert!(audio.samples.is_empty());
    }

    #[test]
    fn project_end_comes_from_loops_and_clips() {
        let project = demo::project();
        assert_eq!(
            project_end_tick(&project),
            Some(odaw_core::Tick::from_raw(16 * odaw_core::PPQ))
        );
        assert_eq!(project_end_frame(&project), Ok(Some(384_000)));
        assert_eq!(
            project_end_tick(&Project::empty("empty", 48_000, 0).unwrap()),
            None
        );
    }
}
