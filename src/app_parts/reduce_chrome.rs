//! TextApp overflow menu and sidebar reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_chrome_toggle_overflow_menu(&mut self) -> Task<Message> {
        self.model.overflow_menu_open = !self.model.overflow_menu_open;
        Task::none()
    }
    pub(crate) fn reduce_chrome_close_overflow_menu(&mut self) -> Task<Message> {
        self.model.overflow_menu_open = false;
        Task::none()
    }
    pub(crate) fn reduce_chrome_toggle_tab_list(&mut self) -> Task<Message> {
        self.model.tab_list_open = !self.model.tab_list_open;
        Task::none()
    }
    // The single dismissal path for anchored popovers; a completed click
    // outside closes them. Idempotent.
    pub(crate) fn reduce_chrome_dismiss_popovers(&mut self) -> Task<Message> {
        // The language menu returns focus to its trigger; `enz_titlebar_actions`
        // restores focus for the overflow menu itself.
        let restore_language = self.model.language_menu_open;
        // The tab list restores focus to its own chip.
        let restore_tab_list = !restore_language && self.model.tab_list_open;
        self.model.language_menu_open = false;
        self.model.overflow_menu_open = false;
        self.model.tab_list_open = false;
        if restore_language {
            restore_focus_task(language_pill_id())
        } else if restore_tab_list {
            restore_focus_task(tab_list_button_id())
        } else {
            Task::none()
        }
    }
    pub(crate) fn reduce_chrome_sidebar_state(&mut self, state: NavSplitState) -> Task<Message> {
        self.nav = state;
        // Compact drawer visibility must not replace the persisted desktop preference.
        let visible = self.nav.preference != SidebarPreference::Hidden;
        if self.model.settings.sidebar_visible != visible {
            self.model.settings.sidebar_visible = visible;
            return self.save_settings();
        }
        Task::none()
    }
    pub(crate) fn reduce_chrome_dismiss_sidebar(&mut self) -> Task<Message> {
        self.nav.dismiss_overlay();
        Task::none()
    }
}
