use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::sync::Arc;

use odaw_core::{
    ChannelLayout, ClipContent, EdgeId, Node, NodeId, NodeKind, NodeRole, ParamAddress, ParamId,
    ParamValue, Port, PortDirection, PortId, Project, TempoError, Tick, ValidationError,
};
use odaw_dsp::{
    AudioClipPlayer, AudioSources, ChannelStrip, Delay, NodeRegistry, PlayableClip,
    STRIP_GAIN_PARAM, STRIP_PAN_PARAM, Silence,
};
use odaw_engine::{
    BufferId, CompiledProject, DeviceContext, DeviceFactory, DeviceProcessor, MeterBank,
    ParamBuffer, PlanStep, RenderPlan,
};
use thiserror::Error;

type UnitId = usize;

struct Unit {
    node: NodeId,
    event_target: Option<NodeId>,
    processor: Option<Box<dyn DeviceProcessor>>,
    dependencies: Vec<UnitId>,
}

pub struct Compiler {
    registry: NodeRegistry,
    factories: Vec<Box<dyn DeviceFactory>>,
    max_block_size: usize,
}

impl Compiler {
    pub fn new(registry: NodeRegistry) -> Self {
        Self {
            registry,
            factories: Vec::new(),
            max_block_size: 8192,
        }
    }

    pub fn builtin() -> Self {
        Self::new(NodeRegistry::builtin())
    }

    /// Add a device backend. Factories are consulted in registration order, then
    /// the compiler falls back to built-in devices.
    pub fn with_factory(mut self, factory: impl DeviceFactory + 'static) -> Self {
        self.factories.push(Box::new(factory));
        self
    }

    /// Maximum frames the host engine may ask a plugin to process in one block.
    pub fn with_max_block_size(mut self, frames: usize) -> Self {
        self.max_block_size = frames.max(1);
        self
    }

    pub fn compile(&self, project: &Project) -> Result<CompiledProject, CompileError> {
        self.compile_with_audio(project, &AudioSources::default())
    }

    pub fn compile_with_audio(
        &self,
        project: &Project,
        audio_sources: &AudioSources,
    ) -> Result<CompiledProject, CompileError> {
        self.compile_project(project, audio_sources)
    }

    fn compile_project(
        &self,
        project: &Project,
        audio_sources: &AudioSources,
    ) -> Result<CompiledProject, CompileError> {
        project.validate()?;

        let masters: Vec<NodeId> = project
            .nodes
            .iter()
            .filter(|(_, node)| node.role == NodeRole::Master)
            .map(|(id, _)| *id)
            .collect();
        if masters.len() != 1 {
            return Err(CompileError::MasterCount(masters.len()));
        }
        let master = masters[0];
        let master_node = project
            .nodes
            .get(&master)
            .ok_or(CompileError::MissingNode(master))?;
        let master_layout = master_layout(master, master_node)?;
        let channels = usize::from(master_layout.channels());
        let tempo_bpm = project.tempo.points().next().map_or(120.0, |point| {
            60_000_000.0 / f64::from(point.micros_per_quarter.max(1))
        });
        if !matches!(channels, 1 | 2) {
            return Err(CompileError::UnsupportedChannels(master_layout.channels()));
        }

        for (node_id, node) in &project.nodes {
            match node.role {
                NodeRole::Track
                | NodeRole::Bus
                | NodeRole::Folder
                | NodeRole::Master
                | NodeRole::Device => {}
                role => {
                    return Err(CompileError::UnsupportedRole {
                        node: *node_id,
                        role,
                    });
                }
            }
        }

        let any_solo = project
            .nodes
            .values()
            .any(|node| node.role == NodeRole::Track && node.soloed);
        let mut owners = Vec::new();
        let mut rack_owner = BTreeMap::new();
        for (node_id, node) in &project.nodes {
            if matches!(
                node.role,
                NodeRole::Track | NodeRole::Bus | NodeRole::Folder | NodeRole::Master
            ) {
                owners.push(*node_id);
                for device in &node.rack {
                    if rack_owner.insert(*device, *node_id).is_some() {
                        return Err(CompileError::DeviceInMultipleRacks(*device));
                    }
                }
            }
        }

        for (node_id, node) in &project.nodes {
            if node.role == NodeRole::Device && !rack_owner.contains_key(node_id) {
                return Err(CompileError::DeviceNotInRack(*node_id));
            }
        }

        for owner in &owners {
            let node = project
                .nodes
                .get(owner)
                .ok_or(CompileError::MissingNode(*owner))?;
            check_ports(*owner, node, channels)?;
        }

        let meters = MeterBank::new(owners.iter().copied());
        let mut units: Vec<Unit> = Vec::new();
        let mut owner_output: BTreeMap<NodeId, UnitId> = BTreeMap::new();
        let mut owner_input: BTreeMap<NodeId, UnitId> = BTreeMap::new();

        for owner in &owners {
            let node = project
                .nodes
                .get(owner)
                .ok_or(CompileError::MissingNode(*owner))?;

            let silenced =
                node.role == NodeRole::Track && (node.muted || (any_solo && !node.soloed));
            if silenced {
                let unit = units.len();
                units.push(Unit {
                    node: *owner,
                    event_target: None,
                    processor: Some(Box::new(Silence::new())),
                    dependencies: Vec::new(),
                });
                owner_output.insert(*owner, unit);
                owner_input.insert(*owner, unit);
                continue;
            }

            let mut first_unit = None;
            let mut previous_unit = None;
            let mut first_has_input = false;

            for (index, device) in node.rack.iter().enumerate() {
                let device_node = project
                    .nodes
                    .get(device)
                    .ok_or(CompileError::MissingNode(*device))?;
                let kind = device_node
                    .kind
                    .as_ref()
                    .ok_or(CompileError::MissingKind(*device))?;
                let context = DeviceContext::new(
                    *device,
                    project.meta.sample_rate,
                    self.max_block_size,
                    channels,
                    tempo_bpm,
                );
                let mut backend_error = None;
                let mut processor = None;
                for factory in &self.factories {
                    if factory.descriptor(kind).is_none() {
                        continue;
                    }
                    match factory.build(device_node, &context) {
                        Ok(built) => {
                            processor = Some(built);
                            break;
                        }
                        Err(error) => {
                            backend_error = Some(error);
                            break;
                        }
                    }
                }
                let processor = match processor {
                    Some(processor) => processor,
                    None => self.registry.build(kind, *device).ok_or_else(|| {
                        backend_error.map_or_else(
                            || CompileError::UnknownDevice {
                                node: *device,
                                kind: kind.clone(),
                            },
                            |error| CompileError::Device {
                                node: *device,
                                message: error.to_string(),
                            },
                        )
                    })?,
                };

                let input = single_port(*device, device_node, PortDirection::Input)?;
                let output = single_port(*device, device_node, PortDirection::Output)?
                    .ok_or(CompileError::MissingOutputPort(*device))?;
                if let Some(port) = input {
                    check_channel(*device, port, channels)?;
                }
                check_channel(*device, output, channels)?;

                if index == 0 {
                    first_has_input = input.is_some();
                } else if input.is_none() {
                    return Err(CompileError::SourceNotFirst(*device));
                }

                let unit_id = units.len();
                units.push(Unit {
                    node: *device,
                    event_target: (node.role == NodeRole::Track).then_some(*owner),
                    processor: Some(processor),
                    dependencies: previous_unit.into_iter().collect(),
                });
                first_unit.get_or_insert(unit_id);
                previous_unit = Some(unit_id);
            }

            // Audio clips are summed at the track input, before the channel
            // strip, so the track fader and meter see both MIDI and audio.
            let mut clip_units = Vec::new();
            if node.role == NodeRole::Track {
                let mut clips = Vec::new();
                for clip_id in &node.clips {
                    let Some(clip) = project.clips.get(clip_id) else {
                        continue;
                    };
                    let ClipContent::Audio {
                        asset,
                        source_start,
                        source_length,
                    } = &clip.content
                    else {
                        continue;
                    };
                    let Some(audio) = audio_sources.get(asset) else {
                        continue;
                    };
                    let start_frame = project.tempo.tick_to_frames(clip.start)?.raw().max(0) as u64;
                    let end_tick =
                        Tick::from_raw(clip.start.raw().saturating_add(clip.length.raw()));
                    let end_frame = project.tempo.tick_to_frames(end_tick)?.raw().max(0) as u64;
                    if end_frame <= start_frame {
                        continue;
                    }
                    clips.push(PlayableClip::new(
                        Arc::clone(audio),
                        start_frame,
                        end_frame,
                        source_start.raw().max(0) as u64,
                        source_length.raw().max(0) as u64,
                    ));
                }
                if !clips.is_empty() {
                    let unit = units.len();
                    units.push(Unit {
                        node: *owner,
                        event_target: None,
                        processor: Some(Box::new(AudioClipPlayer::new(
                            project.meta.sample_rate,
                            clips,
                        ))),
                        dependencies: Vec::new(),
                    });
                    clip_units.push(unit);
                }
            }

            // Every track, bus, folder, and the master ends in a channel strip.
            // Keeping it in the plan (rather than as a document device) means
            // gain and pan stay value-only parameters and never rebuild audio.
            let strip_unit = units.len();
            let strip = meters.slot(*owner).map_or_else(
                || ChannelStrip::new(*owner),
                |meter| ChannelStrip::new(*owner).with_meter(meter),
            );
            let mut dependencies: Vec<UnitId> = previous_unit.into_iter().collect();
            dependencies.extend(clip_units);
            units.push(Unit {
                node: *owner,
                event_target: None,
                processor: Some(Box::new(strip)),
                dependencies,
            });
            owner_output.insert(*owner, strip_unit);
            if node.rack.is_empty() {
                owner_input.insert(*owner, strip_unit);
            } else if first_has_input {
                owner_input.insert(
                    *owner,
                    first_unit.expect("a non-empty rack has a first unit"),
                );
            }
        }

        for (edge_id, edge) in &project.edges {
            let from_node = project
                .nodes
                .get(&edge.from.node)
                .ok_or(CompileError::MissingNode(edge.from.node))?;
            let to_node = project
                .nodes
                .get(&edge.to.node)
                .ok_or(CompileError::MissingNode(edge.to.node))?;
            if from_node.role == NodeRole::Device || to_node.role == NodeRole::Device {
                return Err(CompileError::DeviceEdge(*edge_id));
            }

            let from_port = single_port(edge.from.node, from_node, PortDirection::Output)?
                .ok_or(CompileError::MissingOutputPort(edge.from.node))?;
            let to_port = single_port(edge.to.node, to_node, PortDirection::Input)?
                .ok_or(CompileError::MissingInputPort(edge.to.node))?;
            if from_port.id != edge.from.port {
                return Err(CompileError::UnsupportedPort {
                    edge: *edge_id,
                    node: edge.from.node,
                    port: edge.from.port,
                });
            }
            if to_port.id != edge.to.port {
                return Err(CompileError::UnsupportedPort {
                    edge: *edge_id,
                    node: edge.to.node,
                    port: edge.to.port,
                });
            }
            check_channel(edge.from.node, from_port, channels)?;
            check_channel(edge.to.node, to_port, channels)?;

            let from_unit = owner_output
                .get(&edge.from.node)
                .copied()
                .ok_or(CompileError::MissingOutputPort(edge.from.node))?;
            let to_unit = owner_input
                .get(&edge.to.node)
                .copied()
                .ok_or(CompileError::InputIntoSource(edge.to.node))?;
            units[to_unit].dependencies.push(from_unit);
        }

        compensate_latency(&mut units, channels)?;

        let order = topological_order(&units)?;
        let mut unit_buffer = vec![0usize; units.len()];
        for (position, unit) in order.iter().copied().enumerate() {
            unit_buffer[unit] = position;
        }

        let mut steps = Vec::with_capacity(units.len());
        for (position, unit_id) in order.iter().copied().enumerate() {
            let unit = &mut units[unit_id];
            let inputs: Vec<BufferId> = unit
                .dependencies
                .iter()
                .map(|dependency| unit_buffer[*dependency])
                .collect();
            let processor = unit
                .processor
                .take()
                .expect("every unit keeps its processor until the plan is built");
            steps.push(PlanStep {
                node: unit.node,
                inputs,
                output: position,
                event_target: unit.event_target,
                processor,
            });
        }

        let master_unit = owner_output
            .get(&master)
            .copied()
            .expect("the master owner always has an output unit");
        let master_buffer = unit_buffer[master_unit];

        let mut params = ParamBuffer::new();
        for (node_id, node) in &project.nodes {
            for (param, value) in &node.params {
                params.ensure(
                    ParamAddress {
                        node: *node_id,
                        param: param.clone(),
                    },
                    *value,
                );
            }
        }
        // Devices report their parameters after construction (plugins discover
        // them at load time), so every slot a live device may touch exists even
        // when the document never stored a value for it. Document values win
        // because they were inserted first.
        for step in &steps {
            for (param, value) in step.processor.parameter_defaults() {
                params.ensure(
                    ParamAddress {
                        node: step.node,
                        param,
                    },
                    value,
                );
            }
        }
        // Reserve channel-strip slots for every mixer owner so live fader
        // gestures always have somewhere to land.
        for owner in &owners {
            for param in [STRIP_GAIN_PARAM, STRIP_PAN_PARAM] {
                params.ensure(
                    ParamAddress {
                        node: *owner,
                        param: ParamId::new(param),
                    },
                    ParamValue::Float(0.0),
                );
            }
        }

        Ok(CompiledProject {
            plan: RenderPlan {
                sample_rate: project.meta.sample_rate,
                channels,
                steps,
                master: Some(master_buffer),
            },
            params,
            meters,
        })
    }
}

fn master_layout(node_id: NodeId, node: &Node) -> Result<ChannelLayout, CompileError> {
    if let Some(port) = single_port(node_id, node, PortDirection::Input)? {
        return Ok(port.layout);
    }
    if let Some(port) = single_port(node_id, node, PortDirection::Output)? {
        return Ok(port.layout);
    }
    Err(CompileError::MasterWithoutPort(node_id))
}

fn single_port(
    node_id: NodeId,
    node: &Node,
    direction: PortDirection,
) -> Result<Option<&Port>, CompileError> {
    let mut ports = node.ports.iter().filter(|port| port.direction == direction);
    let first = ports.next();
    if ports.next().is_some() {
        return Err(CompileError::AmbiguousPorts {
            node: node_id,
            direction,
        });
    }
    Ok(first)
}

fn check_ports(node_id: NodeId, node: &Node, channels: usize) -> Result<(), CompileError> {
    if let Some(port) = single_port(node_id, node, PortDirection::Input)? {
        check_channel(node_id, port, channels)?;
    }
    if let Some(port) = single_port(node_id, node, PortDirection::Output)? {
        check_channel(node_id, port, channels)?;
    }
    Ok(())
}

fn check_channel(node_id: NodeId, port: &Port, channels: usize) -> Result<(), CompileError> {
    let actual = usize::from(port.layout.channels());
    if actual != channels {
        return Err(CompileError::ChannelMismatch {
            node: node_id,
            expected: channels as u16,
            actual: port.layout.channels(),
        });
    }
    Ok(())
}

/// Insert delays on parallel branches so every input to a unit arrives with the
/// same latency. The master's own accumulated latency becomes the session's
/// overall output delay, which the transport does not try to hide.
fn compensate_latency(units: &mut Vec<Unit>, channels: usize) -> Result<(), CompileError> {
    let order = topological_order(units)?;
    let mut latency = vec![0u32; units.len()];
    for unit_id in order {
        let dependencies = units[unit_id].dependencies.clone();
        let mut max_dependency_latency = 0u32;
        for dependency in &dependencies {
            let value = *latency
                .get(*dependency)
                .ok_or(CompileError::MissingDependency)?;
            max_dependency_latency = max_dependency_latency.max(value);
        }

        let mut compensated = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            let difference = max_dependency_latency - latency[dependency];
            if difference == 0 {
                compensated.push(dependency);
                continue;
            }
            let delay_id = units.len();
            units.push(Unit {
                node: units[unit_id].node,
                event_target: None,
                processor: Some(Box::new(Delay::new(
                    usize::try_from(difference).map_err(|_| CompileError::LatencyTooLarge {
                        latency: difference,
                    })?,
                    channels,
                ))),
                dependencies: vec![dependency],
            });
            latency.push(max_dependency_latency);
            compensated.push(delay_id);
        }

        units[unit_id].dependencies = compensated;
        let own = units[unit_id]
            .processor
            .as_ref()
            .map_or(0, |processor| processor.latency_frames());
        latency[unit_id] =
            max_dependency_latency
                .checked_add(own)
                .ok_or(CompileError::LatencyTooLarge {
                    latency: max_dependency_latency,
                })?;
    }
    Ok(())
}

fn topological_order(units: &[Unit]) -> Result<Vec<UnitId>, CompileError> {
    let mut indegree = vec![0usize; units.len()];
    let mut dependents: Vec<Vec<UnitId>> = vec![Vec::new(); units.len()];

    for (unit_id, unit) in units.iter().enumerate() {
        let unique: BTreeSet<UnitId> = unit.dependencies.iter().copied().collect();
        for dependency in unique {
            if dependency >= units.len() {
                return Err(CompileError::MissingDependency);
            }
            dependents[dependency].push(unit_id);
            indegree[unit_id] += 1;
        }
    }

    let mut ready = BinaryHeap::new();
    for (unit_id, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            ready.push(Reverse(unit_id));
        }
    }

    let mut order = Vec::with_capacity(units.len());
    while let Some(Reverse(unit_id)) = ready.pop() {
        order.push(unit_id);
        for dependent in &dependents[unit_id] {
            indegree[*dependent] -= 1;
            if indegree[*dependent] == 0 {
                ready.push(Reverse(*dependent));
            }
        }
    }

    if order.len() != units.len() {
        return Err(CompileError::Cycle);
    }
    Ok(order)
}

#[derive(Debug, Error)]
pub enum CompileError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Tempo(#[from] TempoError),
    #[error("project must contain exactly one master node, found {0}")]
    MasterCount(usize),
    #[error("master node {0} has no audio port")]
    MasterWithoutPort(NodeId),
    #[error("unsupported channel count {0}; only mono and stereo are supported")]
    UnsupportedChannels(u16),
    #[error("node {node} has unsupported role {role:?}")]
    UnsupportedRole { node: NodeId, role: NodeRole },
    #[error("device node {0} is not in a rack")]
    DeviceNotInRack(NodeId),
    #[error("device node {0} appears in more than one rack")]
    DeviceInMultipleRacks(NodeId),
    #[error("node {0} does not exist")]
    MissingNode(NodeId),
    #[error("device node {0} has no kind")]
    MissingKind(NodeId),
    #[error("node {node} references unknown device kind {kind}")]
    UnknownDevice { node: NodeId, kind: NodeKind },
    #[error("node {node} could not be built: {message}")]
    Device { node: NodeId, message: String },
    #[error("device latency {latency} frames is too large to compensate")]
    LatencyTooLarge { latency: u32 },
    #[error("node {node} has more than one {direction:?} port")]
    AmbiguousPorts {
        node: NodeId,
        direction: PortDirection,
    },
    #[error("node {0} has no output port")]
    MissingOutputPort(NodeId),
    #[error("node {0} has no input port")]
    MissingInputPort(NodeId),
    #[error("node {node} has {actual} channels but the master has {expected}")]
    ChannelMismatch {
        node: NodeId,
        expected: u16,
        actual: u16,
    },
    #[error("device {0} is a source but is not first in its rack")]
    SourceNotFirst(NodeId),
    #[error("node {0} receives an edge but its first device is a source")]
    InputIntoSource(NodeId),
    #[error("edge {0} connects a device outside a rack")]
    DeviceEdge(EdgeId),
    #[error("edge {edge} uses unsupported port {port} on node {node}")]
    UnsupportedPort {
        edge: EdgeId,
        node: NodeId,
        port: PortId,
    },
    #[error("the signal graph contains a cycle")]
    Cycle,
    #[error("a unit depends on a unit that does not exist")]
    MissingDependency,
}

#[cfg(test)]
mod tests {
    use super::*;
    use odaw_core::{
        ChannelLayout, Edge, EdgeId, Node, NodeId, NodeKind, NodePort, NodeRole, ParamId,
        ParamValue, Port, PortId, Project,
    };
    use odaw_dsp::demo;
    use odaw_engine::{Block, DeviceContext, DeviceFactory};

    #[derive(Debug, Default)]
    struct LatencyPass {
        previous: Vec<f32>,
    }

    impl DeviceProcessor for LatencyPass {
        fn latency_frames(&self) -> u32 {
            1
        }

        fn process(&mut self, block: &mut Block<'_>) {
            self.previous.resize(block.channels, 0.0);
            for frame in 0..block.frames {
                for channel in 0..block.channels {
                    let index = frame * block.channels + channel;
                    block.output[index] = self.previous[channel];
                    self.previous[channel] = block.input[index];
                }
            }
        }
    }

    struct LatencyFactory;

    impl DeviceFactory for LatencyFactory {
        fn descriptor(&self, kind: &NodeKind) -> Option<odaw_engine::DeviceDescriptor> {
            (kind.as_str() == "test.latency").then(|| odaw_engine::DeviceDescriptor {
                kind: kind.clone(),
                label: "Latency".to_owned(),
                vendor: "test".to_owned(),
                instrument: false,
                parameters: Vec::new(),
            })
        }

        fn build(
            &self,
            _node: &Node,
            _context: &DeviceContext,
        ) -> Result<Box<dyn DeviceProcessor>, odaw_engine::DeviceError> {
            Ok(Box::new(LatencyPass::default()))
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Effect {
        Bias,
        Double,
    }

    #[derive(Debug)]
    struct TestEffect(Effect);

    impl DeviceProcessor for TestEffect {
        fn process(&mut self, block: &mut Block<'_>) {
            for (destination, input) in block.output.iter_mut().zip(block.input) {
                *destination = match self.0 {
                    Effect::Bias => *input + 0.5,
                    Effect::Double => *input * 2.0,
                };
            }
        }
    }

    struct ChainFactory;

    impl DeviceFactory for ChainFactory {
        fn descriptor(&self, kind: &NodeKind) -> Option<odaw_engine::DeviceDescriptor> {
            let effect = match kind.as_str() {
                "test.bias" => Effect::Bias,
                "test.double" => Effect::Double,
                _ => return None,
            };
            Some(odaw_engine::DeviceDescriptor {
                kind: kind.clone(),
                label: format!("{effect:?}"),
                vendor: "test".to_owned(),
                instrument: false,
                parameters: Vec::new(),
            })
        }

        fn build(
            &self,
            node: &Node,
            context: &DeviceContext,
        ) -> Result<Box<dyn DeviceProcessor>, odaw_engine::DeviceError> {
            let effect = match node.kind.as_ref().map(NodeKind::as_str) {
                Some("test.bias") => Effect::Bias,
                Some("test.double") => Effect::Double,
                _ => {
                    return Err(odaw_engine::DeviceError::new(
                        context.node,
                        "unknown test effect",
                    ));
                }
            };
            Ok(Box::new(TestEffect(effect)))
        }
    }

    fn compile(project: &Project) -> Result<CompiledProject, CompileError> {
        Compiler::builtin().compile(project)
    }

    fn master(id: u64) -> (NodeId, Node) {
        (
            NodeId::from_raw(id),
            Node::new(NodeRole::Master).with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ]),
        )
    }

    fn track(id: u64, rack: &[u64]) -> (NodeId, Node) {
        (
            NodeId::from_raw(id),
            Node::new(NodeRole::Track)
                .with_ports([
                    Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                    Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                ])
                .with_rack(rack.iter().copied().map(NodeId::from_raw)),
        )
    }

    fn device(id: u64, kind: &str, mono: bool) -> (NodeId, Node) {
        let layout = if mono {
            ChannelLayout::Mono
        } else {
            ChannelLayout::Stereo
        };
        (
            NodeId::from_raw(id),
            Node::device(NodeKind::new(kind)).with_ports([
                Port::input(PortId::from_raw(0), layout),
                Port::output(PortId::from_raw(1), layout),
            ]),
        )
    }

    fn connect(project: &mut Project, edge: u64, from: NodeId, to: NodeId) {
        project.edges.insert(
            EdgeId::from_raw(edge),
            Edge {
                from: NodePort {
                    node: from,
                    port: PortId::from_raw(1),
                },
                to: NodePort {
                    node: to,
                    port: PortId::from_raw(0),
                },
            },
        );
    }

    #[test]
    fn demo_project_compiles_in_deterministic_order() {
        let compiled = compile(&demo::project()).unwrap();
        assert_eq!(compiled.plan.sample_rate, 48_000);
        assert_eq!(compiled.plan.channels, 2);
        assert_eq!(compiled.plan.steps.len(), 4);
        assert_eq!(
            compiled
                .plan
                .steps
                .iter()
                .map(|step| step.node.raw())
                .collect::<Vec<_>>(),
            vec![2, 3, 1, 4]
        );
        assert_eq!(compiled.plan.master, Some(3));
        assert_eq!(compiled.params.len(), 6);
        assert_eq!(
            compiled.params.get(&ParamAddress {
                node: NodeId::from_raw(2),
                param: ParamId::new("gain"),
            }),
            Some(ParamValue::Float(0.25))
        );
        assert_eq!(
            compiled.params.get(&ParamAddress {
                node: NodeId::from_raw(1),
                param: ParamId::new(STRIP_GAIN_PARAM),
            }),
            Some(ParamValue::Float(0.0))
        );
        assert_eq!(
            compiled
                .plan
                .steps
                .iter()
                .map(|step| step.event_target)
                .collect::<Vec<_>>(),
            vec![
                Some(NodeId::from_raw(1)),
                Some(NodeId::from_raw(1)),
                None,
                None
            ]
        );
    }

    #[test]
    fn every_owner_gets_a_channel_strip() {
        let compiled = compile(&demo::project()).unwrap();
        let master = compiled.plan.master.unwrap();
        let master_step = &compiled.plan.steps[master];
        assert_eq!(master_step.node, NodeId::from_raw(4));
        assert!(master_step.inputs.contains(&2));
        assert_eq!(
            compiled.params.get(&ParamAddress {
                node: NodeId::from_raw(4),
                param: ParamId::new(STRIP_PAN_PARAM),
            }),
            Some(ParamValue::Float(0.0))
        );
    }

    #[test]
    fn every_owner_gets_a_stereo_meter() {
        let compiled = compile(&demo::project()).unwrap();
        assert_eq!(compiled.meters.len(), 2);
        assert!(compiled.meters.slot(NodeId::from_raw(1)).is_some());
        assert!(compiled.meters.slot(NodeId::from_raw(4)).is_some());
        assert!(compiled.meters.slot(NodeId::from_raw(2)).is_none());
    }

    #[test]
    fn audio_clips_are_mixed_into_their_track() {
        use std::num::NonZeroUsize;

        use odaw_core::{Asset, AssetId, AssetKind, Clip, ClipContent, ClipId, Frames};
        use odaw_dsp::AudioAsset;
        use odaw_engine::Engine;

        let mut project = Project::empty("audio", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);
        let track_id = NodeId::from_raw(2);
        let clip_id = ClipId::from_raw(3);
        project.nodes.insert(
            track_id,
            Node::new(NodeRole::Track)
                .with_ports([
                    Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                    Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                ])
                .with_clips([clip_id]),
        );
        let asset_id = AssetId::from_bytes([7; 32]);
        project.assets.insert(
            asset_id,
            Asset {
                name: "tone.wav".into(),
                kind: AssetKind::Audio,
            },
        );
        project.clips.insert(
            clip_id,
            Clip {
                node: track_id,
                start: Tick::ZERO,
                length: Tick::from_raw(4 * odaw_core::PPQ),
                content: ClipContent::Audio {
                    asset: asset_id,
                    source_start: Frames::ZERO,
                    source_length: Frames::from_raw(2_000),
                },
            },
        );
        connect(&mut project, 4, track_id, master_id);

        let mut sources = AudioSources::new();
        sources.insert(
            asset_id,
            Arc::new(AudioAsset::from_samples(48_000, 2, vec![0.5; 2 * 2_000]).unwrap()),
        );
        let compiled = Compiler::builtin()
            .compile_with_audio(&project, &sources)
            .unwrap();
        assert_eq!(compiled.plan.steps.len(), 3);

        let mut engine = Engine::new(compiled, NonZeroUsize::new(8).unwrap()).unwrap();
        let mut output = vec![0.0; 8];
        engine.process_block(4, &[], &mut output).unwrap();
        assert_eq!(output, vec![0.5; 8]);
    }

    #[test]
    fn compilation_is_deterministic() {
        let first = compile(&demo::project()).unwrap();
        let second = compile(&demo::project()).unwrap();
        assert_eq!(
            first
                .plan
                .steps
                .iter()
                .map(|step| (step.node, step.inputs.clone(), step.output))
                .collect::<Vec<_>>(),
            second
                .plan
                .steps
                .iter()
                .map(|step| (step.node, step.inputs.clone(), step.output))
                .collect::<Vec<_>>()
        );
        assert_eq!(first.plan.master, second.plan.master);
    }

    #[test]
    fn missing_master_is_rejected() {
        let mut project = Project::empty("empty", 48_000, 0).unwrap();
        project
            .nodes
            .insert(NodeId::from_raw(1), Node::new(NodeRole::Track));
        assert!(matches!(
            compile(&project),
            Err(CompileError::MasterCount(0))
        ));
    }

    #[test]
    fn multiple_masters_are_rejected() {
        let mut project = Project::empty("two", 48_000, 0).unwrap();
        let (first_id, first) = master(1);
        let (second_id, second) = master(2);
        project.nodes.insert(first_id, first);
        project.nodes.insert(second_id, second);
        assert!(matches!(
            compile(&project),
            Err(CompileError::MasterCount(2))
        ));
    }

    #[test]
    fn devices_outside_a_rack_are_rejected() {
        let mut project = Project::empty("loose", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);
        let (device_id, device_node) = device(2, "builtin.gain", false);
        project.nodes.insert(device_id, device_node);
        assert!(matches!(
            compile(&project),
            Err(CompileError::DeviceNotInRack(node)) if node == NodeId::from_raw(2)
        ));
    }

    #[test]
    fn unknown_devices_are_rejected() {
        let mut project = Project::empty("unknown", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);
        let (track_id, track_node) = track(2, &[3]);
        project.nodes.insert(track_id, track_node);
        let (device_id, device_node) = device(3, "builtin.unknown", false);
        project.nodes.insert(device_id, device_node);
        assert!(matches!(
            compile(&project),
            Err(CompileError::UnknownDevice { node, .. }) if node == NodeId::from_raw(3)
        ));
    }

    #[test]
    fn channel_mismatches_are_rejected() {
        let mut project = Project::empty("mono", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);
        let (track_id, track_node) = track(2, &[3]);
        project.nodes.insert(track_id, track_node);
        let (device_id, device_node) = device(3, "builtin.gain", true);
        project.nodes.insert(device_id, device_node);
        connect(&mut project, 4, track_id, master_id);
        assert!(matches!(
            compile(&project),
            Err(CompileError::ChannelMismatch { node, .. }) if node == NodeId::from_raw(3)
        ));
    }

    #[test]
    fn cycles_are_rejected() {
        let mut project = Project::empty("cycle", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);
        let (first_id, first) = track(2, &[3]);
        project.nodes.insert(first_id, first);
        let (second_id, second) = track(4, &[5]);
        project.nodes.insert(second_id, second);
        let (first_device_id, first_device) = device(3, "builtin.gain", false);
        project.nodes.insert(first_device_id, first_device);
        let (second_device_id, second_device) = device(5, "builtin.gain", false);
        project.nodes.insert(second_device_id, second_device);
        connect(&mut project, 6, first_id, second_id);
        connect(&mut project, 7, second_id, first_id);
        assert!(matches!(compile(&project), Err(CompileError::Cycle)));
    }

    #[test]
    fn input_into_a_source_rack_is_rejected() {
        let mut project = Project::empty("source", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);
        let (source_id, source) = track(2, &[3]);
        project.nodes.insert(source_id, source);
        let (sink_id, sink) = track(4, &[5]);
        project.nodes.insert(sink_id, sink);
        project.nodes.insert(
            NodeId::from_raw(3),
            Node::device(NodeKind::new("builtin.sine"))
                .with_ports([Port::output(PortId::from_raw(1), ChannelLayout::Stereo)]),
        );
        let (gain_id, gain) = device(5, "builtin.gain", false);
        project.nodes.insert(gain_id, gain);
        connect(&mut project, 6, sink_id, source_id);
        assert!(matches!(
            compile(&project),
            Err(CompileError::InputIntoSource(node)) if node == NodeId::from_raw(2)
        ));
    }

    #[test]
    fn sources_must_be_first_in_a_rack() {
        let mut project = Project::empty("order", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);
        let (track_id, track_node) = track(2, &[3, 4]);
        project.nodes.insert(track_id, track_node);
        let (gain_id, gain) = device(3, "builtin.gain", false);
        project.nodes.insert(gain_id, gain);
        project.nodes.insert(
            NodeId::from_raw(4),
            Node::device(NodeKind::new("builtin.sine"))
                .with_ports([Port::output(PortId::from_raw(1), ChannelLayout::Stereo)]),
        );
        assert!(matches!(
            compile(&project),
            Err(CompileError::SourceNotFirst(node)) if node == NodeId::from_raw(4)
        ));
    }

    #[test]
    fn unsupported_roles_are_rejected() {
        let mut project = Project::empty("modulator", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);
        project
            .nodes
            .insert(NodeId::from_raw(2), Node::new(NodeRole::Modulator));
        assert!(matches!(
            compile(&project),
            Err(CompileError::UnsupportedRole { .. })
        ));
    }

    #[test]
    fn effects_stack_in_rack_order() {
        use odaw_engine::Engine;

        let build = |rack: [NodeId; 2]| {
            let mut project = Project::empty("chain", 48_000, 0).unwrap();
            let (master_id, master_node) = master(1);
            project.nodes.insert(master_id, master_node);
            let track_id = NodeId::from_raw(2);
            project.nodes.insert(
                track_id,
                Node::new(NodeRole::Track)
                    .with_ports([
                        Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                        Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                    ])
                    .with_rack([NodeId::from_raw(3), rack[0], rack[1]]),
            );
            project.nodes.insert(
                NodeId::from_raw(3),
                Node::device(NodeKind::new("builtin.sine"))
                    .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
                    .with_param(ParamId::new("frequency"), ParamValue::Float(12_000.0))
                    .with_param(ParamId::new("gain"), ParamValue::Float(1.0)),
            );
            for id in rack {
                let kind = if id == NodeId::from_raw(4) {
                    "test.bias"
                } else {
                    "test.double"
                };
                project.nodes.insert(
                    id,
                    Node::device(NodeKind::new(kind)).with_ports([
                        Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                        Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                    ]),
                );
            }
            connect(&mut project, 6, track_id, master_id);
            project
        };

        let render = |rack: [NodeId; 2]| {
            let compiled = Compiler::builtin()
                .with_factory(ChainFactory)
                .compile(&build(rack))
                .unwrap();
            let mut engine =
                Engine::new(compiled, std::num::NonZeroUsize::new(4).unwrap()).unwrap();
            let mut output = vec![0.0; 8];
            engine.process_block(4, &[], &mut output).unwrap();
            output
        };

        let bias_then_double = render([NodeId::from_raw(4), NodeId::from_raw(5)]);
        let double_then_bias = render([NodeId::from_raw(5), NodeId::from_raw(4)]);
        assert!(
            (bias_then_double[2] - 3.0).abs() < 1e-6,
            "{bias_then_double:?}"
        );
        assert!(
            (double_then_bias[2] - 2.5).abs() < 1e-6,
            "{double_then_bias:?}"
        );
    }

    #[test]
    fn parallel_paths_are_latency_compensated() {
        use odaw_engine::Engine;

        let mut project = Project::empty("latency", 48_000, 0).unwrap();
        let (master_id, master_node) = master(1);
        project.nodes.insert(master_id, master_node);

        let first = NodeId::from_raw(2);
        let first_sine = NodeId::from_raw(3);
        let latency = NodeId::from_raw(4);
        project.nodes.insert(
            first,
            Node::new(NodeRole::Track)
                .with_ports([
                    Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                    Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                ])
                .with_rack([first_sine, latency]),
        );
        project.nodes.insert(
            first_sine,
            Node::device(NodeKind::new("builtin.sine"))
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
                .with_param(ParamId::new("frequency"), ParamValue::Float(12_000.0))
                .with_param(ParamId::new("gain"), ParamValue::Float(1.0)),
        );
        project.nodes.insert(
            latency,
            Node::device(NodeKind::new("test.latency")).with_ports([
                Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
            ]),
        );

        let second = NodeId::from_raw(5);
        let second_sine = NodeId::from_raw(6);
        project.nodes.insert(
            second,
            Node::new(NodeRole::Track)
                .with_ports([
                    Port::input(PortId::from_raw(0), ChannelLayout::Stereo),
                    Port::output(PortId::from_raw(1), ChannelLayout::Stereo),
                ])
                .with_rack([second_sine]),
        );
        project.nodes.insert(
            second_sine,
            Node::device(NodeKind::new("builtin.sine"))
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)])
                .with_param(ParamId::new("frequency"), ParamValue::Float(12_000.0))
                .with_param(ParamId::new("gain"), ParamValue::Float(1.0)),
        );
        connect(&mut project, 7, first, master_id);
        connect(&mut project, 8, second, master_id);

        let compiled = Compiler::builtin()
            .with_factory(LatencyFactory)
            .compile(&project)
            .unwrap();
        let mut engine = Engine::new(compiled, std::num::NonZeroUsize::new(4).unwrap()).unwrap();
        let mut output = vec![0.0; 8];
        engine.process_block(4, &[], &mut output).unwrap();
        assert!((output[4] - 2.0).abs() < 1e-6, "{output:?}");
        assert!((output[5] - 2.0).abs() < 1e-6, "{output:?}");
    }

    #[test]
    fn unsupported_channel_counts_are_rejected() {
        let mut project = Project::empty("wide", 48_000, 0).unwrap();
        project.nodes.insert(
            NodeId::from_raw(1),
            Node::new(NodeRole::Master)
                .with_ports([Port::input(PortId::from_raw(0), ChannelLayout::Channels(3))]),
        );
        assert!(matches!(
            compile(&project),
            Err(CompileError::UnsupportedChannels(3))
        ));
    }
}
