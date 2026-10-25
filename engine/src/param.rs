use std::collections::BTreeMap;

use odaw_core::{ParamAddress, ParamValue};

#[derive(Clone, Debug, Default)]
pub struct ParamBuffer {
    slots: Vec<ParamValue>,
    index: BTreeMap<ParamAddress, usize>,
}

impl ParamBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure(&mut self, address: ParamAddress, initial: ParamValue) -> usize {
        if let Some(slot) = self.index.get(&address) {
            return *slot;
        }
        let slot = self.slots.len();
        self.slots.push(initial);
        self.index.insert(address, slot);
        slot
    }

    pub fn set(&mut self, address: &ParamAddress, value: ParamValue) -> bool {
        let Some(slot) = self.index.get(address).copied() else {
            return false;
        };
        self.slots[slot] = value;
        true
    }

    pub fn get(&self, address: &ParamAddress) -> Option<ParamValue> {
        self.index
            .get(address)
            .and_then(|slot| self.slots.get(*slot))
            .copied()
    }

    pub fn float(&self, address: &ParamAddress, default: f64) -> f64 {
        match self.get(address) {
            Some(ParamValue::Float(value)) => value,
            Some(ParamValue::Int(value)) => value as f64,
            Some(ParamValue::Bool(value)) => f64::from(u8::from(value)),
            Some(ParamValue::Enum(value)) => f64::from(value),
            None => default,
        }
    }

    pub fn slot(&self, address: &ParamAddress) -> Option<usize> {
        self.index.get(address).copied()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use odaw_core::{NodeId, ParamId};

    fn address(name: &str) -> ParamAddress {
        ParamAddress {
            node: NodeId::from_raw(1),
            param: ParamId::new(name),
        }
    }

    #[test]
    fn slots_are_stable_and_typed() {
        let mut buffer = ParamBuffer::new();
        let gain = buffer.ensure(address("gain"), ParamValue::Float(0.5));
        let pan = buffer.ensure(address("pan"), ParamValue::Float(0.0));
        assert_eq!(gain, 0);
        assert_eq!(pan, 1);
        assert_eq!(buffer.ensure(address("gain"), ParamValue::Float(9.0)), 0);
        assert_eq!(buffer.len(), 2);

        assert!(buffer.set(&address("gain"), ParamValue::Float(0.25)));
        assert!(!buffer.set(&address("missing"), ParamValue::Float(1.0)));
        assert_eq!(buffer.float(&address("gain"), 0.0), 0.25);
        assert_eq!(buffer.float(&address("missing"), 0.75), 0.75);
    }
}
