//! TextApp root update entry point.

use super::*;

impl TextApp {
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        // The crash ring must see every message before any message-specific
        // branch can return.
        enzui_core::crash_report::observe(&message);
        if matches!(message, Message::RevealActiveTab) {
            return self.reveal_active_tab();
        }
        let blocking_terminal = message.is_blocking_terminal();
        let busy_blocking_terminal = message.is_busy_blocking_terminal();
        let blocking_retry_tick = matches!(message, Message::BlockingRetryTick);
        let close_drain_tick = matches!(message, Message::CloseDrainTick);
        // A watch turn that changed nothing on screen skips the gutter sync,
        // which is a second view rebuild.
        let watch_turn = matches!(
            message,
            Message::FileWatchTick | Message::FileWatchBatchSettled { .. }
        )
        .then(|| self.watch_visible_state());
        let unrelated_turn_with_retry_debt =
            self.blocking_retry_pending && !blocking_terminal && !blocking_retry_tick;
        let a11y_task = self.begin_a11y_turn(&message);

        // Post-layout publication must not repeat document/session reconciliation.
        if matches!(message, Message::SyncGutterScroll) {
            self.publish_virtual_text_state();
            self.sync_virtual_text_binding();
            return a11y_task
                .chain(self.sync_gutter_scroll())
                .chain(Task::done(Message::RevealActiveTab));
        }
        // Modal ownership is reconciled on both sides of every reducer turn.
        self.sync_settings_presentation();
        let before_focus = self.sync_dialog_focus();
        let task = self.update_inner(message);
        self.sync_settings_presentation();
        self.settle_desktop_drops();
        let desktop = self.flush_desktop_opens();
        let parent = self.picker.take_parent_task();
        let task = Task::batch([task, desktop, parent]);
        let after_focus = self.sync_dialog_focus();
        let task = Task::batch([a11y_task, before_focus, task, after_focus]);
        // The paged viewer's row count depends on the banners drawn above it,
        // which this message may have raised.
        self.sync_paged_visible_rows();
        // `model.doc_tabs` is derived; reconciling once per message keeps it
        // from drifting from `tabs`/`active_tab_id`.
        self.model.sync_doc_tabs();
        self.refresh_desktop_search();
        // Supersede an in-flight boot restore before it can seed over a
        // projection the user changed this turn.
        self.detect_session_projection_change();
        if busy_blocking_terminal {
            self.blocking_retry_pending = true;
        }
        if blocking_retry_tick {
            self.blocking_retry_pending = false;
        }
        let (session, settings, recovery, print_retry) =
            if busy_blocking_terminal || close_drain_tick || unrelated_turn_with_retry_debt {
                (Task::none(), Task::none(), Task::none(), Task::none())
            } else {
                let session = if blocking_retry_tick
                    && self
                        .session_debt
                        .as_ref()
                        .is_some_and(|debt| debt.kind() == session::SessionDebtKind::Busy)
                {
                    self.dispatch_session_restore(false)
                } else {
                    self.queue_session_sync()
                };
                self.pump_pending_work();
                let settings = self.dispatch_settings_debt();
                let recovery = if blocking_retry_tick
                    && matches!(self.recovery_discard, Some(RecoveryDiscard::Pending { .. }))
                {
                    self.dispatch_recovery_tombstone()
                } else if self.close_drain.is_some() || blocking_retry_tick {
                    self.publish_recovery()
                } else {
                    Task::none()
                };
                let print_retry =
                    if blocking_retry_tick && !self.print_preparing && !self.print_retry_blocked {
                        self.print_retry
                            .clone()
                            .map_or_else(Task::none, |attempt| self.dispatch_print_attempt(attempt))
                    } else {
                        Task::none()
                    };
                (session, settings, recovery, print_retry)
            };
        let deferred = Task::batch(std::mem::take(&mut self.deferred_tasks));
        let close = self.drive_close_drain();
        // IME and widget messages can enqueue a mutation this same turn; drain
        // again before publishing app-originated state.
        self.drain_virtual_text_mutations();
        self.publish_virtual_text_state();
        self.sync_virtual_text_binding();
        self.blocking_retry_pending = self.has_retryable_blocking_debt();
        self.sync_desktop_activity();
        let quiet_watch_turn =
            watch_turn.is_some_and(|before| before == self.watch_visible_state());
        let task = if quiet_watch_turn {
            task
        } else {
            task.chain(Task::done(Message::SyncGutterScroll))
        };
        Task::batch([
            task,
            settings,
            session,
            recovery,
            print_retry,
            deferred,
            close,
        ])
    }

    pub(crate) fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::RevealActiveTab => self.reveal_active_tab(),
            Message::TabStripScrolled(viewport) => {
                self.tab_strip.offset = viewport.absolute_offset().x;
                Task::none()
            }
            Message::TabStripWheel(delta) => self.reduce_tab_strip_wheel(delta),
            Message::TabStripEvent(event) => self.reduce_tab_strip_event(event),
            Message::Desktop(event) => self.reduce_desktop(event),
            Message::NoOp | Message::A11yWake(_) | Message::BlockingRetryTick => Task::none(),
            Message::VirtualInputMethod(event) => self.reduce_workers_virtual_input_method(event),
            // Normally intercepted by `update`; handled here so the reducer
            // stays total for direct `update_inner` calls.
            Message::SyncGutterScroll => self.sync_gutter_scroll(),
            Message::BlockingWorkFailed(failure) => self.reduce_blocking_failed(failure),
            Message::SearchCompleted {
                generation,
                from,
                result,
                binding,
            } => self.reduce_workers_search_completed(generation, from, result, binding),
            Message::StatsCompleted {
                tab,
                generation,
                text,
                document,
            } => self.reduce_workers_stats_completed(tab, generation, text, document),
            Message::SelectionStatsCompleted { tab, key, stats } => {
                self.reduce_workers_selection_stats_completed(tab, key, stats)
            }

            // Statistics debounce: a selection is copied to a worker only after
            // two consecutive ticks see the same selection generation.
            Message::SelectionStatsSettleTick => self.reduce_workers_selection_stats_settle_tick(),

            Message::SaveCompleted {
                tab,
                generation,
                destination,
                rebind,
                guard_attempt,
                result,
            } => self.reduce_save_save_completed(
                tab,
                generation,
                destination,
                rebind,
                guard_attempt,
                result,
            ),
            Message::EditorDragUpdated { tab, cursor } => {
                self.reduce_save_editor_drag_updated(tab, cursor)
            }
            Message::EditorAction(action) => self.reduce_editor_action(action),
            #[cfg(feature = "center")]
            Message::Center(event) => self.center_event(event),
            Message::VirtualCopy => self.reduce_clipboard_virtual_copy(),
            Message::VirtualCut => self.reduce_clipboard_virtual_cut(),

            Message::NewTab => self.reduce_tab_new_tab(),

            Message::CloseTab(id) => self.reduce_tab_close_tab(id),

            Message::CloseActiveTab => self.reduce_tab_close_active_tab(),

            Message::SwitchTab(id) => self.reduce_tab_switch_tab(id),

            Message::NextTab => self.reduce_tab_next_tab(),

            Message::PrevTab => self.reduce_tab_prev_tab(),

            Message::SaveFile => self.reduce_file_save_file(),
            // Shares `PickerTick` with Open; the picker tags the operation so
            // the outcome reaches `finish_pick_save`, which also rebinds
            // `file_path` so the next Ctrl+S targets the new path.
            Message::SaveFileAs => self.reduce_file_save_file_as(),

            Message::OpenFile => self.reduce_file_open_file(),
            Message::PickerTick => self.reduce_file_picker_tick(),
            Message::PickerEvent(event) => self.picker.handle(event),
            Message::PickerParent(event) => self.picker.parent_event(event),
            Message::DismissPickerFailure => self.reduce_file_dismiss_picker_failure(),
            Message::SessionRestoreCompleted { generation, result } => {
                self.reduce_session_session_restore_completed(generation, result)
            }
            Message::SessionWriteSettled {
                generation,
                handoff,
                result,
            } => self.session_write_settled(generation, handoff, result),
            Message::SessionDiscardSettled {
                generation,
                window,
                handoff,
                result,
            } => self.session_discard_settled(generation, window, handoff, result),
            Message::SessionRestoreRetry => {
                self.session_notice_dismissed = None;
                self.dispatch_session_restore(false)
            }
            Message::SessionRestoreRetire => {
                self.session_notice_dismissed = None;
                self.dispatch_session_restore(true)
            }
            Message::DismissSessionNotice => {
                self.session_notice_dismissed =
                    self.session_debt.as_ref().map(session::SessionDebt::kind);
                Task::none()
            }
            Message::DismissAuthorityNotice => {
                let tab_id = self.model.active_tab().id.clone();
                self.authority_notice_dismissed.insert(tab_id);
                Task::none()
            }
            Message::DismissLargeLoadNotice => {
                let tab = self.model.active_tab();
                self.large_load_notice_dismissed = tab
                    .buffer
                    .large()
                    .and_then(super::large_doc::LargeDocument::load_error)
                    .map(|error| (tab.id.clone(), error));
                Task::none()
            }
            Message::SessionRestoreOpenLocation => {
                self.reduce_session_session_restore_open_location()
            }
            Message::AddWorkspace => self.reduce_session_add_workspace(),
            Message::WorkspaceSelect(root) => self.reduce_session_workspace_select(root),
            Message::WorkspaceToggle(path) => match self.workspaces.begin_toggle(&path) {
                Some(pending) => self.begin_workspace_read(pending),
                None => {
                    self.workspace_debt.remove(&path);
                    self.workspace_retry_blocked.remove(&path);
                    Task::none()
                }
            },
            Message::WorkspaceReadSettled {
                path,
                generation,
                result,
            } => {
                if self.workspace_inflight.get(&path) == Some(&generation) {
                    self.workspace_inflight.remove(&path);
                }
                let current = self
                    .workspace_debt
                    .get(&path)
                    .is_some_and(|pending| pending.generation == generation);
                if !current {
                    return Task::none();
                }
                match result {
                    Ok(load) => {
                        self.workspace_debt.remove(&path);
                        self.workspace_retry_blocked.remove(&path);
                        let _ = self.workspaces.apply_directory_load(&path, load);
                    }
                    Err(BlockingTaskError::Busy) => {}
                    Err(_) => {
                        self.workspace_retry_blocked.insert(path.clone());
                        self.picker.failure = Some(enzui_core::localization::text(
                            "text-the-folder-could-not-be-listed-retry-639b17",
                        ));
                    }
                }
                Task::none()
            }
            Message::WorkspaceSidebarScrolled(offset) => {
                self.reduce_session_workspace_sidebar_scrolled(offset)
            }
            Message::WorkspaceOpenFile { workspace, path } => {
                self.reduce_session_workspace_open_file(workspace, path)
            }
            Message::DocumentLoaded {
                tab_id,
                workspace,
                path,
                token,
                reason,
                result,
            } => {
                self.reduce_document_document_loaded(tab_id, workspace, path, token, reason, result)
            }
            Message::PagedPageLoaded {
                tab_id,
                token,
                result,
            } => self.reduce_document_paged_page_loaded(tab_id, token, *result),
            Message::PagedNavigate(movement) => self.reduce_document_paged_navigate(movement),
            Message::WorkspaceContext(root, at) => self.reduce_context_workspace_context(root, at),
            Message::SettingsSaved { generation, result } => {
                self.reduce_context_settings_saved(generation, result)
            }

            Message::WorkspaceContextEvent(event) => {
                self.reduce_context_workspace_context_event(event)
            }

            Message::WorkspaceRemoveEvent(event) => {
                self.reduce_context_workspace_remove_event(event)
            }

            Message::ToggleCounts => self.reduce_toggles_toggle_counts(),

            Message::ToggleLineNumbers => self.reduce_toggles_toggle_line_numbers(),

            Message::ToggleWordWrap => self.reduce_toggles_toggle_word_wrap(),

            // Tab-indent mode is runtime-only. Ctrl+M toggles it;
            // `SetTabIndentMode(false)` is the Escape return to navigation. The
            // paged viewer has no editable buffer and ignores both.
            Message::ToggleTabIndentMode => self.reduce_toggles_toggle_tab_indent_mode(),
            Message::SetTabIndentMode(on) => self.reduce_toggles_set_tab_indent_mode(on),

            Message::ToggleFind => self.reduce_find_toggle_find(),
            Message::OpenFind => self.reduce_find_open_find(),
            Message::OpenReplace => self.reduce_find_open_replace(),
            Message::CloseFind => self.reduce_find_close_find(),
            Message::FindQueryChanged(q) => self.reduce_find_find_query_changed(q),
            Message::ReplaceValueChanged(v) => self.reduce_find_replace_value_changed(v),
            Message::FindNext => self.reduce_find_find_next(),
            Message::FindPrev => self.reduce_find_find_prev(),
            Message::ReplaceCurrent => self.reduce_find_replace_current(),
            Message::ReplaceAll => self.reduce_find_replace_all(),
            Message::ToggleCaseSensitive => self.reduce_find_toggle_case_sensitive(),
            Message::ToggleWholeWord => self.reduce_find_toggle_whole_word(),
            Message::ToggleRegex => self.reduce_find_toggle_regex(),
            Message::SetReplaceScope(idx) => self.reduce_find_set_replace_scope(idx),
            Message::ToggleReplaceBar => self.reduce_find_toggle_replace_bar(),

            Message::OpenGoto => self.reduce_goto_open_goto(),
            Message::CloseGoto => self.reduce_goto_close_goto(),
            Message::GotoQueryChanged(q) => self.reduce_goto_goto_query_changed(q),
            Message::GotoSubmit => self.reduce_goto_goto_submit(),

            Message::OpenStats => self.reduce_goto_open_stats(),
            Message::CloseStats => self.reduce_goto_close_stats(),
            Message::OpenSettings => self.reduce_goto_open_settings(),
            Message::CloseSettings => self.reduce_goto_close_settings(),
            Message::SettingsExitSettled(key) => self.reduce_goto_settings_exit_settled(key),
            Message::OpenShortcuts => self.reduce_goto_open_shortcuts(),
            Message::CloseShortcuts => self.reduce_goto_close_shortcuts(),
            Message::SetFontSize(scale) => self.reduce_goto_set_font_size(scale),
            Message::SetEditorTheme(theme) => {
                self.model.settings.editor_theme = theme;
                self.save_settings()
            }
            Message::ToggleMarkdownPreview { tab, version } => {
                if self.model.active_tab_id != tab
                    || self.model.active_tab().snapshot_version() != version
                {
                    return Task::none();
                }
                if self.model.active_tab_mut().toggle_preview() {
                    if self.model.active_tab().preview_visible() {
                        Task::none()
                    } else {
                        restore_focus_task(editor_id())
                    }
                } else {
                    self.picker.failure = Some(crate::i18n::domain_text(
                        "Markdown preview is limited to 1 MiB and 4096 display elements. Source editing remains available.",
                    ));
                    Task::none()
                }
            }
            Message::MarkdownPreviewScrolled {
                tab,
                version,
                offset,
            } => {
                if let Some(tab) = self
                    .model
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tab && t.snapshot_version() == version)
                {
                    tab.preview.scroll =
                        enzui_core::metrics::FiniteNonNeg::new_or(offset, 0.0).get();
                }
                Task::none()
            }
            Message::WorkspaceAccentChosen {
                root,
                epoch,
                accent,
            } => self.choose_workspace_accent(root, epoch, accent),
            Message::ToggleRestoreSession => self.reduce_goto_toggle_restore_session(),
            Message::ToggleLanguageMenu => self.reduce_goto_toggle_language_menu(),
            Message::SetLanguage(label) => self.reduce_goto_set_language(label),

            Message::CycleLineEnding => self.reduce_goto_cycle_line_ending(),
            Message::SetLineEnding(ending) => self.reduce_goto_set_line_ending(ending),

            Message::CloseConfirmEvent(event) => self.reduce_toggles_close_confirm_event(event),

            Message::ToggleOverflowMenu => self.reduce_chrome_toggle_overflow_menu(),
            Message::CloseOverflowMenu => self.reduce_chrome_close_overflow_menu(),

            Message::ToggleTabList => self.reduce_chrome_toggle_tab_list(),
            // The single dismissal path for anchored popovers; a completed click
            // outside closes them. Idempotent.
            Message::DismissPopovers => self.reduce_chrome_dismiss_popovers(),

            Message::SidebarState(state) => self.reduce_chrome_sidebar_state(state),
            Message::DismissSidebar => self.reduce_chrome_dismiss_sidebar(),
            Message::DialogEvent(event) => self.reduce_dialog_host(event),
            Message::SettingsDialogEvent(event) => self.reduce_settings_host(event),
            Message::StackDialogEvent(event) => self.reduce_stack_host(event),
            Message::FocusNext => self.reduce_focus_focus_next(),
            Message::FocusPrevious => self.reduce_focus_focus_previous(),

            Message::CloseRequested(request) => self.reduce_focus_close_requested(request),
            Message::WindowCloseSaveAll => self.reduce_focus_window_close_save_all(),
            Message::WindowCloseDiscard => self.reduce_focus_window_close_discard(),
            Message::WindowCloseCancel => self.reduce_focus_window_close_cancel(),
            Message::Window(command) => self.reduce_focus_window(command),
            Message::Resized(w, h) => self.reduce_shell_resized(w, h),

            // Shared-theme-host events fold through the framework integration;
            // views read the resolved `EnzTheme` from `self.themes`.
            Message::Theme(event) => self.reduce_shell_theme(*event),

            Message::RecoveryTick => self.publish_recovery(),
            Message::RecoveryWritten { generation, result } => {
                self.reduce_shell_recovery_written(generation, result)
            }
            Message::RecoveryDiscardSettled { generation, result } => {
                self.reduce_shell_recovery_discard_settled(generation, result)
            }
            Message::RecoveryDiscardEvent(event) => self.recovery_discard_choice(event),
            Message::PrintDocument => self.begin_print(),
            Message::PrintPrepared {
                generation,
                prepared,
            } => self.finish_print_prepare(generation, prepared),
            Message::PrintParented {
                generation,
                prepared,
                parent,
            } => self.finish_print_parent(generation, prepared, parent),
            Message::PrintTick => self.reduce_watch_print_tick(),
            Message::DismissPrintStatus => self.reduce_watch_dismiss_print_status(),
            Message::FileWatchTick => self.begin_watcher_batch(),

            Message::FileWatchBatchSettled {
                generation,
                retry,
                result,
            } => self.reduce_watch_file_watch_batch_settled(generation, retry, result),

            Message::FileWatchRefresh => self.reduce_watch_file_watch_refresh(),

            Message::CloseDrainTick => self.drive_close_drain(),

            Message::FileWatchDismiss => self.reduce_watch_file_watch_dismiss(),
        }
    }
}
