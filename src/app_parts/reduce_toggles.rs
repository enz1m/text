//! TextApp settings toggle and close-confirm reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_toggles_toggle_counts(&mut self) -> Task<Message> {
        self.model.settings.show_counts = !self.model.settings.show_counts;
        self.save_settings()
    }
    pub(crate) fn reduce_toggles_toggle_line_numbers(&mut self) -> Task<Message> {
        self.model.settings.show_line_numbers = !self.model.settings.show_line_numbers;
        self.save_settings()
    }
    pub(crate) fn reduce_toggles_toggle_word_wrap(&mut self) -> Task<Message> {
        // The paged viewer draws unwrapped rows; the settings switch is also
        // disabled.
        if self.refuse_capability(large_doc::Capability::WordWrap) {
            return Task::none();
        }
        self.model.settings.word_wrap = !self.model.settings.word_wrap;
        self.save_settings()
    }
    // The explicit editor Tab mode (Ctrl+M) is runtime-only and never
    // persisted.
    pub(crate) fn reduce_toggles_toggle_tab_indent_mode(&mut self) -> Task<Message> {
        self.tab_indent_mode = !self.tab_indent_mode;
        Task::none()
    }
    pub(crate) fn reduce_toggles_set_tab_indent_mode(&mut self, on: bool) -> Task<Message> {
        self.tab_indent_mode = on;
        Task::none()
    }
    pub(crate) fn reduce_toggles_close_confirm_event(
        &mut self,
        event: ConfirmDialogEvent,
    ) -> Task<Message> {
        // `from_cause` maps every dialog exit to consent: only the confirm
        // button closes. Focus events belong to the modal host.
        match event {
            ConfirmDialogEvent::Resolved(cause) => {
                let confirmed =
                    enzui_core::confirm::ConfirmDecision::from_cause(cause).is_confirmed();
                if let Some(id) = self.model.pending_close.take()
                    && confirmed
                {
                    self.model.close_tab(&id);
                    self.abort_search_scan();
                    self.advance_session_projection_revision();
                }
                return self.close_stack_dialog(crate::dialog_focus::DialogFocusKind::TabClose);
            }
            ConfirmDialogEvent::CheckboxToggled(_)
            | ConfirmDialogEvent::FocusNext
            | ConfirmDialogEvent::FocusPrevious => {}
        }
        Task::none()
    }
}
