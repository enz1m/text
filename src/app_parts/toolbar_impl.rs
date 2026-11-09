//! TextApp toolbar action impl.

use super::*;

impl ToolbarAction {
    pub(crate) const ALL: [ToolbarAction; 9] = [
        ToolbarAction::Open,
        ToolbarAction::Save,
        ToolbarAction::SaveAs,
        ToolbarAction::Print,
        ToolbarAction::Find,
        ToolbarAction::Replace,
        ToolbarAction::GotoLine,
        ToolbarAction::Statistics,
        ToolbarAction::Settings,
    ];

    pub(crate) fn overflow_label(self) -> String {
        match self {
            ToolbarAction::Open => enzui_core::localization::text("text-open-file-1e8d18"),
            ToolbarAction::Find => enzui_core::localization::text("text-find-822b2a"),
            ToolbarAction::Replace => enzui_core::localization::text("text-replace-95e154"),
            ToolbarAction::GotoLine => enzui_core::localization::text("text-go-to-line-77922d"),
            ToolbarAction::Statistics => enzui_core::localization::text("text-statistics-a65610"),
            ToolbarAction::Settings => enzui_core::localization::text("text-settings-74a883"),
            ToolbarAction::Save => enzui_core::localization::text("text-save-1509f5"),
            ToolbarAction::SaveAs => enzui_core::localization::text("text-save-as-088203"),
            ToolbarAction::Print => enzui_core::localization::text("text-print-07c371"),
        }
    }

    pub(crate) fn keybinding(self) -> keybindings::EditorAction {
        use keybindings::EditorAction as A;
        match self {
            ToolbarAction::Open => A::Open,
            ToolbarAction::Find => A::Find,
            ToolbarAction::Replace => A::Replace,
            ToolbarAction::GotoLine => A::GotoLine,
            ToolbarAction::Statistics => A::Statistics,
            ToolbarAction::Settings => A::Settings,
            ToolbarAction::Save => A::Save,
            ToolbarAction::SaveAs => A::SaveAs,
            ToolbarAction::Print => A::Print,
        }
    }

    pub(crate) fn inline_label(self) -> String {
        keybindings::label_with_chord(self.keybinding())
    }

    pub(crate) fn descriptor_id(self) -> &'static str {
        match self {
            ToolbarAction::Open => "file.open",
            ToolbarAction::Find => "edit.find",
            ToolbarAction::Replace => "edit.replace",
            ToolbarAction::GotoLine => "edit.goto_line",
            ToolbarAction::Statistics => "view.statistics",
            ToolbarAction::Settings => "app.settings",
            ToolbarAction::Save => "file.save",
            ToolbarAction::SaveAs => "file.save_as",
            ToolbarAction::Print => "file.print",
        }
    }

    pub(crate) fn group(self) -> String {
        match self {
            ToolbarAction::Open
            | ToolbarAction::Save
            | ToolbarAction::SaveAs
            | ToolbarAction::Print => enzui_core::localization::text("text-file-50009c"),
            ToolbarAction::Find | ToolbarAction::Replace | ToolbarAction::GotoLine => {
                enzui_core::localization::text("text-edit-464c4f")
            }
            ToolbarAction::Statistics => enzui_core::localization::text("text-view-dcc839"),
            ToolbarAction::Settings => enzui_core::localization::text("text-app-0d04bf"),
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            ToolbarAction::Open => "folder-open",
            ToolbarAction::Find => "search",
            ToolbarAction::Replace => "replace",
            ToolbarAction::GotoLine => "list-ordered",
            ToolbarAction::Statistics => "chart-no-axes-column",
            ToolbarAction::Settings => "settings",
            ToolbarAction::Save => "save",
            ToolbarAction::SaveAs => "save-all",
            ToolbarAction::Print => "printer",
        }
    }

    pub(crate) fn message(self) -> Message {
        match self {
            ToolbarAction::Open => Message::OpenFile,
            // The toolbar Find button toggles the bar; `OpenFind` stays the
            // keyboard/menu-open route.
            ToolbarAction::Find => Message::ToggleFind,
            ToolbarAction::Replace => Message::OpenReplace,
            ToolbarAction::GotoLine => Message::OpenGoto,
            ToolbarAction::Statistics => Message::OpenStats,
            ToolbarAction::Settings => Message::OpenSettings,
            ToolbarAction::Save => Message::SaveFile,
            ToolbarAction::SaveAs => Message::SaveFileAs,
            ToolbarAction::Print => Message::PrintDocument,
        }
    }
}
