//! TextApp background work pump helpers.

use super::*;

impl TextApp {
    // Called from every worker completion. The queue refuses only when full,
    // so a completion is always coming to wake every refused intent.
    pub(crate) fn pump_pending_work(&mut self) {
        self.pump_load_debt();
        self.pump_paged_debt();
        self.pump_workspace_debt();
        self.pump_watcher_debt();
        if let Some(attempt) = self
            .selection_stats_retry
            .values()
            .find(|attempt| !self.selection_stats_retry_blocked.contains(&attempt.tab))
            .cloned()
        {
            self.selection_stats_retry.remove(&attempt.tab);
            self.dispatch_selection_stats_attempt(attempt);
        }
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            return;
        }
        const CLASSES: usize = 3;

        for _ in 0..CLASSES {
            // `pump_cursor` is a round-robin index, never compared against an
            // async completion, so wrapping arithmetic only rotates the start.
            let class = self.pump_cursor % CLASSES;
            self.pump_cursor = self.pump_cursor.wrapping_add(1);
            let admission = match class {
                0 => self.pump_pending_stats(),
                1 => self.pump_pending_saves(),
                _ => self.start_queued_search(),
            };
            if admission == Admission::Refused {
                // The queue is full; the next completion resumes at the class
                // after this one, so nothing starves.
                return;
            }
        }
    }

    pub(crate) fn has_retryable_blocking_debt(&self) -> bool {
        (self.settings_debt.is_some()
            && self.settings_inflight.is_none()
            && !self.settings_retry_blocked)
            || (self.recovery_retry.is_some()
                && self.recovery_inflight.is_none()
                && !self.recovery_retry_blocked)
            || (self.recovery_retry.is_none()
                && self.recovery_inflight.is_none()
                && self.recovery_discard.is_none()
                && self.recovery_has_debt())
            || (matches!(self.recovery_discard, Some(RecoveryDiscard::Pending { .. }))
                && !self.recovery_discard_submitted
                && !self.recovery_discard_retry_blocked)
            || self
                .load_debt
                .keys()
                .any(|tab| !self.load_retry_blocked.contains(tab))
            || self
                .paged_debt
                .keys()
                .any(|tab| !self.paged_retry_blocked.contains(tab))
            || self.workspace_debt.values().any(|pending| {
                !self.workspace_inflight.contains_key(&pending.path)
                    && !self.workspace_retry_blocked.contains(&pending.path)
            })
            || (self.watcher_debt && !self.watcher_inflight && !self.watcher_retry_blocked)
            || self.model.tabs.iter().any(|tab| {
                (tab.save_queued.is_some() || self.save_debt.contains_key(&tab.id))
                    && !tab.saving
                    && !self.save_retry_blocked.contains(&tab.id)
            })
            || ((self.search_queued.is_some() || self.search_retry.is_some())
                && self.search_scan.is_none()
                && !self.search_retry_blocked)
            || (self.session_write_debt.is_some()
                && !self.session_write_inflight
                && !self.session_retry_blocked)
            || self
                .session_debt
                .as_ref()
                .is_some_and(|debt| debt.kind() == session::SessionDebtKind::Busy)
            || (self.session_discard_window.is_some()
                && !self.session_write_inflight
                && !self.session_retry_blocked)
            || (self.print_retry.is_some() && !self.print_preparing && !self.print_retry_blocked)
            || self
                .stats_retry
                .keys()
                .any(|tab| !self.stats_retry_blocked.contains(tab))
            || self
                .selection_stats_retry
                .keys()
                .any(|tab| !self.selection_stats_retry_blocked.contains(tab))
            || self
                .model
                .tabs
                .iter()
                .any(|tab| tab.stats_startable() && !self.stats_retry_blocked.contains(&tab.id))
            || (self.model.stats_open
                && self.model.active_tab().wants_selection_stats()
                && !self
                    .selection_stats_retry_blocked
                    .contains(&self.model.active_tab_id))
    }

    pub(crate) fn begin_watcher_batch(&mut self) -> Task<Message> {
        if self.watcher_retry_blocked {
            return Task::none();
        }
        if self.watcher_inflight {
            self.watcher_debt = true;
            return Task::none();
        }
        if let Some(retry) = self.watcher_retry.clone() {
            return self.dispatch_watcher_batch(self.watcher_generation, retry);
        }
        let probes: Vec<_> = self
            .model
            .tabs
            .iter()
            .filter_map(|tab| {
                tab.watcher
                    .as_ref()
                    .map(|watcher| (tab.id.clone(), watcher.prepare_probe()))
            })
            .collect();
        if probes.is_empty() {
            self.watcher_debt = false;
            return Task::none();
        }
        // Checked mint: a wrapped generation would let a stale batch settle
        // against the reused key. On exhaustion no batch starts, watchers stay
        // seeded, and `watcher_debt` stays set for a retry.
        let Some(generation) = self.watcher_generation.checked_add(1) else {
            self.watcher_retry_blocked = true;
            self.picker.failure = Some(enzui_core::localization::text(
                "text-file-watch-generations-are-exhausted-restart-the-749ca5",
            ));
            return Task::none();
        };
        self.watcher_generation = generation;
        self.watcher_retry_paths = probes
            .iter()
            .map(|(tab, operation)| (tab.clone(), operation.path().to_path_buf()))
            .collect();
        let retry = RetryPayload::new(probes);
        self.watcher_retry = Some(retry.clone());
        self.dispatch_watcher_batch(generation, retry)
    }

    pub(crate) fn dispatch_watcher_batch(
        &mut self,
        generation: u64,
        retry: RetryPayload<Vec<(String, enzui_core::file_watch::ProbeOp)>>,
    ) -> Task<Message> {
        self.watcher_inflight = true;
        self.watcher_debt = false;
        let worker_retry = retry.clone();
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                worker_retry
                    .take()
                    .expect("watch batch payload consumed once")
                    .into_iter()
                    .map(|(tab, operation)| (tab, operation.execute()))
                    .collect()
            })
        });
        Task::perform(task, move |settlement| Message::FileWatchBatchSettled {
            generation,
            retry,
            result: settlement.into_result(),
        })
    }

    pub(crate) fn pump_watcher_debt(&mut self) {
        if self.watcher_debt && !self.watcher_inflight && !self.watcher_retry_blocked {
            let task = self.begin_watcher_batch();
            self.deferred_tasks.push(task);
        }
    }

    pub(crate) fn pump_paged_debt(&mut self) {
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            return;
        }
        let Some(tab) = self
            .paged_debt
            .keys()
            .find(|tab| !self.paged_retry_blocked.contains(*tab))
            .cloned()
        else {
            return;
        };
        let Some((token, operation)) = self.paged_debt.remove(&tab) else {
            return;
        };
        let task = self.paged_page_task(tab, token, operation);
        self.deferred_tasks.push(task);
    }

    pub(crate) fn pump_workspace_debt(&mut self) {
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            return;
        }
        let Some(pending) = self
            .workspace_debt
            .values()
            .find(|pending| {
                !self.workspace_inflight.contains_key(&pending.path)
                    && !self.workspace_retry_blocked.contains(&pending.path)
            })
            .cloned()
        else {
            return;
        };
        let task = self.dispatch_workspace_read(pending.path, pending.generation);
        self.deferred_tasks.push(task);
    }

    pub(crate) fn pump_load_debt(&mut self) {
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            return;
        }
        let Some(tab) = self
            .load_debt
            .keys()
            .find(|tab| !self.load_retry_blocked.contains(*tab))
            .cloned()
        else {
            return;
        };
        let Some(pending) = self.load_debt.remove(&tab) else {
            return;
        };
        let task = self.dispatch_document_load(
            tab,
            pending.path,
            pending.workspace,
            pending.token,
            pending.reason,
        );
        self.deferred_tasks.push(task);
    }

    pub(crate) fn pump_pending_stats(&mut self) -> Admission {
        let count = self.model.tabs.len();
        if count == 0 {
            return Admission::NothingToDo;
        }
        let mut outcome = Admission::NothingToDo;
        for step in 0..count {
            let index = (self.pump_cursor.wrapping_add(step)) % count;
            let id = self.model.tabs[index].id.clone();
            if self.stats_retry_blocked.contains(&id) {
                continue;
            }
            if let Some(attempt) = self.stats_retry.remove(&id) {
                self.dispatch_stats_attempt(attempt);
                self.model.tabs[index].stats_started();
                outcome = Admission::Started;
                continue;
            }
            match self.spawn_stats(&id) {
                Admission::Started => outcome = Admission::Started,
                Admission::Refused => return Admission::Refused,
                Admission::NothingToDo => {}
            }
        }
        outcome
    }

    pub(crate) fn pump_pending_saves(&mut self) -> Admission {
        let mut outcome = Admission::NothingToDo;
        for index in 0..self.model.tabs.len() {
            if self.model.tabs[index].saving
                || self.save_retry_blocked.contains(&self.model.tabs[index].id)
            {
                continue;
            }
            let id = self.model.tabs[index].id.clone();
            if let Some(request) = self.save_debt.remove(&id) {
                self.model.tabs[index].save_queued = None;
                match self.submit_save_request(index, request) {
                    SaveDispatch::InFlight if self.model.tabs[index].saving => {
                        outcome = Admission::Started;
                        continue;
                    }
                    SaveDispatch::InFlight => return Admission::Refused,
                    SaveDispatch::Failed => {
                        outcome = Admission::Started;
                        continue;
                    }
                }
            }
            let Some(rebind) = self.model.tabs[index].save_queued.clone() else {
                continue;
            };
            let Some(path) = rebind
                .clone()
                .or_else(|| self.model.tabs[index].file_path.clone())
            else {
                // Nowhere to write it: drop the intent rather than retry it
                // forever.
                self.model.tabs[index].save_queued = None;
                continue;
            };
            self.model.tabs[index].save_queued = None;
            match self.dispatch_streaming_save(index, &path, rebind) {
                // `dispatch_streaming_save` re-queues on refusal, which is how
                // this reports back.
                SaveDispatch::InFlight if self.model.tabs[index].saving => {
                    outcome = Admission::Started;
                }
                SaveDispatch::InFlight => return Admission::Refused,
                SaveDispatch::Failed => outcome = Admission::Started,
            }
        }
        outcome
    }
}
