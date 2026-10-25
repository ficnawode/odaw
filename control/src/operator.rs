use std::collections::BTreeMap;
use std::fmt;

use odaw_core::{NodeId, ParamId, ParamValue, Project};
use thiserror::Error;

use crate::command::{Command, CommandKind, MetaCommand, ParamChange, ParamCommand};
use crate::ids::IdGenerator;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperatorId(String);

impl OperatorId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for OperatorId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for OperatorId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for OperatorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperatorRef {
    pub id: OperatorId,
    pub version: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetRef {
    Explicit(NodeId),
}

impl TargetRef {
    pub fn resolve(self, project: &Project) -> Option<NodeId> {
        match self {
            Self::Explicit(node) => project.nodes.contains_key(&node).then_some(node),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum OperatorParams {
    None,
    RenameProject {
        name: String,
    },
    SetParam {
        target: TargetRef,
        param: ParamId,
        value: ParamValue,
        change: ParamChange,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct OperatorInvocation {
    pub id: OperatorId,
    pub version: u16,
    pub params: OperatorParams,
}

impl OperatorInvocation {
    pub fn reference(&self) -> OperatorRef {
        OperatorRef {
            id: self.id.clone(),
            version: self.version,
        }
    }
}

pub trait Operator: fmt::Debug {
    fn invocation(&self) -> OperatorInvocation;
    fn label(&self) -> String;
    fn enabled(&self, project: &Project) -> bool;
    fn run(&self, project: &Project, ids: &mut IdGenerator) -> Result<Vec<Command>, OperatorError>;
}

#[derive(Clone, Debug)]
pub struct RenameProject {
    pub name: String,
}

impl RenameProject {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Operator for RenameProject {
    fn invocation(&self) -> OperatorInvocation {
        OperatorInvocation {
            id: OperatorId::new("builtin.rename-project"),
            version: 1,
            params: OperatorParams::RenameProject {
                name: self.name.clone(),
            },
        }
    }

    fn label(&self) -> String {
        "Rename project".to_owned()
    }

    fn enabled(&self, _project: &Project) -> bool {
        true
    }

    fn run(
        &self,
        _project: &Project,
        _ids: &mut IdGenerator,
    ) -> Result<Vec<Command>, OperatorError> {
        Ok(vec![Command::new(CommandKind::Meta(MetaCommand::SetName(
            self.name.clone(),
        )))])
    }
}

#[derive(Clone, Debug)]
pub struct SetParam {
    pub target: TargetRef,
    pub param: ParamId,
    pub value: ParamValue,
    pub change: ParamChange,
}

impl SetParam {
    pub fn new(target: TargetRef, param: ParamId, value: ParamValue) -> Self {
        Self {
            target,
            param,
            value,
            change: ParamChange::Value,
        }
    }

    pub fn structural(target: TargetRef, param: ParamId, value: ParamValue) -> Self {
        Self {
            target,
            param,
            value,
            change: ParamChange::Structural,
        }
    }
}

impl Operator for SetParam {
    fn invocation(&self) -> OperatorInvocation {
        OperatorInvocation {
            id: OperatorId::new("builtin.set-param"),
            version: 1,
            params: OperatorParams::SetParam {
                target: self.target,
                param: self.param.clone(),
                value: self.value,
                change: self.change,
            },
        }
    }

    fn label(&self) -> String {
        format!("Set {}", self.param)
    }

    fn enabled(&self, project: &Project) -> bool {
        self.target.resolve(project).is_some()
    }

    fn run(
        &self,
        project: &Project,
        _ids: &mut IdGenerator,
    ) -> Result<Vec<Command>, OperatorError> {
        let node = self
            .target
            .resolve(project)
            .ok_or(OperatorError::UnresolvedTarget(self.target))?;
        Ok(vec![Command::new(CommandKind::Param(ParamCommand::Set {
            address: odaw_core::ParamAddress {
                node,
                param: self.param.clone(),
            },
            value: self.value,
            change: self.change,
        }))])
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum OperatorError {
    #[error("target {0:?} does not resolve to a node")]
    UnresolvedTarget(TargetRef),
    #[error("unknown operator {id} version {version}")]
    UnknownOperator { id: OperatorId, version: u16 },
    #[error("invalid parameters for operator {operator}")]
    InvalidParams { operator: OperatorId },
}

type Builder = fn(OperatorParams) -> Result<Box<dyn Operator>, OperatorError>;

#[derive(Default)]
pub struct OperatorRegistry {
    builders: BTreeMap<(OperatorId, u16), Builder>,
}

impl OperatorRegistry {
    pub fn new() -> Self {
        Self {
            builders: BTreeMap::new(),
        }
    }

    pub fn builtin() -> Self {
        let mut registry = Self::new();
        registry.register(
            OperatorId::new("builtin.rename-project"),
            1,
            build_rename_project,
        );
        registry.register(OperatorId::new("builtin.set-param"), 1, build_set_param);
        registry
    }

    pub fn register(&mut self, id: OperatorId, version: u16, builder: Builder) {
        self.builders.insert((id, version), builder);
    }

    pub fn build(
        &self,
        invocation: &OperatorInvocation,
    ) -> Result<Box<dyn Operator>, OperatorError> {
        let key = (invocation.id.clone(), invocation.version);
        let builder = self
            .builders
            .get(&key)
            .ok_or_else(|| OperatorError::UnknownOperator {
                id: invocation.id.clone(),
                version: invocation.version,
            })?;
        builder(invocation.params.clone())
    }
}

fn build_rename_project(params: OperatorParams) -> Result<Box<dyn Operator>, OperatorError> {
    match params {
        OperatorParams::RenameProject { name } => Ok(Box::new(RenameProject::new(name))),
        _ => Err(OperatorError::InvalidParams {
            operator: OperatorId::new("builtin.rename-project"),
        }),
    }
}

fn build_set_param(params: OperatorParams) -> Result<Box<dyn Operator>, OperatorError> {
    match params {
        OperatorParams::SetParam {
            target,
            param,
            value,
            change,
        } => Ok(Box::new(SetParam {
            target,
            param,
            value,
            change,
        })),
        _ => Err(OperatorError::InvalidParams {
            operator: OperatorId::new("builtin.set-param"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use odaw_core::{ChannelLayout, Node, NodeRole, ParamAddress, Port, PortId, Project};

    use super::*;

    fn project_with_track() -> Project {
        let mut project = Project::empty("demo", 48_000, 0).unwrap();
        project.nodes.insert(
            NodeId::from_raw(1),
            Node::new(NodeRole::Track)
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)]),
        );
        project
    }

    #[test]
    fn rename_project_produces_one_meta_command() {
        let operator = RenameProject::new("renamed");
        let commands = operator
            .run(
                &Project::empty("demo", 48_000, 0).unwrap(),
                &mut IdGenerator::new(1),
            )
            .unwrap();
        assert_eq!(
            commands,
            vec![Command::new(CommandKind::Meta(MetaCommand::SetName(
                "renamed".into()
            )))]
        );
        assert_eq!(operator.label(), "Rename project");
    }

    #[test]
    fn set_param_requires_an_existing_target() {
        let missing = SetParam::new(
            TargetRef::Explicit(NodeId::from_raw(9)),
            ParamId::new("gain"),
            ParamValue::Float(0.5),
        );
        assert!(!missing.enabled(&project_with_track()));

        let existing = SetParam::new(
            TargetRef::Explicit(NodeId::from_raw(1)),
            ParamId::new("gain"),
            ParamValue::Float(0.5),
        );
        assert!(existing.enabled(&project_with_track()));
    }

    #[test]
    fn set_param_builds_a_param_command() {
        let operator = SetParam::new(
            TargetRef::Explicit(NodeId::from_raw(1)),
            ParamId::new("gain"),
            ParamValue::Float(0.5),
        );
        let commands = operator
            .run(&project_with_track(), &mut IdGenerator::new(1))
            .unwrap();
        assert_eq!(
            commands,
            vec![Command::new(CommandKind::Param(ParamCommand::Set {
                address: ParamAddress {
                    node: NodeId::from_raw(1),
                    param: "gain".into(),
                },
                value: ParamValue::Float(0.5),
                change: ParamChange::Value,
            }))]
        );
    }

    #[test]
    fn builtin_registry_builds_both_operators() {
        let registry = OperatorRegistry::builtin();
        let rename = registry
            .build(&OperatorInvocation {
                id: OperatorId::new("builtin.rename-project"),
                version: 1,
                params: OperatorParams::RenameProject { name: "x".into() },
            })
            .unwrap();
        assert_eq!(rename.label(), "Rename project");

        let set_param = registry
            .build(&OperatorInvocation {
                id: OperatorId::new("builtin.set-param"),
                version: 1,
                params: OperatorParams::SetParam {
                    target: TargetRef::Explicit(NodeId::from_raw(1)),
                    param: "gain".into(),
                    value: ParamValue::Float(0.25),
                    change: ParamChange::Value,
                },
            })
            .unwrap();
        assert_eq!(set_param.label(), "Set gain");
    }

    #[test]
    fn unknown_operator_is_reported() {
        let registry = OperatorRegistry::builtin();
        let result = registry.build(&OperatorInvocation {
            id: OperatorId::new("nope"),
            version: 1,
            params: OperatorParams::None,
        });
        assert!(matches!(result, Err(OperatorError::UnknownOperator { .. })));
    }
}
