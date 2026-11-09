//! TextApp close and session paths.

use super::*;

impl TextApp {
    pub(crate) fn active_tab_index_for_save(&self) -> usize {
        self.model
            .tabs
            .iter()
            .position(|tab| tab.id == self.model.active_tab_id)
            .unwrap_or(0)
    }

    pub(crate) fn close_window(&mut self, id: iced::window::Id) -> Task<Message> {
        self.blocking_retry_suspended = false;
        self.recovery_retry_blocked = false;
        if self.close_drain.is_some() {
            return Task::none();
        }
        self.close_drain = Some(CloseDrain {
            window: id,
            deadline: std::time::Instant::now() + CLOSE_DRAIN_DEADLINE,
        });
        let settings = self.queue_settings_save();
        let recovery = self.publish_recovery();
        Task::batch([settings, recovery])
    }

    pub(crate) fn drive_close_drain(&mut self) -> Task<Message> {
        let Some(drain) = self.close_drain else {
            return Task::none();
        };
        if std::time::Instant::now() >= drain.deadline {
            self.close_drain = None;
            self.blocking_retry_suspended = true;
            self.close_request_window = None;
            self.close_save_attempt = None;
            let _ = self.close_guard.decide(CloseDecision::Cancel);
            self.picker.failure = Some(enzui_core::localization::text(
                "text-close-was-cancelled-because-durable-work-did-237bf9",
            ));
            return Task::none();
        }
        if self.blocking_tasks.dispatcher().snapshot().live() != 0
            || self.close_guard.is_pending()
            || self.model.tabs.iter().any(|tab| tab.dirty)
            || self.settings_inflight.is_some()
            || self.settings_debt.is_some()
            || self.session_write_inflight
            || self.session_discard_window.is_some()
            // Exhaustion is a direct terminal gate, so a settlement reporting
            // Ok cannot re-authorize Close by clearing `session_retry_blocked`.
            || self.session_retry_blocked
            || self.session_projection_revision_exhausted
            || self.recovery_inflight.is_some()
            || self.recovery_has_debt()
            || self
                .model
                .tabs
                .iter()
                .any(|tab| tab.saving || tab.save_queued.is_some())
        {
            return Task::none();
        }
        self.close_drain = None;
        self.close_request_window = None;
        self.print_retry = None;
        self.print_preparing = false;
        self.print.abort_for_close();
        #[cfg(feature = "center")]
        self.shutdown_center();
        self.picker.parent_closed(drain.window);
        iced::window::close::<Message>(drain.window).chain(iced::exit())
    }

    pub(crate) fn veto_close_for_durability(&mut self, message: String) {
        if self.close_drain.take().is_some() {
            self.blocking_retry_suspended = true;
            self.close_request_window = None;
            self.close_save_attempt = None;
            let _ = self.close_guard.decide(CloseDecision::Cancel);
            // A failure naming its cause is the more useful banner; the generic
            // veto shows only when nothing more specific is.
            if self.picker.failure.is_none() {
                self.picker.failure = Some(message);
            }
        }
    }

    pub(crate) fn apply_close_guard_action(
        &mut self,
        mut action: CloseGuardAction<String>,
    ) -> Task<Message> {
        loop {
            match action {
                CloseGuardAction::Hold => return Task::none(),
                CloseGuardAction::Close(id) => return self.close_window(id),
                CloseGuardAction::Save { key: id, attempt } => {
                    self.close_save_attempt = Some(attempt.clone());
                    let Some(tab) = self.model.tabs.iter().find(|tab| tab.id == id) else {
                        self.close_save_attempt = None;
                        action = self
                            .close_guard
                            .save_finished(&id, &attempt, SaveOutcome::Failed)
                            .expect("the guard requested this missing tab key");
                        continue;
                    };
                    if tab.file_path.is_none() {
                        let switching = self.model.active_tab_id != id;
                        self.model.active_tab_id.clone_from(&id);
                        if switching {
                            self.advance_session_projection_revision();
                        }
                        if self.start_save_as() {
                            return Task::none();
                        }
                        self.close_save_attempt = None;
                        action = self
                            .close_guard
                            .save_finished(&id, &attempt, SaveOutcome::Failed)
                            .expect("the guard requested this untitled tab key");
                    } else {
                        let drain = self
                            .close_request_window
                            .map_or_else(Task::none, |window| self.close_window(window));
                        let outcome = match self.save_tab(&id) {
                            SaveDispatch::Failed => SaveOutcome::Failed,
                            // Holding until `SaveCompleted` keeps the app from
                            // exiting mid-write.
                            SaveDispatch::InFlight => return drain,
                        };
                        self.close_save_attempt = None;
                        action = self
                            .close_guard
                            .save_finished(&id, &attempt, outcome)
                            .expect("the guard requested this titled tab key");
                        if outcome == SaveOutcome::Failed {
                            self.veto_close_for_durability(enzui_core::localization::text(
                                "text-close-was-cancelled-because-a-document-was-a7581c",
                            ));
                        }
                        if !matches!(action, CloseGuardAction::Close(_)) {
                            return drain;
                        }
                    }
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn open_path(&mut self, path: &std::path::Path) -> bool {
        if let Some(existing) = self
            .model
            .tabs
            .iter()
            .find(|tab| tab.file_path.as_deref() == Some(path))
            .map(|tab| tab.id.clone())
        {
            let switching = self.model.active_tab_id != existing;
            self.model.active_tab_id = existing;
            self.sync_cursor_readout();
            if switching {
                self.advance_session_projection_revision();
            }
            return true;
        }
        let workspace = self.active_workspace_context();
        let reuse_active = {
            let t = self.model.active_tab();
            t.file_path.is_none() && !t.dirty && t.document().is_empty()
        };
        let tab_id = if reuse_active {
            self.model.active_tab().id.clone()
        } else {
            let id = self.model.fresh_tab_id();
            self.model.add_tab(Tab::new(id.clone()));
            id
        };
        // Both branches change tab-path membership, as `begin_open` does.
        self.advance_session_projection_revision();
        // Refuses when the token counter is exhausted.
        let Some(token) = self.model.load_tokens.next() else {
            return false;
        };
        let threshold = self.model.large_threshold;
        if let Some(tab) = self.model.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.load_token = token;
            tab.file_path = Some(path.to_path_buf());
            tab.loading = true;
        }
        let Ok(loaded) = large_doc::load_document(path, threshold) else {
            return false;
        };
        let task = self.finish_document_load(
            &tab_id,
            path,
            workspace,
            token,
            LoadReason::Open,
            Ok(loaded),
        );
        drop(task);
        let _ = self.drain_paged_pages_for_test();
        true
    }

    #[cfg(test)]
    pub(crate) fn active_workspace_context(&self) -> Option<std::path::PathBuf> {
        match &self.workspaces.active {
            workspace::ActiveWorkspace::OpenFiles => None,
            workspace::ActiveWorkspace::Named(root) => Some(root.clone()),
        }
    }

    pub(crate) fn dispatch_session_retry_pending(&mut self) -> Task<Message> {
        self.session_retry_blocked = false;
        if self.session_write_inflight {
            return Task::none();
        }
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            return Task::none();
        }
        // On exhaustion no worker starts; the retained debt keeps this pending
        // path valid for a restart.
        let Some(generation) = self.session_write_generation.checked_add(1) else {
            self.session_retry_blocked = true;
            self.picker.failure = Some(enzui_core::localization::text(
                "text-session-write-generations-are-exhausted-restart-the-ab8dbe",
            ));
            self.veto_close_for_durability(enzui_core::localization::text(
                "text-close-was-cancelled-because-session-writes-cannot-fcd505",
            ));
            return Task::none();
        };
        self.session_write_generation = generation;
        let (_, placeholder, _) = session::SessionStore::open_disabled();
        let store = std::mem::replace(&mut self.session_store, placeholder);
        // Only a bounded projection may enter the retry-bytes slot; an
        // over-bound current projection becomes typed debt instead.
        let current = session::persisted_snapshot(&self.model, &self.workspaces);
        match session::preflight_bounds(&current) {
            Ok(()) => {
                self.session_write_debt = Some(current);
                self.session_over_bound_debt = None;
            }
            Err(summary) => {
                drop(current);
                self.session_over_bound_debt = Some(OverBoundReducerDebt {
                    revision_at_refusal: self.session_projection_revision,
                    summary,
                });
                self.session_write_debt = None;
            }
        }
        let prior_baseline = self.session_write_baseline.clone();
        let dispatch_revision = self.session_projection_revision;
        let handoff = SessionWriteHandoff::pending(store, prior_baseline, dispatch_revision);
        let worker_handoff = handoff.clone();
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                worker_handoff.execute(session::SessionStore::retry_pending);
            });
        });
        self.session_write_inflight = true;
        Task::perform(task, move |settlement| Message::SessionWriteSettled {
            generation,
            handoff,
            result: settlement.into_result(),
        })
    }

    // Called from a reducer arm that mutated a persisted-projection field.
    // Overflow latches `session_projection_revision_exhausted`, which refuses
    // further publication even where no `SessionRequest` exists to raise a
    // banner; `picker.failure` and the close-veto then carry the signal.
    pub(crate) fn advance_session_projection_revision(&mut self) {
        match self.session_projection_revision.checked_add(1) {
            Some(next) => self.session_projection_revision = next,
            None => {
                self.session_projection_revision_exhausted = true;
                self.session_over_bound_debt = None;
                self.session_write_debt = None;
                self.session_retry_blocked = true;
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-session-projection-revisions-are-exhausted-restart-the-3a2ae2",
                ));
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-session-projection-revisions-ec8e2c",
                ));
                if let Some(request) = self.session_restore_request.clone() {
                    self.session_debt =
                        Some(session::SessionDebt::publish_revision_exhausted(request));
                }
            }
        }
    }

    pub(crate) fn queue_session_sync(&mut self) -> Task<Message> {
        if self.session_discard_window.is_some() {
            return self.dispatch_session_discard();
        }
        if self.session_restore_pending.is_some() {
            return Task::none();
        }
        // Terminal and not clearable; held apart from `session_debt` so a
        // `SessionRequest::for_roots(...) == None` path is refused as visibly.
        if self.session_projection_revision_exhausted {
            return Task::none();
        }
        let revision = self.session_projection_revision;
        if let Some(refused) = self.session_over_bound_debt.as_ref() {
            if refused.revision_at_refusal == revision {
                return Task::none();
            }
            self.session_over_bound_debt = None;
            self.session_retry_blocked = false;
            self.session_debt = None;
        }
        let snapshot = session::persisted_snapshot(&self.model, &self.workspaces);
        // A refused projection is dropped; only the bounded typed debt
        // survives, so `session_write_debt` never holds an over-bound value.
        if let Err(summary) = session::preflight_bounds(&snapshot) {
            drop(snapshot);
            self.session_over_bound_debt = Some(OverBoundReducerDebt {
                revision_at_refusal: revision,
                summary,
            });
            self.session_retry_blocked = true;
            self.session_write_debt = None;
            if let Some(request) = self.session_restore_request.clone() {
                self.session_debt =
                    Some(session::SessionDebt::publish_over_bound(request, summary));
            }
            return Task::none();
        }
        if snapshot != self.session_write_baseline {
            self.session_write_debt = Some(snapshot);
        }
        if self.session_retry_blocked {
            Task::none()
        } else {
            self.dispatch_session_sync()
        }
    }

    pub(crate) fn dispatch_session_sync(&mut self) -> Task<Message> {
        if self.session_write_inflight || self.session_retry_blocked {
            return Task::none();
        }
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            return Task::none();
        }
        let Some(next) = self.session_write_debt.take() else {
            return Task::none();
        };
        // On exhaustion the debt is put back for a restart and no worker starts.
        let Some(generation) = self.session_write_generation.checked_add(1) else {
            self.session_write_debt = Some(next);
            self.session_retry_blocked = true;
            self.picker.failure = Some(enzui_core::localization::text(
                "text-session-write-generations-are-exhausted-restart-the-ab8dbe",
            ));
            self.veto_close_for_durability(enzui_core::localization::text(
                "text-close-was-cancelled-because-session-writes-cannot-fcd505",
            ));
            return Task::none();
        };
        self.session_write_generation = generation;
        let (_, placeholder, _) = session::SessionStore::open_disabled();
        let store = std::mem::replace(&mut self.session_store, placeholder);
        let task_snapshot = next.clone();
        let dispatch_revision = self.session_projection_revision;
        let handoff = SessionWriteHandoff::pending(store, next, dispatch_revision);
        let worker_handoff = handoff.clone();
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                worker_handoff.execute(|store| store.sync_snapshot(task_snapshot));
            });
        });
        self.session_write_inflight = true;
        Task::perform(task, move |settlement| Message::SessionWriteSettled {
            generation,
            handoff,
            result: settlement.into_result(),
        })
    }

    pub(crate) fn session_write_settled(
        &mut self,
        generation: u64,
        handoff: SessionWriteHandoff,
        result: Result<(), BlockingTaskError>,
    ) -> Task<Message> {
        if generation != self.session_write_generation || !self.session_write_inflight {
            return Task::none();
        }
        self.session_write_inflight = false;
        let Some((mut store, persisted, snapshot, dispatch_revision)) = handoff.take() else {
            return Task::none();
        };
        let debt = store.take_debt();
        self.session_store = store;
        match result {
            Ok(()) => {
                if persisted || debt.is_none() {
                    self.session_write_baseline = snapshot;
                    self.session_over_bound_debt = None;
                } else if let Some(session::SessionDebtKind::PublishOverBound(summary)) =
                    debt.as_ref().map(session::SessionDebt::kind)
                {
                    // Stamped with `dispatch_revision` so an older settlement
                    // cannot latch a newer live projection.
                    self.session_over_bound_debt = Some(OverBoundReducerDebt {
                        revision_at_refusal: dispatch_revision,
                        summary,
                    });
                    self.session_write_debt = None;
                    drop(snapshot);
                } else if self.session_write_debt.is_none() {
                    self.session_write_debt = Some(snapshot);
                    self.session_over_bound_debt = None;
                } else {
                    self.session_over_bound_debt = None;
                }
                // Terminal exhaustion latched before this settlement must not
                // be cleared by an older, pre-overflow write reporting Ok.
                self.session_retry_blocked =
                    debt.is_some() || self.session_projection_revision_exhausted;
                self.session_debt = debt;
                // Keep the terminal banner past the older settlement.
                if self.session_projection_revision_exhausted
                    && self.session_debt.is_none()
                    && let Some(request) = self.session_restore_request.clone()
                {
                    self.session_debt =
                        Some(session::SessionDebt::publish_revision_exhausted(request));
                }
            }
            Err(BlockingTaskError::Busy) => {
                if self.session_write_debt.is_none() {
                    self.session_write_debt = Some(snapshot);
                }
                self.session_retry_blocked = self.session_projection_revision_exhausted;
                self.blocking_retry_pending = true;
            }
            Err(error) => {
                if self.session_write_debt.is_none() {
                    self.session_write_debt = Some(snapshot);
                }
                self.session_retry_blocked = true;
                self.session_debt = debt.or_else(|| {
                    self.session_restore_request
                        .clone()
                        .map(|request| session::SessionDebt::from_worker_error(request, error))
                });
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-the-workspace-session-could-not-be-saved-c0de3e",
                ));
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-the-workspace-session-a1bfda",
                ));
            }
        }
        Task::none()
    }

    pub(crate) fn begin_session_discard(&mut self, window: iced::window::Id) -> Task<Message> {
        if !self.session_write_inflight {
            self.session_retry_blocked = false;
            self.session_discard_window = Some(window);
        }
        self.dispatch_session_discard()
    }

    pub(crate) fn dispatch_session_discard(&mut self) -> Task<Message> {
        if self.session_write_inflight || self.session_retry_blocked {
            return Task::none();
        }
        let Some(window) = self.session_discard_window else {
            return Task::none();
        };
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            return Task::none();
        }
        // On exhaustion the discard cannot start; the discard window is kept
        // so a restart can complete the close.
        let Some(generation) = self.session_write_generation.checked_add(1) else {
            self.session_retry_blocked = true;
            self.picker.failure = Some(enzui_core::localization::text(
                "text-session-write-generations-are-exhausted-restart-the-ab8dbe",
            ));
            self.veto_close_for_durability(enzui_core::localization::text(
                "text-close-was-cancelled-because-session-writes-cannot-fcd505",
            ));
            return Task::none();
        };
        self.session_write_generation = generation;
        let (_, placeholder, _) = session::SessionStore::open_disabled();
        let store = std::mem::replace(&mut self.session_store, placeholder);
        let empty = session::SessionSnapshot::empty_for_app();
        let dispatch_revision = self.session_projection_revision;
        let handoff = SessionWriteHandoff::pending(store, empty, dispatch_revision);
        let worker_handoff = handoff.clone();
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                worker_handoff.execute(session::SessionStore::discard_open_tabs);
            });
        });
        self.session_write_inflight = true;
        Task::perform(task, move |settlement| Message::SessionDiscardSettled {
            generation,
            window,
            handoff,
            result: settlement.into_result(),
        })
    }

    pub(crate) fn session_discard_settled(
        &mut self,
        generation: u64,
        window: iced::window::Id,
        handoff: SessionWriteHandoff,
        result: Result<(), BlockingTaskError>,
    ) -> Task<Message> {
        if generation != self.session_write_generation || !self.session_write_inflight {
            return Task::none();
        }
        self.session_write_inflight = false;
        // The discard route stamps no over-bound debt, so the revision is unused.
        let Some((mut store, persisted, snapshot, _dispatch_revision)) = handoff.take() else {
            return Task::none();
        };
        self.session_debt = store.take_debt();
        self.session_store = store;
        match result {
            Ok(()) if persisted => {
                self.session_write_baseline = snapshot;
                self.session_write_debt = None;
                self.session_discard_window = None;
                self.begin_recovery_discard(window)
            }
            Err(BlockingTaskError::Busy) => {
                self.session_write_debt = Some(snapshot);
                self.session_retry_blocked = self.session_projection_revision_exhausted;
                self.blocking_retry_pending = true;
                Task::none()
            }
            Ok(()) | Err(_) => {
                self.session_write_debt = Some(snapshot);
                self.session_retry_blocked = true;
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-session-durability-is-99169f",
                ));
                Task::none()
            }
        }
    }
}
