//! TextApp application struct.

use super::*;

pub(crate) struct TextApp {
    pub(crate) tab_strip: crate::tab_strip::TabStripState,
    pub(crate) desktop: crate::desktop::Desktop,
    pub(crate) startup_editor_focus: Option<large_doc::LoadToken>,
    pub(crate) model: Model,
    pub(crate) dialog_focus: Option<(
        crate::dialog_focus::DialogFocusKind,
        enzui_widgets::modal_host::ModalFocusLifecycle,
    )>,
    pub(crate) settings_focus: Option<enzui_widgets::modal_host::ModalFocusLifecycle>,
    pub(crate) dialogs:
        enzui_widgets::modal_host::DialogStack<crate::dialog_focus::DialogFocusKind>,
    pub(crate) settings_presentation: settings_surface::SettingsPresentation,
    pub(crate) themes: ThemeIntegration,
    pub(crate) nav: NavSplitState,
    pub(crate) close_confirm_state: ConfirmState,
    pub(crate) workspace_remove_confirm_state: ConfirmState,
    pub(crate) pending_workspace_remove: Option<PathBuf>,
    pub(crate) workspace_context_menu: WorkspaceContextMenu,
    pub(crate) workspace_context_epoch: u64,
    pub(crate) close_guard: CloseGuard<String>,
    pub(crate) close_save_attempt: Option<SaveAttemptToken>,
    pub(crate) picker: picker::Picker,
    pub(crate) print: print::PrintState,
    pub(crate) document_failure_diagnostic: Option<String>,
    pub(crate) workspaces: workspace::WorkspaceState,
    pub(crate) session_store: session::SessionStore,
    pub(crate) session_restore_gen: u64,
    pub(crate) session_restore_pending: Option<SessionRestorePending>,
    pub(crate) session_restore_request: Option<session::SessionRequest>,
    pub(crate) session_debt: Option<session::SessionDebt>,
    pub(crate) session_notice_dismissed: Option<session::SessionDebtKind>,
    // Tabs whose "authority exhausted" error toast the user dismissed; the
    // state itself persists for the tab, so the toast stays hidden for it.
    pub(crate) authority_notice_dismissed: std::collections::BTreeSet<String>,
    // The (tab id, error) of a dismissed large-document load error; a new
    // error, or another tab's, shows again.
    pub(crate) large_load_notice_dismissed: Option<(String, String)>,
    pub(crate) session_write_generation: u64,
    pub(crate) session_write_inflight: bool,
    pub(crate) session_write_baseline: session::SessionSnapshot,
    // Only snapshots already admitted by `preflight_bounds` are stashed
    // here; an over-bound refusal enters `session_over_bound_debt` instead.
    pub(crate) session_write_debt: Option<session::SessionSnapshot>,
    pub(crate) session_over_bound_debt: Option<OverBoundReducerDebt>,
    // Advanced from reducer arms that mutate the persisted projection
    // (tab-path membership, active path, workspace root/name/accent,
    // tab-workspace association). An unrelated tick reaches no advance site
    // and allocates no snapshot.
    pub(crate) session_projection_revision: u64,
    // Terminal flag independent of `session_debt`: `queue_session_sync`
    // stops constructing snapshots and dispatching once this is set, even
    // when `SessionRequest` is `None` and no banner debt exists.
    pub(crate) session_projection_revision_exhausted: bool,
    pub(crate) session_retry_blocked: bool,
    pub(crate) session_discard_window: Option<iced::window::Id>,
    pub(crate) blocking_tasks: BlockingTaskOwner,
    pub(crate) deferred_tasks: Vec<Task<Message>>,
    #[cfg(feature = "center")]
    pub(crate) center: Option<enzui_agent::center::CenterHost<crate::center::CenterCall>>,
    pub(crate) settings_persistence_enabled: bool,
    // Unresolved or inert test roots must never authorize persistence.
    pub(crate) settings_config_path: std::path::PathBuf,
    pub(crate) settings_generation: u64,
    pub(crate) settings_inflight: Option<(u64, settings::Settings)>,
    pub(crate) settings_debt: Option<(u64, settings::Settings)>,
    pub(crate) settings_retry_blocked: bool,
    pub(crate) close_request_window: Option<iced::window::Id>,
    pub(crate) close_drain: Option<CloseDrain>,
    // Compatibility probes keep post-layout observations; numbering reads
    // editor metrics directly.
    pub(crate) gutter_scroll_offset: f32,
    pub(crate) tab_indent_mode: bool,
    pub(crate) recovery_store: recovery::RecoveryStore,
    pub(crate) recovery_published: std::collections::HashMap<String, u64>,
    pub(crate) recovery_inflight: Option<(u64, std::collections::HashMap<String, u64>)>,
    pub(crate) recovery_retry: Option<RecoveryPublishAttempt>,
    pub(crate) recovery_retry_blocked: bool,
    pub(crate) recovery_gen: u64,
    pub(crate) recovery_discard: Option<RecoveryDiscard>,
    pub(crate) recovery_discard_submitted: bool,
    pub(crate) recovery_discard_retry_blocked: bool,
    pub(crate) print_retry: Option<PrintAttempt>,
    pub(crate) print_preparing: bool,
    pub(crate) print_retry_blocked: bool,
    pub(crate) blocking_retry_pending: bool,
    pub(crate) blocking_retry_suspended: bool,
    // A request arriving mid-scan cancels it and is coalesced into
    // `search_queued`: a second scan would hold a second snapshot handle and
    // make the next keystroke copy the whole document in `Arc::make_mut`.
    pub(crate) search_scan: Option<std::sync::Arc<worker::Cancel>>,
    pub(crate) search_attempt: Option<PendingSearch>,
    pub(crate) search_retry: Option<PendingSearch>,
    pub(crate) search_retry_blocked: bool,
    pub(crate) search_queued: Option<Option<usize>>,
    pub(crate) stats_attempts: std::collections::HashMap<String, PendingStats>,
    pub(crate) stats_retry: std::collections::HashMap<String, PendingStats>,
    pub(crate) stats_retry_blocked: std::collections::HashSet<String>,
    pub(crate) selection_stats_attempts: std::collections::HashMap<String, PendingSelectionStats>,
    pub(crate) selection_stats_retry: std::collections::HashMap<String, PendingSelectionStats>,
    pub(crate) selection_stats_retry_blocked: std::collections::HashSet<String>,
    pub(crate) pump_cursor: usize,
    pub(crate) load_debt: std::collections::HashMap<String, PendingLoad>,
    pub(crate) load_retry_blocked: std::collections::HashSet<String>,
    pub(crate) paged_debt: std::collections::HashMap<
        String,
        (
            large_doc::LoadToken,
            enzui_core::paged_text::PagedTextOperation,
        ),
    >,
    pub(crate) paged_retry_blocked: std::collections::HashSet<String>,
    pub(crate) save_destinations: std::collections::HashMap<PathBuf, String>,
    pub(crate) save_inflight: std::collections::HashMap<String, SaveRequest>,
    pub(crate) save_debt: std::collections::HashMap<String, SaveRequest>,
    pub(crate) save_retry_blocked: std::collections::HashSet<String>,
    pub(crate) workspace_generation: u64,
    pub(crate) workspace_debt: std::collections::HashMap<PathBuf, PendingWorkspaceRead>,
    pub(crate) workspace_inflight: std::collections::HashMap<PathBuf, u64>,
    pub(crate) workspace_retry_blocked: std::collections::HashSet<PathBuf>,
    pub(crate) watcher_generation: u64,
    pub(crate) watcher_inflight: bool,
    pub(crate) watcher_debt: bool,
    pub(crate) watcher_retry: Option<RetryPayload<Vec<(String, enzui_core::file_watch::ProbeOp)>>>,
    pub(crate) watcher_retry_paths: Vec<(String, PathBuf)>,
    pub(crate) watcher_retry_blocked: bool,
    pub(crate) window_focused: bool,
    // A tick seeing a different generation records it and waits; seeing the
    // same one means the selection settled and permits one range scan.
    // Cleared on dispatch, on dialog open, and when nothing is wanted.
    pub(crate) selection_settle_probe: Option<(String, crate::model::SelectionStatsKey)>,

    pub(crate) gutter_window_first_line: std::cell::Cell<usize>,

    pub(crate) gutter_window_placement: std::cell::Cell<Option<f32>>,

    pub(crate) gutter_window_size: std::cell::Cell<Option<(f32, f32)>>,
    pub(crate) file_watch_backend: std::sync::Arc<dyn FileWatchBackend>,
    pub(crate) bound_virtual_text:
        Option<std::sync::Arc<enzui_core::virtual_text::VirtualTextDocument>>,
    pub(crate) bound_virtual_text_wake: Option<enzui_core::virtual_text::MailboxWakeRegistration>,
    pub(crate) virtual_text_surface: iced::advanced::widget::Id,
}
