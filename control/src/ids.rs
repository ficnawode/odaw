use odaw_core::{ClipId, EdgeId, MarkerId, NodeId, Project, SectionId};

use crate::ChangeSet;

#[derive(Clone, Debug)]
pub struct IdGenerator {
    next: u64,
}

impl IdGenerator {
    pub fn new(seed: u64) -> Self {
        Self { next: seed.max(1) }
    }

    pub fn from_project(project: &Project) -> Self {
        let mut generator = Self::new(0);
        generator.observe_project(project);
        generator
    }

    pub fn next_node(&mut self) -> NodeId {
        NodeId::from_raw(self.take())
    }

    pub fn next_edge(&mut self) -> EdgeId {
        EdgeId::from_raw(self.take())
    }

    pub fn next_clip(&mut self) -> ClipId {
        ClipId::from_raw(self.take())
    }

    pub fn next_section(&mut self) -> SectionId {
        SectionId::from_raw(self.take())
    }

    pub fn next_marker(&mut self) -> MarkerId {
        MarkerId::from_raw(self.take())
    }

    pub fn observe_project(&mut self, project: &Project) {
        for id in project.nodes.keys() {
            self.observe(id.raw());
        }
        for id in project.edges.keys() {
            self.observe(id.raw());
        }
        for id in project.clips.keys() {
            self.observe(id.raw());
        }
        for id in project.sections.keys() {
            self.observe(id.raw());
        }
        for id in project.markers.keys() {
            self.observe(id.raw());
        }
    }

    pub fn observe_changes(&mut self, changes: &ChangeSet) {
        for id in &changes.nodes {
            self.observe(id.raw());
        }
        for id in &changes.edges {
            self.observe(id.raw());
        }
        for id in &changes.clips {
            self.observe(id.raw());
        }
        for id in &changes.sections {
            self.observe(id.raw());
        }
        for id in &changes.markers {
            self.observe(id.raw());
        }
    }

    fn observe(&mut self, id: u64) {
        let next = id.checked_add(1).expect("id space exhausted");
        self.next = self.next.max(next);
    }

    fn take(&mut self) -> u64 {
        let value = self.next;
        self.next = self.next.checked_add(1).expect("id space exhausted");
        value
    }
}

#[cfg(test)]
mod tests {
    use odaw_core::{Node, NodeRole, Project};

    use super::*;

    #[test]
    fn ids_are_deterministic() {
        let mut first = IdGenerator::new(7);
        let mut second = IdGenerator::new(7);
        assert_eq!(first.next_node(), second.next_node());
        assert_eq!(first.next_edge(), second.next_edge());
        assert_eq!(first.next_node(), NodeId::from_raw(9));
    }

    #[test]
    fn ids_continue_after_existing_project_ids() {
        let mut project = Project::empty("demo", 48_000, 0).unwrap();
        project
            .nodes
            .insert(NodeId::from_raw(41), Node::new(NodeRole::Track));

        let mut generator = IdGenerator::from_project(&project);
        assert_eq!(generator.next_node(), NodeId::from_raw(42));
    }

    #[test]
    fn observing_changes_moves_the_counter_forward() {
        let mut generator = IdGenerator::new(1);
        let mut changes = ChangeSet::default();
        changes.nodes.insert(NodeId::from_raw(100));
        generator.observe_changes(&changes);
        assert_eq!(generator.next_node(), NodeId::from_raw(101));
    }
}
