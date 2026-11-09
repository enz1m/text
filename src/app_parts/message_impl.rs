//! TextApp message impls.

use super::*;

impl Message {
    pub(crate) fn is_blocking_terminal(&self) -> bool {
        matches!(
            self,
            Self::BlockingWorkFailed(..)
                | Self::SearchCompleted { .. }
                | Self::StatsCompleted { .. }
                | Self::SelectionStatsCompleted { .. }
                | Self::SaveCompleted { .. }
                | Self::SessionRestoreCompleted { .. }
                | Self::SessionWriteSettled { .. }
                | Self::SessionDiscardSettled { .. }
                | Self::WorkspaceReadSettled { .. }
                | Self::DocumentLoaded { .. }
                | Self::PagedPageLoaded { .. }
                | Self::SettingsSaved { .. }
                | Self::RecoveryWritten { .. }
                | Self::RecoveryDiscardSettled { .. }
                | Self::PrintPrepared { .. }
                | Self::FileWatchBatchSettled { .. }
        )
    }

    pub(crate) fn is_busy_blocking_terminal(&self) -> bool {
        match self {
            Self::BlockingWorkFailed(failure) => failure.error() == BlockingTaskError::Busy,
            Self::SettingsSaved {
                result: Err(error), ..
            }
            | Self::RecoveryWritten {
                result: Err(error), ..
            }
            | Self::RecoveryDiscardSettled {
                result: Err(error), ..
            }
            | Self::SessionRestoreCompleted {
                result: Err(error), ..
            }
            | Self::SessionWriteSettled {
                result: Err(error), ..
            }
            | Self::SessionDiscardSettled {
                result: Err(error), ..
            }
            | Self::WorkspaceReadSettled {
                result: Err(error), ..
            }
            | Self::FileWatchBatchSettled {
                result: Err(error), ..
            } => *error == BlockingTaskError::Busy,
            _ => false,
        }
    }
}

impl enzui_core::crash_report::MessageKind for Message {
    fn message_kind(&self) -> &'static str {
        match self {
            Self::Desktop(_) => "Desktop",
            Self::NoOp => "NoOp",
            Self::TabStripScrolled(..) => "TabStripScrolled",
            Self::TabStripWheel(..) => "TabStripWheel",
            Self::TabStripEvent(..) => "TabStripEvent",
            Self::RevealActiveTab => "RevealActiveTab",
            #[cfg(feature = "center")]
            Self::Center(..) => "Center",
            Self::A11yWake(..) => "A11yWake",
            Self::VirtualInputMethod(..) => "VirtualInputMethod",
            Self::EditorAction(..) => "EditorAction",
            Self::VirtualCopy => "VirtualCopy",
            Self::VirtualCut => "VirtualCut",
            Self::EditorDragUpdated { .. } => "EditorDragUpdated",
            Self::SyncGutterScroll => "SyncGutterScroll",
            Self::BlockingWorkFailed(..) => "BlockingWorkFailed",
            Self::SearchCompleted { .. } => "SearchCompleted",
            Self::SaveCompleted { .. } => "SaveCompleted",
            Self::StatsCompleted { .. } => "StatsCompleted",
            Self::SelectionStatsCompleted { .. } => "SelectionStatsCompleted",
            Self::SelectionStatsSettleTick => "SelectionStatsSettleTick",
            Self::NewTab => "NewTab",
            Self::CloseTab(..) => "CloseTab",
            Self::CloseActiveTab => "CloseActiveTab",
            Self::SwitchTab(..) => "SwitchTab",
            Self::NextTab => "NextTab",
            Self::PrevTab => "PrevTab",
            Self::SaveFile => "SaveFile",
            Self::SaveFileAs => "SaveFileAs",
            Self::OpenFile => "OpenFile",
            Self::PickerTick => "PickerTick",
            Self::PickerParent(_) => "PickerParent",
            Self::PickerEvent(_) => "PickerEvent",
            Self::DismissPickerFailure => "DismissPickerFailure",
            Self::SessionRestoreCompleted { .. } => "SessionRestoreCompleted",
            Self::SessionWriteSettled { .. } => "SessionWriteSettled",
            Self::SessionDiscardSettled { .. } => "SessionDiscardSettled",
            Self::SessionRestoreRetry => "SessionRestoreRetry",
            Self::SessionRestoreRetire => "SessionRestoreRetire",
            Self::DismissSessionNotice => "DismissSessionNotice",
            Self::DismissAuthorityNotice => "DismissAuthorityNotice",
            Self::DismissLargeLoadNotice => "DismissLargeLoadNotice",
            Self::SessionRestoreOpenLocation => "SessionRestoreOpenLocation",
            Self::AddWorkspace => "AddWorkspace",
            Self::ToggleFind => "ToggleFind",
            Self::WorkspaceSelect(..) => "WorkspaceSelect",
            Self::WorkspaceToggle(..) => "WorkspaceToggle",
            Self::WorkspaceSidebarScrolled(..) => "WorkspaceSidebarScrolled",
            Self::WorkspaceOpenFile { .. } => "WorkspaceOpenFile",
            Self::WorkspaceContext(..) => "WorkspaceContext",
            Self::WorkspaceContextEvent(..) => "WorkspaceContextEvent",
            Self::WorkspaceRemoveEvent(..) => "WorkspaceRemoveEvent",
            Self::WorkspaceReadSettled { .. } => "WorkspaceReadSettled",
            Self::DocumentLoaded { .. } => "DocumentLoaded",
            Self::PagedPageLoaded { .. } => "PagedPageLoaded",
            Self::PagedNavigate(..) => "PagedNavigate",
            Self::SettingsSaved { .. } => "SettingsSaved",
            Self::ToggleCounts => "ToggleCounts",
            Self::ToggleLineNumbers => "ToggleLineNumbers",
            Self::ToggleWordWrap => "ToggleWordWrap",
            Self::ToggleTabIndentMode => "ToggleTabIndentMode",
            Self::SetTabIndentMode(..) => "SetTabIndentMode",
            Self::OpenFind => "OpenFind",
            Self::OpenReplace => "OpenReplace",
            Self::CloseFind => "CloseFind",
            Self::FindQueryChanged(..) => "FindQueryChanged",
            Self::ReplaceValueChanged(..) => "ReplaceValueChanged",
            Self::FindNext => "FindNext",
            Self::FindPrev => "FindPrev",
            Self::ReplaceCurrent => "ReplaceCurrent",
            Self::ReplaceAll => "ReplaceAll",
            Self::ToggleCaseSensitive => "ToggleCaseSensitive",
            Self::ToggleWholeWord => "ToggleWholeWord",
            Self::ToggleRegex => "ToggleRegex",
            Self::SetReplaceScope(..) => "SetReplaceScope",
            Self::ToggleReplaceBar => "ToggleReplaceBar",
            Self::OpenGoto => "OpenGoto",
            Self::CloseGoto => "CloseGoto",
            Self::GotoQueryChanged(..) => "GotoQueryChanged",
            Self::GotoSubmit => "GotoSubmit",
            Self::OpenStats => "OpenStats",
            Self::CloseStats => "CloseStats",
            Self::OpenSettings => "OpenSettings",
            Self::CloseSettings => "CloseSettings",
            Self::SettingsExitSettled(..) => "SettingsExitSettled",
            Self::OpenShortcuts => "OpenShortcuts",
            Self::CloseShortcuts => "CloseShortcuts",
            Self::SetFontSize(..) => "SetFontSize",
            Self::SetEditorTheme(..) => "SetEditorTheme",
            Self::ToggleMarkdownPreview { .. } => "ToggleMarkdownPreview",
            Self::MarkdownPreviewScrolled { .. } => "MarkdownPreviewScrolled",
            Self::WorkspaceAccentChosen { .. } => "WorkspaceAccentChosen",
            Self::ToggleRestoreSession => "ToggleRestoreSession",
            Self::ToggleLanguageMenu => "ToggleLanguageMenu",
            Self::SetLanguage(..) => "SetLanguage",
            Self::CycleLineEnding => "CycleLineEnding",
            Self::SetLineEnding(..) => "SetLineEnding",
            Self::CloseConfirmEvent(..) => "CloseConfirmEvent",
            Self::ToggleOverflowMenu => "ToggleOverflowMenu",
            Self::CloseOverflowMenu => "CloseOverflowMenu",
            Self::ToggleTabList => "ToggleTabList",
            Self::DismissPopovers => "DismissPopovers",
            Self::SidebarState(_) => "SidebarState",
            Self::DismissSidebar => "DismissSidebar",
            Self::Window(..) => "Window",
            Self::DialogEvent(..) => "DialogEvent",
            Self::SettingsDialogEvent(..) => "SettingsDialogEvent",
            Self::StackDialogEvent(..) => "StackDialogEvent",
            Self::FocusNext => "FocusNext",
            Self::FocusPrevious => "FocusPrevious",
            Self::CloseRequested(..) => "CloseRequested",
            Self::WindowCloseSaveAll => "WindowCloseSaveAll",
            Self::WindowCloseDiscard => "WindowCloseDiscard",
            Self::WindowCloseCancel => "WindowCloseCancel",
            Self::Resized(..) => "Resized",
            Self::Theme(..) => "Theme",
            Self::RecoveryTick => "RecoveryTick",
            Self::RecoveryWritten { .. } => "RecoveryWritten",
            Self::RecoveryDiscardSettled { .. } => "RecoveryDiscardSettled",
            Self::RecoveryDiscardEvent(..) => "RecoveryDiscardEvent",
            Self::PrintDocument => "PrintDocument",
            Self::PrintPrepared { .. } => "PrintPrepared",
            Self::PrintParented { .. } => "PrintParented",
            Self::PrintTick => "PrintTick",
            Self::DismissPrintStatus => "DismissPrintStatus",
            Self::FileWatchTick => "FileWatchTick",
            Self::FileWatchBatchSettled { .. } => "FileWatchBatchSettled",
            Self::CloseDrainTick => "CloseDrainTick",
            Self::BlockingRetryTick => "BlockingRetryTick",
            Self::FileWatchRefresh => "FileWatchRefresh",
            Self::FileWatchDismiss => "FileWatchDismiss",
        }
    }
}
