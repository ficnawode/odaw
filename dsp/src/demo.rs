use odaw_core::{
    ChannelLayout, Clip, ClipContent, ClipId, Edge, EdgeId, LoopRegion, Marker, MarkerId, Node,
    NodeId, NodeKind, NodePort, NodeRole, Note, NoteId, ParamId, ParamValue, Port, PortId, Project,
    Tick,
};

use crate::devices::{GAIN_KIND, POLY_KIND};

pub fn project() -> Project {
    let mut project = Project::empty("demo", 48_000, 1).expect("the demo sample rate is valid");

    let track = NodeId::from_raw(1);
    let instrument = NodeId::from_raw(2);
    let gain = NodeId::from_raw(3);
    let master = NodeId::from_raw(4);

    project.nodes.insert(
        track,
        Node::new(NodeRole::Track)
            .with_name("Lead")
            .with_color(0x4c8bf5)
            .with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ])
            .with_rack([instrument, gain]),
    );
    project.nodes.insert(
        instrument,
        Node::device(NodeKind::new(POLY_KIND))
            .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
            .with_param(ParamId::new("gain"), ParamValue::Float(0.25)),
    );
    project.nodes.insert(
        gain,
        Node::device(NodeKind::new(GAIN_KIND))
            .with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ])
            .with_param(ParamId::new("gain"), ParamValue::Float(0.5)),
    );
    project.nodes.insert(
        master,
        Node::new(NodeRole::Master).with_ports([
            Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
            Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
        ]),
    );

    project.edges.insert(
        EdgeId::from_raw(5),
        Edge {
            from: NodePort {
                node: track,
                port: PortId::from_raw(1),
            },
            to: NodePort {
                node: master,
                port: PortId::from_raw(0),
            },
        },
    );

    project.loops.push_back(LoopRegion {
        start: Tick::ZERO,
        end: Tick::from_raw(16 * odaw_core::PPQ),
    });
    project.markers.insert(
        MarkerId::from_raw(6),
        Marker {
            name: "start".into(),
            tick: Tick::ZERO,
        },
    );
    let pitches = [
        60, 64, 67, 72, 67, 64, 60, 55, 60, 64, 67, 72, 76, 72, 67, 64,
    ];
    let notes: Vec<Note> = pitches
        .into_iter()
        .enumerate()
        .map(|(index, pitch)| Note {
            id: NoteId::from_raw(100 + index as u64),
            pitch,
            start: Tick::from_raw(index as i64 * odaw_core::PPQ),
            length: Tick::from_raw(odaw_core::PPQ),
            velocity: 100,
            channel: 0,
        })
        .collect();

    project.clips.insert(
        ClipId::from_raw(7),
        Clip {
            node: track,
            start: Tick::ZERO,
            length: Tick::from_raw(16 * odaw_core::PPQ),
            content: ClipContent::Midi {
                source_start: Tick::ZERO,
                source_length: Tick::from_raw(16 * odaw_core::PPQ),
                notes: notes.into_iter().collect(),
            },
        },
    );
    project
        .nodes
        .get_mut(&track)
        .expect("track exists")
        .clips
        .push_back(ClipId::from_raw(7));

    project
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_project_is_valid() {
        let project = project();
        assert_eq!(project.validate(), Ok(()));
        assert_eq!(project.meta.name, "demo");
        assert_eq!(project.nodes.len(), 4);
        assert_eq!(project.loops.len(), 1);
        let ClipContent::Midi { notes, .. } = &project.clips[&ClipId::from_raw(7)].content else {
            panic!("demo clip should be MIDI");
        };
        assert_eq!(notes.len(), 16);
    }
}
