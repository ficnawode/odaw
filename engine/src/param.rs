use std::collections::BTreeMap;

use odaw_core::{ParamAddress, ParamValue};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Ramp {
    start: f64,
    end: f64,
}

#[derive(Clone, Debug, Default)]
pub struct ParamBuffer {
    slots: Vec<ParamValue>,
    bases: Vec<ParamValue>,
    ramps: Vec<Option<Ramp>>,
    versions: Vec<u64>,
    touched: Vec<bool>,
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
        self.bases.push(initial);
        self.ramps.push(None);
        self.versions.push(0);
        self.touched.push(false);
        self.index.insert(address, slot);
        slot
    }

    /// Write the document/base value. Automation overwrites the effective value
    /// on its next block; trim automation rebuilds it from this base.
    pub fn set(&mut self, address: &ParamAddress, value: ParamValue) -> bool {
        let Some(slot) = self.index.get(address).copied() else {
            return false;
        };
        let changed = self.slots[slot] != value || self.ramps[slot].is_some();
        self.bases[slot] = value;
        self.slots[slot] = value;
        self.ramps[slot] = None;
        if changed {
            self.versions[slot] = self.versions[slot].wrapping_add(1);
        }
        true
    }

    /// Write the value automation computed for this block.
    pub fn set_effective(&mut self, address: &ParamAddress, value: ParamValue) -> bool {
        let Some(slot) = self.index.get(address).copied() else {
            return false;
        };
        if self.slots[slot] == value && self.ramps[slot].is_none() {
            return true;
        }
        self.slots[slot] = value;
        self.ramps[slot] = None;
        self.versions[slot] = self.versions[slot].wrapping_add(1);
        true
    }

    /// Store a linear ramp for this block. `end` is the value at the start of
    /// the next block, so sample `i` of `frames` is `start + (end - start) * i / frames`.
    pub fn set_ramp(&mut self, address: &ParamAddress, start: f64, end: f64) -> bool {
        let Some(slot) = self.index.get(address).copied() else {
            return false;
        };
        let ramp = Ramp { start, end };
        let value = ParamValue::Float(start);
        let changed = self.slots[slot] != value || self.ramps[slot] != Some(ramp);
        self.slots[slot] = value;
        self.ramps[slot] = Some(ramp);
        if changed {
            self.versions[slot] = self.versions[slot].wrapping_add(1);
        }
        true
    }

    pub fn clear_ramps(&mut self) {
        self.ramps.fill(None);
    }

    pub fn set_touch(&mut self, address: &ParamAddress, active: bool) -> bool {
        let Some(slot) = self.index.get(address).copied() else {
            return false;
        };
        self.touched[slot] = active;
        true
    }

    pub fn is_touched(&self, slot: usize) -> bool {
        self.touched.get(slot).copied().unwrap_or(false)
    }

    /// Monotonic revision of one slot. Devices cache this per parameter and only
    /// forward values whose revision moved, instead of comparing every value every
    /// block (and without the audio thread having to clear a dirty set).
    pub fn version(&self, slot: usize) -> Option<u64> {
        self.versions.get(slot).copied()
    }

    pub fn get(&self, address: &ParamAddress) -> Option<ParamValue> {
        self.index
            .get(address)
            .and_then(|slot| self.slots.get(*slot))
            .copied()
    }

    pub fn base(&self, address: &ParamAddress) -> Option<ParamValue> {
        self.index
            .get(address)
            .and_then(|slot| self.bases.get(*slot))
            .copied()
    }

    pub fn ramp(&self, address: &ParamAddress) -> Option<(f64, f64)> {
        let slot = self.index.get(address)?;
        self.ramps[*slot].map(|ramp| (ramp.start, ramp.end))
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

    /// Effective value at a sample inside a block. Devices that care about
    /// smooth automation use this; block-rate devices keep using `float`.
    pub fn float_at(&self, address: &ParamAddress, sample: usize, frames: usize) -> Option<f64> {
        let slot = *self.index.get(address)?;
        if let Some(ramp) = self.ramps[slot] {
            let ratio = if frames == 0 {
                0.0
            } else {
                (sample.min(frames.saturating_sub(1)) as f64) / frames as f64
            };
            return Some(ramp.start + (ramp.end - ramp.start) * ratio);
        }
        self.slots.get(slot).map(|value| match value {
            ParamValue::Float(value) => *value,
            ParamValue::Int(value) => *value as f64,
            ParamValue::Bool(value) => f64::from(u8::from(*value)),
            ParamValue::Enum(value) => f64::from(*value),
        })
    }

    pub fn slot(&self, address: &ParamAddress) -> Option<usize> {
        self.index.get(address).copied()
    }

    pub fn slots(&self) -> &BTreeMap<ParamAddress, usize> {
        &self.index
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

    #[test]
    fn versions_only_move_when_a_value_changes() {
        let mut buffer = ParamBuffer::new();
        let slot = buffer.ensure(address("gain"), ParamValue::Float(0.5));
        assert_eq!(buffer.version(slot), Some(0));
        buffer.set(&address("gain"), ParamValue::Float(0.5));
        assert_eq!(buffer.version(slot), Some(0));
        buffer.set(&address("gain"), ParamValue::Float(0.75));
        assert_eq!(buffer.version(slot), Some(1));
    }

    #[test]
    fn base_and_effective_values_are_separate() {
        let mut buffer = ParamBuffer::new();
        buffer.ensure(address("gain"), ParamValue::Float(0.5));
        buffer.set_effective(&address("gain"), ParamValue::Float(0.9));
        assert_eq!(buffer.get(&address("gain")), Some(ParamValue::Float(0.9)));
        assert_eq!(buffer.base(&address("gain")), Some(ParamValue::Float(0.5)));
        buffer.set(&address("gain"), ParamValue::Float(0.25));
        assert_eq!(buffer.get(&address("gain")), Some(ParamValue::Float(0.25)));
        assert_eq!(buffer.base(&address("gain")), Some(ParamValue::Float(0.25)));
    }

    #[test]
    fn ramps_interpolate_from_the_absolute_start() {
        let mut buffer = ParamBuffer::new();
        buffer.ensure(address("gain"), ParamValue::Float(0.0));
        buffer.set_ramp(&address("gain"), 0.0, 1.0);
        assert_eq!(buffer.float_at(&address("gain"), 0, 4), Some(0.0));
        assert_eq!(buffer.float_at(&address("gain"), 1, 4), Some(0.25));
        assert_eq!(buffer.float_at(&address("gain"), 2, 4), Some(0.5));
        assert_eq!(buffer.float_at(&address("gain"), 3, 4), Some(0.75));
        assert_eq!(buffer.ramp(&address("gain")), Some((0.0, 1.0)));
    }
}
