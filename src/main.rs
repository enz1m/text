//! Edit plain text and source files.

#![deny(unconditional_recursion)]
#![cfg_attr(
    test,
    allow(
        clippy::panic,
        clippy::unwrap_in_result,
        clippy::string_slice,
        clippy::missing_assert_message,
        reason = "Test assertions report values and fixture failures by panicking"
    )
)]

mod a11y;
mod app_instance;
#[path = "app_parts/constants.rs"]
mod constants;
mod desktop;
mod desktop_search;
mod i18n;
mod portal_parent;
#[path = "app_parts/subscription.rs"]
mod subscription;

pub(crate) use constants::{
    BAR_HEIGHT, EDITOR_INTERNAL_PAD, EDITOR_LINE_HEIGHT, EDITOR_PANE_MIN_WIDTH,
    FILE_WATCH_PROBE_INTERVAL, FONT_SCALES, LOCATION_POLL_INTERVAL, PRINT_POLL_INTERVAL,
    PRINT_STATUS_DWELL, RECOVERY_DEBOUNCE, SELECTION_STATS_DEBOUNCE, SIDEBAR_WIDTH_PX,
};
#[path = "app_parts/work_done.rs"]
mod work_done;

pub(crate) use work_done::WorkDone;
#[path = "app_parts/gutter_window.rs"]
mod gutter_window;

pub(crate) use gutter_window::visible_line_window;
#[path = "app_parts/admission.rs"]
mod admission;
#[path = "app_parts/bar_style.rs"]
mod bar_style;

pub(crate) use admission::{Admission, SaveDispatch};
#[path = "app_parts/editor_input.rs"]
mod editor_input;

pub(crate) use editor_input::{
    editor_indent_binding, editor_redo_binding, has_selection_anchor, normalize_pasted_line_endings,
};
#[path = "app_parts/toolbar_impl.rs"]
mod toolbar_impl;

pub(crate) use bar_style::{ToolbarAction, bar_style};
#[path = "app_parts/widget_ids.rs"]
mod widget_ids;

pub(crate) use widget_ids::{
    editor_id, find_close_id, find_field_id, goto_close_id, goto_field_id, language_pill_id,
    line_ending_pill_id, replace_field_id, restore_focus_task, settings_invoker_id,
    stats_button_id, tab_list_button_id,
};
#[path = "app_parts/load_reason.rs"]
mod load_reason;

pub(crate) use load_reason::LoadReason;
#[path = "app_parts/handoff_types.rs"]
mod handoff_types;

pub(crate) use handoff_types::{OverBoundReducerDebt, SessionWriteHandoff};
#[path = "app_parts/attempt_types.rs"]
mod attempt_types;
#[path = "app_parts/message.rs"]
mod message;

pub(crate) use attempt_types::{
    PendingSearch, PendingSelectionStats, PendingStats, PrintAttempt, RecoveryPublishAttempt,
    SaveRequest,
};
#[path = "app_parts/pending_types.rs"]
mod pending_types;

pub(crate) use pending_types::{CloseDrain, PendingLoad, PendingWorkspaceRead, RetryPayload};
#[path = "app_parts/work_failure.rs"]
mod work_failure;

pub(crate) use work_failure::BlockingWorkFailure;
#[path = "app_parts/panel_types.rs"]
mod panel_types;

pub(crate) use panel_types::{
    BLOCKING_RETRY_INTERVAL, CLOSE_DRAIN_DEADLINE, CLOSE_DRAIN_POLL, SaveAsFinish,
    WorkspaceCommand, WorkspaceContextMenu, app_blocking_owner,
};
#[path = "app_parts/message_impl.rs"]
mod message_impl;

pub(crate) use message::Message;
#[path = "app_parts/app_struct.rs"]
mod app_struct;

pub(crate) use app_struct::TextApp;
#[path = "app_parts/boot_env.rs"]
mod boot_env;

#[cfg(test)]
pub(crate) use boot_env::TEST_INERT_SETTINGS_PATH;
pub(crate) use boot_env::{BootEnv, capture_settle_requested};
#[path = "app_parts/boot_disclosure.rs"]
mod boot_disclosure;

pub(crate) use boot_disclosure::recovery_boot_disclosure;
#[path = "app_parts/restore_buffers.rs"]
mod restore_buffers;

pub(crate) use restore_buffers::restore_recovered_buffers;
#[path = "app_parts/recovery_types.rs"]
mod recovery_types;

pub(crate) use recovery_types::{
    RecoveryBootReport, RecoveryDiscard, RecoveryLease, SessionRestorePending, existing_tab_for,
    recovered_tab_id, recovery_lease_for, restore_caret_selection,
};
#[path = "app_parts/action_helpers.rs"]
mod action_helpers;
mod atomic_save;
#[path = "app_parts/boot.rs"]
mod boot;
#[path = "app_parts/buffer_helpers.rs"]
mod buffer_helpers;
#[path = "app_parts/close_session.rs"]
mod close_session;
mod dialog_focus;
mod doc_index;
mod doc_stats;
mod document_failure;

mod editor_gutter;

pub(crate) use action_helpers::{
    guarded_window_close_message, message_for_action, restore_startup_tabs,
};
#[path = "app_parts/event_classify.rs"]
mod event_classify;
#[path = "app_parts/open.rs"]
mod open;
#[path = "app_parts/reduce_chrome.rs"]
mod reduce_chrome;
#[path = "app_parts/reduce_clipboard.rs"]
mod reduce_clipboard;
#[path = "app_parts/reduce_context.rs"]
mod reduce_context;
#[path = "app_parts/reduce_dialogs.rs"]
mod reduce_dialogs;
#[path = "app_parts/reduce_document.rs"]
mod reduce_document;
#[path = "app_parts/reduce_edit.rs"]
mod reduce_edit;
#[path = "app_parts/reduce_file.rs"]
mod reduce_file;
#[path = "app_parts/reduce_find.rs"]
mod reduce_find;
#[path = "app_parts/reduce_focus.rs"]
mod reduce_focus;
#[path = "app_parts/reduce_save_done.rs"]
mod reduce_save_done;
#[path = "app_parts/reduce_session.rs"]
mod reduce_session;
#[path = "app_parts/reduce_shell.rs"]
mod reduce_shell;
#[path = "app_parts/reduce_tabs.rs"]
mod reduce_tabs;
#[path = "app_parts/reduce_toggles.rs"]
mod reduce_toggles;
#[path = "app_parts/reduce_watch.rs"]
mod reduce_watch;
#[path = "app_parts/reduce_workers.rs"]
mod reduce_workers;
#[path = "app_parts/save.rs"]
mod save;
#[path = "app_parts/search_replace.rs"]
mod search_replace;
#[path = "app_parts/settings_persist.rs"]
mod settings_persist;
#[path = "app_parts/update_root.rs"]
mod update_root;

pub(crate) use event_classify::{
    EventClassifier, classify_event, classify_event_close_find, classify_event_close_goto,
    classify_event_dismiss_popovers, classify_event_modal_owns_keyboard,
};
#[path = "app_parts/view_chrome.rs"]
mod view_chrome;
#[path = "app_parts/view_content.rs"]
mod view_content;
#[path = "app_parts/view_dialogs.rs"]
mod view_dialogs;
#[path = "app_parts/view_shell.rs"]
mod view_shell;
#[path = "app_parts/work_dispatch.rs"]
mod work_dispatch;
#[path = "app_parts/work_pumps.rs"]
mod work_pumps;

#[cfg(test)]
#[path = "blocking_owner_tests.rs"]
mod blocking_owner_tests;
#[cfg(test)]
mod blocking_save_route_tests;
#[cfg(feature = "center")]
mod center;

mod keybindings;
mod large_doc;
mod line_ending;

#[path = "match_ink.rs"]
mod match_ink;

mod model;
#[cfg(test)]
mod perf_counters;
mod picker;
mod print;

mod editor_theme;
mod markdown_preview;
mod recovery;
mod safe_text;
mod search;
mod session;
mod settings;

mod settings_surface;
mod shell;
mod stats;
mod tab_strip;
#[cfg(test)]
mod task_driver;
#[cfg(test)]
mod test_fixture;
#[cfg(test)]
mod test_support;
mod worker;
mod workspace;
mod workspace_context;

#[cfg(test)]
#[path = "large_edit_contract_tests.rs"]
mod large_edit_contract_tests;

use std::num::NonZeroUsize;
use std::path::PathBuf;

use iced::keyboard;
use iced::widget::{container, row, text_editor};
use iced::{Element, Length, Padding, Subscription, Task, Theme, advanced::widget::operate};

use enzui_core::blocking_task::{BlockingTaskError, BlockingTaskOwner};
use enzui_core::live_region::Announcement;
use enzui_core::placement::{Alignment as PlacementAlignment, Placement, Side};
use enzui_core::{Action as CoreAction, Icon};
use enzui_widgets::anchored_overlay::anchored_overlay;

use enzui_core::action_overflow::Descriptor;
use enzui_core::close_guard::{
    CloseDecision, CloseGuard, CloseGuardAction, CloseRequest, SaveAttemptToken, SaveOutcome,
    UnsavedItem,
};
use enzui_core::confirm::ConfirmState;
use enzui_core::context_menu::{InvocationKind, Point as ContextPoint};
use enzui_core::file_watch::{FileIdentity, FileWatchBackend, FsBackend, Probe};
use enzui_core::focus::{Traversal, traversal_from_event, traversal_operation as traverse};
use enzui_core::semantics::HeadingLevel;
use enzui_core::theme_file::integration::{ThemeEvent, ThemeIntegration};
#[cfg(test)]
use enzui_theme::EnzTheme;
use enzui_theme::{
    Palette, SpacingScale, TypographyScale, WorkspaceAccentKey, WorkspaceAccentPalette,
};
use enzui_widgets::action_bar::ActionItem as TitlebarActionItem;
use enzui_widgets::app_shell::{ShellLayout, enz_app_shell, enz_window_settings};
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_widgets::confirm_dialog::{ConfirmDialogEvent, enz_confirm_dialog};
use enzui_widgets::context_menu_host::{ContextMenuEvent, ContextMenuItem};
use enzui_widgets::editor_scrollbar::enz_editor_scrollbar;
use enzui_widgets::enz_text::{TextSize, enz_text};
use enzui_widgets::find_replace_bar::FindReplaceEvent;
use enzui_widgets::find_replace_bar::enz_find_replace_bar;
use enzui_widgets::form_layout::enz_form_layout;
use enzui_widgets::icon_label::enz_icon_label;
use enzui_widgets::labeled_content::enz_labeled_content;
use enzui_widgets::list_row_surface::enz_list_row_surface;
use enzui_widgets::menu::{EnzMenuItem, enz_menu};
use enzui_widgets::modal_host::enz_modal_host;
use enzui_widgets::modal_surface::enz_modal_surface;
use enzui_widgets::motion::{MotionAxis, MotionEffect, motion};
use enzui_widgets::nav_split::{NavSplitState, SidebarPreference, SidebarPresentation};
use enzui_widgets::save_changes_dialog::{
    SaveChangesAction, SaveChangesEvent, enz_save_changes_dialog,
};
use enzui_widgets::scroll_view::enz_scroll_view;
use enzui_widgets::shortcuts_dialog::enz_shortcuts_dialog;
use enzui_widgets::sidebar_content::enz_sidebar_content;
use enzui_widgets::status_bar::{StatusBarItem, enz_status_bar_strip};
use enzui_widgets::status_page::{StatusPageTone, StatusPageVariant, enz_status_page};
use enzui_widgets::text_field::enz_text_field;
use enzui_widgets::title_bar::enz_title_bar;
use enzui_widgets::title_bar_gesture::WindowCommand;
use enzui_widgets::toolbar_icon::enz_toolbar_icon;
use enzui_widgets::tree_view::{EnzTreeItem, enz_tree_view};

use model::{Model, Tab};
use settings::FontSizeScale;
use shell::nav_from_settings;

fn main() -> iced::Result {
    enzui_core::logging::init_default();
    enzui_core::crash_report::CrashReporter::for_app("text-app", env!("CARGO_PKG_VERSION"));
    let (instance, launch) = match app_instance::AppInstance::start(DESKTOP_APP_ID) {
        app_instance::InstanceStart::Owner(instance, launch) => (instance, launch),
        app_instance::InstanceStart::Forwarded => return Ok(()),
    };
    let instance = std::sync::Mutex::new(Some(instance));
    let rejected_uri = matches!(&launch, enzui_core::activation::ApplicationIntent::Open(uris) if desktop::local_paths(&launch).len() != uris.len());
    let localizer = i18n::for_locales(enzui_core::localization::preferred_locales());
    enzui_widgets::app_setup::run_with_services(
        iced::application(
            move || {
                let (mut app, task) = TextApp::boot(&launch);
                app.desktop.activity = enzui_activity::Publisher::new(DESKTOP_APP_ID).ok();
                app.desktop.instance = instance
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                    .expect("boot consumes the owning instance once");
                app.serve_desktop_search();
                if rejected_uri {
                    app.picker.failure = Some(enzui_core::localization::text(
                        "text-only-local-files-can-be-opened-in-4f3054",
                    ));
                }
                (app, task)
            },
            TextApp::update,
            TextApp::view,
        )
        .title(TextApp::title)
        .theme(TextApp::theme)
        .subscription(TextApp::subscription)
        .window(with_desktop_identity(enz_window_settings(
            ShellLayout::TitlebarSidebarContent,
        )))
        .exit_on_close_request(enzui_core::close_guard::EXIT_ON_CLOSE_REQUEST),
        enzui_widgets::app_setup::AppServices::default()
            .localizer(localizer)
            .traversal(enzui_core::focus_traversal::TraversalBinding::Disabled)
            .accessibility_actions(false),
    )
}

#[cfg(test)]
fn test_theme_integration(theme: EnzTheme) -> (ThemeIntegration, Task<ThemeEvent>) {
    ThemeIntegration::unmanaged_compiled_theme(theme)
}

#[cfg(test)]
#[path = "main_tests/session_restore_worker_tests.rs"]
mod session_restore_worker_tests;

#[cfg(test)]
#[path = "main_tests/session_authority_tests.rs"]
mod session_authority_tests;

#[cfg(test)]
#[path = "main_tests/find_focus_flow_tests.rs"]
pub(crate) mod find_focus_flow_tests;

#[cfg(test)]
#[path = "main_tests/recovery_wiring_tests.rs"]
mod recovery_wiring_tests;

#[cfg(test)]
#[path = "main_tests/print_route_tests.rs"]
mod print_route_tests;

#[cfg(test)]
#[path = "main_tests/data_loss_tests.rs"]
mod data_loss_tests;

// Must match the basename of `io.github.enzui.Text.desktop` so the compositor
// groups the window with its desktop entry.
const DESKTOP_APP_ID: &str = "io.github.enzui.Text";

fn with_desktop_identity(settings: iced::window::Settings) -> iced::window::Settings {
    enzui_widgets::app_shell::DesktopApplicationIdentity::new(DESKTOP_APP_ID)
        .expect("BUG: DESKTOP_APP_ID must be a portable desktop-entry basename")
        .apply_to_window_settings(settings)
}

#[cfg(test)]
#[path = "main_tests/desktop_tests.rs"]
mod desktop_tests;
