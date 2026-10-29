use std::collections::BTreeSet;

use odaw_core::{AssetId, ClipId, EdgeId, MarkerId, NodeId, NoteId, ParamAddress, SectionId};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub structural: bool,
    pub layout_or_latency: bool,
    pub meta_or_tempo: bool,
    pub arrangement: bool,
    pub nodes: BTreeSet<NodeId>,
    pub edges: BTreeSet<EdgeId>,
    pub clips: BTreeSet<ClipId>,
    pub notes: BTreeSet<NoteId>,
    pub sections: BTreeSet<SectionId>,
    pub markers: BTreeSet<MarkerId>,
    pub assets: BTreeSet<AssetId>,
    pub params: BTreeSet<ParamAddress>,
    pub automation: BTreeSet<ParamAddress>,
}

impl ChangeSet {
    pub fn merge(&mut self, other: Self) {
        self.structural |= other.structural;
        self.layout_or_latency |= other.layout_or_latency;
        self.meta_or_tempo |= other.meta_or_tempo;
        self.arrangement |= other.arrangement;
        self.nodes.extend(other.nodes);
        self.edges.extend(other.edges);
        self.clips.extend(other.clips);
        self.notes.extend(other.notes);
        self.sections.extend(other.sections);
        self.markers.extend(other.markers);
        self.assets.extend(other.assets);
        self.params.extend(other.params);
        self.automation.extend(other.automation);
    }

    pub fn is_empty(&self) -> bool {
        !self.structural
            && !self.layout_or_latency
            && !self.meta_or_tempo
            && !self.arrangement
            && self.nodes.is_empty()
            && self.edges.is_empty()
            && self.clips.is_empty()
            && self.notes.is_empty()
            && self.sections.is_empty()
            && self.markers.is_empty()
            && self.assets.is_empty()
            && self.params.is_empty()
            && self.automation.is_empty()
    }

    pub fn is_structural(&self) -> bool {
        self.structural
    }

    pub fn is_automation_only(&self) -> bool {
        !self.structural
            && !self.layout_or_latency
            && !self.meta_or_tempo
            && !self.arrangement
            && self.nodes.is_empty()
            && self.edges.is_empty()
            && self.clips.is_empty()
            && self.notes.is_empty()
            && self.sections.is_empty()
            && self.markers.is_empty()
            && self.assets.is_empty()
            && self.params.is_empty()
            && !self.automation.is_empty()
    }

    pub fn requires_validation(&self) -> bool {
        self.structural || self.arrangement || self.meta_or_tempo || !self.assets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_is_the_union_of_touches() {
        let mut left = ChangeSet {
            structural: true,
            ..ChangeSet::default()
        };
        left.nodes.insert(NodeId::from_raw(1));
        left.params.insert(ParamAddress {
            node: NodeId::from_raw(1),
            param: "gain".into(),
        });

        let mut right = ChangeSet {
            arrangement: true,
            ..ChangeSet::default()
        };
        right.nodes.insert(NodeId::from_raw(2));

        left.merge(right);

        assert!(left.structural);
        assert!(left.arrangement);
        assert_eq!(
            left.nodes.iter().copied().collect::<Vec<_>>(),
            vec![NodeId::from_raw(1), NodeId::from_raw(2)]
        );
        assert_eq!(left.params.len(), 1);
    }

    #[test]
    fn flags_alone_make_a_change_set_non_empty() {
        let changes = ChangeSet {
            meta_or_tempo: true,
            ..ChangeSet::default()
        };
        assert!(!changes.is_empty());
        assert!(!changes.is_structural());
    }
}
