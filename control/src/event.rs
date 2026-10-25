use crate::change::ChangeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    ProjectChanged { changes: ChangeSet },
    HistoryChanged { can_undo: bool, can_redo: bool },
    SelectionChanged,
}
