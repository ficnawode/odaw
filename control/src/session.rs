use std::num::NonZeroUsize;

use odaw_core::{ClipId, EdgeId, MarkerId, NodeId, NoteId, Project, SectionId, ValidationError};
use thiserror::Error;

use crate::change::ChangeSet;
use crate::command::Command;
use crate::dispatch::{DispatchError, dispatch};
use crate::event::Event;
use crate::history::{History, HistoryEntry};
use crate::ids::IdGenerator;
use crate::operator::{
    Operator, OperatorError, OperatorId, OperatorInvocation, OperatorParams, OperatorRegistry,
};
use crate::selection::Selection;
use crate::tx::Tx;

pub trait Effects {
    fn apply(&mut self, project: &Project, changes: &ChangeSet);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoEffects;

impl Effects for NoEffects {
    fn apply(&mut self, _project: &Project, _changes: &ChangeSet) {}
}

#[derive(Debug)]
pub struct Session<E = NoEffects> {
    history: History,
    ids: IdGenerator,
    selection: Selection,
    effects: E,
    last: Option<OperatorInvocation>,
}

impl Session<NoEffects> {
    pub fn new(project: Project) -> Result<Self, SessionError> {
        Self::with_effects(project, NoEffects)
    }
}

impl<E: Effects> Session<E> {
    pub fn with_effects(project: Project, effects: E) -> Result<Self, SessionError> {
        Self::with_history_limit(project, effects, History::default_limit())
    }

    pub fn with_history_limit(
        project: Project,
        effects: E,
        limit: NonZeroUsize,
    ) -> Result<Self, SessionError> {
        project.validate()?;
        let ids = IdGenerator::from_project(&project);
        Ok(Self {
            history: History::new(project, limit),
            ids,
            selection: Selection::default(),
            effects,
            last: None,
        })
    }

    pub fn project(&self) -> &Project {
        self.history.current()
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn effects(&self) -> &E {
        &self.effects
    }

    pub fn last_invocation(&self) -> Option<&OperatorInvocation> {
        self.last.as_ref()
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    pub fn next_node_id(&mut self) -> NodeId {
        self.ids.next_node()
    }

    pub fn next_edge_id(&mut self) -> EdgeId {
        self.ids.next_edge()
    }

    pub fn next_clip_id(&mut self) -> ClipId {
        self.ids.next_clip()
    }

    pub fn next_note_id(&mut self) -> NoteId {
        self.ids.next_note()
    }

    pub fn next_section_id(&mut self) -> SectionId {
        self.ids.next_section()
    }

    pub fn next_marker_id(&mut self) -> MarkerId {
        self.ids.next_marker()
    }

    pub fn apply(&mut self, operator: &dyn Operator) -> Result<Vec<Event>, SessionError> {
        let invocation = operator.invocation();
        if !operator.enabled(self.history.current()) {
            return Err(SessionError::OperatorDisabled {
                id: invocation.id,
                version: invocation.version,
            });
        }
        let commands = operator.run(self.history.current(), &mut self.ids)?;
        let label = operator.label();
        self.commit(commands, Some(invocation), label)
    }

    pub fn apply_invocation(
        &mut self,
        registry: &OperatorRegistry,
        invocation: &OperatorInvocation,
    ) -> Result<Vec<Event>, SessionError> {
        let operator = registry.build(invocation)?;
        self.apply(operator.as_ref())
    }

    pub fn apply_command(&mut self, command: Command) -> Result<Vec<Event>, SessionError> {
        let label = command.label().to_owned();
        let operator = command
            .operator
            .as_ref()
            .map(|reference| OperatorInvocation {
                id: reference.id.clone(),
                version: reference.version,
                params: OperatorParams::None,
            });
        self.commit(vec![command], operator, label)
    }

    pub fn apply_commands(
        &mut self,
        commands: Vec<Command>,
        label: impl Into<String>,
    ) -> Result<Vec<Event>, SessionError> {
        self.commit(commands, None, label.into())
    }

    pub fn undo(&mut self) -> Vec<Event> {
        let Some(step) = self.history.undo() else {
            return Vec::new();
        };
        self.effects.apply(&step.project, &step.changes);
        let selection_pruned = self.selection.retain_existing(self.history.current());
        let mut events = vec![
            Event::ProjectChanged {
                changes: step.changes,
            },
            Event::HistoryChanged {
                can_undo: self.history.can_undo(),
                can_redo: self.history.can_redo(),
            },
        ];
        if selection_pruned {
            events.push(Event::SelectionChanged);
        }
        events
    }

    pub fn redo(&mut self) -> Vec<Event> {
        let Some(step) = self.history.redo() else {
            return Vec::new();
        };
        self.effects.apply(&step.project, &step.changes);
        let selection_pruned = self.selection.retain_existing(self.history.current());
        let mut events = vec![
            Event::ProjectChanged {
                changes: step.changes,
            },
            Event::HistoryChanged {
                can_undo: self.history.can_undo(),
                can_redo: self.history.can_redo(),
            },
        ];
        if selection_pruned {
            events.push(Event::SelectionChanged);
        }
        events
    }

    pub fn set_selection(&mut self, mut selection: Selection) -> Vec<Event> {
        selection.retain_existing(self.project());
        if selection == self.selection {
            return Vec::new();
        }
        self.selection = selection;
        vec![Event::SelectionChanged]
    }

    pub fn clear_selection(&mut self) -> Vec<Event> {
        self.set_selection(Selection::default())
    }

    fn commit(
        &mut self,
        mut commands: Vec<Command>,
        operator: Option<OperatorInvocation>,
        label: String,
    ) -> Result<Vec<Event>, SessionError> {
        let operator_ref = operator.as_ref().map(OperatorInvocation::reference);
        if let Some(operator_ref) = &operator_ref {
            for command in &mut commands {
                if command.operator.is_none() {
                    command.operator = Some(operator_ref.clone());
                }
            }
        }

        let mut tx = Tx::new(self.history.current().clone());
        for command in &commands {
            dispatch(command, &mut tx)?;
        }
        let outcome = tx.commit();
        if outcome.changes.requires_validation() {
            outcome.project.validate()?;
        }

        if outcome.changes.is_empty() {
            self.last = operator;
            return Ok(outcome.events);
        }

        self.ids.observe_changes(&outcome.changes);
        self.history.push(HistoryEntry::new(
            outcome.project.clone(),
            outcome.changes.clone(),
            commands,
            operator.clone(),
            label,
        ));
        self.effects.apply(&outcome.project, &outcome.changes);
        self.last = operator;

        let selection_pruned = self.selection.retain_existing(self.history.current());
        let mut events = outcome.events;
        events.push(Event::ProjectChanged {
            changes: outcome.changes,
        });
        events.push(Event::HistoryChanged {
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
        });
        if selection_pruned {
            events.push(Event::SelectionChanged);
        }
        Ok(events)
    }
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    #[error(transparent)]
    Operator(#[from] OperatorError),
    #[error("operator {id} version {version} is disabled")]
    OperatorDisabled { id: OperatorId, version: u16 },
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use odaw_core::{
        ChannelLayout, LoopRegion, Node, NodeKind, NodeRole, ParamAddress, ParamValue, Port,
        PortId, SectionRef, Tick,
    };

    use super::*;
    use crate::command::{
        ArrangementCommand, CommandKind, DeviceCommand, LoopCommand, NodeCommand, ParamChange,
        ParamCommand,
    };
    use crate::operator::{RenameProject, SetParam, TargetRef};

    fn empty_project() -> Project {
        Project::empty("demo", 48_000, 0).unwrap()
    }

    fn project_with_track() -> Project {
        let mut project = empty_project();
        project.nodes.insert(
            NodeId::from_raw(1),
            Node::new(NodeRole::Track)
                .with_ports([Port::output(PortId::from_raw(0), ChannelLayout::Stereo)]),
        );
        project
    }

    fn add_track(node: NodeId) -> Command {
        Command::new(CommandKind::Node(NodeCommand::Add {
            node,
            value: Box::new(Node::new(NodeRole::Track)),
        }))
    }

    #[derive(Clone, Debug, Default)]
    struct Recorder {
        applications: Rc<RefCell<Vec<(Project, ChangeSet)>>>,
    }

    impl Effects for Recorder {
        fn apply(&mut self, project: &Project, changes: &ChangeSet) {
            self.applications
                .borrow_mut()
                .push((project.clone(), changes.clone()));
        }
    }

    #[test]
    fn rename_operator_changes_project_and_history() {
        let mut session = Session::new(empty_project()).unwrap();
        let events = session.apply(&RenameProject::new("renamed")).unwrap();

        assert_eq!(session.project().meta.name, "renamed");
        assert_eq!(session.undo_label(), Some("Rename project"));
        assert!(session.can_undo());
        assert_eq!(
            session
                .last_invocation()
                .map(|invocation| invocation.id.as_str()),
            Some("builtin.rename-project")
        );
        assert!(matches!(events[0], Event::ProjectChanged { .. }));
        assert!(matches!(
            session.history().current_entry().operator(),
            Some(invocation) if invocation.id.as_str() == "builtin.rename-project"
        ));

        session.undo();
        assert_eq!(session.project().meta.name, "demo");
        assert!(session.can_redo());

        session.redo();
        assert_eq!(session.project().meta.name, "renamed");
    }

    #[test]
    fn apply_command_stamps_the_operator_reference() {
        let mut session = Session::new(empty_project()).unwrap();
        let operator = crate::operator::OperatorRef {
            id: OperatorId::new("builtin.external"),
            version: 3,
        };
        session
            .apply_command(
                Command::new(CommandKind::Meta(crate::command::MetaCommand::SetSeed(4)))
                    .with_operator(operator.clone()),
            )
            .unwrap();

        let entry = session.history().current_entry();
        assert_eq!(
            entry.operator().map(OperatorInvocation::reference),
            Some(operator)
        );
        assert_eq!(session.project().meta.seed, 4);
    }

    #[test]
    fn failed_dispatch_leaves_the_project_and_history_untouched() {
        let mut session = Session::new(project_with_track()).unwrap();
        let result = session.apply_command(Command::new(CommandKind::Node(NodeCommand::Remove {
            node: NodeId::from_raw(99),
        })));

        assert!(matches!(result, Err(SessionError::Dispatch(_))));
        assert_eq!(session.history().len(), 1);
        assert!(!session.can_undo());
        assert_eq!(session.project(), &project_with_track());
    }

    #[test]
    fn structural_validation_failure_aborts_the_transaction() {
        let mut session = Session::new(empty_project()).unwrap();
        let result = session.apply_command(Command::new(CommandKind::Node(NodeCommand::Add {
            node: NodeId::from_raw(1),
            value: Box::new(Node::new(NodeRole::Device)),
        })));

        assert!(matches!(result, Err(SessionError::Validation(_))));
        assert!(session.project().nodes.is_empty());
        assert!(!session.can_undo());
    }

    #[test]
    fn invalid_loop_changes_are_validated_and_aborted() {
        let mut session = Session::new(empty_project()).unwrap();
        let result =
            session.apply_command(Command::new(CommandKind::Loop(LoopCommand::Set(vec![
                LoopRegion {
                    start: Tick::from_raw(10),
                    end: Tick::ZERO,
                },
            ]))));

        assert!(matches!(result, Err(SessionError::Validation(_))));
        assert!(session.project().loops.is_empty());
        assert!(!session.can_undo());
    }

    #[test]
    fn arrangement_changes_are_validated_and_aborted() {
        let mut session = Session::new(empty_project()).unwrap();
        let result = session.apply_command(Command::new(CommandKind::Arrangement(
            ArrangementCommand::Set(vec![SectionRef {
                section: SectionId::from_raw(9),
                repeat: 1,
                active: true,
            }]),
        )));

        assert!(matches!(result, Err(SessionError::Validation(_))));
        assert!(session.project().arrangement.is_empty());
        assert!(!session.can_undo());
    }

    #[test]
    fn disabled_operator_is_rejected() {
        let mut session = Session::new(empty_project()).unwrap();
        let operator = SetParam::new(
            TargetRef::Explicit(NodeId::from_raw(99)),
            "gain".into(),
            ParamValue::Float(1.0),
        );
        assert!(matches!(
            session.apply(&operator),
            Err(SessionError::OperatorDisabled { .. })
        ));
        assert_eq!(session.history().len(), 1);
    }

    #[test]
    fn set_param_operator_updates_only_the_parameter_buffer_view() {
        let mut session = Session::new(project_with_track()).unwrap();
        let events = session
            .apply(&SetParam::new(
                TargetRef::Explicit(NodeId::from_raw(1)),
                "gain".into(),
                ParamValue::Float(0.25),
            ))
            .unwrap();

        assert_eq!(
            session.project().nodes[&NodeId::from_raw(1)]
                .params
                .get(&"gain".into()),
            Some(&ParamValue::Float(0.25))
        );
        assert!(matches!(
            events[0],
            Event::ProjectChanged { ref changes }
                if !changes.is_structural()
                    && changes.params.contains(&ParamAddress {
                        node: NodeId::from_raw(1),
                        param: "gain".into(),
                    })
        ));
    }

    #[test]
    fn effects_run_for_commit_undo_and_redo() {
        let recorder = Recorder::default();
        let applications = recorder.applications.clone();
        let mut session = Session::with_effects(empty_project(), recorder).unwrap();

        session.apply(&RenameProject::new("one")).unwrap();
        session.undo();
        session.redo();

        assert_eq!(applications.borrow().len(), 3);
        assert_eq!(applications.borrow()[0].0.meta.name, "one");
        assert_eq!(applications.borrow()[1].0.meta.name, "demo");
        assert_eq!(applications.borrow()[2].0.meta.name, "one");
    }

    #[test]
    fn selection_is_pruned_after_a_delete_and_not_restored_by_undo() {
        let mut session = Session::new(project_with_track()).unwrap();
        let mut selection = Selection::default();
        selection.select_node(NodeId::from_raw(1));
        session.set_selection(selection);

        let events = session
            .apply_command(Command::new(CommandKind::Node(NodeCommand::Remove {
                node: NodeId::from_raw(1),
            })))
            .unwrap();
        assert!(session.selection().nodes.is_empty());
        assert!(events.contains(&Event::SelectionChanged));

        session.undo();
        assert!(session.selection().nodes.is_empty());
    }

    #[test]
    fn setting_the_same_selection_is_silent() {
        let mut session = Session::new(project_with_track()).unwrap();
        let mut selection = Selection::default();
        selection.select_node(NodeId::from_raw(1));
        assert_eq!(session.set_selection(selection.clone()).len(), 1);
        assert!(session.set_selection(selection).is_empty());
    }

    #[test]
    fn ids_are_unique_against_existing_project_ids() {
        let mut session = Session::new(project_with_track()).unwrap();
        assert_eq!(session.next_node_id(), NodeId::from_raw(2));
    }

    #[test]
    fn session_history_limit_is_respected() {
        let mut session =
            Session::with_history_limit(empty_project(), NoEffects, NonZeroUsize::new(2).unwrap())
                .unwrap();
        session.apply(&RenameProject::new("one")).unwrap();
        session.apply(&RenameProject::new("two")).unwrap();

        assert_eq!(session.history().len(), 2);
        session.undo();
        assert_eq!(session.project().meta.name, "one");
        assert!(!session.can_undo());
    }

    #[test]
    fn command_replay_is_deterministic() {
        let commands = vec![
            add_track(NodeId::from_raw(1)),
            Command::new(CommandKind::Param(ParamCommand::Set {
                address: ParamAddress {
                    node: NodeId::from_raw(1),
                    param: "gain".into(),
                },
                value: ParamValue::Float(0.5),
                change: ParamChange::Value,
            })),
        ];

        let mut first = Session::new(empty_project()).unwrap();
        let mut second = Session::new(empty_project()).unwrap();
        for command in commands.clone() {
            first.apply_command(command.clone()).unwrap();
            second.apply_command(command).unwrap();
        }
        assert_eq!(first.project(), second.project());
    }

    #[test]
    fn command_batches_form_one_history_entry() {
        let mut session = Session::new(empty_project()).unwrap();
        session
            .apply_commands(
                vec![
                    add_track(NodeId::from_raw(1)),
                    add_track(NodeId::from_raw(2)),
                ],
                "Add two tracks",
            )
            .unwrap();

        assert_eq!(session.project().nodes.len(), 2);
        assert_eq!(session.history().len(), 2);
        assert_eq!(session.undo_label(), Some("Add two tracks"));
        session.undo();
        assert!(session.project().nodes.is_empty());
    }

    #[test]
    fn new_edit_after_undo_drops_redo() {
        let mut session = Session::new(empty_project()).unwrap();
        session.apply(&RenameProject::new("one")).unwrap();
        session.undo();
        assert!(session.can_redo());

        session.apply(&RenameProject::new("two")).unwrap();
        assert!(!session.can_redo());
        assert_eq!(session.project().meta.name, "two");
    }

    #[test]
    fn apply_invocation_builds_from_the_registry() {
        let registry = OperatorRegistry::builtin();
        let mut session = Session::new(empty_project()).unwrap();
        let invocation = OperatorInvocation {
            id: OperatorId::new("builtin.rename-project"),
            version: 1,
            params: OperatorParams::RenameProject {
                name: "from-registry".into(),
            },
        };

        session.apply_invocation(&registry, &invocation).unwrap();
        assert_eq!(session.project().meta.name, "from-registry");
    }

    #[test]
    fn device_command_requires_a_device_node() {
        let mut session = Session::new(project_with_track()).unwrap();
        let result =
            session.apply_command(Command::new(CommandKind::Device(DeviceCommand::SetKind {
                node: NodeId::from_raw(1),
                kind: NodeKind::new("builtin.gain"),
            })));
        assert!(matches!(result, Err(SessionError::Dispatch(_))));
        assert_eq!(session.history().len(), 1);
    }

    #[test]
    fn marker_ids_can_be_generated() {
        let mut session = Session::new(empty_project()).unwrap();
        assert_eq!(session.next_marker_id(), MarkerId::from_raw(1));
        assert_eq!(session.next_clip_id(), ClipId::from_raw(2));
        assert_eq!(session.next_section_id(), SectionId::from_raw(3));
        assert_eq!(session.next_edge_id(), EdgeId::from_raw(4));
        assert_eq!(session.next_node_id(), NodeId::from_raw(5));
        assert_eq!(session.next_note_id(), NoteId::from_raw(6));
    }
}
