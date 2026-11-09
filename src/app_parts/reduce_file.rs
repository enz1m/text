//! TextApp file save, open and picker reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_file_save_file(&mut self) -> Task<Message> {
        let id = self.model.active_tab_id.clone();
        // Refused here so neither Save nor the untitled Save As fall-through
        // can run on a read-only tab.
        if self.refuse_write_if_read_only(&id) {
            return Task::none();
        }
        if self.model.active_tab().file_path.is_some() {
            let _ = self.save_tab(&id);
        } else {
            // Ctrl+S on an untitled buffer is Save As.
            let _ = self.start_save_as();
        }
        Task::none()
    }
    // Shares `PickerTick` with Open; the picker tags the operation so the
    // outcome reaches `finish_pick_save`, which also rebinds `file_path` so the
    // next Ctrl+S targets the new path.
    pub(crate) fn reduce_file_save_file_as(&mut self) -> Task<Message> {
        let id = self.model.active_tab_id.clone();
        if self.refuse_write_if_read_only(&id) {
            return Task::none();
        }
        let _ = self.start_save_as();
        Task::none()
    }
    pub(crate) fn reduce_file_open_file(&mut self) -> Task<Message> {
        // A refusal (no portal, undeclared capability) goes through the same
        // handler as a late outcome so the user sees it.
        if let Err(outcome) = self.picker.start() {
            return self.finish_pick(outcome);
        }
        Task::none()
    }
    pub(crate) fn reduce_file_picker_tick(&mut self) -> Task<Message> {
        if let Some((op, outcome)) = self.picker.pump() {
            match op {
                picker::PickerOp::Open => return self.finish_pick(outcome),
                picker::PickerOp::Workspace => self.finish_workspace_pick(outcome),
                picker::PickerOp::SaveAs => {
                    let guarded_key = self
                        .close_guard
                        .is_saving()
                        .then(|| {
                            self.close_guard
                                .pending_items()
                                .next()
                                .map(|item| item.key.clone())
                        })
                        .flatten();
                    let drain = guarded_key
                        .as_ref()
                        .and(self.close_request_window)
                        .map_or_else(Task::none, |window| self.close_window(window));
                    let result = self.finish_pick_save(outcome);
                    if let Some(key) = guarded_key {
                        let outcome = match result {
                            SaveAsFinish::Cancelled => SaveOutcome::Cancelled,
                            SaveAsFinish::Failed => SaveOutcome::Failed,
                            // Still writing. `SaveCompleted`
                            // resumes the guard when the file is real.
                            SaveAsFinish::InFlight => return drain,
                        };
                        let attempt = self
                            .close_save_attempt
                            .take()
                            .expect("the guarded Save As attempt must still be pending");
                        let action = self
                            .close_guard
                            .save_finished(&key, &attempt, outcome)
                            .expect("the guarded Save As key must still be pending");
                        if outcome != SaveOutcome::Saved {
                            self.veto_close_for_durability(enzui_core::localization::text(
                                "text-close-was-cancelled-because-save-as-did-b76ca8",
                            ));
                        }
                        return Task::batch([drain, self.apply_close_guard_action(action)]);
                    }
                }
            }
        }
        self.picker.pump_location();
        Task::none()
    }
    pub(crate) fn reduce_file_dismiss_picker_failure(&mut self) -> Task<Message> {
        self.picker.dismiss_failure();
        // The tab-scoped exhaustion banner is authoritative; the picker-failure
        // surface only repeats it for accessibility, so republish while the tab
        // is still exhausted to keep dismissal from hiding the state.
        if self.model.active_tab().authority_exhausted() {
            self.publish_authority_exhausted_sentence();
        }
        Task::none()
    }
}
