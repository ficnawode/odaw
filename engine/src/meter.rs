use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use odaw_core::NodeId;

/// One stereo peak meter. The audio thread writes the peak of the last block;
/// the UI reads it lock-free and applies its own decay.
#[derive(Debug, Default)]
pub struct Meter {
    left: AtomicU32,
    right: AtomicU32,
}

impl Meter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store(&self, left: f32, right: f32) {
        self.left.store(encode(left), Ordering::Relaxed);
        self.right.store(encode(right), Ordering::Relaxed);
    }

    pub fn store_block(&self, samples: &[f32], channels: usize) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        match channels {
            0 => {}
            1 => {
                for sample in samples {
                    left = left.max(sample.abs());
                }
                right = left;
            }
            _ => {
                for frame in samples.chunks_exact(channels) {
                    left = left.max(frame[0].abs());
                    right = right.max(frame[1].abs());
                }
            }
        }
        self.store(left, right);
    }

    pub fn levels(&self) -> (f32, f32) {
        (
            decode(self.left.load(Ordering::Relaxed)),
            decode(self.right.load(Ordering::Relaxed)),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct MeterBank {
    entries: Arc<BTreeMap<NodeId, Arc<Meter>>>,
}

impl MeterBank {
    pub fn new(nodes: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            entries: Arc::new(
                nodes
                    .into_iter()
                    .map(|node| (node, Arc::new(Meter::new())))
                    .collect(),
            ),
        }
    }

    pub fn slot(&self, node: NodeId) -> Option<Arc<Meter>> {
        self.entries.get(&node).cloned()
    }

    pub fn levels(&self, node: NodeId) -> Option<(f32, f32)> {
        self.entries.get(&node).map(|meter| meter.levels())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn encode(value: f32) -> u32 {
    let value = if value.is_finite() {
        value.abs().min(16.0)
    } else {
        0.0
    };
    value.to_bits()
}

fn decode(value: u32) -> f32 {
    f32::from_bits(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meters_store_interleaved_stereo_peaks() {
        let meter = Meter::new();
        meter.store_block(&[0.5, -0.25, 0.75, -1.0], 2);
        assert_eq!(meter.levels(), (0.75, 1.0));
    }

    #[test]
    fn mono_meters_report_both_channels() {
        let meter = Meter::new();
        meter.store_block(&[0.1, -0.4], 1);
        assert_eq!(meter.levels(), (0.4, 0.4));
    }

    #[test]
    fn invalid_samples_are_zeroed() {
        let meter = Meter::new();
        meter.store(f32::NAN, f32::INFINITY);
        assert_eq!(meter.levels(), (0.0, 0.0));
    }

    #[test]
    fn banks_look_up_meters_by_node() {
        let bank = MeterBank::new([NodeId::from_raw(1), NodeId::from_raw(2)]);
        assert_eq!(bank.len(), 2);
        assert_eq!(bank.levels(NodeId::from_raw(9)), None);
        bank.slot(NodeId::from_raw(1)).unwrap().store(0.25, 0.5);
        assert_eq!(bank.levels(NodeId::from_raw(1)), Some((0.25, 0.5)));
    }
}
