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
