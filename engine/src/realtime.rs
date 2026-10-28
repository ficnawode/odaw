use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use odaw_core::{ParamAddress, ParamValue, Project};
use rtrb::{Consumer, Producer, PushError, RingBuffer};
use thiserror::Error;

use crate::engine::{Engine, EngineError};
use crate::meter::MeterBank;
use crate::plan::CompiledProject;
use crate::scheduler::{Scheduler, SchedulerError};

#[derive(Debug)]
pub struct RealtimeState {
    pub engine: Engine,
    pub scheduler: Scheduler,
    pub end_frame: Option<u64>,
    pub meters: MeterBank,
}

impl RealtimeState {
    pub fn new(
        compiled: CompiledProject,
        project: &Project,
        block_size: NonZeroUsize,
        end_frame: Option<u64>,
    ) -> Result<Self, RealtimeStateError> {
        Self::new_at(compiled, project, block_size, end_frame, 0)
    }

    pub fn new_at(
        compiled: CompiledProject,
        project: &Project,
        block_size: NonZeroUsize,
        end_frame: Option<u64>,
        start_frame: u64,
    ) -> Result<Self, RealtimeStateError> {
        let meters = compiled.meters.clone();
        Ok(Self {
            engine: Engine::new_at(compiled, block_size, start_frame)?,
            scheduler: Scheduler::new(project)?,
            end_frame,
            meters,
        })
    }
}

enum ToAudio {
    Replace(Box<RealtimeState>),
    Seek(u64),
    Param {
        address: ParamAddress,
        value: ParamValue,
    },
    Play,
    Stop,
}

pub struct RealtimeEngine {
    commands: Consumer<ToAudio>,
    retired: Producer<Box<RealtimeState>>,
    state: Option<RealtimeState>,
    pending_retired: Option<Box<RealtimeState>>,
    playing: bool,
    events: Vec<crate::NoteEvent>,
    position: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
}

pub struct RealtimeHandle {
    commands: Producer<ToAudio>,
    retired: Consumer<Box<RealtimeState>>,
    position: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    meters: MeterBank,
}

pub fn realtime(state: RealtimeState, capacity: usize) -> (RealtimeEngine, RealtimeHandle) {
    let capacity = capacity.max(1);
    let (commands, audio_commands) = RingBuffer::new(capacity);
    let (audio_retired, retired) = RingBuffer::new(capacity);
    let position = Arc::new(AtomicU64::new(0));
    let finished = Arc::new(AtomicBool::new(false));

    let meters = state.meters.clone();
    let engine = RealtimeEngine {
        commands: audio_commands,
        retired: audio_retired,
        state: Some(state),
        pending_retired: None,
        playing: false,
        events: Vec::with_capacity(4096),
        position: Arc::clone(&position),
        finished: Arc::clone(&finished),
    };
    let handle = RealtimeHandle {
        commands,
        retired,
        position,
        finished,
        meters,
    };
    (engine, handle)
}

impl RealtimeEngine {
    pub fn process(&mut self, output: &mut [f32], channels: usize, sample_rate: u32) {
        self.flush_retired();
        if self.pending_retired.is_none() {
            self.drain_commands();
        }

        let Some(state) = self.state.as_mut() else {
            output.fill(0.0);
            return;
        };
        if !self.playing {
            output.fill(0.0);
            return;
        }
        if channels == 0
            || channels != state.engine.channels()
            || sample_rate != state.engine.sample_rate()
        {
            output.fill(0.0);
            self.playing = false;
            self.finished.store(true, Ordering::Release);
            return;
        }

        output.fill(0.0);
        let frames = output.len() / channels;
        let mut frame_offset = 0;
        while frame_offset < frames {
            let start = state.engine.start_frame();
            if state.end_frame.is_some_and(|end| start >= end) {
                self.playing = false;
                self.finished.store(true, Ordering::Release);
                break;
            }

            let mut chunk = (frames - frame_offset).min(state.engine.block_size());
            if let Some(end) = state.end_frame {
                let remaining = usize::try_from(end.saturating_sub(start)).unwrap_or(usize::MAX);
                chunk = chunk.min(remaining);
            }
            if chunk == 0 {
                break;
            }

            state
                .scheduler
                .events_for_block(start, chunk, &mut self.events);
            let slice_start = frame_offset * channels;
            let slice_end = slice_start + chunk * channels;
            if state
                .engine
                .process_block(chunk, &self.events, &mut output[slice_start..slice_end])
                .is_err()
            {
                self.playing = false;
                self.finished.store(true, Ordering::Release);
                break;
            }
            frame_offset += chunk;
        }

        self.position
            .store(state.engine.start_frame(), Ordering::Relaxed);
        if state
            .end_frame
            .is_some_and(|end| state.engine.start_frame() >= end)
        {
            self.playing = false;
            self.finished.store(true, Ordering::Release);
        }
    }

    fn drain_commands(&mut self) {
        while let Ok(command) = self.commands.pop() {
            match command {
                ToAudio::Replace(state) => {
                    if let Some(previous) = self.state.replace(*state) {
                        self.pending_retired = Some(Box::new(previous));
                    }
                    if let Some(state) = &self.state {
                        self.position
                            .store(state.engine.start_frame(), Ordering::Relaxed);
                    }
                    self.finished.store(false, Ordering::Relaxed);
                    if self.pending_retired.is_some() {
                        break;
                    }
                }
                ToAudio::Seek(frame) => {
                    if let Some(state) = &mut self.state {
                        state.engine.seek(frame);
                    }
                    self.position.store(frame, Ordering::Relaxed);
                    self.finished.store(false, Ordering::Relaxed);
                }
                ToAudio::Param { address, value } => {
                    if let Some(state) = &mut self.state {
                        state.engine.params_mut().set(&address, value);
                    }
                }
                ToAudio::Play => {
                    self.playing = true;
                    self.finished.store(false, Ordering::Relaxed);
                }
                ToAudio::Stop => {
                    self.playing = false;
                }
            }
        }
    }

    fn flush_retired(&mut self) {
        let Some(state) = self.pending_retired.take() else {
            return;
        };
        if let Err(PushError::Full(state)) = self.retired.push(state) {
            self.pending_retired = Some(state);
        }
    }
}

impl RealtimeHandle {
    pub fn play(&mut self) -> Result<(), RealtimeError> {
        self.finished.store(false, Ordering::Release);
        self.send(ToAudio::Play)
    }

    pub fn stop(&mut self) -> Result<(), RealtimeError> {
        self.send(ToAudio::Stop)
    }

    pub fn set_param(
        &mut self,
        address: ParamAddress,
        value: ParamValue,
    ) -> Result<(), RealtimeError> {
        self.send(ToAudio::Param { address, value })
    }

    pub fn publish(&mut self, state: RealtimeState) -> Result<(), RealtimeError> {
        self.publish_at(state, 0)
    }

    pub fn publish_at(
        &mut self,
        mut state: RealtimeState,
        start_frame: u64,
    ) -> Result<(), RealtimeError> {
        state.engine.seek(start_frame);
        self.collect_retired();
        self.meters = state.meters.clone();
        self.position.store(start_frame, Ordering::Relaxed);
        self.finished.store(false, Ordering::Release);
        self.send(ToAudio::Replace(Box::new(state)))
    }

    pub fn meter_levels(&self, node: odaw_core::NodeId) -> Option<(f32, f32)> {
        self.meters.levels(node)
    }

    pub fn seek(&mut self, frame: u64) -> Result<(), RealtimeError> {
        self.position.store(frame, Ordering::Relaxed);
        self.finished.store(false, Ordering::Release);
        self.send(ToAudio::Seek(frame))
    }

    pub fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn collect_retired(&mut self) {
        while self.retired.pop().is_ok() {}
    }

    fn send(&mut self, command: ToAudio) -> Result<(), RealtimeError> {
        self.commands
            .push(command)
            .map_err(|_| RealtimeError::QueueFull)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RealtimeError {
    #[error("realtime command queue is full")]
    QueueFull,
}

#[derive(Debug, Error)]
pub enum RealtimeStateError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::ParamBuffer;
    use crate::plan::{Block, DeviceProcessor, PlanStep, RenderPlan};
    use odaw_core::NodeId;

    #[derive(Debug)]
    struct Constant(f32);

    impl DeviceProcessor for Constant {
        fn process(&mut self, block: &mut Block<'_>) {
            block.output.fill(self.0);
        }
    }

    #[derive(Debug)]
    struct ParamSource;

    impl DeviceProcessor for ParamSource {
        fn process(&mut self, block: &mut Block<'_>) {
            let address = ParamAddress {
                node: block.node,
                param: odaw_core::ParamId::new("value"),
            };
            block.output.fill(block.params.float(&address, 0.0) as f32);
        }
    }

    fn state(processor: impl DeviceProcessor + 'static, end_frame: Option<u64>) -> RealtimeState {
        let plan = RenderPlan {
            sample_rate: 48_000,
            channels: 2,
            steps: vec![PlanStep {
                node: NodeId::from_raw(1),
                inputs: Vec::new(),
                output: 0,
                event_target: None,
                processor: Box::new(processor),
            }],
            master: Some(0),
        };
        let engine = Engine::new(
            CompiledProject {
                plan,
                params: ParamBuffer::new(),
                meters: MeterBank::default(),
            },
            NonZeroUsize::new(64).unwrap(),
        )
        .unwrap();
        RealtimeState {
            engine,
            scheduler: Scheduler::default(),
            end_frame,
            meters: MeterBank::default(),
        }
    }

    #[test]
    fn play_and_stop_control_output() {
        let (mut audio, mut handle) = realtime(state(Constant(0.5), None), 8);
        let mut output = [0.0; 4];

        audio.process(&mut output, 2, 48_000);
        assert_eq!(output, [0.0; 4]);

        handle.play().unwrap();
        audio.process(&mut output, 2, 48_000);
        assert_eq!(output, [0.5; 4]);
        assert_eq!(handle.position(), 2);

        handle.stop().unwrap();
        audio.process(&mut output, 2, 48_000);
        assert_eq!(output, [0.0; 4]);
    }

    #[test]
    fn end_frame_stops_with_silence_and_finishes() {
        let (mut audio, mut handle) = realtime(state(Constant(0.5), Some(3)), 8);
        handle.play().unwrap();
        let mut output = [0.0; 8];
        audio.process(&mut output, 2, 48_000);
        assert_eq!(&output[..6], &[0.5; 6]);
        assert_eq!(&output[6..], &[0.0; 2]);
        assert!(handle.finished());
        assert_eq!(handle.position(), 3);
    }

    #[test]
    fn parameters_can_be_updated_without_replacing_the_engine() {
        let address = ParamAddress {
            node: NodeId::from_raw(1),
            param: odaw_core::ParamId::new("value"),
        };
        let mut initial = state(ParamSource, None);
        initial
            .engine
            .params_mut()
            .ensure(address.clone(), ParamValue::Float(0.25));
        let (mut audio, mut handle) = realtime(initial, 8);
        handle.play().unwrap();
        let mut output = [0.0; 4];
        audio.process(&mut output, 2, 48_000);
        assert_eq!(output, [0.25; 4]);

        handle.set_param(address, ParamValue::Float(0.75)).unwrap();
        audio.process(&mut output, 2, 48_000);
        assert_eq!(output, [0.75; 4]);
    }

    #[test]
    fn publishing_a_new_engine_swaps_at_a_block_boundary() {
        let (mut audio, mut handle) = realtime(state(Constant(0.25), None), 8);
        handle.play().unwrap();
        let mut output = [0.0; 4];
        audio.process(&mut output, 2, 48_000);
        assert_eq!(output, [0.25; 4]);

        handle.publish(state(Constant(0.75), None)).unwrap();
        audio.process(&mut output, 2, 48_000);
        assert_eq!(output, [0.75; 4]);
        handle.collect_retired();
    }

    #[test]
    fn meters_are_shared_with_the_handle() {
        #[derive(Debug)]
        struct Metered {
            meter: Arc<crate::Meter>,
        }

        impl DeviceProcessor for Metered {
            fn process(&mut self, block: &mut Block<'_>) {
                block.output.fill(0.5);
                self.meter.store_block(block.output, block.channels);
            }
        }

        let node = NodeId::from_raw(1);
        let bank = MeterBank::new([node]);
        let processor = Metered {
            meter: bank.slot(node).expect("the bank has a meter"),
        };
        let plan = RenderPlan {
            sample_rate: 48_000,
            channels: 2,
            steps: vec![PlanStep {
                node,
                inputs: Vec::new(),
                output: 0,
                event_target: None,
                processor: Box::new(processor),
            }],
            master: Some(0),
        };
        let engine = Engine::new(
            CompiledProject {
                plan,
                params: ParamBuffer::new(),
                meters: bank.clone(),
            },
            NonZeroUsize::new(64).unwrap(),
        )
        .unwrap();
        let state = RealtimeState {
            engine,
            scheduler: Scheduler::default(),
            end_frame: None,
            meters: bank,
        };
        let (mut audio, mut handle) = realtime(state, 8);
        handle.play().unwrap();
        let mut output = [0.0; 4];
        audio.process(&mut output, 2, 48_000);
        assert_eq!(handle.meter_levels(node), Some((0.5, 0.5)));
    }

    #[test]
    fn seeking_moves_the_engine_position() {
        let (mut audio, mut handle) = realtime(state(Constant(0.5), None), 8);
        handle.play().unwrap();
        let mut output = [0.0; 4];
        audio.process(&mut output, 2, 48_000);
        assert_eq!(handle.position(), 2);

        handle.seek(1_000).unwrap();
        audio.process(&mut output, 2, 48_000);
        assert_eq!(handle.position(), 1_002);
    }

    #[test]
    fn publishing_at_a_frame_starts_there_and_reports_it() {
        let (mut audio, mut handle) = realtime(state(Constant(0.25), None), 8);
        handle.play().unwrap();
        handle.publish_at(state(Constant(0.75), None), 500).unwrap();
        assert_eq!(handle.position(), 500);

        let mut output = [0.0; 4];
        audio.process(&mut output, 2, 48_000);
        assert_eq!(output, [0.75; 4]);
        assert_eq!(handle.position(), 502);
        handle.collect_retired();
    }

    #[test]
    fn a_full_command_queue_is_reported() {
        let (_audio, mut handle) = realtime(state(Constant(0.0), None), 1);
        handle.play().unwrap();
        assert_eq!(handle.play(), Err(RealtimeError::QueueFull));
    }

    #[test]
    fn a_channel_mismatch_stops_the_engine() {
        let (mut audio, mut handle) = realtime(state(Constant(0.5), None), 8);
        handle.play().unwrap();
        let mut output = [1.0; 4];
        audio.process(&mut output, 1, 48_000);
        assert_eq!(output, [0.0; 4]);
        assert!(handle.finished());
    }
}
