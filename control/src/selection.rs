use std::collections::BTreeSet;

use odaw_core::{ClipId, NodeId, Project, TickRange};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub nodes: BTreeSet<NodeId>,
    pub clips: BTreeSet<ClipId>,
    pub range: Option<TickRange>,
    pub focused_node: Option<NodeId>,
}

impl Selection {
    pub fn select_node(&mut self, node: NodeId) {
        self.nodes.insert(node);
        self.focused_node = Some(node);
    }

    pub fn toggle_node(&mut self, node: NodeId) {
        if !self.nodes.remove(&node) {
            self.nodes.insert(node);
        }
        self.focused_node = Some(node);
    }

    pub fn select_clip(&mut self, clip: ClipId) {
        self.clips.insert(clip);
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.clips.clear();
        self.range = None;
        self.focused_node = None;
    }

    pub fn retain_existing(&mut self, project: &Project) -> bool {
        let previous = self.clone();
        self.nodes.retain(|node| project.nodes.contains_key(node));
        self.clips.retain(|clip| project.clips.contains_key(clip));
        if self
            .focused_node
            .is_some_and(|node| !project.nodes.contains_key(&node))
        {
            self.focused_node = None;
        }
        *self != previous
    }
}

#[cfg(test)]
mod tests {
    use odaw_core::{Clip, ClipContent, Node, NodeRole, Tick};

    use super::*;

    fn project() -> Project {
        let mut project = Project::empty("demo", 48_000, 0).unwrap();
        project
            .nodes
            .insert(NodeId::from_raw(1), Node::new(NodeRole::Track));
        project.clips.insert(
            ClipId::from_raw(2),
            Clip {
                node: NodeId::from_raw(1),
                start: Tick::ZERO,
                length: Tick::ZERO,
                content: ClipContent::Empty,
            },
        );
        project
    }

    #[test]
    fn selecting_a_node_also_focuses_it() {
        let mut selection = Selection::default();
        selection.select_node(NodeId::from_raw(1));
        assert!(selection.nodes.contains(&NodeId::from_raw(1)));
        assert_eq!(selection.focused_node, Some(NodeId::from_raw(1)));
    }

    #[test]
    fn toggling_a_node_removes_it_but_keeps_focus() {
        let mut selection = Selection::default();
        selection.select_node(NodeId::from_raw(1));
        selection.toggle_node(NodeId::from_raw(1));
        assert!(!selection.nodes.contains(&NodeId::from_raw(1)));
        assert_eq!(selection.focused_node, Some(NodeId::from_raw(1)));
    }

    #[test]
    fn retain_existing_drops_stale_nodes_and_clips() {
        let mut selection = Selection::default();
        selection.select_node(NodeId::from_raw(1));
        selection.select_node(NodeId::from_raw(9));
        selection.select_clip(ClipId::from_raw(2));
        selection.select_clip(ClipId::from_raw(8));
        selection.range = Some(TickRange {
            start: Tick::ZERO,
            end: Tick::from_raw(10),
        });

        assert!(selection.retain_existing(&project()));
        assert_eq!(
            selection.nodes.iter().copied().collect::<Vec<_>>(),
            vec![NodeId::from_raw(1)]
        );
        assert_eq!(
            selection.clips.iter().copied().collect::<Vec<_>>(),
            vec![ClipId::from_raw(2)]
        );
        assert_eq!(selection.focused_node, None);
        assert!(selection.range.is_some());
    }

    #[test]
    fn retain_existing_reports_when_nothing_changed() {
        let mut selection = Selection::default();
        selection.select_node(NodeId::from_raw(1));
        assert!(!selection.retain_existing(&project()));
    }
}
