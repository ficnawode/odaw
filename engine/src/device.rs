use odaw_core::{Node, NodeId, NodeKind, ParamId, ParamValue};
use thiserror::Error;

use crate::plan::DeviceProcessor;

/// Everything a device factory needs to see about the render plan it is being
/// built into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviceContext {
    pub node: NodeId,
    pub sample_rate: u32,
    pub max_block_size: usize,
    pub channels: usize,
    /// Initial project tempo. Tempo edits rebuild the plan, so devices can
    /// configure tempo once at build time.
    pub tempo_bpm: f64,
}

impl DeviceContext {
    pub const fn new(
        node: NodeId,
        sample_rate: u32,
        max_block_size: usize,
        channels: usize,
        tempo_bpm: f64,
    ) -> Self {
        Self {
            node,
            sample_rate,
            max_block_size,
            channels,
            tempo_bpm,
        }
    }
}

/// One parameter exposed by a device. Values are normalized to `0.0..=1.0`
/// for plugin parameters; built-in devices can report their natural range.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceParameter {
    pub id: ParamId,
    pub label: String,
    pub default: ParamValue,
    pub min: f64,
    pub max: f64,
    pub step_count: u32,
    pub unit: String,
    pub read_only: bool,
}

impl DeviceParameter {
    pub fn normalized(id: impl Into<ParamId>, label: impl Into<String>, default: f64) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            default: ParamValue::Float(default),
            min: 0.0,
            max: 1.0,
            step_count: 0,
            unit: String::new(),
            read_only: false,
        }
    }

    pub fn clamp(&self, value: f64) -> f64 {
        if value.is_finite() {
            value.clamp(self.min, self.max)
        } else {
            self.min
        }
    }

    pub fn format(&self, value: f64) -> String {
        let value = self.clamp(value);
        if self.step_count == 1 {
            return if value >= 0.5 { "On" } else { "Off" }.to_owned();
        }
        if self.step_count > 1 {
            let steps = f64::from(self.step_count) + 1.0;
            let index = ((value - self.min) / (self.max - self.min) * steps) as u32;
            let index = index.min(self.step_count);
            return if self.unit.is_empty() {
                index.to_string()
            } else {
                format!("{index} {}", self.unit)
            };
        }
        if self.unit.is_empty() {
            format!("{value:.3}")
        } else {
            format!("{value:.3} {}", self.unit)
        }
    }
}

/// Metadata a client can use before (or without) instantiating a device.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceDescriptor {
    pub kind: NodeKind,
    pub label: String,
    pub vendor: String,
    pub instrument: bool,
    pub parameters: Vec<DeviceParameter>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("could not build device {node}: {message}")]
pub struct DeviceError {
    pub node: odaw_core::NodeId,
    pub message: String,
}

impl DeviceError {
    pub fn new(node: odaw_core::NodeId, message: impl Into<String>) -> Self {
        Self {
            node,
            message: message.into(),
        }
    }
}

/// Binds a family of `NodeKind`s to concrete `DeviceProcessor`s.
///
/// The compiler consults factories in registration order before falling back to
/// the built-in `NodeRegistry`. Factories own their non-realtime resources
/// (plugin catalogues, loaded modules, caches) and must be `Send + Sync`.
pub trait DeviceFactory: Send + Sync {
    fn descriptor(&self, kind: &NodeKind) -> Option<DeviceDescriptor>;

    fn build(
        &self,
        node: &Node,
        context: &DeviceContext,
    ) -> Result<Box<dyn DeviceProcessor>, DeviceError>;
}
