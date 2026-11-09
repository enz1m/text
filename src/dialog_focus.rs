use enzui_core::modal::DismissCause;
use enzui_widgets::modal_host::{ModalFocusLifecycle, ModalHostEvent};
use iced::Task;

use crate::{Message, TextApp, editor_id, settings_invoker_id, stats_button_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogFocusKind {
    RecoveryDiscardFailure,
    WorkspaceRemove,
    WindowClose,
    TabClose,
    Stats,
    Shortcuts,
}

pub(crate) const DIALOG_SCOPE_KEY: &str = "text-app-dialog";

pub(crate) const WINDOW_CLOSE_ID_PREFIX: &str = "text-app-window-close";

impl TextApp {
    pub(crate) fn active_dialog_focus_kind(&self) -> Option<DialogFocusKind> {
        if matches!(
            self.recovery_discard,
            Some(crate::RecoveryDiscard::Failed { .. })
        ) {
            Some(DialogFocusKind::RecoveryDiscardFailure)
        } else if self.close_guard.is_pending() {
            Some(DialogFocusKind::WindowClose)
        } else {
            None
        }
    }

    pub(crate) fn sync_dialog_focus(&mut self) -> Task<Message> {
        let desired = self.active_dialog_focus_kind();
        let current = self.dialog_focus.as_ref().map(|(kind, _)| *kind);
        let mut tasks = self.sync_stack_focus();
        if self.model.settings_open && self.settings_focus.is_none() {
            let (lifecycle, operation) = ModalFocusLifecycle::open::<Message>(
                "text-app-settings-host",
                Some(settings_invoker_id()),
            );
            self.settings_focus = Some(lifecycle);
            tasks.push(iced::advanced::widget::operate(operation));
        } else if !self.model.settings_open
            && let Some(lifecycle) = self.settings_focus.take()
        {
            tasks.push(lifecycle.close_for_host::<Message>().into_task());
        }
        if desired == current {
            return Task::batch(tasks);
        }
        if let Some((_, lifecycle)) = self.dialog_focus.take() {
            tasks.push(lifecycle.close_for_host::<Message>().into_task());
        }
        if let Some(kind) = desired {
            let (lifecycle, operation) =
                ModalFocusLifecycle::open::<Message>(DIALOG_SCOPE_KEY, None);
            // Discard is first in focus order, so this dialog overrides the host fallback.
            let operation = if kind == DialogFocusKind::WindowClose {
                enzui_widgets::save_changes_dialog::focus_default_action_operation::<Message>(
                    WINDOW_CLOSE_ID_PREFIX,
                    lifecycle.scope().clone(),
                )
            } else {
                operation
            };
            self.dialog_focus = Some((kind, lifecycle));
            tasks.push(iced::advanced::widget::operate(operation));
        }
        Task::batch(tasks)
    }

    pub(crate) fn dismiss_settings_dialog(&mut self, cause: DismissCause) -> Task<Message> {
        self.settings_focus
            .take()
            .map_or_else(Task::none, |lifecycle| {
                lifecycle
                    .reduce::<Message>(ModalHostEvent::Dismissed(cause))
                    .into_task()
            })
    }

    pub(crate) fn reduce_settings_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some(lifecycle) = self.settings_focus.as_ref() else {
            return Task::none();
        };
        let outcome = lifecycle.reduce::<Message>(event);
        if outcome.dismissed().is_some() {
            self.settings_focus = None;
            self.model.settings_open = false;
        }
        outcome.into_task()
    }

    fn stack_invoker(&self, kind: DialogFocusKind) -> Option<iced::advanced::widget::Id> {
        match kind {
            DialogFocusKind::Stats => Some(stats_button_id()),
            DialogFocusKind::WorkspaceRemove => {
                self.pending_workspace_remove.as_ref().and_then(|root| {
                    crate::workspace_context::restore_target(root)
                        .invoker_id()
                        .cloned()
                })
            }
            _ => Some(editor_id()),
        }
    }

    fn sync_stack_focus(&mut self) -> Vec<Task<Message>> {
        let mut tasks = Vec::new();
        let desired: Vec<_> = [
            (DialogFocusKind::Shortcuts, self.model.shortcuts_open),
            (DialogFocusKind::Stats, self.model.stats_open),
            (
                DialogFocusKind::TabClose,
                self.model.pending_close.is_some(),
            ),
            (
                DialogFocusKind::WorkspaceRemove,
                self.pending_workspace_remove.is_some(),
            ),
        ]
        .into_iter()
        .filter_map(|(kind, open)| open.then_some(kind))
        .collect();
        let rendered = &desired[desired
            .len()
            .saturating_sub(enzui_widgets::modal_host::DIALOG_STACK_DEPTH)..];
        for kind in [
            DialogFocusKind::Shortcuts,
            DialogFocusKind::Stats,
            DialogFocusKind::TabClose,
            DialogFocusKind::WorkspaceRemove,
        ] {
            if !rendered.contains(&kind) {
                let _ = self.dialogs.close(&kind);
            }
        }
        for kind in rendered {
            if !self.dialogs.is_open(kind) {
                let invoker = self.stack_invoker(*kind);
                tasks.push(self.dialogs.open(*kind, invoker));
            }
        }
        tasks
    }

    fn cancel_stack_dialog(&mut self, kind: DialogFocusKind) {
        match kind {
            DialogFocusKind::Stats => self.model.stats_open = false,
            DialogFocusKind::Shortcuts => self.model.shortcuts_open = false,
            DialogFocusKind::TabClose => {
                self.model.pending_close = None;
                self.close_confirm_state = enzui_core::confirm::ConfirmState::new(None);
            }
            DialogFocusKind::WorkspaceRemove => {
                self.pending_workspace_remove = None;
                self.workspace_remove_confirm_state = enzui_core::confirm::ConfirmState::new(None);
            }
            _ => {}
        }
    }

    pub(crate) fn close_stack_dialog(&mut self, kind: DialogFocusKind) -> Task<Message> {
        let (closed, focus) = self.dialogs.dismiss(&kind, DismissCause::CloseButton);
        for kind in closed {
            self.cancel_stack_dialog(kind);
        }
        focus
    }

    pub(crate) fn reduce_stack_host(
        &mut self,
        event: enzui_widgets::modal_host::DialogEvent<DialogFocusKind>,
    ) -> Task<Message> {
        let (closed, focus) = self.dialogs.update(event);
        for kind in closed {
            self.cancel_stack_dialog(kind);
        }
        focus
    }

    pub(crate) fn reduce_dialog_host(&mut self, event: ModalHostEvent) -> Task<Message> {
        let Some((_, lifecycle)) = self.dialog_focus.as_ref() else {
            return Task::none();
        };
        let outcome = lifecycle.reduce::<Message>(event);
        if outcome.dismissed().is_none() {
            return outcome.into_task();
        }
        self.dialog_focus = None;
        let state_task = if matches!(
            self.recovery_discard,
            Some(crate::RecoveryDiscard::Failed { .. })
        ) {
            // Dismissing the tombstone-failure prompt (Escape/scrim) is
            // "Keep window open" — abandon the discard, leave recovery intact.
            self.recovery_discard = None;
            let action = self
                .close_guard
                .decide(enzui_core::close_guard::CloseDecision::Cancel);
            self.apply_close_guard_action(action)
        } else if self.close_guard.is_pending() {
            let action = self
                .close_guard
                .decide(enzui_core::close_guard::CloseDecision::Cancel);
            self.apply_close_guard_action(action)
        } else {
            Task::none()
        };
        Task::batch([outcome.into_task(), state_task])
    }
}
