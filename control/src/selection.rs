use std::collections::BTreeSet;

use odaw_core::{ClipContent, ClipId, NodeId, NoteId, Project, TickRange};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub nodes: BTreeSet<NodeId>,
    pub clips: BTreeSet<ClipId>,
    pub notes: BTreeSet<NoteId>,
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

    pub fn select_note(&mut self, note: NoteId) {
        self.notes.insert(note);
    }

    pub fn toggle_note(&mut self, note: NoteId) {
        if !self.notes.remove(&note) {
            self.notes.insert(note);
        }
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.clips.clear();
        self.notes.clear();
        self.range = None;
        self.focused_node = None;
    }

    pub fn retain_existing(&mut self, project: &Project) -> bool {
        let previous = self.clone();
        self.nodes.retain(|node| project.nodes.contains_key(node));
        self.clips.retain(|clip| project.clips.contains_key(clip));
        let existing_notes: BTreeSet<NoteId> = project
            .clips
            .values()
            .filter_map(|clip| match &clip.content {
                ClipContent::Midi { notes, .. } => Some(notes),
                ClipContent::Audio { .. } | ClipContent::Empty => None,
            })
            .flat_map(|notes| notes.iter().map(|note| note.id))
            .collect();
        self.notes.retain(|note| existing_notes.contains(note));
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
    use odaw_core::{Clip, ClipContent, Node, NodeRole, Note, NoteId, Tick};

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
        project.clips.insert(
            ClipId::from_raw(3),
            Clip {
                node: NodeId::from_raw(1),
                start: Tick::ZERO,
                length: Tick::from_raw(10),
                content: ClipContent::Midi {
                    source_start: Tick::ZERO,
                    source_length: Tick::from_raw(10),
                    notes: [
                        Note {
                            id: NoteId::from_raw(10),
                            pitch: 60,
                            start: Tick::ZERO,
                            length: Tick::from_raw(10),
                            velocity: 100,
                            channel: 0,
                        },
                        Note {
                            id: NoteId::from_raw(11),
                            pitch: 64,
                            start: Tick::ZERO,
                            length: Tick::from_raw(10),
                            velocity: 100,
                            channel: 0,
                        },
                    ]
                    .into_iter()
                    .collect(),
                },
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

    #[test]
    fn retain_existing_drops_stale_notes() {
        let mut selection = Selection::default();
        selection.select_note(NoteId::from_raw(10));
        selection.select_note(NoteId::from_raw(99));
        assert!(selection.retain_existing(&project()));
        assert_eq!(
            selection.notes.iter().copied().collect::<Vec<_>>(),
            vec![NoteId::from_raw(10)]
        );
    }

    #[test]
    fn toggling_and_clearing_notes_works() {
        let mut selection = Selection::default();
        selection.select_note(NoteId::from_raw(10));
        selection.toggle_note(NoteId::from_raw(10));
        assert!(selection.notes.is_empty());
        selection.select_note(NoteId::from_raw(11));
        selection.clear();
        assert!(selection.notes.is_empty());
    }
}
