//! TextApp tab reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_tab_new_tab(&mut self) -> Task<Message> {
        let id = self.model.fresh_tab_id();
        self.model.add_tab(Tab::new(id));
        self.advance_session_projection_revision();
        self.sync_cursor_readout();
        restore_focus_task(editor_id())
    }
    pub(crate) fn reduce_tab_close_tab(&mut self, id: String) -> Task<Message> {
        if self.model.request_close(&id) {
            // The single search worker may belong to any tab; cancel it on every
            // close so no closed generation stays retained.
            self.abort_search_scan();
            self.advance_session_projection_revision();
        }
        Task::none()
    }
    pub(crate) fn reduce_tab_close_active_tab(&mut self) -> Task<Message> {
        let id = self.model.active_tab_id.clone();
        if self.model.request_close(&id) {
            self.abort_search_scan();
            self.advance_session_projection_revision();
        }
        Task::none()
    }
    // Each tab owns its `Content`, so the caret is per tab, but every switch
    // route must end at `sync_cursor_readout`.
    pub(crate) fn reduce_tab_switch_tab(&mut self, id: String) -> Task<Message> {
        if self.model.tabs.iter().any(|t| t.id == id) {
            self.tab_strip.request_reveal();
            let switching = id != self.model.active_tab_id;
            self.model.active_tab_id = id;
            self.sync_cursor_readout();
            if switching {
                self.retire_search_for_active_change();
                self.advance_session_projection_revision();
            }
        }
        // Picking a document from the tab list closes the list.
        self.model.tab_list_open = false;
        Task::none()
    }
    pub(crate) fn reduce_tab_next_tab(&mut self) -> Task<Message> {
        if self.model.tabs.len() > 1 {
            let idx = self
                .model
                .tabs
                .iter()
                .position(|t| t.id == self.model.active_tab_id)
                .unwrap_or(0);
            let next = (idx + 1) % self.model.tabs.len();
            let next_id = self.model.tabs[next].id.clone();
            let switching = next_id != self.model.active_tab_id;
            self.model.active_tab_id = next_id;
            self.sync_cursor_readout();
            if switching {
                self.retire_search_for_active_change();
                self.advance_session_projection_revision();
            }
        }
        Task::none()
    }
    pub(crate) fn reduce_tab_prev_tab(&mut self) -> Task<Message> {
        if self.model.tabs.len() > 1 {
            let idx = self
                .model
                .tabs
                .iter()
                .position(|t| t.id == self.model.active_tab_id)
                .unwrap_or(0);
            let prev = if idx == 0 {
                self.model.tabs.len() - 1
            } else {
                idx - 1
            };
            let prev_id = self.model.tabs[prev].id.clone();
            let switching = prev_id != self.model.active_tab_id;
            self.model.active_tab_id = prev_id;
            self.sync_cursor_readout();
            if switching {
                self.retire_search_for_active_change();
                self.advance_session_projection_revision();
            }
        }
        Task::none()
    }
}
