//! Save-completion reducers.

use super::*;

impl TextApp {
    pub(crate) fn reduce_save_save_completed(
        &mut self,
        tab: String,
        generation: u64,
        destination: std::path::PathBuf,
        rebind: Option<std::path::PathBuf>,
        guard_attempt: Option<SaveAttemptToken>,
        result: Result<Option<FileIdentity>, document_failure::DocumentFailure>,
    ) -> Task<Message> {
        let outcome = self.finish_save(&tab, generation, destination, rebind, result);
        if let Some(attempt) = guard_attempt {
            if outcome == SaveOutcome::Failed {
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-a-document-was-a7581c",
                ));
            }
            // The close guard has held the close since the write started.
            if let Ok(action) = self.close_guard.save_finished(&tab, &attempt, outcome) {
                return self.apply_close_guard_action(action);
            }
        }
        Task::none()
    }
    pub(crate) fn reduce_save_editor_drag_updated(
        &mut self,
        tab: String,
        cursor: text_editor::Cursor,
    ) -> Task<Message> {
        // The notification may arrive after a newer drag or tab switch, so the
        // owner may be inactive or gone. A matching inactive owner still gets
        // its generation bump; only the active tab updates the status bar.
        let Some(index) = self
            .model
            .tabs
            .iter()
            .position(|candidate| candidate.id == tab)
        else {
            return Task::none();
        };
        if self.model.tabs[index].content.cursor() != cursor {
            return Task::none();
        }
        self.model.tabs[index].sync_virtual_selection();
        self.model.tabs[index].note_editor_action();
        if self.model.active_tab_id == tab {
            self.sync_cursor_readout();
        }
        Task::none()
    }
}
