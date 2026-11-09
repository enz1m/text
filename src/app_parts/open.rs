//! TextApp open and pick paths.

use super::*;

impl TextApp {
    pub(crate) fn start_pending_opens(
        &mut self,
        pending: Vec<model::PendingOpen>,
    ) -> Task<Message> {
        self.start_pending_opens_settled(pending, capture_settle_requested())
    }

    pub(crate) fn apply_session_restore(
        &mut self,
        generation: u64,
        result: Result<session::SessionRestore, BlockingTaskError>,
    ) -> Task<Message> {
        if generation != self.session_restore_gen {
            return Task::none();
        }
        let Some(pending) = self.session_restore_pending.take() else {
            return Task::none();
        };
        let restore = match result {
            Ok(restore) => restore,
            Err(error) => {
                self.session_store.revoke_write_authority();
                self.session_debt = pending.debt.as_ref().map_or_else(
                    || {
                        self.session_restore_request
                            .clone()
                            .map(|request| session::SessionDebt::from_worker_error(request, error))
                    },
                    |debt| Some(debt.with_worker_error(error)),
                );
                return Task::none();
            }
        };
        let (snapshot, mut store, _origin) = session::SessionStore::install(restore);
        self.session_debt = store.take_debt();
        self.session_store = store;
        if pending.cli_opened || !self.model.settings.restore_session {
            return Task::none();
        }
        self.workspaces = workspace::WorkspaceState::from_persisted(snapshot.workspaces().to_vec());
        let pending_opens = self.model.restore_persisted_tabs(
            &snapshot.tabs().tabs,
            snapshot.tabs().active_path.as_deref(),
        );
        self.model.sync_doc_tabs();
        // A pre-restore over-bound refusal must not latch the restored projection.
        self.advance_session_projection_revision();
        self.start_pending_opens(pending_opens)
    }

    pub(crate) fn dispatch_session_restore(&mut self, retire: bool) -> Task<Message> {
        if self.session_restore_pending.is_some() {
            return Task::none();
        }
        let Some(debt) = self.session_debt.as_ref() else {
            return Task::none();
        };
        let request = debt.request().clone();
        let expected = debt.expected_identity();
        let expected_absent = debt.expected_absent();
        let kind = debt.kind();
        if retire && expected.is_none() {
            return Task::none();
        }
        if !retire && kind == session::SessionDebtKind::DurabilityUncertain {
            return self.dispatch_session_retry_pending();
        }
        if !retire && kind == session::SessionDebtKind::CustodyLost {
            return Task::none();
        }
        // Retry re-preflights the current projection without re-reading the file;
        // revision exhaustion is terminal.
        if !retire && matches!(kind, session::SessionDebtKind::PublishRevisionExhausted) {
            return Task::none();
        }
        if !retire && matches!(kind, session::SessionDebtKind::PublishOverBound(_)) {
            self.session_over_bound_debt = None;
            self.session_retry_blocked = false;
            self.session_debt = None;
            return self.queue_session_sync();
        }
        // A wrapped generation would let a superseded completion match a reused
        // key. On exhaustion write authority stays revoked and the debt is kept
        // for a later restart.
        let Some(generation) = self.session_restore_gen.checked_add(1) else {
            self.picker.failure = Some(enzui_core::localization::text(
                "text-session-restore-generations-are-exhausted-restart-the-eed02a",
            ));
            self.veto_close_for_durability(enzui_core::localization::text(
                "text-close-was-cancelled-because-session-restore-cannot-e227c6",
            ));
            return Task::none();
        };
        self.session_store.revoke_write_authority();
        self.session_restore_gen = generation;
        self.session_restore_request = Some(request.clone());
        self.session_restore_pending = Some(SessionRestorePending {
            // Retry may acquire authority, but it never reseeds over state that
            // became live before the user requested recovery.
            cli_opened: true,
            baseline: session::projection(&self.model, &self.workspaces),
            debt: Some(debt.clone()),
        });
        let worker = self.blocking_tasks.dispatcher();
        let task = if let Some(expected) = retire.then_some(expected).flatten() {
            request.dispatch_retire(&worker, generation, expected)
        } else {
            request.dispatch_retry(&worker, generation, expected, expected_absent)
        };
        Task::perform(task, |settlement| Message::SessionRestoreCompleted {
            generation: settlement.generation(),
            result: settlement.into_result(),
        })
    }

    pub(crate) fn invalidate_session_restore(&mut self) {
        if let Some(pending) = self.session_restore_pending.take() {
            let debt = pending
                .debt
                .as_ref()
                .map(session::SessionDebt::superseded)
                .or_else(|| {
                    self.session_restore_request
                        .clone()
                        .map(session::SessionDebt::superseded_request)
                })
                .or_else(|| self.session_store.superseded_debt());
            self.session_store.revoke_write_authority();
            // Checked so a superseded completion cannot match a wrapped key; on
            // exhaustion a late completion finds no pending restore and is dropped.
            match self.session_restore_gen.checked_add(1) {
                Some(next) => self.session_restore_gen = next,
                None => {
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-session-restore-generations-are-exhausted-restart-the-eed02a",
                    ));
                }
            }
            self.session_debt = debt;
        }
    }

    pub(crate) fn detect_session_projection_change(&mut self) {
        let changed = match self.session_restore_pending.as_ref() {
            // CLI launches install only session authority, never saved tabs or workspaces.
            Some(pending) if pending.cli_opened => false,
            Some(pending) => session::projection(&self.model, &self.workspaces) != pending.baseline,
            None => return,
        };
        if changed {
            self.invalidate_session_restore();
        }
    }

    pub(crate) fn start_pending_opens_settled(
        &mut self,
        pending: Vec<model::PendingOpen>,
        settled: bool,
    ) -> Task<Message> {
        if pending.is_empty() {
            return Task::none();
        }
        let threshold = self.model.large_threshold;
        if settled {
            for open in pending {
                let result = large_doc::load_document(&open.path, threshold);
                // The returned first-page task is dropped: the inline drain below
                // performs that read.
                let superseded = self.finish_document_load(
                    &open.tab_id,
                    &open.path,
                    open.workspace.clone(),
                    open.token,
                    LoadReason::Open,
                    result,
                );
                drop(superseded);
                self.settle_paged_pages(&open.tab_id);
            }
            self.model.sync_doc_tabs();
            self.sync_paged_visible_rows();
            return Task::none();
        }
        // capture.sh greps the log for this marker (`CAPTURE_BUSY_PATTERN`) and
        // fails a capture whose first frame is still optimistic.
        eprintln!(
            "[CAPTURE_BUSY] text-app: startup tabs not settled — {} document \
             load(s) dispatched asynchronously; the first frame shows loading \
             tabs",
            pending.len()
        );
        let dispatcher = self.blocking_tasks.dispatcher();
        let tasks: Vec<Task<Message>> = pending
            .into_iter()
            .map(|open| {
                let path = open.path.clone();
                let path_for_message = open.path.clone();
                let failure_path = open.path.clone();
                let tab_id = open.tab_id.clone();
                let failure_tab = open.tab_id.clone();
                let workspace = open.workspace.clone();
                let failure_workspace = open.workspace.clone();
                let token = open.token;
                let task =
                    dispatcher.submit(0, move |_| large_doc::load_document(&path, threshold));
                Task::perform(task, move |settlement| match settlement.into_result() {
                    Ok(result) => Message::DocumentLoaded {
                        tab_id: tab_id.clone(),
                        workspace: workspace.clone(),
                        path: path_for_message.clone(),
                        token,
                        reason: LoadReason::Open,
                        result,
                    },
                    Err(error) => Message::BlockingWorkFailed(BlockingWorkFailure::Load {
                        tab: failure_tab.clone(),
                        path: failure_path.clone(),
                        workspace: failure_workspace.clone(),
                        token,
                        reason: LoadReason::Open,
                        error,
                    }),
                })
            })
            .collect();
        Task::batch(tasks)
    }

    pub(crate) fn settle_paged_pages(&mut self, tab_id: &str) {
        for _ in 0..4 {
            let Some(tab) = self.model.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                return;
            };
            let Some(doc) = tab.buffer.large_mut() else {
                return;
            };
            let Some(operation) = doc.resolve() else {
                return;
            };
            let result = operation.execute();
            if doc.apply_page(result).is_none() {
                return;
            }
        }
    }

    pub(crate) fn finish_pick(
        &mut self,
        outcome: enzui_widgets::file_chooser::PortalOutcome<
            enzui_widgets::file_chooser::FileResult,
        >,
    ) -> Task<Message> {
        use enzui_widgets::file_chooser::PortalOutcome;

        self.picker.note_outcome(&outcome);

        match outcome {
            PortalOutcome::Completed(result) => {
                let Some(path) = result.first_local_path() else {
                    // A non-local URI (network location).
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-that-location-is-not-a-local-file-f1cdc9",
                    ));
                    return Task::none();
                };
                // The file joins the current named workspace, if any.
                let workspace = match &self.workspaces.active {
                    workspace::ActiveWorkspace::OpenFiles => None,
                    workspace::ActiveWorkspace::Named(root) => Some(root.clone()),
                };
                self.begin_open(&path, workspace)
            }
            PortalOutcome::Cancelled(_) => Task::none(),
            _ => {
                // `note_outcome` already stored the sentence.
                Task::none()
            }
        }
    }

    pub(crate) fn begin_open(
        &mut self,
        path: &std::path::Path,
        workspace: Option<std::path::PathBuf>,
    ) -> Task<Message> {
        let explicit_workspace = workspace.is_some();
        let workspace =
            workspace.or_else(|| self.workspaces.containing_root(path).map(PathBuf::from));
        if let Some(existing_id) = self
            .model
            .tabs
            .iter()
            .find(|tab| tab.file_path.as_deref() == Some(path))
            .map(|tab| tab.id.clone())
        {
            let switching = self.model.active_tab_id != existing_id;
            let mut context_changed = false;
            if let Some(root) = workspace
                && let Some(tab) = self.model.tabs.iter_mut().find(|tab| tab.id == existing_id)
                && (explicit_workspace || tab.workspace_root.is_none())
                && tab.workspace_root.as_ref() != Some(&root)
            {
                tab.workspace_root = Some(root);
                context_changed = true;
            }
            self.model.active_tab_id = existing_id;
            self.sync_cursor_readout();
            if switching || context_changed {
                self.advance_session_projection_revision();
            }
            return Task::none();
        }
        // As in `apply_open_file_in`, an empty, clean, untitled active tab is
        // reused rather than left beside the loading tab.
        let reuse_active = {
            let t = self.model.active_tab();
            t.file_path.is_none() && !t.dirty && t.document().is_empty()
        };
        let tab_id = if reuse_active {
            let t = self.model.active_tab_mut();
            t.file_path = Some(path.to_path_buf());
            t.workspace_root.clone_from(&workspace);
            t.loading = true;
            t.id.clone()
        } else {
            let id = self.model.fresh_tab_id();
            let mut tab = Tab::new(id.clone());
            tab.file_path = Some(path.to_path_buf());
            tab.workspace_root.clone_from(&workspace);
            tab.loading = true;
            tab.opened_from = Some(self.model.active_tab_id.clone());
            self.model.add_tab(tab);
            id
        };
        // Both branches change tab-path membership.
        self.advance_session_projection_revision();
        self.begin_load(&tab_id, path, workspace, LoadReason::Open)
    }

    // Load generations invalidate late completions and held paged descriptors.
    pub(crate) fn begin_load(
        &mut self,
        tab_id: &str,
        path: &std::path::Path,
        workspace: Option<std::path::PathBuf>,
        reason: LoadReason,
    ) -> Task<Message> {
        self.load_debt.remove(tab_id);
        self.load_retry_blocked.remove(tab_id);
        self.paged_debt.remove(tab_id);
        self.paged_retry_blocked.remove(tab_id);
        self.stats_retry_blocked.remove(tab_id);
        self.selection_stats_retry_blocked.remove(tab_id);
        // On token exhaustion the tab keeps its previous token, so an in-flight
        // completion still lands, and the action is refused.
        let Some(token) = self.model.load_tokens.next() else {
            self.picker.failure = Some(enzui_core::localization::text(
                "text-document-load-generations-are-exhausted-restart-the-4b1cbc",
            ));
            return Task::none();
        };
        if let Some(tab) = self.model.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.load_token = token;
            // A superseded paged document must refuse late pages from its old
            // descriptor.
            if let Some(doc) = tab.buffer.large_mut() {
                doc.close();
            }
        } else {
            // A load never creates a tab.
            return Task::none();
        }
        self.dispatch_document_load(
            tab_id.to_owned(),
            path.to_path_buf(),
            workspace,
            token,
            reason,
        )
    }

    pub(crate) fn dispatch_document_load(
        &self,
        tab_id: String,
        path: PathBuf,
        workspace: Option<PathBuf>,
        token: large_doc::LoadToken,
        reason: LoadReason,
    ) -> Task<Message> {
        let threshold = self.model.large_threshold;
        let target = path.clone();
        let failure_tab = tab_id.clone();
        let failure_path = path.clone();
        let failure_workspace = workspace.clone();
        let task = self
            .blocking_tasks
            .submit(0, move |_| large_doc::load_document(&target, threshold));
        Task::perform(task, move |settlement| match settlement.into_result() {
            Ok(result) => Message::DocumentLoaded {
                tab_id: tab_id.clone(),
                workspace: workspace.clone(),
                path: path.clone(),
                token,
                reason,
                result,
            },
            Err(error) => Message::BlockingWorkFailed(BlockingWorkFailure::Load {
                tab: failure_tab.clone(),
                path: failure_path.clone(),
                workspace: failure_workspace.clone(),
                token,
                reason,
                error,
            }),
        })
    }

    pub(crate) fn publish_document_failure(
        &mut self,
        failure: &document_failure::DocumentFailure,
        summary: String,
    ) {
        let diagnostic = failure.escaped_diagnostic().to_owned();
        eprintln!("[text-app] document I/O failed: {diagnostic}");
        self.document_failure_diagnostic = Some(diagnostic);
        self.picker.failure = Some(summary);
    }

    // A completion for a closed tab or with a superseded token is dropped
    // without touching any buffer or recreating the tab.
    pub(crate) fn finish_document_load(
        &mut self,
        tab_id: &str,
        path: &std::path::Path,
        workspace: Option<std::path::PathBuf>,
        token: large_doc::LoadToken,
        reason: LoadReason,
        result: Result<large_doc::LoadedDocument, document_failure::DocumentFailure>,
    ) -> Task<Message> {
        let Some(index) = self.model.tabs.iter().position(|tab| {
            tab.id == tab_id && tab.load_token == token && tab.file_path.as_deref() == Some(path)
        }) else {
            return Task::none();
        };
        let rows = self.initial_paged_visible_rows();
        let backend = std::sync::Arc::clone(&self.file_watch_backend);

        match result {
            Ok(loaded) => {
                let load_identity = loaded.identity;
                let decoded_lossily = loaded.decoded_lossily;
                let first_page =
                    match self
                        .model
                        .install_loaded_document(tab_id, path, loaded.contents, rows)
                    {
                        Ok(operation) => operation,
                        Err(error) => {
                            let summary = match reason {
                                LoadReason::Open => error.open_summary(),
                                LoadReason::Reload => error.reload_summary(),
                            };
                            self.publish_document_failure(&error, summary);
                            return Task::none();
                        }
                    };
                let mut tab_workspace_changed = false;
                if let Some(tab) = self.model.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                    tab.decoded_lossily = decoded_lossily;
                    if reason == LoadReason::Open
                        && let Some(root) = workspace
                    {
                        tab_workspace_changed = tab.workspace_root.as_deref() != Some(&root);
                        tab.workspace_root = Some(root);
                    }
                    tab.install_watcher(backend);
                    // Seeding from the identity actually read makes a pathname
                    // replacement before the first tick report `Replaced`.
                    if let Some(identity) = load_identity
                        && let Some(watcher) = &mut tab.watcher
                    {
                        watcher.acknowledge(identity);
                    }
                }
                // Large buffers arrive without counts; count off the frame.
                let _ = self.spawn_stats(tab_id);
                // Association change: the tab's workspace root advances the
                // projection revision even though the file path is unchanged.
                if tab_workspace_changed {
                    self.advance_session_projection_revision();
                }
                // `sync_doc_tabs()` belongs to the reducer tail; calling it here
                // would mask a missing tail.
                self.sync_cursor_readout();
                match first_page {
                    Some(operation) => self.paged_page_task(tab_id.to_owned(), token, operation),
                    None => Task::none(),
                }
            }
            Err(error) => {
                match reason {
                    LoadReason::Reload => {
                        let summary = error.reload_summary();
                        self.publish_document_failure(&error, summary);
                    }
                    LoadReason::Open => {
                        // Withdraw the optimistic tab; a reused untitled tab resets
                        // instead, so the set never loses its only tab.
                        if self.model.tabs[index].loading {
                            if self.model.tabs.len() == 1 {
                                {
                                    let tab = &mut self.model.tabs[index];
                                    tab.file_path = None;
                                    tab.workspace_root = None;
                                    tab.loading = false;
                                    tab.buffer = large_doc::DocumentBuffer::Editable;
                                }
                                self.advance_session_projection_revision();
                            } else {
                                let removed_id = self.model.tabs[index].id.clone();
                                let opened_from = self.model.tabs[index].opened_from.clone();
                                self.model.tabs.remove(index);
                                if self.model.active_tab_id == removed_id {
                                    // Back to the tab the user opened this from,
                                    // else its neighbour, never an unrelated one.
                                    let back =
                                        opened_from
                                            .filter(|id| {
                                                self.model.tabs.iter().any(|tab| &tab.id == id)
                                            })
                                            .or_else(|| {
                                                self.model
                                                    .tabs
                                                    .get(index.min(
                                                        self.model.tabs.len().saturating_sub(1),
                                                    ))
                                                    .map(|tab| tab.id.clone())
                                            });
                                    if let Some(id) = back {
                                        self.model.active_tab_id = id;
                                    } else {
                                        let id = self.model.fresh_tab_id();
                                        self.model.add_tab(Tab::new(id));
                                    }
                                }
                                self.advance_session_projection_revision();
                            }
                        }
                        let summary = error.open_summary();
                        self.publish_document_failure(&error, summary);
                    }
                }
                Task::none()
            }
        }
    }

    pub(crate) fn paged_page_task(
        &mut self,
        tab_id: String,
        token: large_doc::LoadToken,
        operation: enzui_core::paged_text::PagedTextOperation,
    ) -> Task<Message> {
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            self.paged_debt.insert(tab_id, (token, operation));
            return Task::none();
        }
        let failure_tab = tab_id.clone();
        let failure_operation = RetryPayload::new(operation.clone());
        let worker_localizer = enzui_core::localization::current();
        let task = self
            .blocking_tasks
            .submit(0, move |_| worker_localizer.scope(|| operation.execute()));
        Task::perform(task, move |settlement| match settlement.into_result() {
            Ok(result) => Message::PagedPageLoaded {
                tab_id: tab_id.clone(),
                token,
                result: Box::new(result),
            },
            Err(error) => Message::BlockingWorkFailed(BlockingWorkFailure::Page {
                tab: failure_tab.clone(),
                token,
                operation: failure_operation,
                error,
            }),
        })
    }

    // Borrows another paged tab's published count, else
    // [`large_doc::DEFAULT_VISIBLE_ROWS`]; `sync_paged_visible_rows` replaces it
    // after the first frame.
    pub(crate) fn initial_paged_visible_rows(&self) -> usize {
        self.model
            .tabs
            .iter()
            .filter_map(|tab| tab.buffer.large())
            .filter_map(|doc| doc.viewport_state().resolved())
            .map(|metrics| metrics.visible_rows)
            .max()
            .unwrap_or(large_doc::DEFAULT_VISIBLE_ROWS)
    }

    // The value may lag by one layout pass; the policy's overscan absorbs that.
    pub(crate) fn sync_paged_visible_rows(&mut self) {
        for tab in &mut self.model.tabs {
            if let Some(doc) = tab.buffer.large_mut() {
                let rows = doc.published_visible_rows();
                doc.set_visible_rows(rows);
            }
        }
    }

    pub(crate) fn begin_workspace_read(&mut self, path: PathBuf) -> Task<Message> {
        self.workspace_retry_blocked.remove(&path);
        // A wrapped generation would let a stale `WorkspaceReadSettled` match a
        // reused key; on exhaustion nothing is dispatched or changed.
        let Some(generation) = self.workspace_generation.checked_add(1) else {
            self.picker.failure = Some(enzui_core::localization::text(
                "text-workspace-scan-generations-are-exhausted-restart-the-4c651a",
            ));
            return Task::none();
        };
        self.workspace_generation = generation;
        self.workspace_debt.insert(
            path.clone(),
            PendingWorkspaceRead {
                generation,
                path: path.clone(),
            },
        );
        if self.workspace_inflight.contains_key(&path) {
            Task::none()
        } else {
            self.dispatch_workspace_read(path, generation)
        }
    }

    pub(crate) fn dispatch_workspace_read(
        &mut self,
        path: PathBuf,
        generation: u64,
    ) -> Task<Message> {
        let snapshot = self.blocking_tasks.dispatcher().snapshot();
        if snapshot.live() >= snapshot.live_capacity() {
            return Task::none();
        }
        let read_path = path.clone();
        self.workspace_inflight.insert(path.clone(), generation);
        let task = self
            .blocking_tasks
            .submit(generation, move |_| workspace::read_directory(&read_path));
        Task::perform(task, move |settlement| Message::WorkspaceReadSettled {
            path: path.clone(),
            generation,
            result: settlement.into_result(),
        })
    }

    pub(crate) fn finish_workspace_pick(
        &mut self,
        outcome: enzui_widgets::file_chooser::PortalOutcome<
            enzui_widgets::file_chooser::FileResult,
        >,
    ) {
        use enzui_widgets::file_chooser::PortalOutcome;
        self.picker.note_outcome(&outcome);
        match outcome {
            PortalOutcome::Completed(result) => {
                let Some(path) = result.first_local_path() else {
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-that-location-is-not-a-local-folder-647eda",
                    ));
                    return;
                };
                if !path.is_dir() {
                    self.picker.failure = Some(
                        enzui_core::localized!("text-p0-is-not-a-readable-folder-and-349f55", "p0" => crate::safe_text::bounded_path(&path)),
                    );
                    return;
                }
                if self.workspaces.add(path.clone()) {
                    self.advance_session_projection_revision();
                } else {
                    self.picker.failure = Some(
                        enzui_core::localized!("text-p0-is-already-in-workspaces-f67a52", "p0" => crate::safe_text::bounded_path(&path)),
                    );
                }
            }
            PortalOutcome::Cancelled(_) => {}
            _ => {
                // Never fall back to a guessed folder.
            }
        }
    }

    // Returns `false` when the context did not change (already active or
    // unknown root).
    pub(crate) fn switch_workspace(&mut self, target: workspace::ActiveWorkspace) -> bool {
        if self.workspaces.active == target {
            return false;
        }
        match &target {
            workspace::ActiveWorkspace::OpenFiles => self.workspaces.select_open_files(),
            workspace::ActiveWorkspace::Named(root) => {
                if !self.workspaces.select_workspace(root) {
                    return false;
                }
            }
        }
        if self.sidebar_presentation().is_overlay() {
            self.nav.dismiss_overlay();
        }
        true
    }

    // Success writes the buffer to the picked path and rebinds the tab's
    // `file_path` there. Cancellation changes nothing; failures use Open's
    // banner.
    pub(crate) fn finish_pick_save(
        &mut self,
        outcome: enzui_widgets::file_chooser::PortalOutcome<
            enzui_widgets::file_chooser::FileResult,
        >,
    ) -> SaveAsFinish {
        use enzui_widgets::file_chooser::PortalOutcome;

        self.picker.note_outcome(&outcome);

        match outcome {
            PortalOutcome::Completed(result) => {
                // Recheck read-only: a reload may have reclassified the tab while
                // the dialog was open.
                let active = self.model.active_tab_id.clone();
                if self.refuse_write_if_read_only(&active) {
                    return SaveAsFinish::Failed;
                }
                let Some(path) = result.first_local_path() else {
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-that-location-is-not-a-local-file-320821",
                    ));
                    return SaveAsFinish::Failed;
                };
                // Same line-ending conversion and crash-safe publication as Save,
                // streamed from the snapshot without a second copy.
                let index = self.active_tab_index_for_save();
                self.save_retry_blocked.remove(&active);
                match self.dispatch_streaming_save(index, &path, Some(path.clone())) {
                    SaveDispatch::Failed => SaveAsFinish::Failed,
                    SaveDispatch::InFlight => SaveAsFinish::InFlight,
                }
            }
            PortalOutcome::Cancelled(_) => SaveAsFinish::Cancelled,
            _ => {
                // `failure_text` covers every non-success variant.
                SaveAsFinish::Failed
            }
        }
    }
}
