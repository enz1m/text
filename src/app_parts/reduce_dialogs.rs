//! TextApp goto, stats and settings reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_goto_open_goto(&mut self) -> Task<Message> {
        // A paged document's lines are not indexed.
        if self.refuse_capability(large_doc::Capability::Goto) {
            return Task::none();
        }
        if self.model.search.open {
            let _ = self.reduce_find_close_find();
        }
        self.model.goto_open = true;
        self.model.goto_query.clear();
        // `enz_text_field` has no focus-on-mount route.
        iced::widget::operation::focus(goto_field_id())
    }
    pub(crate) fn reduce_goto_close_goto(&mut self) -> Task<Message> {
        self.model.goto_open = false;
        // Closing the sibling bar drops editor focus.
        restore_focus_task(editor_id())
    }
    pub(crate) fn reduce_goto_goto_query_changed(&mut self, q: String) -> Task<Message> {
        self.model.goto_query = q.chars().filter(char::is_ascii_digit).collect();
        Task::none()
    }
    pub(crate) fn reduce_goto_goto_submit(&mut self) -> Task<Message> {
        // A tab switch or direct message must not clamp against an empty
        // paged snapshot.
        if self.refuse_capability(large_doc::Capability::Goto) {
            self.model.goto_open = false;
            return Task::none();
        }
        if let Ok(line) = self.model.goto_query.parse::<usize>() {
            self.goto_line(line);
        }
        self.model.goto_open = false;
        restore_focus_task(editor_id())
    }
    pub(crate) fn reduce_goto_open_stats(&mut self) -> Task<Message> {
        let active = self.model.active_tab_id.clone();
        self.stats_retry_blocked.remove(&active);
        self.selection_stats_retry_blocked.remove(&active);
        // A paged tab has no snapshot to count.
        if self.refuse_capability(large_doc::Capability::Stats) {
            return Task::none();
        }
        self.model.stats_open = true;
        // The selection is already settled, so the first count skips the
        // debounce; later counts go through the settle tick.
        self.selection_settle_probe = None;
        self.spawn_selection_stats();
        Task::none()
    }
    pub(crate) fn reduce_goto_close_stats(&mut self) -> Task<Message> {
        self.model.stats_open = false;
        self.close_stack_dialog(crate::dialog_focus::DialogFocusKind::Stats)
    }
    pub(crate) fn reduce_goto_open_settings(&mut self) -> Task<Message> {
        self.model.settings_open = true;
        Task::none()
    }
    pub(crate) fn reduce_goto_close_settings(&mut self) -> Task<Message> {
        self.model.settings_open = false;
        self.dismiss_settings_dialog(enzui_core::modal::DismissCause::CloseButton)
    }
    pub(crate) fn reduce_goto_settings_exit_settled(
        &mut self,
        key: enzui_widgets::modal_host::ModalPresentationKey,
    ) -> Task<Message> {
        self.finish_settings_exit(key);
        Task::none()
    }
    pub(crate) fn reduce_goto_open_shortcuts(&mut self) -> Task<Message> {
        self.model.shortcuts_open = true;
        Task::none()
    }
    pub(crate) fn reduce_goto_close_shortcuts(&mut self) -> Task<Message> {
        self.model.shortcuts_open = false;
        self.close_stack_dialog(crate::dialog_focus::DialogFocusKind::Shortcuts)
    }
    pub(crate) fn reduce_goto_set_font_size(&mut self, scale: FontSizeScale) -> Task<Message> {
        self.model.settings.font_size = scale;
        self.save_settings()
    }
    pub(crate) fn reduce_goto_toggle_restore_session(&mut self) -> Task<Message> {
        self.model.settings.restore_session = !self.model.settings.restore_session;
        self.save_settings()
    }
    pub(crate) fn reduce_goto_toggle_language_menu(&mut self) -> Task<Message> {
        if self.refuse_capability(large_doc::Capability::Syntax) {
            return Task::none();
        }
        self.model.language_menu_open = !self.model.language_menu_open;
        Task::none()
    }
    pub(crate) fn reduce_goto_set_language(&mut self, label: String) -> Task<Message> {
        // The paged viewer runs no highlighter.
        if self.refuse_capability(large_doc::Capability::Syntax) {
            self.model.language_menu_open = false;
            return Task::none();
        }
        self.model.active_tab_mut().language_override = Some(label);
        self.model.language_menu_open = false;
        restore_focus_task(language_pill_id())
    }
    pub(crate) fn reduce_goto_cycle_line_ending(&mut self) -> Task<Message> {
        // Changing the line ending dirties the tab, which a read-only tab
        // cannot save.
        if self.refuse_capability(large_doc::Capability::LineEnding) {
            return Task::none();
        }
        self.model.active_tab_mut().cycle_line_ending();
        Task::none()
    }
    pub(crate) fn reduce_goto_set_line_ending(
        &mut self,
        ending: line_ending::LineEnding,
    ) -> Task<Message> {
        if self.refuse_capability(large_doc::Capability::LineEnding) {
            return Task::none();
        }
        self.model.active_tab_mut().set_line_ending(ending);
        Task::none()
    }
}
