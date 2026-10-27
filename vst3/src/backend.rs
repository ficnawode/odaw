use std::collections::BTreeMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use odaw_core::{Node, NodeId, NodeKind, ParamAddress, ParamId, ParamValue};
use odaw_engine::{
    Block, DeviceContext, DeviceDescriptor, DeviceError, DeviceFactory, DeviceParameter,
    DeviceProcessor, NoteEventKind,
};
use vst3_host::audio::{AudioBuffers, AudioBusLayout};
use vst3_host::midi::{MidiChannel, MidiEvent};
use vst3_host::{Plugin, Vst3Host};

use crate::catalog::{Catalog, PluginEntry, class_id_from_kind};

pub const PARAM_PREFIX: &str = "param.";

/// Loading several plugin modules concurrently is not reliable across all
/// vendor factories. Compilation is control-thread work, so serialise it.
static LOAD_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub struct Vst3Backend {
    catalog: Arc<Catalog>,
    instances: Arc<Mutex<BTreeMap<NodeId, Arc<Mutex<Plugin>>>>>,
    schemas: Arc<Mutex<BTreeMap<NodeId, Vec<DeviceParameter>>>>,
}

impl fmt::Debug for Vst3Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Vst3Backend")
            .field("plugins", &self.catalog.entries.len())
            .finish_non_exhaustive()
    }
}

impl Vst3Backend {
    pub fn new(catalog: Catalog) -> Self {
        Self {
            catalog: Arc::new(catalog),
            instances: Arc::default(),
            schemas: Arc::default(),
        }
    }

    pub fn load_cached() -> Self {
        Self::new(Catalog::load_or_empty())
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Replace the catalogue without dropping live plugin instances. A rescan
    /// keeps the current patch and only affects the next build.
    pub fn set_catalog(&mut self, catalog: Catalog) {
        self.catalog = Arc::new(catalog);
    }

    pub fn factory(&self) -> Vst3Factory {
        Vst3Factory {
            backend: self.clone(),
        }
    }

    /// Return the live plugin for a node. Used by the editor window bridge and
    /// by state capture.
    pub fn plugin(&self, node: NodeId) -> Option<Arc<Mutex<Plugin>>> {
        self.instances
            .lock()
            .ok()
            .and_then(|instances| instances.get(&node).cloned())
    }

    pub fn parameters(&self, node: NodeId) -> Option<Vec<DeviceParameter>> {
        self.schemas
            .lock()
            .ok()
            .and_then(|schemas| schemas.get(&node).cloned())
    }

    /// Serialize every live plugin's state. The caller turns this into document
    /// state when saving; no audio-thread work is added.
    pub fn capture_states(&self) -> BTreeMap<NodeId, Vec<u8>> {
        let instances: Vec<(NodeId, Arc<Mutex<Plugin>>)> = self
            .instances
            .lock()
            .map(|instances| {
                instances
                    .iter()
                    .map(|(node, plugin)| (*node, Arc::clone(plugin)))
                    .collect()
            })
            .unwrap_or_default();
        let mut states = BTreeMap::new();
        for (node, plugin) in instances {
            if let Ok(plugin) = plugin.lock() {
                if let Ok(state) = plugin.save_state() {
                    states.insert(node, state);
                }
            }
        }
        states
    }

    /// Ask the plugin to format a normalized parameter value (for example
    /// `"440.00 Hz"`). Used by the generic panel while a control is active.
    pub fn format_parameter(&self, node: NodeId, param: &str, value: f64) -> Option<String> {
        let id = param_id(param)?;
        let plugin = self.plugin(node)?;
        let plugin = plugin.lock().ok()?;
        plugin.format_parameter(id, value.clamp(0.0, 1.0)).ok()
    }

    /// Forget instances for nodes that no longer exist in the document. The
    /// processor's own `Arc` keeps a still-retiring plan alive until it is
    /// dropped by the control thread.
    pub fn retain_nodes(&self, nodes: &std::collections::BTreeSet<NodeId>) {
        if let Ok(mut instances) = self.instances.lock() {
            instances.retain(|node, _| nodes.contains(node));
        }
        if let Ok(mut schemas) = self.schemas.lock() {
            schemas.retain(|node, _| nodes.contains(node));
        }
    }

    /// Current normalized parameter values for every live plugin. Saving and
    /// rebuilding merge these into the document so editor-made changes survive
    /// a reload even when they were never committed as odaw parameters.
    pub fn capture_parameters(&self) -> BTreeMap<NodeId, Vec<(ParamId, ParamValue)>> {
        let instances: Vec<(NodeId, Arc<Mutex<Plugin>>)> = self
            .instances
            .lock()
            .map(|instances| {
                instances
                    .iter()
                    .map(|(node, plugin)| (*node, Arc::clone(plugin)))
                    .collect()
            })
            .unwrap_or_default();
        let mut captured = BTreeMap::new();
        for (node, plugin) in instances {
            let Ok(plugin) = plugin.lock() else {
                continue;
            };
            let Ok(parameters) = plugin.get_parameters() else {
                continue;
            };
            captured.insert(
                node,
                parameters
                    .into_iter()
                    .map(|parameter| {
                        (
                            ParamId::new(format!("{PARAM_PREFIX}{}", parameter.id)),
                            ParamValue::Float(parameter.value.clamp(0.0, 1.0)),
                        )
                    })
                    .collect(),
            );
        }
        captured
    }

    fn build_inner(
        &self,
        node: &Node,
        context: &DeviceContext,
    ) -> Result<Box<dyn DeviceProcessor>, DeviceError> {
        let kind = node
            .kind
            .as_ref()
            .ok_or_else(|| DeviceError::new(context.node, "the device has no kind"))?;
        let class_id = class_id_from_kind(kind.as_str())
            .ok_or_else(|| DeviceError::new(context.node, "not a VST3 device kind"))?;
        let Some(entry) = self.catalog.get(class_id) else {
            if let Ok(mut instances) = self.instances.lock() {
                instances.remove(&context.node);
            }
            if let Ok(mut schemas) = self.schemas.lock() {
                schemas.remove(&context.node);
            }
            return Ok(Box::new(MissingPlugin {
                label: format!("Missing VST3 plugin ({class_id})"),
            }));
        };
        let _load_guard = LOAD_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut host = Vst3Host::builder()
            .sample_rate(f64::from(context.sample_rate))
            .block_size(context.max_block_size)
            .input_channels(context.channels)
            .build()
            .map_err(|error| DeviceError::new(context.node, error.to_string()))?;
        let plugin = host
            .load_plugin_class(&entry.path, &entry.class_id)
            .map_err(|error| DeviceError::new(context.node, error.to_string()))?;
        let plugin = Arc::new(Mutex::new(plugin));

        let state = node.state.as_deref();
        let result = configure_plugin(&plugin, state, node, entry, context);
        match result {
            Ok(schema) => {
                if let Ok(mut schemas) = self.schemas.lock() {
                    schemas.insert(context.node, schema);
                }
                if let Ok(mut instances) = self.instances.lock() {
                    instances.insert(context.node, Arc::clone(&plugin));
                }
                let processor = Vst3Processor::new(plugin, entry.clone(), node, context)
                    .map_err(|error| DeviceError::new(context.node, error))?;
                Ok(Box::new(processor))
            }
            Err(error) => {
                if let Ok(mut plugin) = plugin.lock() {
                    let _ = plugin.stop_processing();
                }
                Err(DeviceError::new(context.node, error))
            }
        }
    }
}

fn configure_plugin(
    plugin: &Arc<Mutex<Plugin>>,
    state: Option<&[u8]>,
    node: &Node,
    entry: &PluginEntry,
    context: &DeviceContext,
) -> Result<Vec<DeviceParameter>, String> {
    let mut plugin = plugin.lock().map_err(|error| error.to_string())?;
    if let Some(state) = state {
        plugin
            .load_state(state)
            .map_err(|error| format!("could not restore plugin state: {error}"))?;
    }
    let known: std::collections::BTreeSet<u32> = plugin
        .get_parameters()
        .map(|parameters| {
            parameters
                .into_iter()
                .map(|parameter| parameter.id)
                .collect()
        })
        .unwrap_or_default();
    for (id, value) in &node.params {
        let Some(id) = param_id(id.as_str()) else {
            continue;
        };
        if !known.contains(&id) {
            continue;
        }
        if let Some(value) = normalized(value) {
            // A stale or read-only parameter must not make the project
            // unloadable; keep the closest valid patch instead.
            let _ = plugin.set_parameter(id, value);
        }
    }

    let _ = plugin.set_tempo(context.tempo_bpm.max(1.0));
    let _ = plugin.set_playing(true);
    plugin
        .start_processing()
        .map_err(|error| format!("could not start plugin: {error}"))?;
    let layout = plugin
        .audio_bus_layout()
        .map_err(|error| format!("could not read plugin buses: {error}"))?;
    validate_layout(&layout, context.channels, entry.is_instrument())?;

    let parameters = plugin
        .get_parameters()
        .map_err(|error| format!("could not read parameters: {error}"))?;
    Ok(parameters
        .into_iter()
        .map(|parameter| DeviceParameter {
            id: ParamId::new(format!("{PARAM_PREFIX}{}", parameter.id)),
            label: parameter.name,
            default: ParamValue::Float(parameter.default.clamp(0.0, 1.0)),
            min: 0.0,
            max: 1.0,
            step_count: u32::try_from(parameter.step_count).unwrap_or(u32::MAX),
            unit: parameter.unit,
            read_only: parameter.is_read_only,
        })
        .collect())
}

fn validate_layout(
    layout: &AudioBusLayout,
    channels: usize,
    instrument: bool,
) -> Result<(), String> {
    if layout.outputs.len() != 1 {
        return Err(format!(
            "only one main audio output bus is supported, plugin has {}",
            layout.outputs.len()
        ));
    }
    if instrument && !layout.inputs.is_empty() {
        return Err("instrument plugins with audio input buses are not supported".to_owned());
    }
    if !instrument && layout.inputs.len() > 1 {
        return Err(format!(
            "only one main audio input bus is supported, plugin has {}",
            layout.inputs.len()
        ));
    }
    let output = &layout.outputs[0];
    if !(1..=2).contains(&output.channel_count) {
        return Err(format!(
            "only mono/stereo plugin outputs are supported, plugin reports {} channels",
            output.channel_count
        ));
    }
    if !output.active {
        return Err("the plugin's main output bus is inactive".to_owned());
    }
    if let Some(input) = layout.inputs.first() {
        if !(0..=2).contains(&input.channel_count) {
            return Err(format!(
                "only mono/stereo plugin inputs are supported, plugin reports {} channels",
                input.channel_count
            ));
        }
        if !input.active {
            return Err("the plugin's main input bus is inactive".to_owned());
        }
    }
    if !instrument && layout.inputs.is_empty() {
        return Err("effect plugins need one main audio input bus".to_owned());
    }
    if channels == 0 {
        return Err("the project has no audio channels".to_owned());
    }
    Ok(())
}

fn param_id(value: &str) -> Option<u32> {
    value.strip_prefix(PARAM_PREFIX)?.parse().ok()
}

fn normalized(value: &ParamValue) -> Option<f64> {
    match value {
        ParamValue::Float(value) => Some(value.clamp(0.0, 1.0)),
        ParamValue::Int(value) => Some((*value as f64).clamp(0.0, 1.0)),
        ParamValue::Bool(value) => Some(f64::from(u8::from(*value))),
        ParamValue::Enum(value) => Some(f64::from(*value).clamp(0.0, 1.0)),
    }
}

#[derive(Clone, Debug)]
pub struct Vst3Factory {
    backend: Vst3Backend,
}

impl DeviceFactory for Vst3Factory {
    fn descriptor(&self, kind: &NodeKind) -> Option<DeviceDescriptor> {
        let class_id = class_id_from_kind(kind.as_str())?;
        let entry = self.backend.catalog.get(class_id);
        Some(DeviceDescriptor {
            kind: kind.clone(),
            label: entry.map_or_else(
                || format!("Missing VST3 plugin ({class_id})"),
                |entry| entry.name.clone(),
            ),
            vendor: entry.map_or_else(String::new, |entry| entry.vendor.clone()),
            instrument: entry.is_some_and(PluginEntry::is_instrument),
            parameters: Vec::new(),
        })
    }

    fn build(
        &self,
        node: &Node,
        context: &DeviceContext,
    ) -> Result<Box<dyn DeviceProcessor>, DeviceError> {
        self.backend.build_inner(node, context)
    }
}

/// Placeholder for a plugin that is not installed. Effects pass audio through;
/// instruments (no input buffer) produce silence, so a project still opens and
/// the rest of the graph keeps working.
#[derive(Debug)]
pub struct MissingPlugin {
    label: String,
}

impl DeviceProcessor for MissingPlugin {
    fn process(&mut self, block: &mut Block<'_>) {
        if block.input.len() == block.output.len() {
            block.output.copy_from_slice(block.input);
        } else {
            block.output.fill(0.0);
        }
    }
}

impl MissingPlugin {
    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Debug)]
struct ParamSlot {
    id: u32,
    address: ParamAddress,
    default: f64,
    slot: Option<usize>,
    version: u64,
}

pub struct Vst3Processor {
    node: NodeId,
    plugin: Arc<Mutex<Plugin>>,
    info: PluginEntry,
    params: Vec<ParamSlot>,
    buffers: AudioBuffers,
    plugin_input_channels: usize,
    plugin_output_channels: usize,
    accepts_notes: bool,
    tempo_bpm: f64,
    failed: bool,
}

impl fmt::Debug for Vst3Processor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Vst3Processor")
            .field("plugin", &self.info.name)
            .field("class_id", &self.info.class_id)
            .field("inputs", &self.plugin_input_channels)
            .field("outputs", &self.plugin_output_channels)
            .finish()
    }
}

impl Vst3Processor {
    fn new(
        plugin: Arc<Mutex<Plugin>>,
        info: PluginEntry,
        node: &Node,
        context: &DeviceContext,
    ) -> Result<Self, String> {
        let (parameters, plugin_input_channels, plugin_output_channels, has_midi_input) = {
            let plugin = plugin.lock().map_err(|error| error.to_string())?;
            let layout = plugin
                .audio_bus_layout()
                .map_err(|error| error.to_string())?;
            let input = layout.inputs.first().map_or(0, |bus| bus.channel_count);
            let output = layout.outputs.first().map_or(0, |bus| bus.channel_count);
            let parameters = plugin.get_parameters().map_err(|error| error.to_string())?;
            (parameters, input, output, plugin.info().has_midi_input)
        };
        let params = parameters
            .into_iter()
            .map(|parameter| {
                let param = ParamId::new(format!("{PARAM_PREFIX}{}", parameter.id));
                let value = node
                    .params
                    .get(&param)
                    .and_then(normalized)
                    .unwrap_or(parameter.default.clamp(0.0, 1.0));
                ParamSlot {
                    id: parameter.id,
                    address: ParamAddress {
                        node: context.node,
                        param,
                    },
                    default: value,
                    slot: None,
                    version: 0,
                }
            })
            .collect();
        let accepts_notes = has_midi_input || info.is_instrument();
        Ok(Self {
            node: context.node,
            plugin,
            info,
            params,
            buffers: AudioBuffers::new(
                plugin_input_channels,
                plugin_output_channels,
                context.max_block_size,
                f64::from(context.sample_rate),
            ),
            plugin_input_channels,
            plugin_output_channels,
            accepts_notes,
            tempo_bpm: context.tempo_bpm.max(1.0),
            failed: false,
        })
    }

    fn bind_node(&mut self, node: NodeId) {
        if self.node != node {
            self.node = node;
            for parameter in &mut self.params {
                parameter.address.node = node;
                parameter.slot = None;
                parameter.version = 0;
            }
        }
    }

    fn sync_parameters(&mut self, block: &Block<'_>, plugin: &mut Plugin) {
        for parameter in &mut self.params {
            let Some(slot) = parameter
                .slot
                .or_else(|| block.params.slot(&parameter.address))
            else {
                continue;
            };
            parameter.slot = Some(slot);
            let version = block.params.version(slot).unwrap_or(0);
            if version == parameter.version {
                continue;
            }
            parameter.version = version;
            let value = block.params.float(&parameter.address, parameter.default);
            let value = if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                parameter.default
            };
            let _ = plugin.set_parameter(parameter.id, value);
        }
    }

    fn send_events(&mut self, block: &Block<'_>, plugin: &mut Plugin) {
        if !self.accepts_notes {
            return;
        }
        let Some(target) = block.event_target else {
            return;
        };
        for event in block.events {
            if event.target != target {
                continue;
            }
            let offset = i32::try_from(event.frame).unwrap_or(i32::MAX);
            let event = match event.kind {
                NoteEventKind::On => MidiEvent::NoteOn {
                    channel: MidiChannel::Ch1,
                    note: event.pitch.min(127),
                    velocity: event.velocity.min(127),
                },
                NoteEventKind::Off => MidiEvent::NoteOff {
                    channel: MidiChannel::Ch1,
                    note: event.pitch.min(127),
                    velocity: event.velocity.min(127),
                },
            };
            let _ = plugin.send_midi_event_at(event, offset);
        }
    }

    fn copy_input(&mut self, block: &Block<'_>) {
        let frames = block.frames;
        let project_channels = block.channels;
        let plugin_channels = self.plugin_input_channels;
        for channel in 0..plugin_channels {
            let buffer = &mut self.buffers.inputs[channel];
            buffer.clear();
            buffer.resize(frames, 0.0);
            for (frame, sample) in buffer.iter_mut().enumerate() {
                *sample = match (plugin_channels, project_channels) {
                    (1, 1) => block.input[frame],
                    (2, 1) => block.input[frame],
                    (1, _) => block.input[frame * project_channels],
                    (2, 2) => block.input[frame * project_channels + channel],
                    (_, _) if channel < project_channels => {
                        block.input[frame * project_channels + channel]
                    }
                    _ => 0.0,
                };
            }
        }
    }

    fn copy_output(&mut self, block: &mut Block<'_>) {
        let frames = block.frames;
        let project_channels = block.channels;
        let plugin_channels = self.plugin_output_channels;
        for frame in 0..frames {
            for channel in 0..project_channels {
                let sample = match (plugin_channels, project_channels) {
                    (1, _) => self.buffers.outputs[0][frame],
                    (2, 1) => {
                        (self.buffers.outputs[0][frame] + self.buffers.outputs[1][frame]) * 0.5
                    }
                    (_, _) if channel < plugin_channels => self.buffers.outputs[channel][frame],
                    _ => self.buffers.outputs[0][frame],
                };
                block.output[frame * project_channels + channel] = sample;
            }
        }
    }
}

impl DeviceProcessor for Vst3Processor {
    fn latency_frames(&self) -> u32 {
        self.plugin
            .lock()
            .map(|plugin| plugin.latency_samples())
            .unwrap_or(0)
    }

    fn parameter_defaults(&self) -> Vec<(ParamId, ParamValue)> {
        self.params
            .iter()
            .map(|parameter| {
                (
                    parameter.address.param.clone(),
                    ParamValue::Float(parameter.default),
                )
            })
            .collect()
    }

    fn process(&mut self, block: &mut Block<'_>) {
        self.bind_node(block.node);
        if self.failed {
            block.output.fill(0.0);
            return;
        }
        let frames = block.frames;
        if frames == 0 {
            block.output.fill(0.0);
            return;
        }
        let plugin_handle = Arc::clone(&self.plugin);
        let Ok(mut plugin) = plugin_handle.lock() else {
            block.output.fill(0.0);
            self.failed = true;
            return;
        };

        self.sync_parameters(block, &mut plugin);
        self.send_events(block, &mut plugin);
        let seconds = block.start_frame as f64 / f64::from(block.sample_rate.max(1));
        let _ = plugin.set_transport_position(vst3_host::TransportPosition {
            samples: i64::try_from(block.start_frame).unwrap_or(i64::MAX),
            quarter_notes: seconds * self.tempo_bpm / 60.0,
        });
        self.copy_input(block);
        for output in &mut self.buffers.outputs {
            output.clear();
            output.resize(frames, 0.0);
        }

        let result = catch_unwind(AssertUnwindSafe(|| plugin.process_audio(&mut self.buffers)));
        match result {
            Ok(Ok(())) => {
                self.copy_output(block);
                // `process_audio` was called with shorter output slices; restore
                // the configured capacity for the next block without allocating.
                self.restore_capacity();
            }
            Ok(Err(_)) | Err(_) => {
                // The audio thread never logs or allocates here. A failed plugin
                // stays silent until the next rebuild gives it a fresh instance.
                block.output.fill(0.0);
                self.failed = true;
            }
        }
    }
}

impl Vst3Processor {
    fn restore_capacity(&mut self) {
        let block_size = self.buffers.block_size;
        for buffer in self
            .buffers
            .inputs
            .iter_mut()
            .chain(&mut self.buffers.outputs)
        {
            buffer.resize(block_size, 0.0);
        }
    }
}

impl Drop for Vst3Processor {
    fn drop(&mut self) {
        if let Ok(mut plugin) = self.plugin.lock() {
            let _ = plugin.stop_processing();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use odaw_engine::ParamBuffer;

    #[test]
    fn a_missing_plugin_becomes_a_pass_through() {
        let backend = Vst3Backend::new(Catalog::default());
        let node = Node::device(NodeKind::new("vst3:00000000000000000000000000000000"));
        let context = DeviceContext::new(NodeId::from_raw(4), 48_000, 16, 2, 120.0);
        let mut processor = backend.factory().build(&node, &context).unwrap();
        let params = ParamBuffer::new();
        let input = vec![0.25; 32];
        let mut output = vec![0.0; 32];
        let mut block = Block {
            node: NodeId::from_raw(4),
            event_target: None,
            sample_rate: 48_000,
            channels: 2,
            start_frame: 0,
            frames: 16,
            input: &input,
            output: &mut output,
            params: &params,
            events: &[],
        };
        processor.process(&mut block);
        assert_eq!(output, input);
    }
}
