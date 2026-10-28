use odaw_core::{
    Asset, AssetId, AutomationLane, Clip, ClipContent, ClipId, Edge, EdgeId, LoopRegion, Marker,
    MarkerId, ModulationLink, Node, NodeId, ParamAddress, ParamId, ParamValue, PersistentVector,
    Project, Section, SectionId, SectionRef, TempoMap, TimeSignature,
};

use crate::change::ChangeSet;
use crate::event::Event;

#[derive(Debug)]
pub(crate) struct Tx {
    project: Project,
    changes: ChangeSet,
    events: Vec<Event>,
}

#[derive(Debug)]
pub struct TxOutcome {
    pub project: Project,
    pub changes: ChangeSet,
    pub events: Vec<Event>,
}

impl Tx {
    pub(crate) fn new(project: Project) -> Self {
        Self {
            project,
            changes: ChangeSet::default(),
            events: Vec::new(),
        }
    }

    pub(crate) fn project(&self) -> &Project {
        &self.project
    }

    pub(crate) fn node(&self, node: NodeId) -> Option<&Node> {
        self.project.nodes.get(&node)
    }

    pub(crate) fn edge(&self, edge: EdgeId) -> Option<&Edge> {
        self.project.edges.get(&edge)
    }

    pub(crate) fn clip(&self, clip: ClipId) -> Option<&Clip> {
        self.project.clips.get(&clip)
    }

    pub(crate) fn section(&self, section: SectionId) -> Option<&Section> {
        self.project.sections.get(&section)
    }

    pub(crate) fn commit(self) -> TxOutcome {
        TxOutcome {
            project: self.project,
            changes: self.changes,
            events: self.events,
        }
    }

    pub(crate) fn insert_node(&mut self, node: NodeId, value: Node) {
        self.project.nodes.insert(node, value);
        self.changes.nodes.insert(node);
        self.changes.structural = true;
    }

    pub(crate) fn remove_node(&mut self, node: NodeId) -> Option<Node> {
        let removed = self.project.nodes.remove(&node);
        if removed.is_some() {
            self.changes.nodes.insert(node);
            self.changes.structural = true;
        }
        removed
    }

    pub(crate) fn set_track_order(&mut self, tracks: Vec<NodeId>) {
        self.project.track_order = tracks.into_iter().collect();
        self.changes.structural = true;
    }

    pub(crate) fn insert_edge(&mut self, edge: EdgeId, value: Edge) {
        self.project.edges.insert(edge, value);
        self.changes.edges.insert(edge);
        self.changes.structural = true;
        self.changes.layout_or_latency = true;
    }

    pub(crate) fn remove_edge(&mut self, edge: EdgeId) -> Option<Edge> {
        let removed = self.project.edges.remove(&edge);
        if removed.is_some() {
            self.changes.edges.insert(edge);
            self.changes.structural = true;
            self.changes.layout_or_latency = true;
        }
        removed
    }

    pub(crate) fn insert_clip(&mut self, clip: ClipId, value: Clip) {
        if let ClipContent::Midi { notes, .. } = &value.content {
            for note in notes {
                self.changes.notes.insert(note.id);
            }
        }
        self.project.clips.insert(clip, value);
        self.changes.clips.insert(clip);
        self.changes.structural = true;
    }

    pub(crate) fn remove_clip(&mut self, clip: ClipId) -> Option<Clip> {
        let removed = self.project.clips.remove(&clip);
        if let Some(value) = &removed {
            if let ClipContent::Midi { notes, .. } = &value.content {
                for note in notes {
                    self.changes.notes.insert(note.id);
                }
            }
            self.changes.clips.insert(clip);
            self.changes.structural = true;
        }
        removed
    }

    pub(crate) fn insert_section(&mut self, section: SectionId, value: Section) {
        self.project.sections.insert(section, value);
        self.changes.sections.insert(section);
        self.changes.arrangement = true;
    }

    pub(crate) fn remove_section(&mut self, section: SectionId) -> Option<Section> {
        let removed = self.project.sections.remove(&section);
        if removed.is_some() {
            self.changes.sections.insert(section);
            self.changes.arrangement = true;
        }
        removed
    }

    pub(crate) fn set_arrangement(&mut self, arrangement: Vec<SectionRef>) {
        self.project.arrangement = arrangement.into_iter().collect();
        self.changes.arrangement = true;
    }

    pub(crate) fn set_loops(&mut self, loops: Vec<LoopRegion>) {
        self.project.loops = loops.into_iter().collect();
        self.changes.arrangement = true;
    }

    pub(crate) fn insert_marker(&mut self, marker: MarkerId, value: Marker) {
        self.project.markers.insert(marker, value);
        self.changes.markers.insert(marker);
    }

    pub(crate) fn remove_marker(&mut self, marker: MarkerId) -> Option<Marker> {
        let removed = self.project.markers.remove(&marker);
        if removed.is_some() {
            self.changes.markers.insert(marker);
        }
        removed
    }

    pub(crate) fn insert_asset(&mut self, asset: AssetId, value: Asset) {
        self.project.assets.insert(asset, value);
        self.changes.assets.insert(asset);
        self.changes.structural = true;
    }

    pub(crate) fn remove_asset(&mut self, asset: AssetId) -> Option<Asset> {
        let removed = self.project.assets.remove(&asset);
        if removed.is_some() {
            self.changes.assets.insert(asset);
            self.changes.structural = true;
        }
        removed
    }

    pub(crate) fn set_meta_name(&mut self, name: String) {
        self.project.meta.name = name;
        self.changes.meta_or_tempo = true;
    }

    pub(crate) fn set_meta_seed(&mut self, seed: u64) {
        self.project.meta.seed = seed;
        self.changes.meta_or_tempo = true;
    }

    pub(crate) fn set_tempo(&mut self, tempo: TempoMap) {
        self.project.tempo = tempo;
        self.changes.meta_or_tempo = true;
    }

    pub(crate) fn set_signatures(&mut self, signatures: Vec<TimeSignature>) {
        self.project.signatures = signatures.into_iter().collect();
        self.changes.meta_or_tempo = true;
    }

    pub(crate) fn set_param(&mut self, address: ParamAddress, value: ParamValue) -> bool {
        let Some(node) = self.project.nodes.get_mut(&address.node) else {
            return false;
        };
        node.params.insert(address.param.clone(), value);
        self.changes.params.insert(address);
        true
    }

    pub(crate) fn set_automation(
        &mut self,
        node: NodeId,
        param: ParamId,
        lane: Option<AutomationLane>,
    ) -> bool {
        let Some(target) = self.project.nodes.get_mut(&node) else {
            return false;
        };
        match lane {
            Some(lane) => {
                target.automation.insert(param.clone(), lane);
            }
            None => {
                target.automation.remove(&param);
            }
        }
        self.changes.nodes.insert(node);
        self.changes.params.insert(ParamAddress { node, param });
        self.changes.structural = true;
        true
    }

    pub(crate) fn set_modulations(
        &mut self,
        node: NodeId,
        param: ParamId,
        links: Vec<ModulationLink>,
    ) -> bool {
        let Some(target) = self.project.nodes.get_mut(&node) else {
            return false;
        };
        let links: PersistentVector<ModulationLink> = links.into_iter().collect();
        if links.is_empty() {
            target.modulations.remove(&param);
        } else {
            target.modulations.insert(param.clone(), links);
        }
        self.changes.nodes.insert(node);
        self.changes.params.insert(ParamAddress { node, param });
        self.changes.structural = true;
        self.changes.layout_or_latency = true;
        true
    }

    pub(crate) fn mark_structural(&mut self) {
        self.changes.structural = true;
    }

    pub(crate) fn mark_layout_or_latency(&mut self) {
        self.changes.layout_or_latency = true;
    }

    pub(crate) fn mark_param(&mut self, address: ParamAddress) {
        self.changes.params.insert(address);
    }
}

#[cfg(test)]
mod tests {
    use odaw_core::{AutomationMode, Curve, Keyframe, NodeRole, Project};

    use super::*;

    fn project() -> Project {
        let mut project = Project::empty("demo", 48_000, 0).unwrap();
        project
            .nodes
            .insert(NodeId::from_raw(1), Node::new(NodeRole::Track));
        project
    }

    #[test]
    fn inserting_a_node_is_structural() {
        let mut tx = Tx::new(project());
        tx.insert_node(NodeId::from_raw(2), Node::new(NodeRole::Bus));

        let outcome = tx.commit();
        assert!(outcome.changes.is_structural());
        assert!(outcome.changes.nodes.contains(&NodeId::from_raw(2)));
    }

    #[test]
    fn setting_a_param_touches_only_the_param() {
        let mut tx = Tx::new(project());
        assert!(tx.set_param(
            ParamAddress {
                node: NodeId::from_raw(1),
                param: "gain".into(),
            },
            ParamValue::Float(0.5),
        ));

        let outcome = tx.commit();
        assert!(!outcome.changes.is_structural());
        assert!(outcome.changes.params.contains(&ParamAddress {
            node: NodeId::from_raw(1),
            param: "gain".into(),
        }));
        assert_eq!(
            outcome.project.nodes[&NodeId::from_raw(1)]
                .params
                .get(&ParamId::new("gain")),
            Some(&ParamValue::Float(0.5))
        );
    }

    #[test]
    fn automation_is_structural() {
        let mut tx = Tx::new(project());
        assert!(
            tx.set_automation(
                NodeId::from_raw(1),
                "gain".into(),
                Some(AutomationLane {
                    mode: AutomationMode::Absolute,
                    keyframes: [Keyframe {
                        tick: odaw_core::Tick::ZERO,
                        value: ParamValue::Float(0.5),
                        curve: Curve::Linear,
                    }]
                    .into_iter()
                    .collect(),
                }),
            )
        );

        let outcome = tx.commit();
        assert!(outcome.changes.is_structural());
        assert!(outcome.changes.params.contains(&ParamAddress {
            node: NodeId::from_raw(1),
            param: "gain".into(),
        }));
    }

    #[test]
    fn removing_a_missing_node_does_not_touch_the_change_set() {
        let mut tx = Tx::new(project());
        assert!(tx.remove_node(NodeId::from_raw(9)).is_none());
        assert!(tx.commit().changes.is_empty());
    }
}
