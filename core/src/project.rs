use std::collections::BTreeSet;

use thiserror::Error;

use crate::clip::{Asset, Clip, Section, SectionRef};
use crate::collections::{PersistentMap, PersistentVector};
use crate::graph::{Edge, Node};
use crate::ids::{AssetId, ClipId, EdgeId, MarkerId, NodeId, SectionId};
use crate::time::{LoopRegion, Marker, TempoError, TempoMap, TempoPoint, Tick, TimeSignature};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub name: String,
    pub sample_rate: u32,
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub meta: Meta,
    pub tempo: TempoMap,
    pub signatures: PersistentVector<TimeSignature>,
    pub markers: PersistentMap<MarkerId, Marker>,
    pub sections: PersistentMap<SectionId, Section>,
    pub arrangement: PersistentVector<SectionRef>,
    pub loops: PersistentVector<LoopRegion>,
    pub nodes: PersistentMap<NodeId, Node>,
    /// UI order for tracks. Missing ids are ignored and unlisted tracks are
    /// appended in id order, so this is a hint rather than a validation burden.
    pub track_order: PersistentVector<NodeId>,
    pub edges: PersistentMap<EdgeId, Edge>,
    pub clips: PersistentMap<ClipId, Clip>,
    pub assets: PersistentMap<AssetId, Asset>,
}

impl Project {
    pub fn empty(
        name: impl Into<String>,
        sample_rate: u32,
        seed: u64,
    ) -> Result<Self, ProjectError> {
        Ok(Self {
            meta: Meta {
                name: name.into(),
                sample_rate,
                seed,
            },
            tempo: TempoMap::at_120_bpm(sample_rate)?,
            signatures: [TimeSignature::default()].into_iter().collect(),
            markers: PersistentMap::new(),
            sections: PersistentMap::new(),
            arrangement: PersistentVector::new(),
            loops: PersistentVector::new(),
            nodes: PersistentMap::new(),
            track_order: PersistentVector::new(),
            edges: PersistentMap::new(),
            clips: PersistentMap::new(),
            assets: PersistentMap::new(),
        })
    }

    pub fn with_tempo(mut self, micros_per_quarter: u32) -> Result<Self, TempoError> {
        self.tempo = TempoMap::new(
            self.meta.sample_rate,
            [TempoPoint {
                tick: Tick::ZERO,
                micros_per_quarter,
            }],
        )?;
        Ok(self)
    }

    /// Tracks in display order. The explicit order wins, then any track missing
    /// from it is appended by node id so older projects and newly added tracks
    /// still show up.
    pub fn ordered_track_ids(&self) -> Vec<NodeId> {
        let mut seen = BTreeSet::new();
        let mut order = Vec::new();
        for id in &self.track_order {
            if self
                .nodes
                .get(id)
                .is_some_and(|node| node.role == crate::graph::NodeRole::Track)
                && seen.insert(*id)
            {
                order.push(*id);
            }
        }
        for (id, node) in &self.nodes {
            if node.role == crate::graph::NodeRole::Track && seen.insert(*id) {
                order.push(*id);
            }
        }
        order
    }
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error(transparent)]
    Tempo(#[from] TempoError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_order_falls_back_to_id_order() {
        let mut project = Project::empty("order", 48_000, 0).unwrap();
        for id in [3, 1, 2] {
            project.nodes.insert(
                NodeId::from_raw(id),
                Node::new(crate::graph::NodeRole::Track),
            );
        }
        assert_eq!(
            project.ordered_track_ids(),
            vec![
                NodeId::from_raw(1),
                NodeId::from_raw(2),
                NodeId::from_raw(3)
            ]
        );
        project.track_order = [
            NodeId::from_raw(3),
            NodeId::from_raw(1),
            NodeId::from_raw(9),
            NodeId::from_raw(3),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            project.ordered_track_ids(),
            vec![
                NodeId::from_raw(3),
                NodeId::from_raw(1),
                NodeId::from_raw(2)
            ]
        );
    }

    #[test]
    fn empty_project_starts_at_120_bpm_in_four_four() {
        let project = Project::empty("demo", 48_000, 7).unwrap();
        assert_eq!(project.tempo.points().count(), 1);
        assert_eq!(
            project
                .tempo
                .tick_to_frames(Tick::from_raw(crate::PPQ))
                .unwrap(),
            crate::Frames::from_raw(24_000)
        );
        assert_eq!(
            project.signatures.iter().copied().collect::<Vec<_>>(),
            vec![TimeSignature {
                tick: Tick::ZERO,
                numerator: 4,
                denominator: 4,
            }]
        );
    }

    #[test]
    fn empty_project_rejects_zero_sample_rate() {
        assert!(Project::empty("demo", 0, 0).is_err());
    }
}
