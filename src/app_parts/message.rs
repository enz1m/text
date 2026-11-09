//! TextApp message enum.

use super::*;

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Desktop(crate::desktop::DesktopEvent),
    NoOp,
    TabStripScrolled(iced::widget::scrollable::Viewport),
    TabStripWheel(iced::mouse::ScrollDelta),
    TabStripEvent(enzui_widgets::document_tabs::TabEvent),
    RevealActiveTab,
    #[cfg(feature = "center")]
    Center(enzui_agent::center::ConnectorEvent),
    // Wake from the AccessKit thread or a bound virtual-text mailbox, carrying
    // the opaque receipt for the exact batch it represents.
    A11yWake(enzui_core::app_runtime::AccessibilityWake),
    VirtualInputMethod(iced::advanced::input_method::Event),
    EditorAction(text_editor::Action),
    // Copy/cut for a viewport editor must use its document-wide selection;
    // cosmic can only see the materialized frame.
    VirtualCopy,
    VirtualCut,
    // Notification from the editor's opt-in same-frame drag path.
    // The widget has already performed the drag; this updates app metadata
    // for the owning tab and must never perform the action a second time.
    EditorDragUpdated {
        tab: String,
        cursor: text_editor::Cursor,
    },
    // Re-point the line-number gutter at the editor's laid-out scroll offset.
    //
    // The offset is only true after layout, so this message is queued after
    // every other message and re-queues itself until gutter and editor agree;
    // it is a no-op once they do.
    SyncGutterScroll,
    BlockingWorkFailed(BlockingWorkFailure),
    // A background search scan finished. Rejected unless it is still
    // the current generation, and the retained matches are bound to the
    // tab and snapshot the scan actually read from.
    SearchCompleted {
        generation: u64,
        from: Option<usize>,
        // `None` when the scan stopped because it had been cancelled.
        result: Option<Result<search::Scan, String>>,
        // The `(tab_id, snapshot_version)` the scan was launched against.
        binding: search::MatchBinding,
    },
    // The selection-statistics debounce tick. Cheap and gated: it
    // exists only while an open Statistics dialog is missing counts for the
    // live selection, and two ticks at the same selection generation are what
    // permit one background range scan.
    SelectionStatsSettleTick,
    // A background count of the selection finished. Applied only if it still
    // describes the current selection; a stale generation is dropped.
    SelectionStatsCompleted {
        tab: String,
        key: crate::model::SelectionStatsKey,
        stats: doc_stats::Stats,
    },
    // Background statistics for a large buffer finished. Applied only
    // if the buffer has not changed since they were started.
    StatsCompleted {
        tab: String,
        generation: u64,
        text: stats::TextStats,
        document: doc_stats::Stats,
    },
    // A streaming save finished off the reducer. `generation` is the
    // tab's buffer version at the moment the bytes were snapshotted: the tab
    // stops being dirty only if it has not been edited since.
    SaveCompleted {
        tab: String,
        generation: u64,
        destination: std::path::PathBuf,
        // Save As: the path to bind the tab to, but only if the write landed.
        rebind: Option<std::path::PathBuf>,
        guard_attempt: Option<SaveAttemptToken>,
        result: Result<Option<FileIdentity>, document_failure::DocumentFailure>,
    },
    NewTab,
    CloseTab(String),
    CloseActiveTab,
    SwitchTab(String),
    NextTab,
    PrevTab,
    SaveFile,
    // Open the portal Save-As dialog. On confirm the buffer is written to the
    // chosen path and the tab is rebound to it, so a later Ctrl+S writes there.
    SaveFileAs,
    // Open the host's file chooser. Emitted by Ctrl+O, the toolbar's
    // folder-open button and the narrow-width overflow menu.
    OpenFile,
    PickerTick,
    PickerParent(crate::picker::ParentEvent),
    PickerEvent(enzui_widgets::file_chooser::FallbackEvent),
    DismissPickerFailure,
    SessionRestoreCompleted {
        generation: u64,
        result: Result<session::SessionRestore, BlockingTaskError>,
    },
    SessionWriteSettled {
        generation: u64,
        handoff: SessionWriteHandoff,
        result: Result<(), BlockingTaskError>,
    },
    SessionDiscardSettled {
        generation: u64,
        window: iced::window::Id,
        handoff: SessionWriteHandoff,
        result: Result<(), BlockingTaskError>,
    },
    SessionRestoreRetry,
    SessionRestoreRetire,
    SessionRestoreOpenLocation,
    DismissSessionNotice,
    // Hide the active tab's "authority exhausted" / large-document load
    // error toast until that state changes.
    DismissAuthorityNotice,
    DismissLargeLoadNotice,
    AddWorkspace,
    // The Search command (the titlebar icon and its overflow row) toggles
    // the find bar. `OpenFind`/`CloseFind` serve the keyboard, the Esc
    // route and the bar's own close ×, which are one-directional by design.
    ToggleFind,
    WorkspaceSelect(PathBuf),
    WorkspaceToggle(PathBuf),
    // The workspace tree scrolled to a new absolute vertical offset.
    //
    // The sanitised offset is stored in
    // [`crate::settings::Settings::workspace_sidebar_scroll_offset`] and seeds
    // the next mounted tree, so a reopened sidebar keeps its position.
    WorkspaceSidebarScrolled(f32),
    WorkspaceOpenFile {
        workspace: PathBuf,
        path: PathBuf,
    },
    WorkspaceContext(PathBuf, iced::Point),
    WorkspaceContextEvent(ContextMenuEvent),
    WorkspaceRemoveEvent(ConfirmDialogEvent),
    WorkspaceReadSettled {
        path: PathBuf,
        generation: u64,
        result: Result<workspace::DirectoryLoad, BlockingTaskError>,
    },
    // A background file read for a sidebar-click open finished. Applied
    // through the same `open_path` loader as a CLI open, so line-ending
    // handling cannot diverge.
    DocumentLoaded {
        // The tab the load was started for. Every open route creates or reuses
        // its tab before loading, so a closed tab simply drops the result.
        tab_id: String,
        workspace: Option<PathBuf>,
        path: PathBuf,
        // The app-level generation this load was issued under. The
        // reducer drops the result unless the tab is still waiting on exactly
        // this token.
        token: large_doc::LoadToken,
        reason: LoadReason,
        result: Result<large_doc::LoadedDocument, document_failure::DocumentFailure>,
    },
    // One bounded page of a large read-only document arrived. The framework
    // rejects a stale identity/generation; `token` rejects a page whose tab
    // has moved on.
    PagedPageLoaded {
        tab_id: String,
        token: large_doc::LoadToken,
        result: Box<enzui_core::paged_text::PagedTextResult>,
    },
    // Read-only viewport movement from the shared paged viewer
    // (arrows, Page Up/Down, Home/End, wheel).
    PagedNavigate(enzui_core::paged_text::PagedTextNavigation),
    SettingsSaved {
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    },
    ToggleCounts,
    ToggleLineNumbers,
    ToggleWordWrap,
    // Flip the editor's Tab mode between "Tab moves focus"
    // (navigation, the default that preserves the pure-Tab focus ring) and
    // "Tab indents" (a caret Tab performs a real `Edit::Indent`). Bound to
    // Ctrl+M — the established accessible editor convention — and decoded in
    // `classify_event` like the other navigation chords.
    ToggleTabIndentMode,
    SetTabIndentMode(bool),
    OpenFind,
    OpenReplace,
    CloseFind,
    FindQueryChanged(String),
    ReplaceValueChanged(String),
    FindNext,
    FindPrev,
    ReplaceCurrent,
    ReplaceAll,
    ToggleCaseSensitive,
    ToggleWholeWord,
    ToggleRegex,
    SetReplaceScope(usize),
    ToggleReplaceBar,
    OpenGoto,
    CloseGoto,
    GotoQueryChanged(String),
    GotoSubmit,
    OpenStats,
    CloseStats,
    OpenSettings,
    CloseSettings,
    SettingsExitSettled(enzui_widgets::modal_host::ModalPresentationKey),
    OpenShortcuts,
    CloseShortcuts,
    SetFontSize(FontSizeScale),
    SetEditorTheme(crate::editor_theme::EditorTheme),
    ToggleMarkdownPreview {
        tab: String,
        version: u64,
    },
    MarkdownPreviewScrolled {
        tab: String,
        version: u64,
        offset: f32,
    },
    WorkspaceAccentChosen {
        root: PathBuf,
        epoch: u64,
        accent: Option<WorkspaceAccentKey>,
    },
    ToggleRestoreSession,
    ToggleLanguageMenu,
    SetLanguage(String),
    CycleLineEnding,
    SetLineEnding(line_ending::LineEnding),
    // Any exit reported by the framework `enz_confirm_dialog` presenting the
    // unsaved-changes alert: confirm button, cancel button, Escape or scrim.
    // `ConfirmDecision::from_cause` maps the cause to yes/no in `update`, so
    // there is one funnel for every way the dialog can end.
    CloseConfirmEvent(ConfirmDialogEvent),
    ToggleOverflowMenu,
    CloseOverflowMenu,
    ToggleTabList,
    // The crash-recovery debounce tick. While unpublished revision debt
    // exists it captures cheap per-buffer leases and dispatches one off-thread
    // atomic write; a clean editor never schedules this tick.
    RecoveryTick,
    // An off-thread recovery write completed. `generation` makes a stale
    // completion inert (a newer write superseded it); on success the committed
    // revisions become the on-disk truth and any remaining debt re-publishes.
    RecoveryWritten {
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    },
    // The off-thread Discard tombstone at `generation` completed. On
    // durable success recovery authority is retired and the deferred window
    // close fires (exactly once); on failure the window stays open and the
    // Retry / Close-without-clearing / Cancel prompt is shown.
    RecoveryDiscardSettled {
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    },
    RecoveryDiscardEvent(SaveChangesEvent),
    PrintDocument,
    // The off-thread PostScript render for `generation` finished — a
    // sealed document handoff or a typed render error. A generation mismatch
    // means a newer request superseded this one, so the handoff is dropped.
    PrintPrepared {
        generation: u64,
        prepared: Result<print::PreparedHandoff, print::PrintRenderError>,
    },
    PrintParented {
        generation: u64,
        prepared: print::PreparedHandoff,
        parent: Option<crate::portal_parent::ParentHandoff>,
    },
    // A bounded wake while a print is live; poll the spool once.
    PrintTick,
    DismissPrintStatus,
    FileWatchTick,
    FileWatchBatchSettled {
        generation: u64,
        retry: RetryPayload<Vec<(String, enzui_core::file_watch::ProbeOp)>>,
        result: Result<Vec<(String, Probe)>, BlockingTaskError>,
    },
    CloseDrainTick,
    BlockingRetryTick,
    // The user chose to reload the active tab from disk after an external
    // change was detected.
    FileWatchRefresh,
    // The user chose to keep the current buffer and dismiss the
    // external-change affordance (acknowledging the new on-disk state so
    // the prompt does not recur).
    FileWatchDismiss,
    // Dismiss any transient anchored popover (language menu, overflow menu).
    //
    // Unlike the toggles, this only closes and is idempotent.
    DismissPopovers,
    SidebarState(NavSplitState),
    DismissSidebar,
    Window(WindowCommand),
    // A modal-host event from the stats / settings / close dialog (Escape /
    // scrim / Tab).
    //
    // Carries the real event so the reducer can split dismissal from focus
    // traversal; a Tab press meant to move focus inside the dialog must not
    // close it.
    DialogEvent(enzui_widgets::modal_host::ModalHostEvent),
    SettingsDialogEvent(enzui_widgets::modal_host::ModalHostEvent),
    StackDialogEvent(enzui_widgets::modal_host::DialogEvent<crate::dialog_focus::DialogFocusKind>),
    FocusNext,
    FocusPrevious,
    // The compositor asked to close the window. The app disables iced's
    // automatic close so this message is the only path that can discard all
    // open buffers.
    CloseRequested(CloseRequest),
    WindowCloseSaveAll,
    WindowCloseDiscard,
    WindowCloseCancel,
    Resized(f32, f32),
    Theme(Box<ThemeEvent>),
}
