//! Worker spawn and dispatch helpers.

use super::*;

impl TextApp {
    // The scan reads a shared snapshot handle; `SearchState::accept` drops a
    // stale answer.
    pub(crate) fn spawn_search(&mut self, from: Option<usize>) {
        if self.model.search.query.is_empty() {
            self.abort_search_scan();
            self.model.search.matches.clear();
            self.model.search.current = 0;
            self.model.search.error.clear();
            return;
        }
        let Some(generation) = self.model.search.next_generation() else {
            self.search_scan = None;
            self.search_attempt = None;
            self.search_retry = None;
            self.search_retry_blocked = false;
            self.search_queued = None;
            return;
        };
        let query = self.model.search.query.clone();
        let options = self.model.search.options;

        if let Some(running) = &self.search_scan {
            // Cancel and queue: the running worker keeps its cancel flag and
            // document handle until it reports back, so at most one scan runs.
            running.cancel();
            self.model.search.searching = true;
            self.search_queued = Some(from);
            return;
        }

        let cancel = std::sync::Arc::new(worker::Cancel::default());
        let active = self.model.active_tab();
        let binding = search::MatchBinding {
            tab_id: active.id.clone(),
            snapshot_version: active.snapshot_version(),
        };
        let attempt = PendingSearch {
            generation,
            from,
            query,
            options,
            document: active.snapshot_handle(),
            cancel,
            binding,
        };
        self.search_retry = None;
        self.search_retry_blocked = false;
        self.dispatch_search_attempt(attempt);
    }

    pub(crate) fn dispatch_search_attempt(&mut self, attempt: PendingSearch) {
        let worker_attempt = attempt.clone();
        let generation = attempt.generation;
        let from = attempt.from;
        let binding = attempt.binding.clone();
        let scan = std::sync::Arc::clone(&attempt.cancel);
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |context| {
            worker_localizer.scope(|| {
                if context.is_cancelled() {
                    return None;
                }
                match search::find_all_snapshot_cancellable(
                    &worker_attempt.document,
                    &worker_attempt.query,
                    worker_attempt.options,
                    &worker_attempt.cancel,
                ) {
                    search::Cancellable::Done(result) => Some(result),
                    search::Cancellable::Cancelled => None,
                }
            })
        });
        self.deferred_tasks
            .push(Task::perform(task, move |settlement| {
                match settlement.into_result() {
                    Ok(result) => WorkDone::Search {
                        generation,
                        from,
                        result,
                        binding,
                    }
                    .message(),
                    Err(error) => Message::BlockingWorkFailed(BlockingWorkFailure::Search {
                        generation,
                        from,
                        error,
                    }),
                }
            }));
        self.search_attempt = Some(attempt);
        self.search_scan = Some(scan);
        self.model.search.searching = true;
    }

    // Called only from the debounce settle tick and `OpenStats`, never from
    // `Message::EditorAction`. Does nothing unless the Statistics surface is
    // open.
    pub(crate) fn spawn_selection_stats(&mut self) -> Admission {
        if !self.model.stats_open {
            return Admission::NothingToDo;
        }
        let tab_id = self.model.active_tab_id.clone();
        if self.selection_stats_retry_blocked.contains(&tab_id)
            || self.selection_stats_attempts.contains_key(&tab_id)
        {
            return Admission::NothingToDo;
        }
        if let Some(attempt) = self.selection_stats_retry.remove(&tab_id) {
            self.dispatch_selection_stats_attempt(attempt);
            return Admission::Started;
        }
        let Some(index) = self.model.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Admission::NothingToDo;
        };
        let Some((key, selection)) = self.model.tabs[index].take_selection_stats_job() else {
            return Admission::NothingToDo;
        };
        let attempt = PendingSelectionStats {
            tab: tab_id,
            key,
            selection,
        };
        self.dispatch_selection_stats_attempt(attempt);
        Admission::Started
    }

    pub(crate) fn dispatch_selection_stats_attempt(&mut self, attempt: PendingSelectionStats) {
        let worker_attempt = attempt.clone();
        let tab = attempt.tab.clone();
        let key = attempt.key;
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(0, move |_| {
            worker_localizer.scope(|| {
                let text = worker_attempt.selection.text();
                doc_stats::document_stats(&text)
            })
        });
        self.deferred_tasks
            .push(Task::perform(task, move |settlement| {
                match settlement.into_result() {
                    Ok(stats) => WorkDone::SelectionStats { tab, key, stats }.message(),
                    Err(error) => {
                        Message::BlockingWorkFailed(BlockingWorkFailure::SelectionStats {
                            tab,
                            key,
                            error,
                        })
                    }
                }
            }));
        self.selection_stats_attempts
            .insert(attempt.tab.clone(), attempt);
    }

    // Only for a tab whose counts were left pending by `install_buffer`; a
    // small document already has exact counts by then.
    pub(crate) fn spawn_stats(&mut self, tab_id: &str) -> Admission {
        let Some(index) = self.model.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Admission::NothingToDo;
        };
        if !self.model.tabs[index].stats_startable() {
            return Admission::NothingToDo;
        }
        let attempt = PendingStats {
            tab: tab_id.to_owned(),
            generation: self.model.tabs[index].snapshot_version(),
            document: self.model.tabs[index].snapshot_handle(),
        };
        self.dispatch_stats_attempt(attempt);
        self.model.tabs[index].stats_started();
        Admission::Started
    }

    pub(crate) fn dispatch_stats_attempt(&mut self, attempt: PendingStats) {
        let worker_attempt = attempt.clone();
        let tab = attempt.tab.clone();
        let generation = attempt.generation;
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                (
                    stats::text_stats_snapshot(&worker_attempt.document),
                    doc_stats::document_stats_snapshot(&worker_attempt.document),
                )
            })
        });
        self.deferred_tasks
            .push(Task::perform(task, move |settlement| {
                match settlement.into_result() {
                    Ok((text, document)) => WorkDone::Stats {
                        tab,
                        generation,
                        text,
                        document,
                    }
                    .message(),
                    Err(error) => Message::BlockingWorkFailed(BlockingWorkFailure::Stats {
                        tab,
                        generation,
                        error,
                    }),
                }
            }));
        self.stats_attempts.insert(attempt.tab.clone(), attempt);
    }
}
