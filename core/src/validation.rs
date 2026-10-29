use thiserror::Error;

use crate::automation::AutomationError;
use crate::clip::{ClipContent, Note};
use crate::collections::PersistentVector;
use crate::graph::{NodeRole, PortDirection};
use crate::ids::{AssetId, ClipId, EdgeId, NodeId, NoteId, PortId, SectionId};
use crate::params::ParamId;
use crate::project::Project;
use crate::time::Tick;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeSide {
    Source,
    Target,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("tempo map sample rate {tempo} does not match project sample rate {project}")]
    SampleRateMismatch { project: u32, tempo: u32 },
    #[error("device node {0} has no node kind")]
    MissingDeviceKind(NodeId),
    #[error("non-device node {0} declares a device kind")]
    UnexpectedDeviceKind(NodeId),
    #[error("node {0} has an empty name")]
    EmptyNodeName(NodeId),
    #[error("node {node} has color {color:#08x}, outside 24-bit RGB")]
    InvalidNodeColor { node: NodeId, color: u32 },
    #[error("node {node} holds a rack entry {entry} that is not a device node")]
    RackEntryNotDevice { node: NodeId, entry: NodeId },
    #[error("node {node} references missing rack entry {entry}")]
    MissingRackEntry { node: NodeId, entry: NodeId },
    #[error("node {node} holds a rack but its role cannot own one")]
    RackOnWrongRole { node: NodeId },
    #[error("edge {edge} references missing {side:?} node {node}")]
    MissingEdgeNode {
        edge: EdgeId,
        side: EdgeSide,
        node: NodeId,
    },
    #[error("edge {edge} references missing port {port} on node {node}")]
    MissingPort {
        edge: EdgeId,
        node: NodeId,
        port: PortId,
    },
    #[error("edge {0} must run from an output port to an input port")]
    EdgeDirection(EdgeId),
    #[error("edge {0} joins ports with different channel layouts")]
    EdgeLayoutMismatch(EdgeId),
    #[error("loop region {0} ends before it starts")]
    InvalidLoop(usize),
    #[error("section {section} references missing clip {clip}")]
    MissingSectionClip { section: SectionId, clip: ClipId },
    #[error("node {node} references missing clip {clip}")]
    MissingNodeClip { node: NodeId, clip: ClipId },
    #[error("clip {clip} references missing node {node}")]
    MissingClipNode { clip: ClipId, node: NodeId },
    #[error("clip {clip} is not listed on its node {node}")]
    UnlistedClip { clip: ClipId, node: NodeId },
    #[error("clip {clip} belongs to non-track node {node}")]
    ClipNodeNotTrack { clip: ClipId, node: NodeId },
    #[error("clip {clip} references missing asset {asset}")]
    MissingAsset { clip: ClipId, asset: AssetId },
    #[error("clip {0} has a negative length")]
    NegativeClipLength(ClipId),
    #[error("clip {clip} note {note} is invalid: {reason}")]
    InvalidNote {
        clip: ClipId,
        note: NoteId,
        reason: NoteError,
    },
    #[error("clip {clip} repeats note id {note}")]
    DuplicateNote { clip: ClipId, note: NoteId },
    #[error("arrangement references missing section {section}")]
    MissingArrangementSection { section: SectionId },
    #[error("modulator link on node {node} references missing modulator {modulator}")]
    MissingModulator { node: NodeId, modulator: NodeId },
    #[error("modulator link on node {node} references non-modulator {modulator}")]
    ModulatorRole { node: NodeId, modulator: NodeId },
    #[error("automation lane {param} on node {node} is invalid: {reason}")]
    InvalidAutomation {
        node: NodeId,
        param: ParamId,
        reason: AutomationError,
    },
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum NoteError {
    #[error("pitch {0} is outside 0..=127")]
    Pitch(u8),
    #[error("velocity {0} is outside 0..=127")]
    Velocity(u8),
    #[error("channel {0} is outside 0..=15")]
    Channel(u8),
    #[error("start tick is negative")]
    Start,
    #[error("length must be greater than zero")]
    Length,
}

fn validate_notes(
    clip: ClipId,
    notes: &PersistentVector<Note>,
    seen: &mut std::collections::BTreeSet<NoteId>,
) -> Result<(), ValidationError> {
    for note in notes {
        if !seen.insert(note.id) {
            return Err(ValidationError::DuplicateNote {
                clip,
                note: note.id,
            });
        }
        let reason = if note.pitch > 127 {
            Some(NoteError::Pitch(note.pitch))
        } else if note.velocity > 127 {
            Some(NoteError::Velocity(note.velocity))
        } else if note.channel > 15 {
            Some(NoteError::Channel(note.channel))
        } else if note.start < Tick::ZERO {
            Some(NoteError::Start)
        } else if note.length <= Tick::ZERO {
            Some(NoteError::Length)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(ValidationError::InvalidNote {
                clip,
                note: note.id,
                reason,
            });
        }
    }
    Ok(())
}

impl Project {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.meta.sample_rate != self.tempo.sample_rate() {
            return Err(ValidationError::SampleRateMismatch {
                project: self.meta.sample_rate,
                tempo: self.tempo.sample_rate(),
            });
        }

        for (node_id, node) in &self.nodes {
            if node.role == NodeRole::Device && node.kind.is_none() {
                return Err(ValidationError::MissingDeviceKind(*node_id));
            }
            if node.role != NodeRole::Device && node.kind.is_some() {
                return Err(ValidationError::UnexpectedDeviceKind(*node_id));
            }
            if node
                .name
                .as_deref()
                .is_some_and(|name| name.trim().is_empty())
            {
                return Err(ValidationError::EmptyNodeName(*node_id));
            }
            if let Some(color) = node.color {
                if color > 0x00ff_ffff {
                    return Err(ValidationError::InvalidNodeColor {
                        node: *node_id,
                        color,
                    });
                }
            }
            if !node.role.holds_rack() && !node.rack.is_empty() {
                return Err(ValidationError::RackOnWrongRole { node: *node_id });
            }
            for entry in &node.rack {
                let Some(entry_node) = self.nodes.get(entry) else {
                    return Err(ValidationError::MissingRackEntry {
                        node: *node_id,
                        entry: *entry,
                    });
                };
                if !entry_node.role.is_device() {
                    return Err(ValidationError::RackEntryNotDevice {
                        node: *node_id,
                        entry: *entry,
                    });
                }
            }
            self.validate_node_clips(*node_id, node)?;
            self.validate_modulations(*node_id, node)?;
            self.validate_automation(*node_id, node)?;
        }

        for (edge_id, edge) in &self.edges {
            self.validate_edge(*edge_id, edge)?;
        }

        for (section_id, section) in &self.sections {
            for clip in &section.clips {
                if !self.clips.contains_key(clip) {
                    return Err(ValidationError::MissingSectionClip {
                        section: *section_id,
                        clip: *clip,
                    });
                }
            }
        }

        for section_ref in &self.arrangement {
            if !self.sections.contains_key(&section_ref.section) {
                return Err(ValidationError::MissingArrangementSection {
                    section: section_ref.section,
                });
            }
        }

        for (index, loop_region) in self.loops.iter().enumerate() {
            if loop_region.end < loop_region.start {
                return Err(ValidationError::InvalidLoop(index));
            }
        }

        let mut note_ids = std::collections::BTreeSet::new();
        for (clip_id, clip) in &self.clips {
            let Some(node) = self.nodes.get(&clip.node) else {
                return Err(ValidationError::MissingClipNode {
                    clip: *clip_id,
                    node: clip.node,
                });
            };
            if node.role != NodeRole::Track {
                return Err(ValidationError::ClipNodeNotTrack {
                    clip: *clip_id,
                    node: clip.node,
                });
            }
            if clip.length < crate::Tick::ZERO {
                return Err(ValidationError::NegativeClipLength(*clip_id));
            }
            match &clip.content {
                ClipContent::Audio { asset, .. } => {
                    if !self.assets.contains_key(asset) {
                        return Err(ValidationError::MissingAsset {
                            clip: *clip_id,
                            asset: *asset,
                        });
                    }
                }
                ClipContent::Midi { notes, .. } => validate_notes(*clip_id, notes, &mut note_ids)?,
                ClipContent::Empty => {}
            }
            if !node.clips.contains(clip_id) {
                return Err(ValidationError::UnlistedClip {
                    clip: *clip_id,
                    node: clip.node,
                });
            }
        }

        Ok(())
    }

    fn validate_node_clips(
        &self,
        node_id: NodeId,
        node: &crate::graph::Node,
    ) -> Result<(), ValidationError> {
        for clip in &node.clips {
            if !self.clips.contains_key(clip) {
                return Err(ValidationError::MissingNodeClip {
                    node: node_id,
                    clip: *clip,
                });
            }
        }
        Ok(())
    }

    fn validate_automation(
        &self,
        node_id: NodeId,
        node: &crate::graph::Node,
    ) -> Result<(), ValidationError> {
        for (param, lane) in &node.automation {
            lane.validate()
                .map_err(|reason| ValidationError::InvalidAutomation {
                    node: node_id,
                    param: param.clone(),
                    reason,
                })?;
        }
        Ok(())
    }

    fn validate_modulations(
        &self,
        node_id: NodeId,
        node: &crate::graph::Node,
    ) -> Result<(), ValidationError> {
        for (_, links) in node.modulations.iter() {
            for link in links {
                let Some(source) = self.nodes.get(&link.source) else {
                    return Err(ValidationError::MissingModulator {
                        node: node_id,
                        modulator: link.source,
                    });
                };
                if source.role != NodeRole::Modulator {
                    return Err(ValidationError::ModulatorRole {
                        node: node_id,
                        modulator: link.source,
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_edge(
        &self,
        edge_id: EdgeId,
        edge: &crate::graph::Edge,
    ) -> Result<(), ValidationError> {
        let from_node =
            self.nodes
                .get(&edge.from.node)
                .ok_or(ValidationError::MissingEdgeNode {
                    edge: edge_id,
                    side: EdgeSide::Source,
                    node: edge.from.node,
                })?;
        let from_port = from_node
            .port(edge.from.port)
            .ok_or(ValidationError::MissingPort {
                edge: edge_id,
                node: edge.from.node,
                port: edge.from.port,
            })?;

        let to_node = self
            .nodes
            .get(&edge.to.node)
            .ok_or(ValidationError::MissingEdgeNode {
                edge: edge_id,
                side: EdgeSide::Target,
                node: edge.to.node,
            })?;
        let to_port = to_node
            .port(edge.to.port)
            .ok_or(ValidationError::MissingPort {
                edge: edge_id,
                node: edge.to.node,
                port: edge.to.port,
            })?;

        if from_port.direction != PortDirection::Output || to_port.direction != PortDirection::Input
        {
            return Err(ValidationError::EdgeDirection(edge_id));
        }
        if from_port.layout != to_port.layout {
            return Err(ValidationError::EdgeLayoutMismatch(edge_id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{ChannelLayout, Edge, Node, NodePort, Port};
    use crate::time::{Frames, Tick};
    use crate::{Clip, ClipContent, NodeId, PortId, Project, Section, SectionRef};

    fn track_and_master() -> Project {
        let mut project = Project::empty("test", 48_000, 1).unwrap();
        project.nodes.insert(
            NodeId::from_raw(1),
            Node::new(NodeRole::Track)
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)]),
        );
        project.nodes.insert(
            NodeId::from_raw(2),
            Node::new(NodeRole::Master)
                .with_ports([Port::input(PortId::from_raw(0), ChannelLayout::Stereo)]),
        );
        project
    }

    fn edge(from: NodePort, to: NodePort) -> Edge {
        Edge { from, to }
    }

    fn project_with_midi_notes(notes: Vec<Note>) -> Project {
        let mut project = track_and_master();
        let clip_id = ClipId::from_raw(1);
        let track_id = NodeId::from_raw(1);
        project.clips.insert(
            clip_id,
            Clip {
                node: track_id,
                start: Tick::ZERO,
                length: Tick::from_raw(crate::PPQ),
                content: ClipContent::Midi {
                    source_start: Tick::ZERO,
                    source_length: Tick::from_raw(crate::PPQ),
                    notes: notes.into_iter().collect(),
                },
            },
        );
        project
            .nodes
            .get_mut(&track_id)
            .unwrap()
            .clips
            .push_back(clip_id);
        project
    }

    #[test]
    fn valid_graph_passes() {
        let mut project = track_and_master();
        project.edges.insert(
            EdgeId::from_raw(1),
            edge(
                NodePort {
                    node: NodeId::from_raw(1),
                    port: PortId::from_raw(0),
                },
                NodePort {
                    node: NodeId::from_raw(2),
                    port: PortId::from_raw(0),
                },
            ),
        );
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn missing_edge_node_is_rejected() {
        let mut project = track_and_master();
        project.edges.insert(
            EdgeId::from_raw(1),
            edge(
                NodePort {
                    node: NodeId::from_raw(1),
                    port: PortId::from_raw(0),
                },
                NodePort {
                    node: NodeId::from_raw(99),
                    port: PortId::from_raw(0),
                },
            ),
        );
        assert_eq!(
            project.validate(),
            Err(ValidationError::MissingEdgeNode {
                edge: EdgeId::from_raw(1),
                side: EdgeSide::Target,
                node: NodeId::from_raw(99),
            })
        );
    }

    #[test]
    fn edge_must_run_from_output_to_input() {
        let mut project = track_and_master();
        project.edges.insert(
            EdgeId::from_raw(1),
            edge(
                NodePort {
                    node: NodeId::from_raw(2),
                    port: PortId::from_raw(0),
                },
                NodePort {
                    node: NodeId::from_raw(1),
                    port: PortId::from_raw(0),
                },
            ),
        );
        assert_eq!(
            project.validate(),
            Err(ValidationError::EdgeDirection(EdgeId::from_raw(1)))
        );
    }

    #[test]
    fn edge_requires_matching_channel_layouts() {
        let mut project = track_and_master();
        project.nodes.insert(
            NodeId::from_raw(3),
            Node::new(NodeRole::Master)
                .with_ports([Port::input(PortId::from_raw(0), ChannelLayout::Mono)]),
        );
        project.edges.insert(
            EdgeId::from_raw(1),
            edge(
                NodePort {
                    node: NodeId::from_raw(1),
                    port: PortId::from_raw(0),
                },
                NodePort {
                    node: NodeId::from_raw(3),
                    port: PortId::from_raw(0),
                },
            ),
        );
        assert_eq!(
            project.validate(),
            Err(ValidationError::EdgeLayoutMismatch(EdgeId::from_raw(1)))
        );
    }

    #[test]
    fn rack_entries_must_be_devices() {
        let mut project = Project::empty("test", 48_000, 1).unwrap();
        project.nodes.insert(
            NodeId::from_raw(1),
            Node::new(NodeRole::Track).with_rack([NodeId::from_raw(2)]),
        );
        project
            .nodes
            .insert(NodeId::from_raw(2), Node::new(NodeRole::Master));
        assert_eq!(
            project.validate(),
            Err(ValidationError::RackEntryNotDevice {
                node: NodeId::from_raw(1),
                entry: NodeId::from_raw(2),
            })
        );
    }

    #[test]
    fn audio_clips_require_their_asset() {
        let mut project = track_and_master();
        let clip_id = ClipId::from_raw(1);
        let track_id = NodeId::from_raw(1);
        let asset_id = AssetId::from_bytes([7; 32]);
        project.clips.insert(
            clip_id,
            Clip {
                node: track_id,
                start: Tick::ZERO,
                length: Tick::from_raw(crate::PPQ),
                content: ClipContent::Audio {
                    asset: asset_id,
                    source_start: Frames::ZERO,
                    source_length: Frames::from_raw(48_000),
                },
            },
        );
        project
            .nodes
            .get_mut(&track_id)
            .unwrap()
            .clips
            .push_back(clip_id);

        assert_eq!(
            project.validate(),
            Err(ValidationError::MissingAsset {
                clip: clip_id,
                asset: asset_id,
            })
        );
    }

    #[test]
    fn midi_notes_are_validated() {
        let note = Note {
            id: NoteId::from_raw(1),
            pitch: 60,
            start: Tick::ZERO,
            length: Tick::from_raw(crate::PPQ),
            velocity: 100,
            channel: 0,
        };
        let project = project_with_midi_notes(vec![note]);
        assert_eq!(project.validate(), Ok(()));

        let invalid_pitch = project_with_midi_notes(vec![Note { pitch: 200, ..note }]);
        assert_eq!(
            invalid_pitch.validate(),
            Err(ValidationError::InvalidNote {
                clip: ClipId::from_raw(1),
                note: NoteId::from_raw(1),
                reason: NoteError::Pitch(200),
            })
        );

        let duplicate = project_with_midi_notes(vec![note, note]);
        assert_eq!(
            duplicate.validate(),
            Err(ValidationError::DuplicateNote {
                clip: ClipId::from_raw(1),
                note: NoteId::from_raw(1),
            })
        );

        let invalid_length = project_with_midi_notes(vec![Note {
            length: Tick::ZERO,
            ..note
        }]);
        assert_eq!(
            invalid_length.validate(),
            Err(ValidationError::InvalidNote {
                clip: ClipId::from_raw(1),
                note: NoteId::from_raw(1),
                reason: NoteError::Length,
            })
        );
    }

    #[test]
    fn note_ids_are_unique_across_clips() {
        let note = Note {
            id: NoteId::from_raw(1),
            pitch: 60,
            start: Tick::ZERO,
            length: Tick::from_raw(crate::PPQ),
            velocity: 100,
            channel: 0,
        };
        let mut project = project_with_midi_notes(vec![note]);
        let second_clip = ClipId::from_raw(2);
        project.clips.insert(
            second_clip,
            Clip {
                node: NodeId::from_raw(1),
                start: Tick::ZERO,
                length: Tick::from_raw(crate::PPQ),
                content: ClipContent::Midi {
                    source_start: Tick::ZERO,
                    source_length: Tick::from_raw(crate::PPQ),
                    notes: [note].into_iter().collect(),
                },
            },
        );
        project
            .nodes
            .get_mut(&NodeId::from_raw(1))
            .unwrap()
            .clips
            .push_back(second_clip);

        assert_eq!(
            project.validate(),
            Err(ValidationError::DuplicateNote {
                clip: second_clip,
                note: NoteId::from_raw(1),
            })
        );
    }

    #[test]
    fn node_name_and_color_are_validated() {
        let mut project = track_and_master();
        let track = NodeId::from_raw(1);
        project.nodes.get_mut(&track).unwrap().name = Some("  ".into());
        assert_eq!(
            project.validate(),
            Err(ValidationError::EmptyNodeName(track))
        );
        project.nodes.get_mut(&track).unwrap().name = Some("Lead".into());
        project.nodes.get_mut(&track).unwrap().color = Some(0x0100_0000);
        assert_eq!(
            project.validate(),
            Err(ValidationError::InvalidNodeColor {
                node: track,
                color: 0x0100_0000,
            })
        );
        project.nodes.get_mut(&track).unwrap().color = Some(0x00ff_ffff);
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn arrangement_sections_must_exist() {
        let mut project = Project::empty("test", 48_000, 1).unwrap();
        project.arrangement.push_back(SectionRef {
            section: SectionId::from_raw(4),
            repeat: 1,
            active: true,
        });
        assert_eq!(
            project.validate(),
            Err(ValidationError::MissingArrangementSection {
                section: SectionId::from_raw(4),
            })
        );
    }

    #[test]
    fn section_clips_must_exist() {
        let mut project = Project::empty("test", 48_000, 1).unwrap();
        project.sections.insert(
            SectionId::from_raw(1),
            Section {
                name: "intro".into(),
                length: Tick::from_raw(crate::PPQ),
                clips: [ClipId::from_raw(2)].into_iter().collect(),
            },
        );
        assert_eq!(
            project.validate(),
            Err(ValidationError::MissingSectionClip {
                section: SectionId::from_raw(1),
                clip: ClipId::from_raw(2),
            })
        );
    }

    #[test]
    fn loop_regions_must_not_run_backwards() {
        let mut project = Project::empty("test", 48_000, 1).unwrap();
        project.loops.push_back(crate::LoopRegion {
            start: Tick::from_raw(crate::PPQ),
            end: Tick::ZERO,
        });
        assert_eq!(project.validate(), Err(ValidationError::InvalidLoop(0)));
    }
}
