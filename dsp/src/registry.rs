use std::collections::BTreeMap;

use odaw_core::{NodeId, NodeKind};
use odaw_engine::DeviceProcessor;

use crate::devices::{GAIN_KIND, Gain, PASS_KIND, PassThrough, SINE_KIND, Sine};

type Builder = fn(NodeId) -> Box<dyn DeviceProcessor>;

#[derive(Default)]
pub struct NodeRegistry {
    builders: BTreeMap<NodeKind, Builder>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self {
            builders: BTreeMap::new(),
        }
    }

    pub fn builtin() -> Self {
        let mut registry = Self::new();
        registry.register(SINE_KIND.into(), |node| Box::new(Sine::new(node)));
        registry.register(GAIN_KIND.into(), |node| Box::new(Gain::new(node)));
        registry.register(PASS_KIND.into(), |_| Box::new(PassThrough::new()));
        registry
    }

    pub fn register(&mut self, kind: NodeKind, builder: Builder) {
        self.builders.insert(kind, builder);
    }

    pub fn build(&self, kind: &NodeKind, node: NodeId) -> Option<Box<dyn DeviceProcessor>> {
        self.builders.get(kind).map(|builder| builder(node))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_builds_known_devices() {
        let registry = NodeRegistry::builtin();
        assert!(
            registry
                .build(&NodeKind::new(SINE_KIND), NodeId::from_raw(1))
                .is_some()
        );
        assert!(
            registry
                .build(&NodeKind::new(GAIN_KIND), NodeId::from_raw(1))
                .is_some()
        );
        assert!(
            registry
                .build(&NodeKind::new(PASS_KIND), NodeId::from_raw(1))
                .is_some()
        );
        assert!(
            registry
                .build(&NodeKind::new("builtin.unknown"), NodeId::from_raw(1))
                .is_none()
        );
    }
}
