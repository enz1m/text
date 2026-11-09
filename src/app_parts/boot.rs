//! TextApp boot paths.

use super::*;

impl TextApp {
    // Pure with respect to the process environment, so tests pass literal
    // roots.
    // Precedence: `$XDG_CONFIG_HOME`, then `$HOME/.config`, then
    // `<runtime_fallback>/text-app/settings.conf`, where `runtime_fallback` is
    // the owner-verified 0700 directory from
    // [`enzui_core::runtime_dir::runtime_dir`]. `None` disables
    // persistence rather than writing a predictable world-writable path.
    pub(crate) fn resolve_boot_settings_path(
        xdg_config_home: Option<&std::path::Path>,
        home: Option<&std::path::Path>,
        runtime_fallback: Option<&std::path::Path>,
    ) -> Option<std::path::PathBuf> {
        settings::Settings::config_path_from(xdg_config_home, home)
            .or_else(|| runtime_fallback.map(|dir| dir.join("text-app").join("settings.conf")))
    }

    pub(crate) fn resolve_boot_recovery_path(
        xdg_data_home: Option<&std::path::Path>,
        home: Option<&std::path::Path>,
        runtime_fallback: Option<&std::path::Path>,
    ) -> Option<std::path::PathBuf> {
        let request = session::SessionRequest::for_roots(xdg_data_home, home, runtime_fallback)?;
        // A directory that cannot be established leaves the open to refuse.
        let _ = request.establish_directory();
        Some(request.path().with_file_name("recovery.json"))
    }

    pub(crate) fn is_recoverable(tab: &Tab) -> bool {
        tab.dirty && !tab.is_read_only() && !tab.loading
    }

    pub(crate) fn recovery_has_debt(&self) -> bool {
        if !self.recovery_store.is_enabled() {
            return false;
        }
        for tab in self
            .model
            .tabs
            .iter()
            .filter(|tab| Self::is_recoverable(tab))
            .take(recovery::MAX_RECOVERY_RECORDS)
        {
            if self.recovery_published.get(&tab.id) != Some(&tab.snapshot_version()) {
                return true;
            }
        }
        self.recovery_published.keys().any(|id| {
            !self
                .model
                .tabs
                .iter()
                .filter(|tab| Self::is_recoverable(tab))
                .take(recovery::MAX_RECOVERY_RECORDS)
                .any(|tab| &tab.id == id)
        })
    }

    pub(crate) fn next_recovery_generation(&mut self) -> Option<u64> {
        let next = self.recovery_gen.checked_add(1)?;
        self.recovery_gen = next;
        Some(next)
    }

    // Captures only document handles, never text. One write is in flight at a
    // time, stamped with a monotonic generation so a stale completion cannot
    // resurrect retired recovery.
    pub(crate) fn publish_recovery(&mut self) -> Task<Message> {
        if self.recovery_retry_blocked
            && self.recovery_retry.as_ref().is_some_and(|attempt| {
                self.model.tabs.iter().any(|tab| {
                    Self::is_recoverable(tab)
                        && attempt.captured.get(&tab.id) != Some(&tab.snapshot_version())
                })
            })
        {
            self.recovery_retry = None;
            self.recovery_retry_blocked = false;
        }
        if !self.recovery_store.is_enabled()
            || self.recovery_inflight.is_some()
            // No content write may race a settling Discard's tombstone.
            || self.recovery_discard.is_some()
            || !self.recovery_has_debt()
        {
            return Task::none();
        }
        if self.recovery_retry.is_some() {
            return self.dispatch_recovery_retry();
        }
        let mut leases: Vec<RecoveryLease> = Vec::new();
        let mut captured: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for tab in self
            .model
            .tabs
            .iter()
            .filter(|tab| Self::is_recoverable(tab))
            .take(recovery::MAX_RECOVERY_RECORDS)
        {
            captured.insert(tab.id.clone(), tab.snapshot_version());
            leases.push(recovery_lease_for(tab));
        }
        let Some(generation) = self.next_recovery_generation() else {
            self.recovery_retry_blocked = true;
            self.picker.failure = Some(enzui_core::localization::text(
                "text-recovery-publication-generations-are-exhausted-recovery-is-c43634",
            ));
            self.veto_close_for_durability(enzui_core::localization::text(
                "text-close-was-cancelled-because-recovery-publication-cannot-db1caf",
            ));
            return Task::none();
        };
        let attempt = RecoveryPublishAttempt {
            generation,
            captured,
            leases,
        };
        self.recovery_retry = Some(attempt.clone());
        self.dispatch_recovery_attempt(attempt)
    }

    pub(crate) fn dispatch_recovery_attempt(
        &mut self,
        attempt: RecoveryPublishAttempt,
    ) -> Task<Message> {
        if self.recovery_inflight.is_some() {
            return Task::none();
        }
        let generation = attempt.generation;
        self.recovery_inflight = Some((generation, attempt.captured.clone()));
        let store = self.recovery_store.clone();
        let worker_attempt = attempt;
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                let records = worker_attempt
                    .leases
                    .into_iter()
                    .map(RecoveryLease::into_record)
                    .collect();
                let snapshot = recovery::RecoverySnapshot::from_records(records);
                store.publish(generation, snapshot).committed()
            })
        });
        Task::perform(task, move |settlement| Message::RecoveryWritten {
            generation,
            result: settlement.into_result(),
        })
    }

    pub(crate) fn dispatch_recovery_retry(&mut self) -> Task<Message> {
        if self.recovery_retry_blocked || self.recovery_inflight.is_some() {
            return Task::none();
        }
        let Some(mut attempt) = self.recovery_retry.clone() else {
            return Task::none();
        };
        let Some(generation) = self.next_recovery_generation() else {
            self.recovery_retry_blocked = true;
            self.picker.failure = Some(enzui_core::localization::text(
                "text-recovery-publication-generations-are-exhausted-recovery-is-c43634",
            ));
            self.veto_close_for_durability(enzui_core::localization::text(
                "text-close-was-cancelled-because-recovery-publication-cannot-db1caf",
            ));
            return Task::none();
        };
        attempt.generation = generation;
        self.recovery_retry = Some(attempt.clone());
        self.dispatch_recovery_attempt(attempt)
    }

    pub(crate) fn recovery_write_settled(
        &mut self,
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    ) -> Task<Message> {
        let is_current = matches!(&self.recovery_inflight, Some((g, _)) if *g == generation);
        if !is_current {
            return Task::none();
        }
        let (_, captured) = self
            .recovery_inflight
            .take()
            .expect("matched recovery attempt");
        match result {
            Ok(true) => {
                self.recovery_published = captured;
                self.recovery_retry = None;
                self.recovery_retry_blocked = false;
                if self.recovery_has_debt() {
                    self.blocking_retry_pending = true;
                }
            }
            Err(BlockingTaskError::Busy) => {
                self.blocking_retry_pending = true;
            }
            Err(BlockingTaskError::Cancelled) if self.recovery_gen > generation => {
                self.recovery_retry = None;
            }
            Ok(false) => {
                self.recovery_retry_blocked = true;
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-recovery-data-could-not-be-published-retry-b16f9f",
                ));
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-recovery-data-could-ce4723",
                ));
            }
            Err(
                BlockingTaskError::Cancelled
                | BlockingTaskError::Shutdown
                | BlockingTaskError::Panicked,
            ) => {
                self.recovery_retry_blocked = true;
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-recovery-publication-stopped-retry-close-or-make-e769d2",
                ));
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-recovery-publication-stopped-75ee32",
                ));
            }
            Err(_) => {
                self.recovery_retry_blocked = true;
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-recovery-publication-stopped-retry-close-155f1d",
                ));
                self.veto_close_for_durability(enzui_core::localization::text(
                    "text-close-was-cancelled-because-recovery-publication-stopped-75ee32",
                ));
            }
        }
        Task::none()
    }

    // Freezes content publishing and returns an off-thread task; the window
    // closes on [`Message::RecoveryDiscardSettled`]. Repeated Discards coalesce
    // into one close. A disabled store closes at once.
    pub(crate) fn begin_recovery_discard(&mut self, window: iced::window::Id) -> Task<Message> {
        if self.recovery_discard.is_some() {
            return Task::none();
        }
        if !self.recovery_store.is_enabled() {
            for tab in &mut self.model.tabs {
                tab.dirty = false;
            }
            return self.close_window(window);
        }
        let Some(generation) = self.next_recovery_generation() else {
            self.recovery_discard = Some(RecoveryDiscard::Failed { window });
            self.recovery_discard_retry_blocked = true;
            self.picker.failure = Some(enzui_core::localization::text(
                "text-recovery-cleanup-cannot-advance-its-publication-generation-89425f",
            ));
            return Task::none();
        };
        self.recovery_discard = Some(RecoveryDiscard::Pending { generation, window });
        self.recovery_discard_submitted = false;
        self.recovery_discard_retry_blocked = false;
        self.dispatch_recovery_tombstone()
    }

    pub(crate) fn dispatch_recovery_tombstone(&mut self) -> Task<Message> {
        if self.recovery_discard_submitted || self.recovery_discard_retry_blocked {
            return Task::none();
        }
        let Some(RecoveryDiscard::Pending { generation, .. }) = self.recovery_discard else {
            return Task::none();
        };
        self.recovery_discard_submitted = true;
        let store = self.recovery_store.clone();
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                store
                    .publish(generation, recovery::RecoverySnapshot::default())
                    .committed()
            })
        });
        Task::perform(task, move |settlement| Message::RecoveryDiscardSettled {
            generation,
            result: settlement.into_result(),
        })
    }

    // A stale generation is ignored. On success recovery is retired and the
    // deferred close fires once; on failure the window stays open in the
    // `Failed` state and recovery is kept.
    pub(crate) fn recovery_discard_settled(
        &mut self,
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    ) -> Task<Message> {
        let Some(RecoveryDiscard::Pending {
            generation: pending,
            window,
        }) = self.recovery_discard
        else {
            return Task::none();
        };
        if pending != generation {
            return Task::none();
        }
        self.recovery_discard_submitted = false;
        match result {
            Ok(true) => {
                self.recovery_discard = None;
                self.recovery_published.clear();
                self.recovery_inflight = None;
                self.recovery_retry = None;
                for tab in &mut self.model.tabs {
                    tab.dirty = false;
                }
                self.close_window(window)
            }
            Err(BlockingTaskError::Busy) => {
                self.blocking_retry_pending = true;
                Task::none()
            }
            Err(BlockingTaskError::Cancelled) if self.recovery_gen > generation => Task::none(),
            Ok(false)
            | Err(
                BlockingTaskError::Cancelled
                | BlockingTaskError::Shutdown
                | BlockingTaskError::Panicked,
            ) => {
                self.recovery_discard_retry_blocked = true;
                self.recovery_discard = Some(RecoveryDiscard::Failed { window });
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-recovery-cleanup-did-not-complete-retry-or-150e3d",
                ));
                Task::none()
            }
            Err(_) => {
                self.recovery_discard_retry_blocked = true;
                self.recovery_discard = Some(RecoveryDiscard::Failed { window });
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-recovery-cleanup-stopped-retry-or-keep-recovery-43ffe4",
                ));
                Task::none()
            }
        }
    }

    // Retry re-dispatches the tombstone; Close without clearing closes the
    // window and keeps recovery; Cancel abandons the discard.
    pub(crate) fn recovery_discard_choice(&mut self, event: SaveChangesEvent) -> Task<Message> {
        let action = match event {
            SaveChangesEvent::Action(action) => action,
            SaveChangesEvent::FocusNext => {
                return Task::done(Message::DialogEvent(
                    enzui_widgets::modal_host::ModalHostEvent::FocusNext,
                ));
            }
            SaveChangesEvent::FocusPrevious => {
                return Task::done(Message::DialogEvent(
                    enzui_widgets::modal_host::ModalHostEvent::FocusPrevious,
                ));
            }
        };
        let Some(RecoveryDiscard::Failed { window }) = self.recovery_discard else {
            return Task::none();
        };
        match action {
            // "Retry".
            SaveChangesAction::SaveAll => {
                let Some(generation) = self.next_recovery_generation() else {
                    self.recovery_discard = Some(RecoveryDiscard::Failed { window });
                    self.recovery_discard_retry_blocked = true;
                    self.picker.failure = Some(enzui_core::localization::text(
                        "text-recovery-cleanup-cannot-advance-its-publication-generation-89425f",
                    ));
                    return Task::none();
                };
                self.recovery_discard = Some(RecoveryDiscard::Pending { generation, window });
                self.recovery_discard_submitted = false;
                self.recovery_discard_retry_blocked = false;
                self.dispatch_recovery_tombstone()
            }
            // "Close without clearing": recovery is kept.
            SaveChangesAction::Discard => {
                self.recovery_discard = None;
                for tab in &mut self.model.tabs {
                    tab.dirty = false;
                }
                self.close_window(window)
            }
            // "Keep window open".
            SaveChangesAction::Cancel => {
                self.recovery_discard = None;
                let action = self.close_guard.decide(CloseDecision::Cancel);
                self.apply_close_guard_action(action)
            }
        }
    }

    pub(crate) fn begin_print(&mut self) -> Task<Message> {
        let tab = self.model.active_tab();
        if !tab.supports(large_doc::Capability::Save) || self.print.is_busy() {
            return Task::none();
        }
        let snapshot = tab.snapshot_handle();
        let title = crate::safe_text::bounded_inline(&tab.title());
        // Refuse rather than wrap the print counter.
        let Some(generation) = self.print.begin() else {
            self.picker.failure = Some(enzui_core::localization::text(
                "text-print-generations-are-exhausted-restart-the-app-b562f1",
            ));
            return Task::none();
        };
        self.print_retry_blocked = false;
        let attempt = PrintAttempt {
            generation,
            snapshot,
            title,
        };
        self.print_retry = Some(attempt.clone());
        self.dispatch_print_attempt(attempt)
    }

    pub(crate) fn dispatch_print_attempt(&mut self, attempt: PrintAttempt) -> Task<Message> {
        if self.print_preparing {
            return Task::none();
        }
        self.print_preparing = true;
        let generation = attempt.generation;
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                let text = attempt.snapshot.to_string();
                print::prepare_document(&text, &attempt.title)
            })
        });
        Task::perform(task, move |settlement| match settlement.into_result() {
            Ok(prepared) => Message::PrintPrepared {
                generation,
                prepared,
            },
            Err(error) => {
                Message::BlockingWorkFailed(BlockingWorkFailure::Print { generation, error })
            }
        })
    }

    pub(crate) fn finish_print_prepare(
        &mut self,
        generation: u64,
        prepared: Result<print::PreparedHandoff, print::PrintRenderError>,
    ) -> Task<Message> {
        if self.print.current_generation() != Some(generation) || self.print.has_admitted_inflight()
        {
            if self
                .print_retry
                .as_ref()
                .is_some_and(|attempt| attempt.generation == generation)
            {
                self.print_retry = None;
                self.print_preparing = false;
            }
            return Task::none();
        }
        self.print_retry = None;
        self.print_preparing = false;
        let handoff = match prepared {
            Ok(handoff) => handoff,
            Err(error) => {
                self.print.render_failed(generation, Some(&error));
                return Task::none();
            }
        };
        let Some(document) = handoff.take() else {
            return Task::none();
        };
        if !self
            .picker
            .start_print_parent(generation, print::PreparedHandoff::new(document))
        {
            self.print.render_failed(generation, None);
        }
        Task::none()
    }

    pub(crate) fn finish_print_parent(
        &mut self,
        generation: u64,
        prepared: print::PreparedHandoff,
        parent: Option<crate::portal_parent::ParentHandoff>,
    ) -> Task<Message> {
        if self.print.current_generation() != Some(generation) || self.print.has_admitted_inflight()
        {
            return Task::none();
        }
        let Some(parent) = parent else {
            self.print.render_failed(generation, None);
            return Task::none();
        };
        let Some(parent) = parent.take() else {
            return Task::none();
        };
        let Some(document) = prepared.take() else {
            return Task::none();
        };
        if !parent.is_live(self.picker.current_window()) {
            self.print.render_failed(generation, None);
            return Task::none();
        }
        self.print.submit_parented(generation, document, parent);
        Task::none()
    }

    pub(crate) const UNRESOLVED_SETTINGS_PATH: &'static str =
        "/dev/null/text-app-unresolved/settings.conf";

    pub(crate) fn boot(
        intent: &enzui_core::activation::ApplicationIntent,
    ) -> (Self, Task<Message>) {
        #[cfg(feature = "center")]
        {
            let (mut app, task) = Self::boot_at(&BootEnv::from_launch(intent));
            app.start_center();
            (app, task)
        }
        #[cfg(not(feature = "center"))]
        Self::boot_at(&BootEnv::from_launch(intent))
    }

    // The theme host and monospace-font installer still read process state.
    pub(crate) fn boot_at(env: &BootEnv) -> (Self, Task<Message>) {
        // iced bundles no monospace face on Linux, where monospace otherwise
        // renders oblique.
        enzui_widgets::monospace_fallback::install_monospace_fallback();

        // Test builds use a deterministic compiled dark theme so fixtures never
        // touch the environment or filesystem.
        #[cfg(not(test))]
        let (themes, theme_boot) = ThemeIntegration::boot();
        #[cfg(test)]
        let (themes, theme_boot) = test_theme_integration(EnzTheme::dark());
        let mut model = Model::default();
        // Resolved before load so load and save agree on the file.
        let resolved_settings_path = Self::resolve_boot_settings_path(
            env.xdg_config_home.as_deref(),
            env.home.as_deref(),
            env.runtime_fallback.as_deref(),
        );
        let settings_persistence_enabled = resolved_settings_path.is_some();
        let settings_config_path = resolved_settings_path
            .unwrap_or_else(|| std::path::PathBuf::from(Self::UNRESOLVED_SETTINGS_PATH));
        model.load_settings(
            settings::Settings::load_from(&settings_config_path).unwrap_or_default(),
        );
        // The session file is read off-thread (`Message::SessionRestoreCompleted`)
        // so the first frame draws first; until then a disabled placeholder
        // store refuses every write.
        let (empty_session, session_store, _placeholder_origin) =
            session::SessionStore::open_disabled();
        let mut picker = picker::Picker::new();
        // Recovered records restore after the session's tabs, so both coexist.
        let (recovery_records, recovery_store, recovery_origin) =
            match Self::resolve_boot_recovery_path(
                env.xdg_data_home.as_deref(),
                env.home.as_deref(),
                env.runtime_fallback.as_deref(),
            ) {
                Some(recovery_path) => recovery::RecoveryStore::open(recovery_path),
                None => (
                    Vec::new(),
                    recovery::RecoveryStore::disabled(),
                    recovery::Origin::Missing,
                ),
            };
        // Generations continue above the on-disk value to stay monotonic.
        let recovery_boot_generation = recovery_store.published_generation();
        let workspaces =
            workspace::WorkspaceState::from_persisted(empty_session.workspaces().to_vec());

        // Session tabs are seeded when the off-thread read completes, unless a
        // user action supersedes it.
        let pending_startup = restore_startup_tabs(&mut model, &empty_session, &env.cli_args);
        let session_cli_opened = !pending_startup.is_empty();
        let session_restore_request = session::SessionRequest::for_roots(
            env.xdg_data_home.as_deref(),
            env.home.as_deref(),
            env.runtime_fallback.as_deref(),
        );
        let session_path_resolved = session_restore_request.is_some();
        let blocking_tasks = app_blocking_owner();
        let session_restore_task =
            session_restore_request
                .as_ref()
                .map_or_else(Task::none, |request| {
                    Task::perform(
                        request.clone().dispatch(&blocking_tasks.dispatcher(), 0),
                        |settlement| Message::SessionRestoreCompleted {
                            generation: settlement.generation(),
                            result: settlement.into_result(),
                        },
                    )
                });
        // Navigation starts in Open Files; tabs are global across contexts.

        // Recovered buffers never overwrite disk bytes; disk-changed ones become
        // external-change conflicts. A future-version session banner keeps the
        // disclosure slot.
        let recovery_report = restore_recovered_buffers(&mut model, recovery_records);
        if picker.failure.is_none()
            && let Some(text) = recovery_boot_disclosure(&recovery_report, &recovery_origin)
        {
            picker.failure = Some(text);
        }

        model.sync_doc_tabs();

        // Baseline captured after every boot-time tab/workspace restoration, so
        // only a post-boot projection mutation can diverge from it.
        let session_restore_pending = session_path_resolved.then(|| SessionRestorePending {
            cli_opened: session_cli_opened,
            baseline: session::projection(&model, &workspaces),
            debt: None,
        });

        let task = theme_boot.map(|event| Message::Theme(Box::new(event)));

        // The stored sidebar preference, not a presentation: a narrow window
        // still collapses it.
        let nav = nav_from_settings(&model.settings);
        let session_write_baseline = session::persisted_snapshot(&model, &workspaces);

        let mut app = Self {
            desktop: Default::default(),
            model,
            dialog_focus: None,
            settings_focus: None,
            dialogs: enzui_widgets::modal_host::DialogStack::new(),
            settings_presentation: Default::default(),
            themes,
            nav,
            close_confirm_state: ConfirmState::new(None),
            workspace_remove_confirm_state: ConfirmState::new(None),
            pending_workspace_remove: None,
            workspace_context_menu: WorkspaceContextMenu::new(),
            workspace_context_epoch: 0,
            close_guard: CloseGuard::new(),
            close_save_attempt: None,
            picker,
            document_failure_diagnostic: None,
            workspaces,
            session_store,
            settings_persistence_enabled,
            settings_config_path,
            settings_generation: 0,
            settings_inflight: None,
            settings_debt: None,
            settings_retry_blocked: false,
            close_request_window: None,
            close_drain: None,
            gutter_scroll_offset: 0.0,
            tab_indent_mode: false,
            recovery_store,
            recovery_published: std::collections::HashMap::new(),
            recovery_inflight: None,
            recovery_retry: None,
            recovery_retry_blocked: false,
            recovery_gen: recovery_boot_generation,
            session_restore_gen: 0,
            session_restore_pending,
            session_restore_request,
            session_debt: None,
            session_notice_dismissed: None,
            authority_notice_dismissed: std::collections::BTreeSet::new(),
            large_load_notice_dismissed: None,
            session_write_generation: 0,
            session_write_inflight: false,
            session_write_baseline,
            session_write_debt: None,
            session_over_bound_debt: None,
            session_projection_revision: 1,
            session_projection_revision_exhausted: false,
            session_retry_blocked: false,
            session_discard_window: None,
            blocking_tasks,
            deferred_tasks: Vec::new(),
            tab_strip: crate::tab_strip::TabStripState::default(),
            #[cfg(feature = "center")]
            center: None,
            recovery_discard: None,
            recovery_discard_submitted: false,
            recovery_discard_retry_blocked: false,
            print_retry: None,
            print_preparing: false,
            print_retry_blocked: false,
            blocking_retry_pending: false,
            blocking_retry_suspended: false,
            print: print::PrintState::default(),
            search_scan: None,
            search_attempt: None,
            search_retry: None,
            search_retry_blocked: false,
            search_queued: None,
            stats_attempts: std::collections::HashMap::new(),
            stats_retry: std::collections::HashMap::new(),
            stats_retry_blocked: std::collections::HashSet::new(),
            selection_stats_attempts: std::collections::HashMap::new(),
            selection_stats_retry: std::collections::HashMap::new(),
            selection_stats_retry_blocked: std::collections::HashSet::new(),
            pump_cursor: 0,
            load_debt: std::collections::HashMap::new(),
            load_retry_blocked: std::collections::HashSet::new(),
            paged_debt: std::collections::HashMap::new(),
            paged_retry_blocked: std::collections::HashSet::new(),
            save_destinations: std::collections::HashMap::new(),
            save_inflight: std::collections::HashMap::new(),
            save_debt: std::collections::HashMap::new(),
            save_retry_blocked: std::collections::HashSet::new(),
            workspace_generation: 0,
            workspace_debt: std::collections::HashMap::new(),
            workspace_inflight: std::collections::HashMap::new(),
            workspace_retry_blocked: std::collections::HashSet::new(),
            watcher_generation: 0,
            watcher_inflight: false,
            watcher_debt: false,
            watcher_retry: None,
            watcher_retry_paths: Vec::new(),
            watcher_retry_blocked: false,
            selection_settle_probe: None,
            gutter_window_first_line: std::cell::Cell::new(0),
            gutter_window_placement: std::cell::Cell::new(None),
            gutter_window_size: std::cell::Cell::new(None),
            file_watch_backend: std::sync::Arc::new(FsBackend),
            bound_virtual_text: None,
            bound_virtual_text_wake: None,
            virtual_text_surface: a11y::fresh_virtual_text_surface_id(),
        };
        // Under the capture settle gate these complete before the first frame.
        let startup = app.start_pending_opens(pending_startup);
        #[cfg(not(test))]
        app.sync_virtual_text_binding();
        (app, Task::batch([task, startup, session_restore_task]))
    }
}
