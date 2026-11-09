//! Shell tokens and persisted sidebar navigation state.

use crate::settings;
use enzui_widgets::app_shell::{PagePresentation, PaneStyle, ShellTokens, SidebarHeader};
use enzui_widgets::nav_split::{NavSplitState, SidebarPreference};
use enzui_widgets::surface::SurfaceTone;

// A shown preference restores as automatic so widening a window that booted
// narrow re-reveals a sidebar the user did not explicitly hide.
pub(crate) fn nav_from_settings(settings: &settings::Settings) -> NavSplitState {
    let mut nav = NavSplitState::new();
    nav.preference = if settings.sidebar_visible {
        SidebarPreference::Automatic
    } else {
        SidebarPreference::Hidden
    };
    nav
}

pub(crate) fn shell_tokens() -> ShellTokens {
    ShellTokens {
        sidebar_width: crate::SIDEBAR_WIDTH_PX,
        sidebar_width_adaptive: false,
        content_minimum: crate::EDITOR_PANE_MIN_WIDTH,
        shell_tone: SurfaceTone::Background,
        sidebar_header: SidebarHeader::Inside,
        page_presentation: PagePresentation::Flush,
        content: PaneStyle::blended(),
        ..ShellTokens::default()
    }
}
