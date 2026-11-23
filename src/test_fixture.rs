use crate::*;

pub(super) fn inert_app() -> TextApp {
    enzui_widgets::monospace_fallback::ensure_monospace_fallback();
    let model = Model::default();
    let workspaces = workspace::WorkspaceState::default();
    let session_write_baseline = session::persisted_snapshot(&model, &workspaces);
    TextApp {
        desktop: Default::default(),
        startup_editor_focus: None,
        model,
        dialog_focus: None,
        settings_focus: None,
        dialogs: enzui_widgets::modal_host::DialogStack::new(),
        settings_presentation: Default::default(),
        themes: test_theme_integration(EnzTheme::dark()).0,
        nav: NavSplitState::new(),
        close_confirm_state: ConfirmState::new(None),
        workspace_remove_confirm_state: ConfirmState::new(None),
        pending_workspace_remove: None,
        workspace_context_menu: WorkspaceContextMenu::new(),
        workspace_context_epoch: 0,
        close_guard: CloseGuard::new(),
        close_save_attempt: None,
        picker: picker::Picker::new(),
        document_failure_diagnostic: None,
        workspaces,
        session_store: session::SessionStore::inert(),
        session_restore_gen: 0,
        session_restore_pending: None,
        session_restore_request: None,
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
        blocking_tasks: app_blocking_owner(),
        deferred_tasks: Vec::new(),
        tab_strip: crate::tab_strip::TabStripState::default(),
        #[cfg(feature = "center")]
        center: None,
        settings_persistence_enabled: false,
        settings_config_path: std::path::PathBuf::from(TEST_INERT_SETTINGS_PATH),
        settings_generation: 0,
        settings_inflight: None,
        settings_debt: None,
        settings_retry_blocked: false,
        close_request_window: None,
        close_drain: None,
        gutter_scroll_offset: 0.0,
        tab_indent_mode: false,
        recovery_store: crate::recovery::RecoveryStore::inert(),
        recovery_published: std::collections::HashMap::new(),
        recovery_inflight: None,
        recovery_retry: None,
        recovery_retry_blocked: false,
        recovery_gen: 0,
        recovery_discard: None,
        recovery_discard_submitted: false,
        recovery_discard_retry_blocked: false,
        print_retry: None,
        print_preparing: false,
        print_retry_blocked: false,
        blocking_retry_pending: false,
        blocking_retry_suspended: false,
        print: crate::print::PrintState::default(),
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
        virtual_text_surface: crate::a11y::fresh_virtual_text_surface_id(),
    }
}

#[cfg(test)]
impl TextApp {
    pub(crate) fn load_active_buffer(&mut self, new_text: &str) {
        let tab = self.model.active_tab_mut();
        tab.content = text_editor::Content::with_text(new_text);
        tab.set_snapshot(new_text.to_owned());
        tab.dirty = true;
        self.sync_cursor_readout();
    }

    pub(crate) fn for_test() -> Self {
        test_fixture::inert_app()
    }

    pub(crate) fn drain_paged_pages_for_test(&mut self) -> usize {
        let mut delivered = 0;
        for _ in 0..16 {
            let mut requests: Vec<(String, large_doc::LoadToken, _)> = Vec::new();
            for tab in &mut self.model.tabs {
                let id = tab.id.clone();
                let token = tab.load_token;
                if let Some(doc) = tab.buffer.large_mut()
                    && let Some(operation) = doc.resolve()
                {
                    requests.push((id, token, operation));
                }
            }
            if requests.is_empty() {
                return delivered;
            }
            for (tab_id, token, operation) in requests {
                let result = operation.execute();
                let follow_up = self.update(Message::PagedPageLoaded {
                    tab_id,
                    token,
                    result: Box::new(result),
                });

                drop(follow_up);
                delivered += 1;
            }
        }
        delivered
    }

    pub(crate) fn test_with_session(
        settings: settings::Settings,
        path: std::path::PathBuf,
    ) -> Self {
        let (saved, session_store, _origin) = session::SessionStore::open(path, None);
        let mut model = Model::default();
        model.load_settings(settings);
        let pending = restore_startup_tabs(&mut model, &saved, &[]);
        model.sync_doc_tabs();
        let workspaces = workspace::WorkspaceState::from_persisted(saved.workspaces().to_vec());
        let nav = nav_from_settings(&model.settings);
        let session_write_baseline = session::persisted_snapshot(&model, &workspaces);
        let mut app = inert_app();
        app.model = model;
        app.workspaces = workspaces;
        app.nav = nav;
        app.session_store = session_store;
        app.session_write_baseline = session_write_baseline;
        app.recovery_store = recovery::RecoveryStore::disabled();

        drop(app.start_pending_opens_settled(pending, true));
        app
    }
}

#[cfg(test)]
pub(crate) fn with_locale<T>(operation: impl FnOnce() -> T) -> T {
    crate::i18n::for_locales(enzui_core::localization::preferred_locales()).scope(operation)
}
