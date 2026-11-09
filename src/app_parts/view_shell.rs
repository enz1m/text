//! TextApp root view and application shell methods.

use super::*;

impl TextApp {
    pub(crate) fn capture_startup_settled(&self) -> bool {
        self.session_restore_pending.is_none() && self.model.tabs.iter().all(|tab| !tab.loading)
    }

    pub(crate) fn view(&self) -> Element<'_, Message> {
        self.view_localized()
    }

    fn view_localized(&self) -> Element<'_, Message> {
        static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if std::env::var("ENZUI_CAPTURE_READY").as_deref() == Ok("1")
            && self.capture_startup_settled()
            && !READY.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            eprintln!("[CAPTURE_READY] text-app");
        }

        let framed: Element<Message> = enz_app_shell(move || self.view_content_pane())
            .layout(ShellLayout::TitlebarSidebarContent)
            .tokens(shell::shell_tokens())
            .titlebar(move || self.view_titlebar_with_tabs())
            .on_window_gesture(Message::Window)
            .sidebar_label(enzui_core::localization::text("text-workspaces-137726"))
            .sidebar_titlebar(|| {
                enz_title_bar()
                    .title(enzui_core::localization::text("text-workspaces-137726"))
                    .into_element()
            })
            .sidebar(move || self.view_document_sidebar())
            .state(self.nav)
            .on_sidebar_state(Message::SidebarState)
            .on_dismiss_overlay(Message::DismissSidebar)
            .into_element();

        let key = self.desktop_drop_key();
        let callback_key = key.clone();
        let base: Element<Message> =
            enzui_widgets::file_drop::enz_file_drop_target_with_receipt(framed, move |files| {
                Message::Desktop(desktop::DesktopEvent::Dropped {
                    key: callback_key.clone(),
                    files,
                })
            })
            .copy_only(true)
            .key((key, self.desktop_input_blocked()))
            .into();
        // Mounted while closed too, so opening the menu keeps the page's
        // widget state.
        let base: Element<Message> = self
            .workspace_context_menu
            .host(base, Message::WorkspaceContextEvent)
            .header(move |_| self.view_workspace_colors())
            .menu_width(176.0)
            .label(enzui_core::localization::text(
                "text-workspace-actions-66f56c",
            ))
            .id_prefix("text-app-workspace-context")
            .into_element();
        // Settings retains keyed exit settlement and bottom-edge placement.
        let settings = (self.model.settings_open || self.settings_is_closing())
            .then(|| self.view_settings_dialog());
        let mut settings_host = enz_modal_host(base, settings, Message::SettingsDialogEvent)
            .presentation_open(self.model.settings_open)
            .placement(self.settings_surface_placement())
            .label(enzui_core::localization::text("text-settings-74a883"));
        if let Some(lifecycle) = self.settings_focus.as_ref() {
            settings_host = settings_host.scoped_to(lifecycle);
        }
        if let Some(key) = self.settings_presentation_key() {
            settings_host =
                settings_host.on_exit_settled_with(move || Message::SettingsExitSettled(key));
        }
        let stacked = self.dialogs.view_with(
            settings_host.into_element(),
            |kind| self.view_stack_dialog(*kind),
            Message::StackDialogEvent,
            |kind, host| {
                host.label(match kind {
                    crate::dialog_focus::DialogFocusKind::Stats => {
                        enzui_core::localization::text("text-document-statistics-308052")
                    }
                    crate::dialog_focus::DialogFocusKind::Shortcuts => {
                        enzui_core::localization::text("text-keyboard-shortcuts-59cdaa")
                    }
                    crate::dialog_focus::DialogFocusKind::WorkspaceRemove => {
                        enzui_core::localization::text("text-remove-workspace-9de7ea")
                    }
                    _ => enzui_core::localization::text("text-save-changes-before-closing-6239f0"),
                })
            },
        );
        let guard = match self.active_dialog_focus_kind() {
            Some(crate::dialog_focus::DialogFocusKind::RecoveryDiscardFailure) => {
                Some(self.view_recovery_discard_failure())
            }
            Some(crate::dialog_focus::DialogFocusKind::WindowClose) => {
                Some(self.view_window_close_confirm())
            }
            _ => None,
        };
        let mut host = enz_modal_host(stacked, guard, Message::DialogEvent).policy(
            if self.close_guard.is_saving() {
                enzui_widgets::modal_host::alert_dismiss_policy()
            } else {
                enzui_widgets::modal_host::modal_dismiss_policy()
            },
        );
        if let Some(kind) = self.active_dialog_focus_kind() {
            host = host.label(match kind {
                crate::dialog_focus::DialogFocusKind::RecoveryDiscardFailure => {
                    enzui_core::localization::text("text-couldn-t-clear-crash-recovery-498c6e")
                }
                _ => enzui_core::localization::text("text-save-changes-before-closing-6239f0"),
            });
        }
        if let Some((_, lifecycle)) = self.dialog_focus.as_ref() {
            host = host.scoped_to(lifecycle);
        }
        self.picker.view(enzui_widgets::toast_host::enz_toast_layer(
            host.into_element(),
            self.view_notifications(),
            enzui_widgets::toast_host::ToastOverlayPlacement::FLUSH
                .trailing()
                .bottom_inset(self.themes.theme().sizing.statusbar_height + EDITOR_INTERNAL_PAD),
        ))
    }

    fn view_stack_dialog(
        &self,
        kind: crate::dialog_focus::DialogFocusKind,
    ) -> Element<'_, Message> {
        use crate::dialog_focus::DialogFocusKind;
        match kind {
            DialogFocusKind::Stats => self.view_stats_dialog(self.model.active_tab()),
            DialogFocusKind::Shortcuts => self.view_shortcuts_dialog(),
            DialogFocusKind::WorkspaceRemove => self.pending_workspace_remove.as_ref().map_or_else(
                || iced::widget::Space::new().into(),
                |root| self.view_workspace_remove_confirm(root),
            ),
            DialogFocusKind::TabClose => self.model.pending_close.as_ref().map_or_else(
                || iced::widget::Space::new().into(),
                |id| self.view_close_confirm(id),
            ),
            _ => iced::widget::Space::new().into(),
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        self.themes.iced_theme()
    }

    pub(crate) fn title(&self) -> String {
        self.model.window_title()
    }
}
