use crate::graph::{NodeKind, Port};
use crate::params::{ParamId, ParamSchema};

#[derive(Clone, Debug, PartialEq)]
pub struct NodeSchema {
    pub kind: NodeKind,
    pub ports: Vec<Port>,
    pub parameters: Vec<ParamSchema>,
}

impl NodeSchema {
    pub fn new(kind: NodeKind, ports: Vec<Port>, parameters: Vec<ParamSchema>) -> Self {
        Self {
            kind,
            ports,
            parameters,
        }
    }

    pub fn parameter(&self, id: &ParamId) -> Option<&ParamSchema> {
        self.parameters.iter().find(|parameter| parameter.id == *id)
    }
}
