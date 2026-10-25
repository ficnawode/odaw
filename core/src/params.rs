use std::fmt;

use crate::ids::NodeId;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamId(String);

impl ParamId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ParamId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ParamId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for ParamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Float(f64),
    Int(i64),
    Bool(bool),
    Enum(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Control,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamAddress {
    pub node: NodeId,
    pub param: ParamId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamSchema {
    pub id: ParamId,
    pub label: String,
    pub kind: ParamKind,
    pub default: ParamValue,
    pub structural: bool,
}

impl ParamSchema {
    pub fn new(
        id: ParamId,
        label: impl Into<String>,
        kind: ParamKind,
        default: ParamValue,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            kind,
            default,
            structural: false,
        }
    }

    pub fn structural(mut self) -> Self {
        self.structural = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_schema_is_not_structural_by_default() {
        let schema = ParamSchema::new(
            ParamId::new("gain"),
            "Gain",
            ParamKind::Control,
            ParamValue::Float(1.0),
        );
        assert!(!schema.structural);
    }
}
