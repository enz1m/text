//! TextApp document load reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_document_document_loaded(
        &mut self,
        tab_id: String,
        workspace: Option<PathBuf>,
        path: PathBuf,
        token: large_doc::LoadToken,
        reason: LoadReason,
        result: Result<large_doc::LoadedDocument, document_failure::DocumentFailure>,
    ) -> Task<Message> {
        if self
            .load_debt
            .get(&tab_id)
            .is_some_and(|pending| pending.path == path && pending.token == token)
        {
            self.load_debt.remove(&tab_id);
        }
        let task = self.finish_document_load(&tab_id, &path, workspace, token, reason, result);
        // A reload of the active tab retires the search binding computed
        // against the previous generation, so no stale highlight paints and no
        // stale replace lands on the new bytes.
        if tab_id == self.model.active_tab_id {
            self.retire_search_for_active_change();
        }
        task
    }
    pub(crate) fn reduce_document_paged_page_loaded(
        &mut self,
        tab_id: String,
        token: large_doc::LoadToken,
        result: enzui_core::paged_text::PagedTextResult,
    ) -> Task<Message> {
        self.paged_retry_blocked.remove(&tab_id);
        // `token` rejects a page whose tab was closed, reloaded or reopened; the
        // framework's `apply` rejects a superseded document identity/generation.
        let Some(tab) = self
            .model
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id && tab.load_token == token)
        else {
            return Task::none();
        };
        let Some(doc) = tab.buffer.large_mut() else {
            return Task::none();
        };
        match doc.apply_page(result) {
            Some(operation) => self.paged_page_task(tab_id, token, operation),
            None => Task::none(),
        }
    }
    pub(crate) fn reduce_document_paged_navigate(
        &mut self,
        movement: enzui_core::paged_text::PagedTextNavigation,
    ) -> Task<Message> {
        let id = self.model.active_tab_id.clone();
        let Some(tab) = self.model.tabs.iter_mut().find(|tab| tab.id == id) else {
            return Task::none();
        };
        let token = tab.load_token;
        let Some(doc) = tab.buffer.large_mut() else {
            return Task::none();
        };
        match doc.navigate(movement) {
            Some(operation) => self.paged_page_task(id, token, operation),
            None => Task::none(),
        }
    }
}
