use std::num::NonZeroUsize;

use odaw_compile::Compiler;
use odaw_core::{
    ChannelLayout, Clip, ClipContent, ClipId, Edge, EdgeId, Node, NodeId, NodeKind, NodePort,
    NodeRole, Note, NoteId, ParamId, ParamValue, Port, PortId, Project, Tick,
};
use odaw_dsp::AudioSources;
use odaw_engine::{DeviceContext, DeviceFactory};
use odaw_render::{RenderOptions, render_project_with_compiler};
use odaw_vst3::{Catalog, Vst3Backend};

const DEXED: &str = "ABCDEF019182FAEB4447534244657864";

fn project_with_dexed() -> Project {
    let mut project = Project::empty("dexed", 48_000, 0).unwrap();
    let master = NodeId::from_raw(1);
    project.nodes.insert(
        master,
        Node::new(NodeRole::Master).with_ports([
            Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
            Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
        ]),
    );
    let track = NodeId::from_raw(2);
    let plugin = NodeId::from_raw(3);
    let clip = ClipId::from_raw(4);
    project.nodes.insert(
        track,
        Node::new(NodeRole::Track)
            .with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ])
            .with_rack([plugin])
            .with_clips([clip]),
    );
    project.nodes.insert(
        plugin,
        Node::device(NodeKind::new(format!("vst3:{DEXED}")))
            .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
            .with_param(ParamId::new("param.0"), ParamValue::Float(0.5)),
    );
    project.clips.insert(
        clip,
        Clip {
            node: track,
            start: Tick::ZERO,
            length: Tick::from_raw(4 * odaw_core::PPQ),
            content: ClipContent::Midi {
                source_start: Tick::ZERO,
                source_length: Tick::from_raw(4 * odaw_core::PPQ),
                notes: [Note {
                    id: NoteId::from_raw(1),
                    pitch: 60,
                    velocity: 100,
                    channel: 0,
                    start: Tick::ZERO,
                    length: Tick::from_raw(odaw_core::PPQ),
                }]
                .into_iter()
                .collect(),
            },
        },
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
    project
}

#[test]
fn plugin_identity_and_state_round_trip_through_the_container() {
    let catalog = Catalog::scan();
    if catalog.get(DEXED).is_none() {
        eprintln!("Dexed is not installed; skipping");
        return;
    }
    let backend = Vst3Backend::new(catalog);
    let factory = backend.factory();
    let context = DeviceContext::new(NodeId::from_raw(3), 48_000, 256, 2, 120.0);
    let kind = NodeKind::new(format!("vst3:{DEXED}"));
    let node = Node::device(kind.clone());
    let _processor = factory.build(&node, &context).unwrap();
    let state = backend
        .capture_states()
        .remove(&NodeId::from_raw(3))
        .expect("the live plugin saves state");
    assert!(!state.is_empty());

    let mut project = project_with_dexed();
    project.nodes.get_mut(&NodeId::from_raw(3)).unwrap().state = Some(state.into());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plug.odaw");
    odaw_format::Container::new(&path).save(&project).unwrap();
    let loaded = odaw_format::Container::new(&path).load().unwrap();
    let loaded_node = &loaded.nodes[&NodeId::from_raw(3)];
    assert_eq!(loaded_node.kind.as_ref(), Some(&kind));
    assert!(
        loaded_node
            .state
            .as_ref()
            .is_some_and(|state| !state.is_empty())
    );
}

#[test]
fn a_project_with_a_vst3_instrument_renders_audio() {
    let catalog = Catalog::scan();
    if catalog.get(DEXED).is_none() {
        eprintln!("Dexed is not installed; skipping");
        return;
    }
    let backend = Vst3Backend::new(catalog);
    let compiler = Compiler::builtin().with_factory(backend.factory());
    let audio = render_project_with_compiler(
        &project_with_dexed(),
        RenderOptions {
            block_size: NonZeroUsize::new(256).unwrap(),
            frames: 48_000,
        },
        &AudioSources::default(),
        &compiler,
    )
    .unwrap();
    assert!(audio.samples.iter().any(|sample| sample.abs() > 1e-4));
}
