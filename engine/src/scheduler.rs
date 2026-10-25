use odaw_core::{ClipContent, NodeId, Project, TempoError, Tick};
use thiserror::Error;

use crate::plan::{NoteEvent, NoteEventKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduledNote {
    pub target: NodeId,
    pub start_frame: u64,
    pub end_frame: u64,
    pub pitch: u8,
    pub velocity: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Scheduler {
    notes: Vec<ScheduledNote>,
}

impl Scheduler {
    pub fn new(project: &Project) -> Result<Self, SchedulerError> {
        let mut notes = Vec::new();
        for clip in project.clips.values() {
            let ClipContent::Midi {
                notes: clip_notes, ..
            } = &clip.content
            else {
                continue;
            };
            for note in clip_notes {
                let start_tick = Tick::from_raw(clip.start.raw().saturating_add(note.start.raw()));
                let end_tick = Tick::from_raw(start_tick.raw().saturating_add(note.length.raw()));
                let start_frame = project.tempo.tick_to_frames(start_tick)?.raw().max(0) as u64;
                let end_frame = project.tempo.tick_to_frames(end_tick)?.raw().max(0) as u64;
                if end_frame <= start_frame {
                    continue;
                }
                notes.push(ScheduledNote {
                    target: clip.node,
                    start_frame,
                    end_frame,
                    pitch: note.pitch,
                    velocity: note.velocity,
                });
            }
        }
        notes.sort_unstable_by_key(|note| {
            (
                note.start_frame,
                note.end_frame,
                note.target.raw(),
                note.pitch,
                note.velocity,
            )
        });
        Ok(Self { notes })
    }

    pub fn notes(&self) -> &[ScheduledNote] {
        &self.notes
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    pub fn events_for_block(&self, start_frame: u64, frames: usize, events: &mut Vec<NoteEvent>) {
        events.clear();
        let block_end = start_frame.saturating_add(frames as u64);
        for note in &self.notes {
            if note.start_frame >= start_frame && note.start_frame < block_end {
                push_event(
                    events,
                    NoteEvent {
                        target: note.target,
                        frame: (note.start_frame - start_frame) as u32,
                        kind: NoteEventKind::On,
                        pitch: note.pitch,
                        velocity: note.velocity,
                    },
                );
            }
            if note.end_frame >= start_frame && note.end_frame < block_end {
                push_event(
                    events,
                    NoteEvent {
                        target: note.target,
                        frame: (note.end_frame - start_frame) as u32,
                        kind: NoteEventKind::Off,
                        pitch: note.pitch,
                        velocity: 0,
                    },
                );
            }
        }
        events.sort_unstable_by_key(|event| {
            (
                event.frame,
                event.kind == NoteEventKind::On,
                event.target.raw(),
                event.pitch,
                event.velocity,
            )
        });
    }
}

fn push_event(events: &mut Vec<NoteEvent>, event: NoteEvent) {
    if events.len() < events.capacity() || events.capacity() == 0 {
        events.push(event);
    }
}

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Tempo(#[from] TempoError),
}

#[cfg(test)]
mod tests {
    use odaw_core::{
        Clip, ClipContent, ClipId, Node, NodeId, NodeRole, Note, NoteId, Project, Tick,
    };

    use super::*;

    fn project() -> Project {
        let mut project = Project::empty("demo", 48_000, 0).unwrap();
        let track = NodeId::from_raw(1);
        let clip_id = ClipId::from_raw(2);
        project.nodes.insert(track, Node::new(NodeRole::Track));
        project.clips.insert(
            clip_id,
            Clip {
                node: track,
                start: Tick::ZERO,
                length: Tick::from_raw(16 * odaw_core::PPQ),
                content: ClipContent::Midi {
                    source_start: Tick::ZERO,
                    source_length: Tick::from_raw(16 * odaw_core::PPQ),
                    notes: [
                        Note {
                            id: NoteId::from_raw(10),
                            pitch: 60,
                            start: Tick::ZERO,
                            length: Tick::from_raw(odaw_core::PPQ),
                            velocity: 100,
                            channel: 0,
                        },
                        Note {
                            id: NoteId::from_raw(11),
                            pitch: 64,
                            start: Tick::from_raw(odaw_core::PPQ),
                            length: Tick::from_raw(odaw_core::PPQ),
                            velocity: 90,
                            channel: 0,
                        },
                    ]
                    .into_iter()
                    .collect(),
                },
            },
        );
        project
            .nodes
            .get_mut(&track)
            .unwrap()
            .clips
            .push_back(clip_id);
        project
    }

    #[test]
    fn notes_are_converted_to_absolute_frames() {
        let scheduler = Scheduler::new(&project()).unwrap();
        assert_eq!(scheduler.len(), 2);
        assert_eq!(scheduler.notes()[0].start_frame, 0);
        assert_eq!(scheduler.notes()[0].end_frame, 24_000);
        assert_eq!(scheduler.notes()[1].start_frame, 24_000);
        assert_eq!(scheduler.notes()[1].end_frame, 48_000);
        assert_eq!(scheduler.notes()[0].target, NodeId::from_raw(1));
    }

    #[test]
    fn events_are_sample_accurate_and_ordered() {
        let scheduler = Scheduler::new(&project()).unwrap();
        let mut events = Vec::new();

        scheduler.events_for_block(0, 1_000, &mut events);
        assert_eq!(
            events,
            vec![NoteEvent {
                target: NodeId::from_raw(1),
                frame: 0,
                kind: NoteEventKind::On,
                pitch: 60,
                velocity: 100,
            }]
        );

        scheduler.events_for_block(23_900, 200, &mut events);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, NoteEventKind::Off);
        assert_eq!(events[0].pitch, 60);
        assert_eq!(events[0].frame, 100);
        assert_eq!(events[1].kind, NoteEventKind::On);
        assert_eq!(events[1].pitch, 64);
        assert_eq!(events[1].frame, 100);

        scheduler.events_for_block(48_000, 1_000, &mut events);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, NoteEventKind::Off);
        assert_eq!(events[0].pitch, 64);
        assert_eq!(events[0].frame, 0);

        scheduler.events_for_block(50_000, 1_000, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn preallocated_event_buffers_do_not_grow() {
        let scheduler = Scheduler::new(&project()).unwrap();
        let mut capped = Vec::with_capacity(1);
        scheduler.events_for_block(23_900, 200, &mut capped);
        assert_eq!(capped.len(), 1);
        assert_eq!(capped.capacity(), 1);

        let mut offline = Vec::new();
        scheduler.events_for_block(23_900, 200, &mut offline);
        assert_eq!(offline.len(), 2);
    }
}
