//! TextApp find and replace reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_find_toggle_find(&mut self) -> Task<Message> {
        if self.model.search.open {
            self.update_inner(Message::CloseFind)
        } else {
            self.update_inner(Message::OpenFind)
        }
    }
    pub(crate) fn reduce_find_open_find(&mut self) -> Task<Message> {
        // A paged document is not indexed; the bar does not open.
        if self.refuse_capability(large_doc::Capability::Find) {
            return Task::none();
        }
        self.model.goto_open = false;
        self.model.search.open = true;
        self.model.search.replace_open = false;
        self.open_search_at_caret();
        // `focus_on_mount` cannot unfocus the editor; without this tree-wide
        // focus both would be focused and a keystroke would land in both.
        iced::widget::operation::focus(find_field_id())
    }
    pub(crate) fn reduce_find_open_replace(&mut self) -> Task<Message> {
        // Replace needs Find's match set.
        if self.refuse_capability(large_doc::Capability::Find) {
            return Task::none();
        }
        self.model.goto_open = false;
        self.model.search.open = true;
        self.model.search.replace_open = true;
        self.open_search_at_caret();
        // As in OpenFind, `focus_on_mount` cannot unfocus the editor.
        iced::widget::operation::focus(find_field_id())
    }
    pub(crate) fn reduce_find_close_find(&mut self) -> Task<Message> {
        self.model.search.open = false;
        self.model.search.replace_open = false;
        self.abort_search_scan();
        self.model.search.release_results();
        // Unmounting the sibling find bar invalidates editor focus, so
        // restore it explicitly after the tree changes.
        restore_focus_task(editor_id())
    }
    pub(crate) fn reduce_find_find_query_changed(&mut self, q: String) -> Task<Message> {
        // Guards programmatic routes against recomputing over an empty paged
        // snapshot.
        if self.refuse_capability(large_doc::Capability::Find) {
            return Task::none();
        }
        self.model.search.query = q;
        self.recompute_search();
        Task::none()
    }
    pub(crate) fn reduce_find_replace_value_changed(&mut self, v: String) -> Task<Message> {
        self.model.search.replacement = v;
        Task::none()
    }
    pub(crate) fn reduce_find_find_next(&mut self) -> Task<Message> {
        // F3 and Enter reach this without the bar open.
        if self.refuse_capability(large_doc::Capability::FindStep) {
            return Task::none();
        }
        self.model.search.next();
        self.reveal_current_match();
        Task::none()
    }
    pub(crate) fn reduce_find_find_prev(&mut self) -> Task<Message> {
        if self.refuse_capability(large_doc::Capability::FindStep) {
            return Task::none();
        }
        self.model.search.prev();
        self.reveal_current_match();
        Task::none()
    }
    fn reveal_current_match(&mut self) {
        let Some(found) = self
            .model
            .search
            .matches
            .get(self.model.search.current)
            .copied()
        else {
            return;
        };
        self.model
            .active_tab_mut()
            .reveal_byte_range(found.start, found.end);
        self.sync_cursor_readout();
    }
    pub(crate) fn reduce_find_replace_current(&mut self) -> Task<Message> {
        if self.refuse_replace_if_read_only() {
            return Task::none();
        }
        // Refuse before `load_active_buffer_mapped` rebuilds native `Content`.
        if self.refuse_if_authority_exhausted() {
            return Task::none();
        }
        self.replace_current();
        Task::none()
    }
    pub(crate) fn reduce_find_replace_all(&mut self) -> Task<Message> {
        if self.refuse_replace_if_read_only() {
            return Task::none();
        }
        // Refuse before any buffer rebuild is admitted.
        if self.refuse_if_authority_exhausted() {
            return Task::none();
        }
        self.replace_all();
        Task::none()
    }
    pub(crate) fn reduce_find_toggle_case_sensitive(&mut self) -> Task<Message> {
        if self.refuse_capability(large_doc::Capability::Find) {
            return Task::none();
        }
        self.model.search.options.case_sensitive = !self.model.search.options.case_sensitive;
        self.recompute_search();
        Task::none()
    }
    pub(crate) fn reduce_find_toggle_whole_word(&mut self) -> Task<Message> {
        if self.refuse_capability(large_doc::Capability::Find) {
            return Task::none();
        }
        self.model.search.options.whole_word = !self.model.search.options.whole_word;
        self.recompute_search();
        Task::none()
    }
    pub(crate) fn reduce_find_toggle_regex(&mut self) -> Task<Message> {
        if self.refuse_capability(large_doc::Capability::Find) {
            return Task::none();
        }
        self.model.search.options.regex = !self.model.search.options.regex;
        self.recompute_search();
        Task::none()
    }
    pub(crate) fn reduce_find_set_replace_scope(&mut self, idx: usize) -> Task<Message> {
        self.model.search.replace_scope = search::ReplaceScope::ALL
            .get(idx)
            .copied()
            .unwrap_or_default();
        Task::none()
    }
    pub(crate) fn reduce_find_toggle_replace_bar(&mut self) -> Task<Message> {
        if self.refuse_capability(large_doc::Capability::Replace) {
            return Task::none();
        }
        if self.model.search.replace_open {
            self.model.search.replace_open = false;
        } else {
            self.model.goto_open = false;
            self.model.search.open = true;
            self.model.search.replace_open = true;
        }
        Task::none()
    }
}
