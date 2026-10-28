use std::num::NonZeroUsize;
use std::sync::Mutex;

use odaw_compile::Compiler;
use odaw_core::{
    ChannelLayout, Clip, ClipContent, ClipId, Edge, EdgeId, Node, NodeId, NodeKind, NodePort,
    NodeRole, Note, NoteId, Port, PortId, Project, Tick,
};
use odaw_dsp::AudioSources;
use odaw_engine::{Block, DeviceContext, DeviceFactory, ParamBuffer};
use odaw_render::{RenderOptions, render_project_with_compiler};
use odaw_vst3::{Catalog, PluginEntry, Vst3Backend};

const DEXED: &str = "ABCDEF019182FAEB4447534244657864";

/// DPF-based plugins are not safe to instantiate, process, and tear down from
/// several threads at once. The app is single-threaded for these operations;
/// tests serialise for the same reason.
static PLUGIN_TEST_LOCK: Mutex<()> = Mutex::new(());

fn effects(catalog: &Catalog) -> Vec<PluginEntry> {
    catalog
        .entries
        .iter()
        .filter(|entry| entry.is_effect())
        .cloned()
        .collect()
}

fn effect_node(_id: NodeId, entry: &PluginEntry) -> Node {
    Node::device(NodeKind::new(entry.kind()))
        .with_name(entry.name.clone())
        .with_ports([
            Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
            Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
        ])
}

fn context(id: NodeId) -> DeviceContext {
    DeviceContext::new(id, 48_000, 256, 2, 120.0)
}

#[test]
fn installed_effect_processes_and_round_trips_state() {
    let _guard = PLUGIN_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let catalog = Catalog::scan();
    let entries = effects(&catalog);
    let Some(entry) = entries.first().cloned() else {
        eprintln!("no VST3 effect is installed; skipping");
        return;
    };
    let backend = Vst3Backend::new(catalog);
    let factory = backend.factory();
    let node_id = NodeId::from_raw(10);
    let node = effect_node(node_id, &entry);
    let mut processor = factory.build(&node, &context(node_id)).unwrap();
    let defaults = processor.parameter_defaults();
    eprintln!(
        "{}: {} parameters, latency {}",
        entry.name,
        defaults.len(),
        processor.latency_frames()
    );
    for (id, value) in defaults.iter().take(12) {
        eprintln!("  {id} = {value:?}");
    }

    let mut params = ParamBuffer::new();
    for (id, value) in defaults {
        params.ensure(
            odaw_core::ParamAddress {
                node: node_id,
                param: id,
            },
            value,
        );
    }

    let frames = 256;
    let mut energy = 0.0f32;
    for block_index in 0..64 {
        let mut input = vec![0.0; frames * 2];
        if block_index == 0 {
            input[0] = 0.5;
            input[1] = 0.5;
        }
        let mut output = vec![0.0; frames * 2];
        let mut block = Block {
            node: node_id,
            event_target: None,
            sample_rate: 48_000,
            channels: 2,
            start_frame: block_index * frames as u64,
            frames,
            input: &input,
            output: &mut output,
            params: &params,
            events: &[],
        };
        processor.process(&mut block);
        energy += output.iter().map(|sample| sample.abs()).sum::<f32>();
    }
    eprintln!("{}: tail energy {energy}", entry.name);
    assert!(energy > 0.0, "{} produced silence", entry.name);

    let handle = backend.plugin(node_id).expect("the effect is live");
    {
        let mut plugin = handle.lock().unwrap();
        plugin.set_parameter(2, 0.1).unwrap();
    }
    let adjusted = handle
        .lock()
        .unwrap()
        .get_parameters()
        .unwrap()
        .into_iter()
        .find(|parameter| parameter.id == 2)
        .map(|parameter| parameter.value)
        .expect("the effect still has parameter 2");
    let state = backend
        .capture_states()
        .remove(&node_id)
        .expect("the effect saves state");
    assert!(!state.is_empty());
    let parameters = backend
        .capture_parameters()
        .remove(&node_id)
        .expect("the effect reports parameters");
    let mut restored_node = effect_node(node_id, &entry).with_state(state);
    for (param, value) in parameters {
        restored_node.params.insert(param, value);
    }
    let _restored = factory.build(&restored_node, &context(node_id)).unwrap();
    let restored = backend
        .plugin(node_id)
        .unwrap()
        .lock()
        .unwrap()
        .get_parameters()
        .unwrap()
        .into_iter()
        .find(|parameter| parameter.id == 2)
        .map(|parameter| parameter.value)
        .expect("the restored effect still has parameter 2");
    assert!((restored - adjusted).abs() < 1e-6);
}

fn project_with_effects(effect_count: usize, entries: &[PluginEntry]) -> Project {
    let mut project = Project::empty("fx", 48_000, 0).unwrap();
    let master = NodeId::from_raw(1);
    let track = NodeId::from_raw(2);
    let instrument = NodeId::from_raw(3);
    let clip = ClipId::from_raw(4);
    let mut rack = vec![instrument];
    for (index, entry) in entries.iter().take(effect_count).enumerate() {
        let id = NodeId::from_raw(10 + index as u64);
        rack.push(id);
        project.nodes.insert(id, effect_node(id, entry));
    }
    project.nodes.insert(
        master,
        Node::new(NodeRole::Master).with_ports([
            Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
            Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
        ]),
    );
    project.nodes.insert(
        track,
        Node::new(NodeRole::Track)
            .with_name("Stack")
            .with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ])
            .with_rack(rack)
            .with_clips([clip]),
    );
    project.nodes.insert(
        instrument,
        Node::device(NodeKind::new(format!("vst3:{DEXED}")))
            .with_name("Dexed")
            .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)]),
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
                    length: Tick::from_raw(2 * odaw_core::PPQ),
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

fn render(project: &Project, backend: &Vst3Backend) -> Vec<f32> {
    let compiler = Compiler::builtin().with_factory(backend.factory());
    render_project_with_compiler(
        project,
        RenderOptions {
            block_size: NonZeroUsize::new(256).unwrap(),
            frames: 24_000,
        },
        &AudioSources::default(),
        &compiler,
    )
    .unwrap()
    .samples
}

#[test]
fn bypass_passes_input_through() {
    let _guard = PLUGIN_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let catalog = Catalog::scan();
    let Some(entry) = effects(&catalog).first().cloned() else {
        eprintln!("no VST3 effect is installed; skipping");
        return;
    };
    let backend = Vst3Backend::new(catalog);
    let node_id = NodeId::from_raw(20);
    let mut processor = backend
        .factory()
        .build(&effect_node(node_id, &entry), &context(node_id))
        .unwrap();
    let mut params = ParamBuffer::new();
    for (id, value) in processor.parameter_defaults() {
        params.ensure(
            odaw_core::ParamAddress {
                node: node_id,
                param: id,
            },
            value,
        );
    }
    params.set(
        &odaw_core::ParamAddress {
            node: node_id,
            param: odaw_core::ParamId::new(odaw_vst3::BYPASS_PARAM),
        },
        odaw_core::ParamValue::Float(1.0),
    );

    let frames = 4;
    let input = vec![0.1, -0.2, 0.3, -0.4, 0.5, -0.6, 0.7, -0.8];
    let mut output = vec![0.0; frames * 2];
    let mut block = Block {
        node: node_id,
        event_target: None,
        sample_rate: 48_000,
        channels: 2,
        start_frame: 0,
        frames,
        input: &input,
        output: &mut output,
        params: &params,
        events: &[],
    };
    processor.process(&mut block);
    assert_eq!(output, input);
}

#[test]
fn two_effects_stack_in_a_project_render() {
    let _guard = PLUGIN_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let catalog = Catalog::scan();
    if catalog.get(DEXED).is_none() {
        eprintln!("Dexed is not installed; skipping");
        return;
    }
    let entries = effects(&catalog);
    if entries.len() < 2 {
        eprintln!("fewer than two VST3 effects are installed; skipping");
        return;
    }
    let backend = Vst3Backend::new(catalog);
    let dry = render(&project_with_effects(0, &entries), &backend);
    let one = render(&project_with_effects(1, &entries), &backend);
    let two = render(&project_with_effects(2, &entries), &backend);
    assert!(dry.iter().any(|sample| sample.abs() > 1e-5));
    assert!(one.iter().all(|sample| sample.is_finite()));
    assert!(two.iter().all(|sample| sample.is_finite()));
    assert_ne!(dry, one, "one effect did not change the render");
    assert_ne!(one, two, "the second effect did not change the render");
}
