use std::collections::VecDeque;
use std::num::NonZeroUsize;

use odaw_core::Project;

use crate::change::ChangeSet;
use crate::command::Command;
use crate::operator::OperatorInvocation;

pub const DEFAULT_HISTORY_LIMIT: usize = 512;

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    project: Project,
    changes: ChangeSet,
    commands: Vec<Command>,
    operator: Option<OperatorInvocation>,
    label: String,
}

impl HistoryEntry {
    pub(crate) fn baseline(project: Project) -> Self {
        Self {
            project,
            changes: ChangeSet::default(),
            commands: Vec::new(),
            operator: None,
            label: "Initial".to_owned(),
        }
    }

    pub(crate) fn new(
        project: Project,
        changes: ChangeSet,
        commands: Vec<Command>,
        operator: Option<OperatorInvocation>,
        label: String,
    ) -> Self {
        Self {
            project,
            changes,
            commands,
            operator,
            label,
        }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn changes(&self) -> &ChangeSet {
        &self.changes
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    pub fn operator(&self) -> Option<&OperatorInvocation> {
        self.operator.as_ref()
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Debug)]
pub struct HistoryStep {
    pub project: Project,
    pub changes: ChangeSet,
}

#[derive(Debug)]
pub struct History {
    entries: VecDeque<HistoryEntry>,
    cursor: usize,
    limit: NonZeroUsize,
}

impl History {
    pub fn new(project: Project, limit: NonZeroUsize) -> Self {
        let mut entries = VecDeque::new();
        entries.push_back(HistoryEntry::baseline(project));
        Self {
            entries,
            cursor: 0,
            limit,
        }
    }

    pub fn default_limit() -> NonZeroUsize {
        NonZeroUsize::new(DEFAULT_HISTORY_LIMIT).expect("default history limit is non-zero")
    }

    pub fn current(&self) -> &Project {
        self.current_entry().project()
    }

    pub fn current_entry(&self) -> &HistoryEntry {
        self.entries
            .get(self.cursor)
            .expect("history always contains a current entry")
    }

    pub fn entry(&self, index: usize) -> Option<&HistoryEntry> {
        self.entries.get(index)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn limit(&self) -> NonZeroUsize {
        self.limit
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.can_undo().then(|| self.entries[self.cursor].label())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.can_redo()
            .then(|| self.entries[self.cursor + 1].label())
    }

    pub(crate) fn push(&mut self, entry: HistoryEntry) {
        self.entries.truncate(self.cursor + 1);
        self.entries.push_back(entry);
        self.cursor = self.entries.len() - 1;
        while self.entries.len() > self.limit.get() {
            self.entries.pop_front();
            self.cursor = self.cursor.saturating_sub(1);
        }
    }

    pub(crate) fn undo(&mut self) -> Option<HistoryStep> {
        if !self.can_undo() {
            return None;
        }
        let changes = self.entries[self.cursor].changes.clone();
        self.cursor -= 1;
        Some(HistoryStep {
            project: self.entries[self.cursor].project.clone(),
            changes,
        })
    }

    pub(crate) fn redo(&mut self) -> Option<HistoryStep> {
        if !self.can_redo() {
            return None;
        }
        self.cursor += 1;
        let entry = &self.entries[self.cursor];
        Some(HistoryStep {
            project: entry.project.clone(),
            changes: entry.changes.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> Project {
        Project::empty(name, 48_000, 0).unwrap()
    }

    fn entry(name: &str, label: &str) -> HistoryEntry {
        HistoryEntry::new(
            project(name),
            ChangeSet {
                meta_or_tempo: true,
                ..ChangeSet::default()
            },
            Vec::new(),
            None,
            label.to_owned(),
        )
    }

    fn history() -> History {
        History::new(project("baseline"), NonZeroUsize::new(8).unwrap())
    }

    #[test]
    fn a_fresh_history_cannot_undo() {
        let history = history();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert_eq!(history.current().meta.name, "baseline");
        assert_eq!(history.undo_label(), None);
    }

    #[test]
    fn undo_and_redo_cross_the_same_transition() {
        let mut history = history();
        history.push(entry("first", "First action"));

        assert_eq!(history.undo_label(), Some("First action"));
        let undone = history.undo().unwrap();
        assert_eq!(undone.project.meta.name, "baseline");
        assert!(undone.changes.meta_or_tempo);
        assert_eq!(history.current().meta.name, "baseline");

        assert_eq!(history.redo_label(), Some("First action"));
        let redone = history.redo().unwrap();
        assert_eq!(redone.project.meta.name, "first");
        assert!(redone.changes.meta_or_tempo);
        assert_eq!(history.current().meta.name, "first");
    }

    #[test]
    fn a_new_edit_after_undo_discards_redo() {
        let mut history = history();
        history.push(entry("first", "First"));
        history.undo();
        assert!(history.can_redo());

        history.push(entry("branch", "Branch"));
        assert!(!history.can_redo());
        assert_eq!(history.current().meta.name, "branch");
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn the_limit_evicts_the_oldest_transaction() {
        let mut history = History::new(project("baseline"), NonZeroUsize::new(2).unwrap());
        history.push(entry("first", "First"));
        history.push(entry("second", "Second"));

        assert_eq!(history.len(), 2);
        assert_eq!(history.current().meta.name, "second");
        assert!(history.can_undo());
        assert_eq!(history.undo_label(), Some("Second"));

        history.undo();
        assert_eq!(history.current().meta.name, "first");
        assert!(!history.can_undo());
    }

    #[test]
    fn labels_follow_the_cursor() {
        let mut history = history();
        history.push(entry("first", "First"));
        history.push(entry("second", "Second"));
        assert_eq!(history.undo_label(), Some("Second"));
        assert_eq!(history.redo_label(), None);

        history.undo();
        assert_eq!(history.undo_label(), Some("First"));
        assert_eq!(history.redo_label(), Some("Second"));
    }
}
