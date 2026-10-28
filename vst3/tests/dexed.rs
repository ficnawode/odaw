use std::sync::Mutex;

use odaw_core::{Node, NodeId, ParamAddress, ParamId, ParamValue};
use odaw_engine::{Block, DeviceContext, DeviceFactory, NoteEvent, NoteEventKind, ParamBuffer};
use odaw_vst3::{Catalog, Vst3Backend};

const DEXED: &str = "ABCDEF019182FAEB4447534244657864";

/// Vendor plugin factories are not reliable under concurrent instantiate /
/// process / teardown. The app is single-threaded for plugin work and tests
/// take this lock for the same reason.
static PLUGIN_TEST_LOCK: Mutex<()> = Mutex::new(());

fn guard() -> std::sync::MutexGuard<'static, ()> {
    PLUGIN_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn dexed_catalog() -> Option<Catalog> {
    let catalog = Catalog::scan();
    catalog.get(DEXED)?;
    Some(catalog)
}

#[test]
fn dexed_is_listed_with_metadata() {
    let _guard = guard();
    let Some(catalog) = dexed_catalog() else {
        eprintln!("Dexed is not installed; skipping");
        return;
    };
    let entry = catalog.get(DEXED).unwrap();
    assert_eq!(entry.name, "Dexed");
    assert!(entry.is_instrument());
    assert!(entry.has_gui);
    assert_eq!(entry.kind(), format!("vst3:{DEXED}"));
}

#[test]
fn dexed_instrument_processes_a_note() {
    let _guard = guard();
    let Some(catalog) = dexed_catalog() else {
        eprintln!("Dexed is not installed; skipping");
        return;
    };
    let backend = Vst3Backend::new(catalog);
    let factory = backend.factory();
    let node = Node::device(odaw_core::NodeKind::new(format!("vst3:{DEXED}")))
        .with_param(ParamId::new("param.0"), ParamValue::Float(0.5));
    let context = DeviceContext::new(NodeId::from_raw(1), 48_000, 256, 2, 120.0);
    let mut processor = factory.build(&node, &context).unwrap();
    assert!(processor.parameter_defaults().len() > 10);

    let mut params = ParamBuffer::new();
    for (id, value) in processor.parameter_defaults() {
        params.ensure(
            ParamAddress {
                node: NodeId::from_raw(1),
                param: id,
            },
            value,
        );
    }
    let events = [NoteEvent {
        target: NodeId::from_raw(1),
        frame: 0,
        kind: NoteEventKind::On,
        pitch: 60,
        velocity: 100,
    }];

    let mut energy = 0.0f32;
    for block_index in 0..16 {
        let mut output = vec![0.0; 256 * 2];
        let input = vec![0.0; 256 * 2];
        let mut block = Block {
            node: NodeId::from_raw(1),
            event_target: Some(NodeId::from_raw(1)),
            sample_rate: 48_000,
            channels: 2,
            start_frame: block_index * 256,
            frames: 256,
            input: &input,
            output: &mut output,
            params: &params,
            events: if block_index == 0 { &events } else { &[] },
        };
        processor.process(&mut block);
        energy += output.iter().map(|sample| sample.abs()).sum::<f32>();
    }
    assert!(energy > 0.0, "Dexed produced silence");
}

#[test]
fn a_bypassed_instrument_goes_silent() {
    let _guard = guard();
    let Some(catalog) = dexed_catalog() else {
        eprintln!("Dexed is not installed; skipping");
        return;
    };
    let backend = Vst3Backend::new(catalog);
    let node_id = NodeId::from_raw(1);
    let node = Node::device(odaw_core::NodeKind::new(format!("vst3:{DEXED}")));
    let context = DeviceContext::new(node_id, 48_000, 256, 2, 120.0);
    let mut processor = backend.factory().build(&node, &context).unwrap();
    let mut params = ParamBuffer::new();
    for (id, value) in processor.parameter_defaults() {
        params.ensure(
            ParamAddress {
                node: node_id,
                param: id,
            },
            value,
        );
    }
    params.set(
        &ParamAddress {
            node: node_id,
            param: ParamId::new(odaw_vst3::BYPASS_PARAM),
        },
        ParamValue::Float(1.0),
    );
    let events = [NoteEvent {
        target: node_id,
        frame: 0,
        kind: NoteEventKind::On,
        pitch: 60,
        velocity: 100,
    }];
    let input = vec![0.0; 256 * 2];
    let mut output = vec![1.0; 256 * 2];
    let mut block = Block {
        node: node_id,
        event_target: Some(node_id),
        sample_rate: 48_000,
        channels: 2,
        start_frame: 0,
        frames: 256,
        input: &input,
        output: &mut output,
        params: &params,
        events: &events,
    };
    processor.process(&mut block);
    assert!(output.iter().all(|sample| *sample == 0.0));
}

#[test]
fn state_is_saved_and_restored() {
    let _guard = guard();
    let Some(catalog) = dexed_catalog() else {
        eprintln!("Dexed is not installed; skipping");
        return;
    };
    let backend = Vst3Backend::new(catalog);
    let factory = backend.factory();
    let kind = odaw_core::NodeKind::new(format!("vst3:{DEXED}"));
    let context = DeviceContext::new(NodeId::from_raw(7), 48_000, 256, 2, 120.0);
    let processor = factory
        .build(&Node::device(kind.clone()), &context)
        .unwrap();
    drop(processor);
    let states = backend.capture_states();
    let state = states.get(&NodeId::from_raw(7)).cloned();
    assert!(state.as_ref().is_some_and(|state| !state.is_empty()));

    let node = Node::device(kind).with_state(state.unwrap());
    assert!(factory.build(&node, &context).is_ok());
}
