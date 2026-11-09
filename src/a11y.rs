//! Process-wide AccessKit routing for Text's single native window.
//!
//! The virtual-text tree builder receives the exact publishing iced window id,
//! so requests resolve against that one observed map rather than guessing a
//! node id or using `window::latest()`.

use enzui_core::a11y_action::AccessibilityAction;
use enzui_core::app_runtime;
use iced::Task;

use crate::{Message, TextApp, editor_id};

// Per instance: the provider registry is keyed by this id process-wide, so a
// shared id would let a second live app evict this app's binding.
pub(crate) fn fresh_virtual_text_surface_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::unique()
}

// The message carries the bootstrap's opaque `AccessibilityWake` receipt, so
// the handler drains exactly the batch it was woken for and a late message
// whose batch an ordinary turn already consumed cannot force a stale empty
// drain.
pub(crate) fn subscription() -> iced::Subscription<Message> {
    app_runtime::accessibility_wake_subscription(Message::A11yWake)
}

impl TextApp {
    pub(super) fn drain_virtual_text_mutations(&mut self) -> bool {
        use enzui_core::virtual_text::{MutationKind, MutationReply};

        let mut any_bytes_changed = false;
        for tab in &mut self.model.tabs {
            let Some(provider) = tab.virtual_text_document() else {
                continue;
            };
            let mut tab_bytes_changed = false;
            for pending in provider.mailbox().drain() {
                let _ = pending.resolve_with(|request| {
                    let changes_bytes = matches!(
                        request.mutation(),
                        MutationKind::Replace { .. } | MutationKind::CommitComposition { .. }
                    );
                    let reply = tab.apply_virtual_text_request(&provider, request);
                    if changes_bytes && matches!(reply, MutationReply::Accepted { .. }) {
                        tab_bytes_changed = true;
                    }
                    reply
                });
            }
            tab.clear_finished_virtual_composition();
            if tab_bytes_changed {
                tab.dirty = true;
                any_bytes_changed = true;
            }
        }
        if any_bytes_changed && self.model.search.open {
            self.recompute_search();
        }
        any_bytes_changed
    }

    // Publish app-originated reducer state to each live provider.
    //
    // Not a mutation transport: provider-originated work is consumed only by
    // `drain_virtual_text_mutations`.
    pub(super) fn publish_virtual_text_state(&mut self) {
        for tab in &mut self.model.tabs {
            if let Err(error) = tab.publish_virtual_text_state() {
                eprintln!("a11y: virtual document confirmation refused: {error:?}");
            }
        }
    }

    // Route a production iced IME event into the document-coordinate owner.
    //
    // The surface intercepts commit, so only this path writes committed bytes.
    pub(super) fn handle_virtual_input_method(
        &mut self,
        event: iced::advanced::input_method::Event,
    ) {
        let result = match event {
            iced::advanced::input_method::Event::Opened => {
                self.model.active_tab_mut().begin_virtual_composition()
            }
            iced::advanced::input_method::Event::Preedit(value, _) => self
                .model
                .active_tab_mut()
                .update_virtual_composition(&value),
            iced::advanced::input_method::Event::Commit(value) => {
                let tab = self.model.active_tab_mut();
                tab.update_virtual_composition(&value)
                    .and_then(|()| tab.enqueue_virtual_composition_commit())
            }
            iced::advanced::input_method::Event::Closed => {
                self.model.active_tab_mut().cancel_virtual_composition();
                Ok(())
            }
        };
        if let Err(error) = result {
            eprintln!("a11y: virtual editor IME event refused: {error:?}");
        }
    }

    // Open every reducer turn with the accessibility work that arrived
    // between turns.
    //
    // A wake message drains exactly its receipt's batch; any other message
    // drains everything. The mailbox is drained in the same turn because a
    // native caller is blocked on its ticket.
    pub(super) fn begin_a11y_turn(&mut self, message: &Message) -> Task<Message> {
        use enzui_core::app_runtime as a11y_rt;

        let actions = match message {
            Message::A11yWake(receipt) => {
                a11y_rt::drain_accessibility_wake(receipt.clone()).unwrap_or_default()
            }
            _ => a11y_rt::drain_accessibility_actions(),
        };
        let task = self.apply_a11y_actions(actions);
        self.drain_virtual_text_mutations();
        task
    }

    // Keep the one stable surface id bound to the active chunked tab's
    // current strong provider owner.
    pub(super) fn sync_virtual_text_binding(&mut self) {
        let desired = self.model.active_tab().virtual_text_document();
        let unchanged = desired.as_ref().is_some_and(|document| {
            self.bound_virtual_text
                .as_ref()
                .is_some_and(|bound| std::sync::Arc::ptr_eq(bound, document))
        });
        if unchanged {
            return;
        }
        self.bound_virtual_text_wake = None;
        match desired {
            Some(document) => {
                enzui_core::virtual_text::bind_provider_to_widget(
                    self.virtual_text_surface.clone(),
                    &document,
                );
                // The text protocol bypasses the action router, so the
                // bootstrap's router wake cannot signal this mailbox; the shared
                // channel gives the mutation a turn under the same receipt.
                self.bound_virtual_text_wake = Some(
                    document
                        .mailbox()
                        .set_wake(app_runtime::notify_accessibility_wake),
                );
                self.bound_virtual_text = Some(document);
            }
            None => {
                // Only the owner may unbind: the registry is process-wide, so an
                // unconditional unbind would tear down another live app's binding.
                if self.bound_virtual_text.take().is_some() {
                    enzui_core::virtual_text::unbind_provider_from_widget(
                        &self.virtual_text_surface,
                    );
                }
            }
        }
    }

    // Turn the AccessKit action queue into the widget operations that
    // keyboard and pointer input use.
    //
    // The virtual text range protocol is served by the bound document.
    pub(super) fn apply_a11y_actions(
        &mut self,
        actions: Vec<app_runtime::RoutedAccessibilityAction>,
    ) -> Task<Message> {
        use iced::advanced::widget::operation::focusable::focus;

        let mut tasks = Vec::new();
        for routed in actions {
            // Scope every operation to the publishing window: `focus` is
            // destructive in a multi-window process.
            let window = routed.window();
            match routed.action().clone() {
                AccessibilityAction::Focus(id) if id == self.virtual_text_surface => {
                    tasks.push(app_runtime::window_operation_task(
                        window,
                        focus(editor_id()),
                    ));
                }
                AccessibilityAction::SetValue { id, value } if id == self.virtual_text_surface => {
                    if let Some(document) = self.model.active_tab().virtual_text_document()
                        && let Ok(summary) = document.summary()
                        && let Err(error) = document.enqueue_mutation(
                            summary.document,
                            summary.revision,
                            enzui_core::virtual_text::VirtualTextAction::Replace {
                                range: 0..summary.characters,
                                value,
                            },
                            &enzui_core::virtual_text::VirtualTextCancellation::default(),
                        )
                    {
                        eprintln!("a11y: virtual editor SetValue refused: {error:?}");
                    }
                    tasks.push(app_runtime::window_operation_task(
                        window,
                        focus(editor_id()),
                    ));
                }
                AccessibilityAction::SetValueRefused { id, .. }
                    if id == self.virtual_text_surface =>
                {
                    tasks.push(app_runtime::window_operation_task(
                        window,
                        focus(editor_id()),
                    ));
                }
                _ => tasks.push(enzui_widgets::a11y_actions::apply_accessibility_action(
                    &routed,
                )),
            }
        }
        Task::batch(tasks)
    }
}
