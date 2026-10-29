use odaw_core::{
    Clip, ClipContent, ClipId, Edge, EdgeId, Node, NodeId, NodeRole, NoteId, ParamAddress, ParamId,
    PersistentVector, PortDirection, PortId, SectionId, SectionRef, TempoMap,
};
use thiserror::Error;

use crate::command::{
    ArrangementCommand, AssetCommand, AutomationCommand, COMMAND_VERSION, ClipCommand, Command,
    CommandKind, DeviceCommand, EdgeCommand, LoopCommand, MarkerCommand, MetaCommand,
    ModulationCommand, NodeCommand, NoteCommand, ParamChange, ParamCommand, SectionCommand,
    TempoCommand,
};
use crate::tx::Tx;

pub(crate) fn dispatch(command: &Command, tx: &mut Tx) -> Result<(), DispatchError> {
    if command.version != COMMAND_VERSION {
        return Err(DispatchError::UnsupportedVersion {
            found: command.version,
            supported: COMMAND_VERSION,
        });
    }

    match &command.kind {
        CommandKind::Meta(command) => dispatch_meta(command, tx),
        CommandKind::Asset(command) => dispatch_asset(command, tx),
        CommandKind::Node(command) => dispatch_node(command, tx),
        CommandKind::Device(command) => dispatch_device(command, tx),
        CommandKind::Edge(command) => dispatch_edge(command, tx),
        CommandKind::Clip(command) => dispatch_clip(command, tx),
        CommandKind::Note(command) => dispatch_note(command, tx),
        CommandKind::Section(command) => dispatch_section(command, tx),
        CommandKind::Arrangement(command) => dispatch_arrangement(command, tx),
        CommandKind::Loop(command) => dispatch_loop(command, tx),
        CommandKind::Param(command) => dispatch_param(command, tx),
        CommandKind::Automation(command) => dispatch_automation(command, tx),
        CommandKind::Modulation(command) => dispatch_modulation(command, tx),
        CommandKind::Tempo(command) => dispatch_tempo(command, tx),
        CommandKind::Marker(command) => dispatch_marker(command, tx),
    }
}

fn dispatch_meta(command: &MetaCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        MetaCommand::SetName(name) => tx.set_meta_name(name.clone()),
        MetaCommand::SetSeed(seed) => tx.set_meta_seed(*seed),
    }
    Ok(())
}

fn dispatch_asset(command: &AssetCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        AssetCommand::Set { asset, value } => match value {
            Some(value) => tx.insert_asset(*asset, value.clone()),
            None => {
                tx.remove_asset(*asset);
            }
        },
    }
    Ok(())
}

fn dispatch_node(command: &NodeCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        NodeCommand::Add { node, value } => {
            if tx.node(*node).is_some() {
                return Err(DispatchError::NodeExists(*node));
            }
            tx.insert_node(*node, value.as_ref().clone());
        }
        NodeCommand::Remove { node } => remove_node_cascade(tx, *node)?,
        NodeCommand::Replace { node, value } => {
            if tx.node(*node).is_none() {
                return Err(DispatchError::MissingNode(*node));
            }
            tx.insert_node(*node, value.as_ref().clone());
            tx.mark_layout_or_latency();
        }
        NodeCommand::SetRack { node, rack } => {
            let Some(mut value) = tx.node(*node).cloned() else {
                return Err(DispatchError::MissingNode(*node));
            };
            if !value.role.holds_rack() {
                return Err(DispatchError::NodeCannotHoldRack(*node));
            }
            value.rack = rack.iter().copied().collect();
            tx.insert_node(*node, value);
            tx.mark_layout_or_latency();
        }
        NodeCommand::SetTrackOrder { tracks } => {
            let mut seen = std::collections::BTreeSet::new();
            for track in tracks {
                let Some(node) = tx.node(*track) else {
                    return Err(DispatchError::MissingNode(*track));
                };
                if node.role != NodeRole::Track {
                    return Err(DispatchError::NotATrack(*track));
                }
                if !seen.insert(*track) {
                    return Err(DispatchError::DuplicateTrackOrder(*track));
                }
            }
            tx.set_track_order(tracks.clone());
        }
        NodeCommand::SetPorts { node, ports } => {
            let Some(mut value) = tx.node(*node).cloned() else {
                return Err(DispatchError::MissingNode(*node));
            };
            value.ports = ports.iter().copied().collect();
            tx.insert_node(*node, value);
            tx.mark_layout_or_latency();
        }
        NodeCommand::SetName { node, name } => {
            update_node(tx, *node, |value| value.name = name.clone())?;
        }
        NodeCommand::SetColor { node, color } => {
            update_node(tx, *node, |value| value.color = *color)?;
        }
        NodeCommand::SetMuted { node, muted } => {
            update_node(tx, *node, |value| value.muted = *muted)?;
        }
        NodeCommand::SetSoloed { node, soloed } => {
            update_node(tx, *node, |value| value.soloed = *soloed)?;
        }
    }
    Ok(())
}

fn update_node(
    tx: &mut Tx,
    node: NodeId,
    update: impl FnOnce(&mut Node),
) -> Result<(), DispatchError> {
    let Some(mut value) = tx.node(node).cloned() else {
        return Err(DispatchError::MissingNode(node));
    };
    update(&mut value);
    tx.insert_node(node, value);
    Ok(())
}

fn dispatch_device(command: &DeviceCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        DeviceCommand::SetKind { node, kind } => {
            let Some(mut value) = tx.node(*node).cloned() else {
                return Err(DispatchError::MissingNode(*node));
            };
            if value.role != NodeRole::Device {
                return Err(DispatchError::NotADevice(*node));
            }
            value.kind = Some(kind.clone());
            tx.insert_node(*node, value);
            tx.mark_layout_or_latency();
        }
        DeviceCommand::SetState { node, state } => {
            let Some(mut value) = tx.node(*node).cloned() else {
                return Err(DispatchError::MissingNode(*node));
            };
            if value.role != NodeRole::Device {
                return Err(DispatchError::NotADevice(*node));
            }
            value.state = state.clone();
            tx.insert_node(*node, value);
            tx.mark_layout_or_latency();
        }
    }
    Ok(())
}

fn dispatch_edge(command: &EdgeCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        EdgeCommand::Add { edge, value } => {
            if tx.edge(*edge).is_some() {
                return Err(DispatchError::EdgeExists(*edge));
            }
            validate_edge_endpoints(*edge, value, tx)?;
            tx.insert_edge(*edge, value.clone());
        }
        EdgeCommand::Remove { edge } => {
            if tx.remove_edge(*edge).is_none() {
                return Err(DispatchError::MissingEdge(*edge));
            }
        }
    }
    Ok(())
}

fn dispatch_clip(command: &ClipCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        ClipCommand::Add { clip, value } => {
            if tx.clip(*clip).is_some() {
                return Err(DispatchError::ClipExists(*clip));
            }
            if tx.node(value.node).is_none() {
                return Err(DispatchError::MissingNode(value.node));
            }
            tx.insert_clip(*clip, value.clone());
            attach_clip_to_node(tx, value.node, *clip);
        }
        ClipCommand::Remove { clip } => {
            let value = remove_clip_record(tx, *clip)?;
            detach_clip_from_node(tx, value.node, *clip);
        }
        ClipCommand::Replace { clip, value } => {
            let Some(previous) = tx.clip(*clip).cloned() else {
                return Err(DispatchError::MissingClip(*clip));
            };
            if tx.node(value.node).is_none() {
                return Err(DispatchError::MissingNode(value.node));
            }
            if previous.node != value.node {
                detach_clip_from_node(tx, previous.node, *clip);
            }
            tx.insert_clip(*clip, value.clone());
            attach_clip_to_node(tx, value.node, *clip);
        }
    }
    Ok(())
}

fn dispatch_note(command: &NoteCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        NoteCommand::Add { clip, note } => {
            let Some(mut value) = tx.clip(*clip).cloned() else {
                return Err(DispatchError::MissingClip(*clip));
            };
            let ClipContent::Midi {
                source_start,
                source_length,
                mut notes,
            } = value.content
            else {
                return Err(DispatchError::NotAMidiClip(*clip));
            };
            if notes.iter().any(|existing| existing.id == note.id) {
                return Err(DispatchError::NoteExists {
                    clip: *clip,
                    note: note.id,
                });
            }
            notes.push_back(*note);
            value.content = ClipContent::Midi {
                source_start,
                source_length,
                notes,
            };
            tx.insert_clip(*clip, value);
        }
        NoteCommand::Remove { clip, note } => {
            let Some(mut value) = tx.clip(*clip).cloned() else {
                return Err(DispatchError::MissingClip(*clip));
            };
            let ClipContent::Midi {
                source_start,
                source_length,
                notes,
            } = value.content
            else {
                return Err(DispatchError::NotAMidiClip(*clip));
            };
            if !notes.iter().any(|existing| existing.id == *note) {
                return Err(DispatchError::MissingNote {
                    clip: *clip,
                    note: *note,
                });
            }
            let notes = notes
                .iter()
                .filter(|existing| existing.id != *note)
                .copied()
                .collect();
            value.content = ClipContent::Midi {
                source_start,
                source_length,
                notes,
            };
            tx.insert_clip(*clip, value);
        }
        NoteCommand::Replace { clip, note } => {
            let Some(mut value) = tx.clip(*clip).cloned() else {
                return Err(DispatchError::MissingClip(*clip));
            };
            let ClipContent::Midi {
                source_start,
                source_length,
                notes,
            } = value.content
            else {
                return Err(DispatchError::NotAMidiClip(*clip));
            };
            if !notes.iter().any(|existing| existing.id == note.id) {
                return Err(DispatchError::MissingNote {
                    clip: *clip,
                    note: note.id,
                });
            }
            let notes = notes
                .iter()
                .map(|existing| {
                    if existing.id == note.id {
                        *note
                    } else {
                        *existing
                    }
                })
                .collect();
            value.content = ClipContent::Midi {
                source_start,
                source_length,
                notes,
            };
            tx.insert_clip(*clip, value);
        }
    }
    Ok(())
}

fn dispatch_section(command: &SectionCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        SectionCommand::Add { section, value } => {
            if tx.section(*section).is_some() {
                return Err(DispatchError::SectionExists(*section));
            }
            tx.insert_section(*section, value.clone());
        }
        SectionCommand::Remove { section } => {
            if tx.remove_section(*section).is_none() {
                return Err(DispatchError::MissingSection(*section));
            }
            let arrangement: Vec<SectionRef> = tx
                .project()
                .arrangement
                .iter()
                .copied()
                .filter(|reference| reference.section != *section)
                .collect();
            if arrangement.len() != tx.project().arrangement.len() {
                tx.set_arrangement(arrangement);
            }
        }
        SectionCommand::Replace { section, value } => {
            if tx.section(*section).is_none() {
                return Err(DispatchError::MissingSection(*section));
            }
            tx.insert_section(*section, value.clone());
        }
    }
    Ok(())
}

fn dispatch_arrangement(command: &ArrangementCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        ArrangementCommand::Set(arrangement) => tx.set_arrangement(arrangement.clone()),
    }
    Ok(())
}

fn dispatch_loop(command: &LoopCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        LoopCommand::Set(loops) => tx.set_loops(loops.clone()),
    }
    Ok(())
}

fn dispatch_param(command: &ParamCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        ParamCommand::Set {
            address,
            value,
            change,
        } => {
            if !tx.set_param(address.clone(), *value) {
                return Err(DispatchError::MissingNode(address.node));
            }
            if *change == ParamChange::Structural {
                tx.mark_structural();
                tx.mark_layout_or_latency();
            }
        }
    }
    Ok(())
}

fn dispatch_automation(command: &AutomationCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        AutomationCommand::Set { node, param, lane } => {
            let lane = lane.as_ref().map(|lane| lane.normalized()).transpose()?;
            if let Some(lane) = &lane {
                lane.validate()?;
            }
            if !tx.set_automation(*node, param.clone(), lane) {
                return Err(DispatchError::MissingNode(*node));
            }
        }
    }
    Ok(())
}

fn dispatch_modulation(command: &ModulationCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        ModulationCommand::Set { node, param, links } => {
            if !tx.set_modulations(*node, param.clone(), links.clone()) {
                return Err(DispatchError::MissingNode(*node));
            }
        }
    }
    Ok(())
}

fn dispatch_tempo(command: &TempoCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        TempoCommand::SetTempoMap { points } => {
            let tempo = TempoMap::new(tx.project().meta.sample_rate, points.iter().copied())?;
            tx.set_tempo(tempo);
        }
        TempoCommand::SetSignatures { signatures } => tx.set_signatures(signatures.clone()),
    }
    Ok(())
}

fn dispatch_marker(command: &MarkerCommand, tx: &mut Tx) -> Result<(), DispatchError> {
    match command {
        MarkerCommand::Set { marker, value } => match value {
            Some(value) => tx.insert_marker(*marker, value.clone()),
            None => {
                tx.remove_marker(*marker);
            }
        },
    }
    Ok(())
}

fn validate_edge_endpoints(edge: EdgeId, value: &Edge, tx: &Tx) -> Result<(), DispatchError> {
    let from_node = tx
        .node(value.from.node)
        .ok_or(DispatchError::MissingEdgeNode {
            edge,
            node: value.from.node,
        })?;
    let from_port = from_node
        .port(value.from.port)
        .ok_or(DispatchError::MissingPort {
            edge,
            node: value.from.node,
            port: value.from.port,
        })?;

    let to_node = tx
        .node(value.to.node)
        .ok_or(DispatchError::MissingEdgeNode {
            edge,
            node: value.to.node,
        })?;
    let to_port = to_node
        .port(value.to.port)
        .ok_or(DispatchError::MissingPort {
            edge,
            node: value.to.node,
            port: value.to.port,
        })?;

    if from_port.direction != PortDirection::Output || to_port.direction != PortDirection::Input {
        return Err(DispatchError::EdgeDirection { edge });
    }
    if from_port.layout != to_port.layout {
        return Err(DispatchError::EdgeLayoutMismatch { edge });
    }
    Ok(())
}

fn remove_node_cascade(tx: &mut Tx, node: NodeId) -> Result<(), DispatchError> {
    if tx.node(node).is_none() {
        return Err(DispatchError::MissingNode(node));
    }

    let edges: Vec<EdgeId> = tx
        .project()
        .edges
        .iter()
        .filter(|(_, edge)| edge.from.node == node || edge.to.node == node)
        .map(|(id, _)| *id)
        .collect();
    for edge in edges {
        tx.remove_edge(edge);
    }

    let clips: Vec<ClipId> = tx
        .project()
        .clips
        .iter()
        .filter(|(_, clip)| clip.node == node)
        .map(|(id, _)| *id)
        .collect();
    for clip in clips {
        remove_clip_record(tx, clip)?;
    }

    let others: Vec<NodeId> = tx
        .project()
        .nodes
        .keys()
        .copied()
        .filter(|id| *id != node)
        .collect();
    for other in others {
        let Some(mut value) = tx.node(other).cloned() else {
            continue;
        };
        let mut changed = false;

        if value.rack.contains(&node) {
            value.rack = value
                .rack
                .iter()
                .filter(|id| **id != node)
                .copied()
                .collect();
            changed = true;
        }

        let params: Vec<ParamId> = value.modulations.keys().cloned().collect();
        for param in params {
            let Some(links) = value.modulations.get(&param) else {
                continue;
            };
            if !links.iter().any(|link| link.source == node) {
                continue;
            }
            let filtered: PersistentVector<_> = links
                .iter()
                .filter(|link| link.source != node)
                .cloned()
                .collect();
            if filtered.is_empty() {
                value.modulations.remove(&param);
            } else {
                value.modulations.insert(param.clone(), filtered);
            }
            tx.mark_param(ParamAddress { node: other, param });
            changed = true;
        }

        if changed {
            tx.insert_node(other, value);
        }
    }

    tx.remove_node(node);
    tx.mark_layout_or_latency();
    Ok(())
}

fn remove_clip_record(tx: &mut Tx, clip: ClipId) -> Result<Clip, DispatchError> {
    let value = tx
        .clip(clip)
        .cloned()
        .ok_or(DispatchError::MissingClip(clip))?;
    tx.remove_clip(clip);
    remove_clip_from_sections(tx, clip);
    Ok(value)
}

fn remove_clip_from_sections(tx: &mut Tx, clip: ClipId) {
    let sections: Vec<SectionId> = tx.project().sections.keys().copied().collect();
    for section in sections {
        let Some(mut value) = tx.section(section).cloned() else {
            continue;
        };
        if !value.clips.contains(&clip) {
            continue;
        }
        value.clips = value
            .clips
            .iter()
            .filter(|id| **id != clip)
            .copied()
            .collect();
        tx.insert_section(section, value);
    }
}

fn attach_clip_to_node(tx: &mut Tx, node: NodeId, clip: ClipId) {
    let Some(mut value) = tx.node(node).cloned() else {
        return;
    };
    if value.clips.contains(&clip) {
        return;
    }
    value.clips.push_back(clip);
    tx.insert_node(node, value);
}

fn detach_clip_from_node(tx: &mut Tx, node: NodeId, clip: ClipId) {
    let Some(mut value) = tx.node(node).cloned() else {
        return;
    };
    if !value.clips.contains(&clip) {
        return;
    }
    value.clips = value
        .clips
        .iter()
        .filter(|id| **id != clip)
        .copied()
        .collect();
    tx.insert_node(node, value);
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DispatchError {
    #[error("unsupported command version {found} (this build supports {supported})")]
    UnsupportedVersion { found: u16, supported: u16 },
    #[error("node {0} already exists")]
    NodeExists(NodeId),
    #[error("node {0} does not exist")]
    MissingNode(NodeId),
    #[error("node {0} is not a device")]
    NotADevice(NodeId),
    #[error("node {0} is not a track")]
    NotATrack(NodeId),
    #[error("track {0} appears more than once in the track order")]
    DuplicateTrackOrder(NodeId),
    #[error("node {0} cannot hold a rack")]
    NodeCannotHoldRack(NodeId),
    #[error("edge {0} already exists")]
    EdgeExists(EdgeId),
    #[error("edge {0} does not exist")]
    MissingEdge(EdgeId),
    #[error("edge {edge} references missing node {node}")]
    MissingEdgeNode { edge: EdgeId, node: NodeId },
    #[error("edge {edge} references missing port {port} on node {node}")]
    MissingPort {
        edge: EdgeId,
        node: NodeId,
        port: PortId,
    },
    #[error("edge {edge} must run from an output port to an input port")]
    EdgeDirection { edge: EdgeId },
    #[error("edge {edge} joins incompatible channel layouts")]
    EdgeLayoutMismatch { edge: EdgeId },
    #[error("clip {0} already exists")]
    ClipExists(ClipId),
    #[error("clip {0} does not exist")]
    MissingClip(ClipId),
    #[error("clip {0} does not contain MIDI notes")]
    NotAMidiClip(ClipId),
    #[error("clip {clip} already contains note {note}")]
    NoteExists { clip: ClipId, note: NoteId },
    #[error("clip {clip} does not contain note {note}")]
    MissingNote { clip: ClipId, note: NoteId },
    #[error("section {0} already exists")]
    SectionExists(SectionId),
    #[error("section {0} does not exist")]
    MissingSection(SectionId),
    #[error("invalid tempo map: {0}")]
    Tempo(#[from] odaw_core::TempoError),
    #[error("invalid automation: {0}")]
    Automation(#[from] odaw_core::AutomationError),
}

#[cfg(test)]
mod tests {
    use odaw_core::{
        AutomationLane, AutomationMode, ChannelLayout, Curve, Keyframe, Marker, MarkerId,
        ModulationLink, Node, NodeKind, NodePort, Note, ParamId, ParamValue, Port, Project,
        Section, Tick,
    };

    use super::*;

    fn project() -> Project {
        Project::empty("demo", 48_000, 0).unwrap()
    }

    fn track() -> Node {
        Node::new(NodeRole::Track)
            .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
            .with_param(ParamId::new("gain"), ParamValue::Float(1.0))
    }

    fn device(node: u64) -> Node {
        Node::device(NodeKind::new(format!("builtin.device-{node}")))
    }

    fn master() -> Node {
        Node::new(NodeRole::Master)
            .with_ports([Port::input(PortId::from_raw(0), ChannelLayout::Stereo)])
    }

    fn clip(node: NodeId) -> Clip {
        Clip {
            node,
            start: Tick::ZERO,
            length: Tick::from_raw(odaw_core::PPQ),
            content: odaw_core::ClipContent::Empty,
        }
    }

    fn midi_clip(node: NodeId) -> Clip {
        Clip {
            node,
            start: Tick::ZERO,
            length: Tick::from_raw(odaw_core::PPQ),
            content: ClipContent::Midi {
                source_start: Tick::ZERO,
                source_length: Tick::from_raw(odaw_core::PPQ),
                notes: [].into_iter().collect(),
            },
        }
    }

    fn note(id: u64, pitch: u8, start: i64) -> Note {
        Note {
            id: NoteId::from_raw(id),
            pitch,
            start: Tick::from_raw(start),
            length: Tick::from_raw(odaw_core::PPQ),
            velocity: 100,
            channel: 0,
        }
    }

    fn run(command: Command, tx: &mut Tx) -> Result<(), DispatchError> {
        dispatch(&command, tx)
    }

    #[test]
    fn add_node_then_set_param() {
        let mut tx = Tx::new(project());
        run(
            Command::new(CommandKind::Node(NodeCommand::Add {
                node: NodeId::from_raw(1),
                value: Box::new(Node::new(NodeRole::Track)),
            })),
            &mut tx,
        )
        .unwrap();
        run(
            Command::new(CommandKind::Param(ParamCommand::Set {
                address: ParamAddress {
                    node: NodeId::from_raw(1),
                    param: "gain".into(),
                },
                value: ParamValue::Float(0.25),
                change: ParamChange::Value,
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        assert!(outcome.changes.is_structural());
        assert!(outcome.changes.params.contains(&ParamAddress {
            node: NodeId::from_raw(1),
            param: "gain".into(),
        }));
        assert_eq!(
            outcome.project.nodes[&NodeId::from_raw(1)]
                .params
                .get(&ParamId::new("gain")),
            Some(&ParamValue::Float(0.25))
        );
    }

    #[test]
    fn setting_a_param_on_a_missing_node_fails() {
        let mut tx = Tx::new(project());
        let result = run(
            Command::new(CommandKind::Param(ParamCommand::Set {
                address: ParamAddress {
                    node: NodeId::from_raw(1),
                    param: "gain".into(),
                },
                value: ParamValue::Float(0.25),
                change: ParamChange::Value,
            })),
            &mut tx,
        );
        assert_eq!(result, Err(DispatchError::MissingNode(NodeId::from_raw(1))));
    }

    #[test]
    fn edge_add_validates_ports_and_layouts() {
        let mut tx = Tx::new(project());
        tx.insert_node(
            NodeId::from_raw(1),
            Node::new(NodeRole::Track)
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)]),
        );
        tx.insert_node(
            NodeId::from_raw(2),
            Node::new(NodeRole::Master)
                .with_ports([Port::input(PortId::from_raw(0), ChannelLayout::Mono)]),
        );

        let result = run(
            Command::new(CommandKind::Edge(EdgeCommand::Add {
                edge: EdgeId::from_raw(3),
                value: Edge {
                    from: NodePort {
                        node: NodeId::from_raw(1),
                        port: PortId::from_raw(0),
                    },
                    to: NodePort {
                        node: NodeId::from_raw(2),
                        port: PortId::from_raw(0),
                    },
                },
            })),
            &mut tx,
        );

        assert_eq!(
            result,
            Err(DispatchError::EdgeLayoutMismatch {
                edge: EdgeId::from_raw(3),
            })
        );
    }

    #[test]
    fn clip_add_remove_keeps_node_and_sections_in_sync() {
        let mut tx = Tx::new(project());
        tx.insert_node(NodeId::from_raw(1), track());

        run(
            Command::new(CommandKind::Clip(ClipCommand::Add {
                clip: ClipId::from_raw(5),
                value: clip(NodeId::from_raw(1)),
            })),
            &mut tx,
        )
        .unwrap();
        run(
            Command::new(CommandKind::Section(SectionCommand::Add {
                section: SectionId::from_raw(7),
                value: Section {
                    name: "intro".into(),
                    length: Tick::from_raw(odaw_core::PPQ),
                    clips: [ClipId::from_raw(5)].into_iter().collect(),
                },
            })),
            &mut tx,
        )
        .unwrap();

        run(
            Command::new(CommandKind::Clip(ClipCommand::Remove {
                clip: ClipId::from_raw(5),
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        assert!(!outcome.project.clips.contains_key(&ClipId::from_raw(5)));
        assert!(
            !outcome.project.nodes[&NodeId::from_raw(1)]
                .clips
                .contains(&ClipId::from_raw(5))
        );
        assert!(
            !outcome.project.sections[&SectionId::from_raw(7)]
                .clips
                .contains(&ClipId::from_raw(5))
        );
    }

    #[test]
    fn removing_a_node_cascades_to_edges_clips_and_sections() {
        let mut project = project();
        project.nodes.insert(
            NodeId::from_raw(1),
            track().with_rack([NodeId::from_raw(2)]),
        );
        project.nodes.insert(NodeId::from_raw(2), device(1));
        project.nodes.insert(NodeId::from_raw(4), master());
        project.edges.insert(
            EdgeId::from_raw(6),
            Edge {
                from: NodePort {
                    node: NodeId::from_raw(1),
                    port: PortId::from_raw(0),
                },
                to: NodePort {
                    node: NodeId::from_raw(4),
                    port: PortId::from_raw(0),
                },
            },
        );
        project
            .clips
            .insert(ClipId::from_raw(10), clip(NodeId::from_raw(1)));
        project.sections.insert(
            SectionId::from_raw(7),
            Section {
                name: "intro".into(),
                length: Tick::from_raw(odaw_core::PPQ),
                clips: [ClipId::from_raw(10)].into_iter().collect(),
            },
        );
        let mut tx = Tx::new(project);

        run(
            Command::new(CommandKind::Node(NodeCommand::Remove {
                node: NodeId::from_raw(1),
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        assert!(!outcome.project.nodes.contains_key(&NodeId::from_raw(1)));
        assert!(!outcome.project.edges.contains_key(&EdgeId::from_raw(6)));
        assert!(!outcome.project.clips.contains_key(&ClipId::from_raw(10)));
        assert!(
            !outcome.project.sections[&SectionId::from_raw(7)]
                .clips
                .contains(&ClipId::from_raw(10))
        );
    }

    #[test]
    fn removing_a_modulator_cleans_inbound_links() {
        let mut project = project();
        project.nodes.insert(
            NodeId::from_raw(1),
            Node::new(NodeRole::Track)
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)]),
        );
        project
            .nodes
            .insert(NodeId::from_raw(2), Node::new(NodeRole::Modulator));
        project
            .nodes
            .get_mut(&NodeId::from_raw(1))
            .unwrap()
            .modulations
            .insert(
                ParamId::new("gain"),
                [ModulationLink {
                    source: NodeId::from_raw(2),
                    amount: 0.5,
                }]
                .into_iter()
                .collect(),
            );
        let mut tx = Tx::new(project);

        run(
            Command::new(CommandKind::Node(NodeCommand::Remove {
                node: NodeId::from_raw(2),
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        assert!(
            !outcome.project.nodes[&NodeId::from_raw(1)]
                .modulations
                .contains_key(&ParamId::new("gain"))
        );
        assert!(outcome.changes.params.contains(&ParamAddress {
            node: NodeId::from_raw(1),
            param: "gain".into(),
        }));
    }

    #[test]
    fn removing_a_device_cleans_its_rack_entry() {
        let mut project = project();
        project.nodes.insert(
            NodeId::from_raw(1),
            Node::new(NodeRole::Track).with_rack([NodeId::from_raw(2)]),
        );
        project.nodes.insert(NodeId::from_raw(2), device(1));
        let mut tx = Tx::new(project);

        run(
            Command::new(CommandKind::Node(NodeCommand::Remove {
                node: NodeId::from_raw(2),
            })),
            &mut tx,
        )
        .unwrap();

        assert!(
            tx.commit().project.nodes[&NodeId::from_raw(1)]
                .rack
                .is_empty()
        );
    }

    #[test]
    fn removing_a_section_removes_arrangement_references() {
        let mut project = project();
        project.sections.insert(
            SectionId::from_raw(1),
            Section {
                name: "intro".into(),
                length: Tick::from_raw(odaw_core::PPQ),
                clips: [].into_iter().collect(),
            },
        );
        project.arrangement.push_back(SectionRef {
            section: SectionId::from_raw(1),
            repeat: 1,
            active: true,
        });
        let mut tx = Tx::new(project);

        run(
            Command::new(CommandKind::Section(SectionCommand::Remove {
                section: SectionId::from_raw(1),
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        assert!(outcome.project.sections.is_empty());
        assert!(outcome.project.arrangement.is_empty());
        assert!(outcome.changes.arrangement);
    }

    #[test]
    fn device_commands_require_a_device_node() {
        let mut tx = Tx::new(project());
        tx.insert_node(NodeId::from_raw(1), Node::new(NodeRole::Track));
        let result = run(
            Command::new(CommandKind::Device(DeviceCommand::SetKind {
                node: NodeId::from_raw(1),
                kind: odaw_core::NodeKind::new("builtin.gain"),
            })),
            &mut tx,
        );
        assert_eq!(result, Err(DispatchError::NotADevice(NodeId::from_raw(1))));
    }

    #[test]
    fn tempo_commands_rebuild_the_map() {
        let mut tx = Tx::new(project());
        run(
            Command::new(CommandKind::Tempo(TempoCommand::SetTempoMap {
                points: vec![odaw_core::TempoPoint {
                    tick: Tick::ZERO,
                    micros_per_quarter: 400_000,
                }],
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        assert_eq!(
            outcome
                .project
                .tempo
                .points()
                .next()
                .unwrap()
                .micros_per_quarter,
            400_000
        );
        assert!(outcome.changes.meta_or_tempo);
    }

    #[test]
    fn structural_params_mark_the_change_set_structural() {
        let mut tx = Tx::new(project());
        tx.insert_node(NodeId::from_raw(1), track());
        run(
            Command::new(CommandKind::Param(ParamCommand::Set {
                address: ParamAddress {
                    node: NodeId::from_raw(1),
                    param: "layout".into(),
                },
                value: ParamValue::Enum(2),
                change: ParamChange::Structural,
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        assert!(outcome.changes.is_structural());
        assert!(outcome.changes.layout_or_latency);
    }

    #[test]
    fn marker_and_automation_commands_update_the_project() {
        let mut tx = Tx::new(project());
        tx.insert_node(NodeId::from_raw(1), track());
        run(
            Command::new(CommandKind::Marker(MarkerCommand::Set {
                marker: MarkerId::from_raw(2),
                value: Some(Marker {
                    name: "drop".into(),
                    tick: Tick::from_raw(odaw_core::PPQ),
                }),
            })),
            &mut tx,
        )
        .unwrap();
        run(
            Command::new(CommandKind::Automation(AutomationCommand::Set {
                node: NodeId::from_raw(1),
                param: ParamId::new("gain"),
                lane: Some(AutomationLane {
                    mode: AutomationMode::Absolute,
                    enabled: true,
                    keyframes: [Keyframe {
                        tick: Tick::ZERO,
                        value: ParamValue::Float(0.5),
                        curve: Curve::Linear,
                    }]
                    .into_iter()
                    .collect(),
                }),
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        assert!(outcome.project.markers.contains_key(&MarkerId::from_raw(2)));
        assert!(
            outcome.project.nodes[&NodeId::from_raw(1)]
                .automation
                .contains_key(&ParamId::new("gain"))
        );
        assert!(outcome.changes.markers.contains(&MarkerId::from_raw(2)));
        assert!(outcome.changes.is_structural());
    }

    #[test]
    fn note_commands_edit_midi_clips() {
        let mut tx = Tx::new(project());
        tx.insert_node(NodeId::from_raw(1), track());
        run(
            Command::new(CommandKind::Clip(ClipCommand::Add {
                clip: ClipId::from_raw(2),
                value: midi_clip(NodeId::from_raw(1)),
            })),
            &mut tx,
        )
        .unwrap();

        run(
            Command::new(CommandKind::Note(NoteCommand::Add {
                clip: ClipId::from_raw(2),
                note: note(3, 60, 0),
            })),
            &mut tx,
        )
        .unwrap();
        run(
            Command::new(CommandKind::Note(NoteCommand::Replace {
                clip: ClipId::from_raw(2),
                note: note(3, 64, 120),
            })),
            &mut tx,
        )
        .unwrap();

        let outcome = tx.commit();
        let ClipContent::Midi { notes, .. } = &outcome.project.clips[&ClipId::from_raw(2)].content
        else {
            panic!("clip should be MIDI");
        };
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].pitch, 64);
        assert_eq!(notes[0].start, Tick::from_raw(120));
    }

    #[test]
    fn note_commands_require_a_midi_clip() {
        let mut tx = Tx::new(project());
        tx.insert_node(NodeId::from_raw(1), track());
        tx.insert_clip(ClipId::from_raw(2), clip(NodeId::from_raw(1)));
        let result = run(
            Command::new(CommandKind::Note(NoteCommand::Add {
                clip: ClipId::from_raw(2),
                note: note(3, 60, 0),
            })),
            &mut tx,
        );
        assert_eq!(
            result,
            Err(DispatchError::NotAMidiClip(ClipId::from_raw(2)))
        );
    }

    #[test]
    fn duplicate_and_missing_notes_are_rejected() {
        let mut tx = Tx::new(project());
        tx.insert_node(NodeId::from_raw(1), track());
        tx.insert_clip(ClipId::from_raw(2), midi_clip(NodeId::from_raw(1)));
        run(
            Command::new(CommandKind::Note(NoteCommand::Add {
                clip: ClipId::from_raw(2),
                note: note(3, 60, 0),
            })),
            &mut tx,
        )
        .unwrap();
        assert_eq!(
            run(
                Command::new(CommandKind::Note(NoteCommand::Add {
                    clip: ClipId::from_raw(2),
                    note: note(3, 62, 0),
                })),
                &mut tx,
            ),
            Err(DispatchError::NoteExists {
                clip: ClipId::from_raw(2),
                note: NoteId::from_raw(3),
            })
        );
        assert_eq!(
            run(
                Command::new(CommandKind::Note(NoteCommand::Remove {
                    clip: ClipId::from_raw(2),
                    note: NoteId::from_raw(99),
                })),
                &mut tx,
            ),
            Err(DispatchError::MissingNote {
                clip: ClipId::from_raw(2),
                note: NoteId::from_raw(99),
            })
        );
    }

    #[test]
    fn unsupported_command_versions_are_rejected() {
        let mut tx = Tx::new(project());
        let mut command = Command::new(CommandKind::Meta(MetaCommand::SetSeed(1)));
        command.version = COMMAND_VERSION + 1;
        assert_eq!(
            run(command, &mut tx),
            Err(DispatchError::UnsupportedVersion {
                found: COMMAND_VERSION + 1,
                supported: COMMAND_VERSION,
            })
        );
    }
}
