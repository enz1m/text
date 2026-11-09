//! Editor-action reducer.

use super::*;

use crate::text_editor::Action as EditorAction;

impl TextApp {
    pub(crate) fn reduce_blocking_failed(&mut self, failure: BlockingWorkFailure) -> Task<Message> {
        match failure {
            BlockingWorkFailure::Search {
                generation,
                from,
                error,
            } => {
                self.search_scan = None;
                let exact = self
                    .search_attempt
                    .take()
                    .filter(|attempt| attempt.generation == generation);
                if exact.is_none() && error == BlockingTaskError::Cancelled {
                    return Task::none();
                }
                if let Some(attempt) = exact {
                    self.search_retry = Some(attempt);
                }
                self.search_queued = Some(from);
                self.model.search.searching = true;
                if error == BlockingTaskError::Busy {
                    self.blocking_retry_pending = true;
                } else {
                    self.search_retry_blocked = true;
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-search-stopped-unexpectedly-retry-the-search-eee60c",
                    ));
                }
            }
            BlockingWorkFailure::Stats {
                tab,
                generation,
                error,
            } => {
                let exact = self
                    .stats_attempts
                    .remove(&tab)
                    .filter(|attempt| attempt.generation == generation);
                if exact.is_none() && error == BlockingTaskError::Cancelled {
                    return Task::none();
                }
                if let Some(attempt) = exact {
                    self.stats_retry.insert(tab.clone(), attempt);
                }
                if error == BlockingTaskError::Busy {
                    self.blocking_retry_pending = true;
                } else {
                    self.stats_retry_blocked.insert(tab.clone());
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-statistics-could-not-be-calculated-retry-statistics-753054",
                    ));
                }
                if let Some(tab) = self
                    .model
                    .tabs
                    .iter_mut()
                    .find(|candidate| candidate.id == tab)
                {
                    tab.release_stats_claim();
                }
            }
            BlockingWorkFailure::SelectionStats { tab, key, error } => {
                let exact = self
                    .selection_stats_attempts
                    .remove(&tab)
                    .filter(|attempt| attempt.key == key);
                if exact.is_none() && error == BlockingTaskError::Cancelled {
                    return Task::none();
                }
                if let Some(attempt) = exact {
                    self.selection_stats_retry.insert(tab.clone(), attempt);
                }
                if let Some(index) = self
                    .model
                    .tabs
                    .iter()
                    .position(|candidate| candidate.id == tab)
                {
                    self.model.tabs[index].release_selection_stats_claim();
                }
                if error == BlockingTaskError::Busy {
                    self.blocking_retry_pending = true;
                } else {
                    self.selection_stats_retry_blocked.insert(tab.clone());
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-selection-statistics-could-not-be-calculated-retry-f73751",
                    ));
                }
            }
            BlockingWorkFailure::Save {
                tab,
                generation,
                destination,
                rebind,
                guard_attempt,
                error,
            } => {
                self.save_destinations.remove(&destination);
                let exact_request = self.save_inflight.remove(&tab).filter(|request| {
                    request.generation == generation && request.destination == destination
                });
                if let Some(index) = self
                    .model
                    .tabs
                    .iter()
                    .position(|candidate| candidate.id == tab)
                {
                    self.model.tabs[index].saving = false;
                    self.model.tabs[index].save_queued = Some(rebind);
                }
                if let Some(attempt) = guard_attempt {
                    self.close_save_attempt = Some(attempt);
                }
                if let Some(request) = exact_request {
                    self.save_debt.insert(tab.clone(), request);
                }
                if error == BlockingTaskError::Busy {
                    self.blocking_retry_pending = true;
                } else {
                    self.save_retry_blocked.insert(tab);
                    self.veto_close_for_durability(enzui_core::localization::text(
                        "text-close-was-cancelled-because-a-document-save-165de1",
                    ));
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-the-document-was-not-saved-retry-save-d67dd9",
                    ));
                }
            }
            BlockingWorkFailure::Print { generation, error } => {
                self.print_preparing = false;
                if error == BlockingTaskError::Busy {
                    self.blocking_retry_pending = true;
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-printing-is-busy-retry-is-queued-28d00e",
                    ));
                } else if error == BlockingTaskError::Cancelled
                    && self.print.current_generation() != Some(generation)
                {
                    self.print_retry = None;
                } else {
                    self.print_retry_blocked = true;
                    self.print.render_failed(generation, None);
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-printing-stopped-unexpectedly-retry-print-1e4943",
                    ));
                }
            }
            BlockingWorkFailure::Load {
                tab,
                path,
                workspace,
                token,
                reason,
                error,
            } => {
                let current = self.model.tabs.iter().any(|candidate| {
                    candidate.id == tab
                        && candidate.file_path.as_deref() == Some(&path)
                        && candidate.load_token == token
                });
                if !current && error == BlockingTaskError::Cancelled {
                    return Task::none();
                }
                if current {
                    self.load_debt.insert(
                        tab.clone(),
                        PendingLoad {
                            path,
                            workspace,
                            token,
                            reason,
                        },
                    );
                }
                if error != BlockingTaskError::Busy {
                    self.load_retry_blocked.insert(tab);
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-the-document-could-not-be-loaded-retry-336833",
                    ));
                }
            }
            BlockingWorkFailure::Page {
                tab,
                token,
                operation,
                error,
            } => {
                let current = self
                    .model
                    .tabs
                    .iter()
                    .any(|candidate| candidate.id == tab && candidate.load_token == token);
                if !current && error == BlockingTaskError::Cancelled {
                    return Task::none();
                }
                if error == BlockingTaskError::Busy {
                    if let Some(operation) = operation.take() {
                        self.paged_debt.insert(tab, (token, operation));
                    }
                    self.blocking_retry_pending = true;
                } else {
                    if let Some(operation) = operation.take() {
                        self.paged_debt.insert(tab.clone(), (token, operation));
                    }
                    self.paged_retry_blocked.insert(tab.clone());
                    self.picker.failure = Some(
                        enzui_core::localized!("text-a-visible-page-for-p0-could-not-c438f4", "p0" => crate::safe_text::bounded_inline(&tab)),
                    );
                }
            }
        }
        Task::none()
    }
    pub(crate) fn reduce_editor_action(&mut self, action: EditorAction) -> Task<Message> {
        // Live input supersedes a not-yet-settled recovered scroll.
        self.model.active_tab_mut().pending_scroll = None;
        // Refused before `Content` sees the action, so the native buffer
        // cannot drift from the authoritative document.
        if self.refuse_if_authority_exhausted_on_action(&action) {
            return Task::none();
        }
        // Read-only text still accepts selection and scrolling.
        if action.is_edit()
            && !self
                .model
                .active_tab()
                .supports(large_doc::Capability::Edit)
        {
            return Task::none();
        }
        // Paste is the one action that can inject CR into the LF-normalized
        // document.
        let action = normalize_pasted_line_endings(action);
        // The native editor holds only a window of a large document, so the
        // app owns its wheel scroll.
        if self.model.active_tab_mut().scroll_virtual_view(&action) {
            return Task::none();
        }
        if self
            .model
            .active_tab_mut()
            .perform_virtual_navigation(&action)
        {
            self.model.active_tab_mut().note_editor_action();
            self.sync_cursor_readout();
            return Task::none();
        }
        if let Some(outcome) = self
            .model
            .active_tab_mut()
            .perform_virtual_selection_edit(&action)
        {
            self.model.active_tab_mut().note_editor_action();
            // A refusal changed nothing; its only consequence is the sentence.
            if let Some(sentence) = outcome.refusal_sentence() {
                self.picker.failure = Some(sentence);
            } else if outcome.changed() {
                self.model.active_tab_mut().dirty = true;
                if self.model.search.open {
                    self.recompute_search();
                }
            }
            self.sync_cursor_readout();
            return Task::none();
        }
        if let Some(changed) = self.model.active_tab_mut().perform_virtual_history(&action) {
            if changed {
                self.model.active_tab_mut().dirty = true;
                if self.model.search.open {
                    self.recompute_search();
                }
            }
            self.sync_cursor_readout();
            return Task::none();
        }
        let is_edit = action.is_edit();
        let checkpoint = self.model.active_tab().virtual_edit_checkpoint(&action);
        self.model.active_tab_mut().prepare_editor_action(&action);
        let delta = self.model.active_tab_mut().content.perform(action);
        // The one place an action reaches the editor, so the one place a
        // selection can change without the cursor pair changing.
        self.model.active_tab_mut().note_editor_action();
        // Apply the exact bytes moved; a drifted delta forces a full resync.
        let tab = self.model.active_tab_mut();
        let mut refused = None;
        let mut authority_exhausted = false;
        if let Some(delta) = &delta {
            let changed_bytes = delta.bytes();
            match tab.apply_edit_delta(delta) {
                model::EditDeltaOutcome::Applied(_) => {
                    if let Some(checkpoint) = checkpoint {
                        tab.commit_virtual_edit(checkpoint, changed_bytes);
                    }
                }
                model::EditDeltaOutcome::Drifted => tab.sync_snapshot(),
                // The document is back to its pre-edit bytes, but cosmic
                // still holds the refused join; rebuild the native window.
                model::EditDeltaOutcome::RefusedOverlongLine { line, bytes, limit } => {
                    tab.restore_editor_projection();
                    refused = Some((line, bytes, limit));
                }
                // The counter hit its ceiling after the preflight check.
                // Rebuild `Content` so the UI shows the bytes a save writes.
                model::EditDeltaOutcome::AuthorityExhausted => {
                    tab.restore_content_from_document();
                    tab.restore_editor_projection();
                    authority_exhausted = true;
                }
            }
        }
        if let Some((line, bytes, limit)) = refused {
            self.publish_overlong_refusal(line, bytes, limit);
            self.sync_cursor_readout();
            return Task::none();
        }
        if authority_exhausted {
            self.publish_authority_exhausted_sentence();
            self.sync_cursor_readout();
            return Task::none();
        }
        let tab = self.model.active_tab_mut();
        tab.sync_virtual_selection();
        let is_edit = is_edit && delta.is_some();
        if is_edit {
            self.model.active_tab_mut().dirty = true;
            if self.model.search.open {
                self.recompute_search();
            }
        }
        self.sync_cursor_readout();
        // A drag's pointer move costs only the epoch bump; the settle tick
        // (`SelectionStatsSettleTick`) dispatches the count.
        Task::none()
    }
}
