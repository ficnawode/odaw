use std::num::NonZeroUsize;
use std::path::PathBuf;

use odaw_compile::Compiler;
use odaw_control::{
    ClipCommand, Command, CommandKind, EdgeCommand, Event, NodeCommand, NoteCommand, Session,
    SessionError,
};
use odaw_core::{
    ChannelLayout, Clip, ClipContent, ClipId, Edge, Node, NodeId, NodeKind, NodePort, NodeRole,
    Note, NoteId, PPQ, ParamId, ParamValue, Port, PortDirection, PortId, Project, Tick,
};
use odaw_dsp::demo;
use odaw_engine::{RealtimeEngine, RealtimeHandle, RealtimeState, realtime};
use odaw_format::Container;
use odaw_platform::{AudioBackend, AudioCallback};
use odaw_render::project_end_frame;

const TRACK_LENGTH: i64 = 16 * PPQ;

pub struct Editor {
    pub session: Session,
    pub path: Option<PathBuf>,
    pub selected_clip: Option<ClipId>,
    pub playing: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    audio: Option<AudioSession>,
    audio_error: Option<String>,
}

struct AudioSession {
    backend: AudioBackend,
    handle: RealtimeHandle,
    sample_rate: u32,
    end_frame: u64,
}

struct Player {
    engine: RealtimeEngine,
}

impl AudioCallback for Player {
    fn process(&mut self, output: &mut [f32], channels: usize, sample_rate: u32) {
        self.engine.process(output, channels, sample_rate);
    }
}

impl Editor {
    pub fn new(path: Option<PathBuf>) -> Self {
        let (project, path, load_error) = match path {
            Some(path) => match Container::new(&path).load() {
                Ok(project) => (project, Some(path), None),
                Err(error) => (
                    demo::project(),
                    None,
                    Some(format!("could not open {}: {error}", path.display())),
                ),
            },
            None => (demo::project(), None, None),
        };

        let mut error = load_error;
        let session = match Session::new(project) {
            Ok(session) => session,
            Err(session_error) => {
                error = Some(session_error.to_string());
                Session::new(demo::project()).expect("demo project is valid")
            }
        };
        let mut editor = Self {
            session,
            path,
            selected_clip: None,
            playing: false,
            status: None,
            error,
            audio: None,
            audio_error: None,
        };
        editor.ensure_selection();
        editor.start_audio();
        editor
    }

    pub fn audio_error(&self) -> Option<&str> {
        self.audio_error.as_deref()
    }

    pub fn audio_ready(&self) -> bool {
        self.audio.is_some()
    }

    pub fn start_audio(&mut self) {
        match self.create_audio() {
            Ok(audio) => {
                self.audio = Some(audio);
                self.audio_error = None;
            }
            Err(error) => {
                self.audio = None;
                self.audio_error = Some(error);
                self.playing = false;
            }
        }
    }

    fn create_audio(&self) -> Result<AudioSession, String> {
        let project = self.session.project();
        let (state, end_frame) = prepare_state(project)?;
        let sample_rate = state.engine.sample_rate();
        let channels = u16::try_from(state.engine.channels())
            .map_err(|_| "unsupported channel count".to_owned())?;
        let (engine, handle) = realtime(state, 16);
        let backend = AudioBackend::start(Player { engine }, sample_rate, channels)
            .map_err(|error| error.to_string())?;
        Ok(AudioSession {
            backend,
            handle,
            sample_rate,
            end_frame,
        })
    }

    pub fn tick(&mut self) -> bool {
        let mut failed = false;
        if let Some(audio) = &mut self.audio {
            audio.handle.collect_retired();
            if !audio.backend.is_healthy() {
                failed = true;
            } else if self.playing && audio.handle.finished() {
                self.playing = false;
            }
        }
        if failed {
            self.audio = None;
            self.audio_error = Some("the audio output stream failed".to_owned());
            self.playing = false;
        }
        self.playing
    }

    pub fn toggle_play(&mut self) {
        if self.playing {
            self.stop();
        } else {
            self.play();
        }
    }

    pub fn play(&mut self) {
        if self.audio.is_none() {
            self.start_audio();
        }
        let Some(audio) = &mut self.audio else {
            return;
        };

        if audio.handle.finished() || audio.handle.position() >= audio.end_frame {
            let project = self.session.project().clone();
            match prepare_state(&project) {
                Ok((state, end_frame)) => {
                    if let Err(error) = audio.handle.publish(state) {
                        self.audio_error = Some(error.to_string());
                        return;
                    }
                    audio.end_frame = end_frame;
                }
                Err(error) => {
                    self.audio_error = Some(error);
                    return;
                }
            }
        }

        match audio.handle.play() {
            Ok(()) => {
                self.playing = true;
                self.error = None;
            }
            Err(error) => self.audio_error = Some(error.to_string()),
        }
    }

    pub fn stop(&mut self) {
        if let Some(audio) = &mut self.audio {
            let _ = audio.handle.stop();
        }
        self.playing = false;
    }

    pub fn position_frames(&self) -> u64 {
        self.audio
            .as_ref()
            .map_or(0, |audio| audio.handle.position())
    }

    pub fn position_seconds(&self) -> f64 {
        let Some(audio) = &self.audio else {
            return 0.0;
        };
        self.position_frames() as f64 / f64::from(audio.sample_rate)
    }

    pub fn duration_seconds(&self) -> f64 {
        let Some(audio) = &self.audio else {
            return 0.0;
        };
        audio.end_frame as f64 / f64::from(audio.sample_rate)
    }

    pub fn progress(&self) -> f32 {
        let Some(audio) = &self.audio else {
            return 0.0;
        };
        if audio.end_frame == 0 {
            return 0.0;
        }
        (self.position_frames() as f64 / audio.end_frame as f64).clamp(0.0, 1.0) as f32
    }

    pub fn save(&mut self) {
        let path = self
            .path
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("{}.odaw", self.session.project().meta.name)));
        match Container::new(&path).save(self.session.project()) {
            Ok(()) => {
                self.path = Some(path.clone());
                self.status = Some(format!("saved {}", path.display()));
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub fn undo(&mut self) {
        self.session.undo();
        self.ensure_selection();
        self.rebuild_audio();
    }

    pub fn redo(&mut self) {
        self.session.redo();
        self.ensure_selection();
        self.rebuild_audio();
    }

    pub fn selected_track(&self) -> Option<NodeId> {
        let project = self.session.project();
        let selection = self.session.selection();
        if let Some(node) = selection.focused_node {
            if project
                .nodes
                .get(&node)
                .is_some_and(|node| node.role == NodeRole::Track)
            {
                return Some(node);
            }
        }
        if let Some(node) = selection
            .nodes
            .iter()
            .find(|node| {
                project
                    .nodes
                    .get(node)
                    .is_some_and(|node| node.role == NodeRole::Track)
            })
            .copied()
        {
            return Some(node);
        }
        project
            .nodes
            .iter()
            .find(|(_, node)| node.role == NodeRole::Track)
            .map(|(id, _)| *id)
    }

    pub fn ensure_selection(&mut self) {
        let Some(track) = self.selected_track() else {
            self.selected_clip = None;
            return;
        };

        let valid = self.selected_clip.is_some_and(|clip| {
            self.session
                .project()
                .clips
                .get(&clip)
                .is_some_and(|clip| clip.node == track && is_midi(clip))
        });
        if !valid {
            self.selected_clip = self.first_midi_clip(track);
        }
    }

    pub fn select_track(&mut self, track: NodeId) {
        let mut selection = odaw_control::Selection::default();
        selection.select_node(track);
        self.session.set_selection(selection);
        self.selected_clip = self.first_midi_clip(track);
    }

    pub fn select_clip(&mut self, track: NodeId, clip: ClipId) {
        self.select_track(track);
        self.selected_clip = Some(clip);
    }

    pub fn selected_clip(&self) -> Option<ClipId> {
        let track = self.selected_track()?;
        self.selected_clip
            .filter(|clip| {
                self.session
                    .project()
                    .clips
                    .get(clip)
                    .is_some_and(|clip| clip.node == track && is_midi(clip))
            })
            .or_else(|| self.first_midi_clip(track))
    }

    pub fn selected_clip_data(&self) -> Option<(ClipId, &Clip)> {
        let clip = self.selected_clip()?;
        self.session
            .project()
            .clips
            .get(&clip)
            .map(|value| (clip, value))
    }

    pub fn first_midi_clip(&self, track: NodeId) -> Option<ClipId> {
        let project = self.session.project();
        let node = project.nodes.get(&track)?;
        node.clips
            .iter()
            .copied()
            .find(|clip| project.clips.get(clip).is_some_and(is_midi))
    }

    pub fn notes(&self, clip: ClipId) -> Vec<Note> {
        let Some(clip) = self.session.project().clips.get(&clip) else {
            return Vec::new();
        };
        match &clip.content {
            ClipContent::Midi { notes, .. } => notes.iter().copied().collect(),
            ClipContent::Audio { .. } | ClipContent::Empty => Vec::new(),
        }
    }

    pub fn selected_note(&self) -> Option<Note> {
        let selection = self.session.selection();
        if selection.notes.len() != 1 {
            return None;
        }
        let id = *selection.notes.iter().next()?;
        let clip = self.selected_clip()?;
        self.notes(clip).into_iter().find(|note| note.id == id)
    }

    pub fn select_note(&mut self, note: NoteId, additive: bool) {
        let mut selection = self.session.selection().clone();
        if additive {
            if !selection.notes.remove(&note) {
                selection.notes.insert(note);
            }
        } else {
            selection.notes.clear();
            selection.notes.insert(note);
        }
        if let Some(track) = self.selected_track() {
            selection.nodes.clear();
            selection.nodes.insert(track);
            selection.focused_node = Some(track);
        }
        self.session.set_selection(selection);
    }

    pub fn add_note(&mut self, clip: ClipId, pitch: u8, start: Tick, length: Tick, velocity: u8) {
        let id = self.session.next_note_id();
        let note = Note {
            id,
            pitch,
            start,
            length,
            velocity,
            channel: 0,
        };
        let result =
            self.session
                .apply_command(Command::new(CommandKind::Note(NoteCommand::Add {
                    clip,
                    note,
                })));
        if self.after_edit(result) {
            self.selected_clip = Some(clip);
            self.select_note(id, false);
        }
    }

    pub fn replace_note(&mut self, clip: ClipId, note: Note) {
        let result =
            self.session
                .apply_command(Command::new(CommandKind::Note(NoteCommand::Replace {
                    clip,
                    note,
                })));
        self.after_edit(result);
    }

    pub fn remove_note(&mut self, clip: ClipId, note: NoteId) {
        let result =
            self.session
                .apply_command(Command::new(CommandKind::Note(NoteCommand::Remove {
                    clip,
                    note,
                })));
        self.after_edit(result);
    }

    pub fn delete_selected_notes(&mut self) {
        let selected: Vec<NoteId> = self.session.selection().notes.iter().copied().collect();
        if selected.is_empty() {
            return;
        }
        let project = self.session.project();
        let mut commands = Vec::new();
        for id in selected {
            for (clip_id, clip) in &project.clips {
                if let ClipContent::Midi { notes, .. } = &clip.content {
                    if notes.iter().any(|note| note.id == id) {
                        commands.push(Command::new(CommandKind::Note(NoteCommand::Remove {
                            clip: *clip_id,
                            note: id,
                        })));
                        break;
                    }
                }
            }
        }
        if commands.is_empty() {
            return;
        }
        let result = self.session.apply_commands(commands, "Delete notes");
        self.after_edit(result);
    }

    pub fn add_track(&mut self) {
        match add_track_commands(&mut self.session) {
            Ok((commands, track, clip)) => {
                let result = self.session.apply_commands(commands, "Add track");
                if self.after_edit(result) {
                    self.select_track(track);
                    self.selected_clip = Some(clip);
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn add_midi_clip(&mut self) {
        let Some(track) = self.selected_track() else {
            return;
        };
        match add_clip_commands(&mut self.session, track) {
            Ok((commands, clip)) => {
                let result = self.session.apply_commands(commands, "Add MIDI clip");
                if self.after_edit(result) {
                    self.selected_clip = Some(clip);
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn after_edit(&mut self, result: Result<Vec<Event>, SessionError>) -> bool {
        match result {
            Ok(_) => {
                self.error = None;
                self.ensure_selection();
                if self.audio.is_none() {
                    self.start_audio();
                } else {
                    self.rebuild_audio();
                }
                true
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
    }

    fn rebuild_audio(&mut self) {
        let Some(audio) = &mut self.audio else {
            return;
        };
        let was_playing = self.playing;
        let project = self.session.project().clone();
        match prepare_state(&project) {
            Ok((state, end_frame)) => {
                if let Err(error) = audio.handle.publish(state) {
                    self.audio_error = Some(error.to_string());
                    self.playing = false;
                    return;
                }
                audio.end_frame = end_frame;
                self.audio_error = None;
                if was_playing {
                    match audio.handle.play() {
                        Ok(()) => self.playing = true,
                        Err(error) => {
                            self.audio_error = Some(error.to_string());
                            self.playing = false;
                        }
                    }
                }
            }
            Err(error) => {
                self.audio_error = Some(error);
                self.playing = false;
            }
        }
    }
}

fn prepare_state(project: &Project) -> Result<(RealtimeState, u64), String> {
    let compiled = Compiler::builtin()
        .compile(project)
        .map_err(|error| error.to_string())?;
    let sample_rate = compiled.plan.sample_rate;
    let end_frame = project_end_frame(project)
        .map_err(|error| error.to_string())?
        .unwrap_or(u64::from(sample_rate) * 5);
    let state = RealtimeState::new(
        compiled,
        project,
        NonZeroUsize::new(8192).expect("realtime block size is non-zero"),
        Some(end_frame),
    )
    .map_err(|error| error.to_string())?;
    Ok((state, end_frame))
}

pub fn add_track_commands(session: &mut Session) -> Result<(Vec<Command>, NodeId, ClipId), String> {
    let project = session.project().clone();
    let mut commands = Vec::new();

    let master = project
        .nodes
        .iter()
        .find(|(_, node)| node.role == NodeRole::Master)
        .map(|(id, node)| (*id, node));
    let master_input = match master {
        Some((id, node)) => {
            let port = node
                .ports
                .iter()
                .find(|port| port.direction == PortDirection::Input)
                .ok_or_else(|| "the master node has no input port".to_owned())?;
            (id, port.id)
        }
        None => {
            let id = session.next_node_id();
            commands.push(Command::new(CommandKind::Node(NodeCommand::Add {
                node: id,
                value: Node::new(NodeRole::Master).with_ports([
                    Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                    Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                ]),
            })));
            (id, PortId::from_raw(0))
        }
    };

    let track = session.next_node_id();
    let instrument = session.next_node_id();
    let gain = session.next_node_id();
    let clip = session.next_clip_id();
    let edge = session.next_edge_id();

    commands.push(Command::new(CommandKind::Node(NodeCommand::Add {
        node: track,
        value: Node::new(NodeRole::Track)
            .with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ])
            .with_rack([instrument, gain])
            .with_clips([clip]),
    })));
    commands.push(Command::new(CommandKind::Node(NodeCommand::Add {
        node: instrument,
        value: Node::device(NodeKind::new(odaw_dsp::POLY_KIND))
            .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
            .with_param(ParamId::new("gain"), ParamValue::Float(0.25)),
    })));
    commands.push(Command::new(CommandKind::Node(NodeCommand::Add {
        node: gain,
        value: Node::device(NodeKind::new(odaw_dsp::GAIN_KIND))
            .with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ])
            .with_param(ParamId::new("gain"), ParamValue::Float(0.8)),
    })));
    commands.push(Command::new(CommandKind::Clip(ClipCommand::Add {
        clip,
        value: Clip {
            node: track,
            start: Tick::ZERO,
            length: Tick::from_raw(TRACK_LENGTH),
            content: ClipContent::Midi {
                source_start: Tick::ZERO,
                source_length: Tick::from_raw(TRACK_LENGTH),
                notes: [].into_iter().collect(),
            },
        },
    })));
    commands.push(Command::new(CommandKind::Edge(EdgeCommand::Add {
        edge,
        value: Edge {
            from: NodePort {
                node: track,
                port: PortId::from_raw(1),
            },
            to: NodePort {
                node: master_input.0,
                port: master_input.1,
            },
        },
    })));

    Ok((commands, track, clip))
}

pub fn add_clip_commands(
    session: &mut Session,
    track: NodeId,
) -> Result<(Vec<Command>, ClipId), String> {
    let clip = {
        let project = session.project();
        let node = project
            .nodes
            .get(&track)
            .ok_or_else(|| "the selected track no longer exists".to_owned())?;
        if node.role != NodeRole::Track {
            return Err("the selected node is not a track".to_owned());
        }
        session.next_clip_id()
    };
    Ok((
        vec![Command::new(CommandKind::Clip(ClipCommand::Add {
            clip,
            value: Clip {
                node: track,
                start: Tick::ZERO,
                length: Tick::from_raw(TRACK_LENGTH),
                content: ClipContent::Midi {
                    source_start: Tick::ZERO,
                    source_length: Tick::from_raw(TRACK_LENGTH),
                    notes: [].into_iter().collect(),
                },
            },
        }))],
        clip,
    ))
}

fn is_midi(clip: &Clip) -> bool {
    matches!(&clip.content, ClipContent::Midi { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_a_track_builds_a_valid_transaction() {
        let mut session = Session::new(demo::project()).unwrap();
        let before = session.project().nodes.len();
        let (commands, track, clip) = add_track_commands(&mut session).unwrap();
        session.apply_commands(commands, "Add track").unwrap();

        assert_eq!(session.project().nodes.len(), before + 3);
        assert_eq!(
            session.project().nodes.get(&track).map(|node| node.role),
            Some(NodeRole::Track)
        );
        assert!(session.project().clips.contains_key(&clip));
        assert_eq!(session.project().validate(), Ok(()));
    }

    #[test]
    fn adding_a_clip_targets_the_track() {
        let mut session = Session::new(demo::project()).unwrap();
        let track = NodeId::from_raw(1);
        let (commands, clip) = add_clip_commands(&mut session, track).unwrap();
        session.apply_commands(commands, "Add clip").unwrap();
        assert_eq!(
            session.project().clips.get(&clip).map(|clip| clip.node),
            Some(track)
        );
        assert_eq!(session.project().validate(), Ok(()));
    }

    #[test]
    fn adding_a_track_creates_a_master_when_needed() {
        let mut session = Session::new(Project::empty("empty", 48_000, 0).unwrap()).unwrap();
        let (commands, _, _) = add_track_commands(&mut session).unwrap();
        session.apply_commands(commands, "Add track").unwrap();
        assert!(
            session
                .project()
                .nodes
                .values()
                .any(|node| node.role == NodeRole::Master)
        );
        assert_eq!(session.project().validate(), Ok(()));
    }
}
