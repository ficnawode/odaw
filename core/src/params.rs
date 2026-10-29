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

impl ParamValue {
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Float(value) => value,
            Self::Int(value) => value as f64,
            Self::Bool(value) => f64::from(u8::from(value)),
            Self::Enum(value) => f64::from(value),
        }
    }

    pub fn is_continuous(self) -> bool {
        matches!(self, Self::Float(_) | Self::Int(_))
    }

    /// Interpolate toward `next`. Bool and enum values hold; mixed variants
    /// hold the left value instead of inventing a conversion.
    pub fn interpolate(self, next: Self, ratio: f64) -> Self {
        let ratio = ratio.clamp(0.0, 1.0);
        match (self, next) {
            (Self::Float(left), Self::Float(right)) => Self::Float(left + (right - left) * ratio),
            (Self::Int(left), Self::Int(right)) => {
                let value = left as f64 + (right - left) as f64 * ratio;
                Self::Int(value.round() as i64)
            }
            _ => self,
        }
    }

    /// Add a trim offset. Bool and enum lanes never use trim.
    pub fn offset(self, offset: Self) -> Self {
        match (self, offset) {
            (Self::Float(value), Self::Float(delta)) => Self::Float(value + delta),
            (Self::Int(value), Self::Int(delta)) => Self::Int(value.saturating_add(delta)),
            _ => self,
        }
    }
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
