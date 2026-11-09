//! Bar styles and the toolbar action enum.

use super::*;

pub(crate) fn bar_style(pal: Palette) -> impl Fn(&Theme) -> container::Style {
    move |_theme: &Theme| container::Style {
        background: Some(iced::Background::Color(pal.surface_alt)),
        text_color: Some(pal.text),
        border: iced::Border {
            color: pal.border,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

// When narrow (`Model::is_narrow`) the group collapses to one overflow menu
// that iterates `ToolbarAction::ALL`, so every variant stays reachable without
// a keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ToolbarAction {
    Open,
    Find,
    Replace,
    GotoLine,
    Statistics,
    Settings,
    Save,
    SaveAs,
    Print,
}
