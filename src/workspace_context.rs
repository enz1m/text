//! Workspace tree-row context-menu helpers.
//!
//! Stable tree identities and row focus restoration.

use crate::workspace;
use enzui_widgets::context_menu_host::FocusRestoreTarget;
use std::path::Path;

pub(crate) const TREE_ID_PREFIX: &str = "text-app-workspace-tree";

pub(crate) fn tree_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new(TREE_ID_PREFIX)
}

pub(crate) const TREE_SCROLL_ID: &str = "text-app-workspace-tree-scroll";

pub(crate) fn tree_scroll_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new(TREE_SCROLL_ID)
}

pub(crate) fn restore_target(root: &Path) -> FocusRestoreTarget {
    let key = workspace::workspace_key(root);
    FocusRestoreTarget::tree_row_path(TREE_ID_PREFIX, &[key.as_str()])
}
