//! TextApp titlebar and banner views.

use super::*;

use iced::widget::column;

impl TextApp {
    pub(crate) fn view_titlebar_with_tabs(&self) -> Element<'_, Message> {
        enz_title_bar()
            .padding_x(self.spacing().xs())
            .push(self.view_scroll_tabs())
            .fluid_content(crate::tab_strip::MINIMUM_WIDTH)
            .actions(self.titlebar_actions(), Message::ToggleOverflowMenu)
            .action_dismiss(Message::CloseOverflowMenu)
            .action_overflow_trigger_id(settings_invoker_id())
            .overflow_open(self.model.overflow_menu_open)
            .into_element()
    }

    pub(crate) fn titlebar_actions(&self) -> Vec<TitlebarActionItem<'_, Message>> {
        Self::overflow_menu_actions()
            .into_iter()
            .map(|action| {
                if matches!(action, ToolbarAction::Open) {
                    return TitlebarActionItem::icon(
                        action.descriptor_id(),
                        action.icon(),
                        action.inline_label(),
                        action.message(),
                    )
                    .group(action.group());
                }
                let descriptor = Descriptor::new(action.descriptor_id())
                    .with_presentation(enzui_core::action_overflow::Presentation::Compact);
                let descriptor = match action {
                    ToolbarAction::Save | ToolbarAction::Find => descriptor,
                    _ => descriptor.always_overflow(),
                };
                let shortcut = keybindings::binding(action.keybinding())
                    .expect("every titlebar action must have a shortcut binding")
                    .primary()
                    .clone();
                TitlebarActionItem::command(
                    descriptor,
                    CoreAction::new(action.descriptor_id(), action.message())
                        .with_label(action.overflow_label())
                        .with_icon(Icon::named(action.icon()))
                        .with_shortcut(shortcut)
                        .with_enabled(self.toolbar_action_enabled(action)),
                )
                .group(action.group())
            })
            .collect()
    }

    // `Open` and `Settings` are app-level and always available.
    pub(crate) fn toolbar_action_enabled(&self, action: ToolbarAction) -> bool {
        let tab = self.model.active_tab();
        let capability = match action {
            ToolbarAction::Open | ToolbarAction::Settings => return true,
            // Printable only for a materializable editable document with no
            // print in flight.
            ToolbarAction::Print => {
                return tab.supports(large_doc::Capability::Save) && !self.print.is_busy();
            }
            ToolbarAction::Save | ToolbarAction::SaveAs => large_doc::Capability::Save,
            ToolbarAction::Find => large_doc::Capability::Find,
            ToolbarAction::Replace => large_doc::Capability::Replace,
            ToolbarAction::GotoLine => large_doc::Capability::Goto,
            ToolbarAction::Statistics => large_doc::Capability::Stats,
        };
        tab.supports(capability)
    }

    fn view_workspace_tree(&self, viewport_height: f32) -> Element<'_, Message> {
        use std::rc::Rc;

        let rows = self.workspaces.rows();
        let selected = self.workspace_tree_selected_row(&rows);
        let items: Vec<EnzTreeItem> = rows
            .iter()
            .map(|row| {
                let item = if row.has_children {
                    EnzTreeItem::branch(row.label.clone(), row.level, row.expanded)
                } else {
                    EnzTreeItem::leaf(row.label.clone(), row.level)
                };
                item.key(row.key.clone()).aria_label(row.label.clone())
            })
            .collect();

        // `rows()` is a shared `Rc<[TreeRow]>`; closures clone the reference.
        let rows_for_content = Rc::clone(&rows);
        let rows_for_select = Rc::clone(&rows);
        let rows_for_toggle = Rc::clone(&rows);
        let rows_for_context = rows;
        let selected_key =
            selected.and_then(|index| enzui_widgets::tree_view::selection_key_for(&items, index));
        enz_tree_view(items)
            .viewport_height(viewport_height)
            .id(workspace_context::tree_id())
            .id_prefix(workspace_context::TREE_ID_PREFIX)
            .aria_label(enzui_core::localization::text(
                "text-workspaces-and-files-1f7239",
            ))
            .tree_row_height(36.0)
            // `EnzTreeView` seeds its scrollable once per fresh state, so both
            // `scroll_id` and the persisted `scroll_offset` are passed every build.
            .scroll_id(workspace_context::tree_scroll_id())
            .scroll_offset(self.model.settings.workspace_sidebar_scroll_offset)
            .on_scroll(|vp| Message::WorkspaceSidebarScrolled(vp.absolute_offset().y))
            .selected_key(selected_key)
            .selected_background(if self.workspace_context_menu.is_open() {
                self.palette().overlay_hover()
            } else {
                self.palette().accent_soft
            })
            .row_content(move |index, item| {
                let icon = rows_for_content.get(index).map_or("file", |row| {
                    if matches!(row.kind, workspace::TreeRowKind::File { .. }) {
                        enzui_icons::file_type::FileTypeTable::host().icon_name(&row.label)
                    } else {
                        row.icon
                    }
                });
                enz_icon_label(icon, item.text.clone()).into_element()
            })
            .on_select(
                move |index| match rows_for_select.get(index).map(|row| &row.kind) {
                    // The row body selects and opens; only the chevron's
                    // `WorkspaceToggle` can close the branch.
                    Some(workspace::TreeRowKind::Workspace(root)) => {
                        Message::WorkspaceSelect(root.clone())
                    }
                    Some(workspace::TreeRowKind::Directory { path, .. }) => {
                        Message::WorkspaceToggle(path.clone())
                    }
                    Some(workspace::TreeRowKind::File { workspace, path }) => {
                        Message::WorkspaceOpenFile {
                            workspace: workspace.clone(),
                            path: path.clone(),
                        }
                    }
                    Some(workspace::TreeRowKind::Status) | None => Message::NoOp,
                },
            )
            .on_toggle(
                move |index| match rows_for_toggle.get(index).map(|row| &row.kind) {
                    Some(workspace::TreeRowKind::Workspace(root)) => {
                        Message::WorkspaceToggle(root.clone())
                    }
                    Some(workspace::TreeRowKind::Directory { path, .. }) => {
                        Message::WorkspaceToggle(path.clone())
                    }
                    _ => Message::NoOp,
                },
            )
            .row_on_context(move |index, at| {
                match rows_for_context.get(index).map(|row| &row.kind) {
                    Some(workspace::TreeRowKind::Workspace(root)) => {
                        Message::WorkspaceContext(root.clone(), at)
                    }
                    _ => Message::NoOp,
                }
            })
            .row_context_label(enzui_core::localization::text(
                "text-workspace-actions-66f56c",
            ))
            .into()
    }

    pub(crate) fn workspace_tree_selected_row(&self, rows: &[workspace::TreeRow]) -> Option<usize> {
        // Context selection is presentation-only; dismissal reveals the active document's row.
        self.workspace_context_menu
            .target()
            .and_then(|root| {
                rows.iter().position(|row| {
                    matches!(&row.kind, workspace::TreeRowKind::Workspace(path) if path == root)
                })
            })
            .or_else(|| {
                self.workspaces
                    .selected_row(rows, self.model.active_tab().file_path.as_deref())
            })
    }

    pub(crate) fn view_document_sidebar(&self) -> Element<'_, Message> {
        let sp = self.spacing();

        let content: Element<Message> =
            if let Some((title, description, icon)) = self.workspaces.active_empty_state() {
                let height = self.model.window_size.map_or_else(
                    || {
                        enz_window_settings(ShellLayout::TitlebarSidebarContent)
                            .size
                            .height
                    },
                    |(_, height)| height,
                );
                let tree = self.view_workspace_tree(height);
                let state: Element<Message> = enz_scroll_view(
                    enz_status_page(title)
                        .description(description)
                        .icon(icon)
                        .variant(StatusPageVariant::Inline)
                        .tone(StatusPageTone::Empty)
                        .id_prefix("text-app-workspace-empty")
                        .into_element(),
                )
                .into();
                // The tree hugs its rows; the empty state takes the slack.
                column![
                    container(tree).height(Length::Shrink),
                    container(state).height(Length::Fill),
                ]
                .height(Length::Fill)
                .into()
            } else {
                iced::widget::responsive(move |size| self.view_workspace_tree(size.height)).into()
            };

        // `.fill(false)` shrink-wraps icon and label into one group that
        // `enz_list_row_surface`'s alignment centres, keeping their 8 px gap.
        let centred_add: Element<Message> = enz_icon_label(
            "folder-plus",
            enzui_core::localization::text("text-add-workspace-folder-572893"),
        )
        .fill(false)
        .into_element();
        let add_workspace: Element<Message> =
            enz_list_row_surface(centred_add, Message::AddWorkspace)
                .content_align_x(iced::Alignment::Center)
                .label(enzui_core::localization::text(
                    "text-add-workspace-folder-572893",
                ))
                .id_prefix("text-app-add-workspace")
                .into();
        let body = enz_sidebar_content()
            .spacing_scale(sp)
            .width(Length::Fill)
            .height(Length::Fill);
        let body: Element<Message> = body.push(content).push(add_workspace).into();

        body
    }

    pub(crate) fn tab_description(model: &Model, tab_id: &str, fallback_title: &str) -> String {
        model
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.file_path.as_ref())
            .map_or_else(
                || fallback_title.to_owned(),
                |path| crate::safe_text::bounded_inline(&path.display().to_string()),
            )
    }

    pub(crate) fn view_notifications(&self) -> Option<Element<'_, Message>> {
        use enzui_widgets::toast::{ToastTone, enz_toast};
        let mut notices = Vec::new();
        if let Some(notice) = self.view_print_notification() {
            notices.push(notice);
        }
        if let Some(notice) = self.view_session_notification() {
            notices.push(notice);
        }
        if let Some(failure) = &self.picker.failure {
            notices.push(
                enz_toast(failure.clone(), Message::DismissPickerFailure)
                    .tone(ToastTone::Error)
                    .id_prefix("text-picker-error")
                    .into_element(),
            );
        }
        let tab = self.model.active_tab();
        if tab.authority_exhausted() && !self.authority_notice_dismissed.contains(&tab.id) {
            notices.push(
                enz_toast(
                    crate::i18n::domain_text(model::TAB_AUTHORITY_EXHAUSTED_MESSAGE),
                    Message::DismissAuthorityNotice,
                )
                .tone(ToastTone::Error)
                .id_prefix("text-authority-exhausted")
                .into_element(),
            );
        }
        if let Some(error) = tab
            .buffer
            .large()
            .and_then(super::large_doc::LargeDocument::load_error)
            .filter(|error| {
                self.large_load_notice_dismissed
                    .as_ref()
                    .is_none_or(|(id, seen)| id != &tab.id || seen != error)
            })
        {
            notices.push(
                enz_toast(error, Message::DismissLargeLoadNotice)
                    .tone(ToastTone::Error)
                    .action(
                        enzui_core::localization::text("text-refresh-from-disk-b7ef65"),
                        Message::FileWatchRefresh,
                    )
                    .id_prefix("text-large-load-error")
                    .into_element(),
            );
        }
        if tab.externally_changed {
            let body = if tab.dirty {
                enzui_core::localization::text(
                    "text-refresh-discards-your-unsaved-edits-dismiss-to-b4b86c",
                )
            } else {
                enzui_core::localization::text("text-refresh-to-load-the-disk-version-154d78")
            };
            notices.push(
                enz_toast(
                    enzui_core::localization::text("text-file-changed-on-disk-1a3d3e"),
                    Message::FileWatchDismiss,
                )
                .description(body)
                .tone(ToastTone::Warning)
                .action(
                    enzui_core::localization::text("text-refresh-0e9161"),
                    Message::FileWatchRefresh,
                )
                .id_prefix("text-file-changed")
                .into_element(),
            );
        }
        (!notices.is_empty()).then(|| column(notices).spacing(self.spacing().xs()).into())
    }

    pub(crate) fn view_print_notification(&self) -> Option<Element<'_, Message>> {
        use enzui_widgets::toast::{ToastTone, enz_toast};
        let status = self.print.status()?;
        let tone = match status {
            print::PrintStatus::Submitted => ToastTone::Success,
            print::PrintStatus::SubmittedTruncated { .. } => ToastTone::Warning,
            print::PrintStatus::Cancelled => ToastTone::Info,
            _ => ToastTone::Error,
        };
        Some(
            enz_toast(status.banner(), Message::DismissPrintStatus)
                .tone(tone)
                .id_prefix("text-print-status")
                .into_element(),
        )
    }

    pub(crate) fn view_session_notification(&self) -> Option<Element<'_, Message>> {
        use enzui_widgets::toast::enz_toast;
        let debt = self.session_debt.as_ref()?;
        if self.session_notice_dismissed == Some(debt.kind()) {
            return None;
        }
        let (title, body, tone) = session_notification_copy(debt.kind());
        let mut notice = enz_toast(title, Message::DismissSessionNotice)
            .description(body)
            .tone(tone)
            .id_prefix("text-session-recovery");
        if self.session_restore_pending.is_none() {
            if debt.expected_identity().is_some() {
                notice = notice.action(
                    enzui_core::localization::text("text-start-fresh-652ace"),
                    Message::SessionRestoreRetire,
                );
            } else if debt.can_retry() {
                let label = if debt.kind() == session::SessionDebtKind::Superseded {
                    enzui_core::localization::text("text-restore-a76e13")
                } else {
                    enzui_core::localization::text("text-try-again-d8b839")
                };
                notice = notice.action(label, Message::SessionRestoreRetry);
            }
        }
        if matches!(
            debt.kind(),
            session::SessionDebtKind::Refused(_)
                | session::SessionDebtKind::Replaced
                | session::SessionDebtKind::FutureVersion
                | session::SessionDebtKind::CustodyLost
        ) {
            notice = notice.secondary_action(
                enzui_core::localization::text("text-show-folder-5655ae"),
                Message::SessionRestoreOpenLocation,
            );
        }
        Some(notice.into_element())
    }
}

fn session_notification_copy(
    kind: session::SessionDebtKind,
) -> (String, String, enzui_widgets::toast::ToastTone) {
    use enzui_widgets::toast::ToastTone;
    use session::{SessionDebtKind as Kind, SessionRefusal as Refusal};
    const NOT_REMEMBERED: &str =
        "Open tabs won't be remembered until this is fixed. The saved file is untouched.";
    match kind {
        Kind::Busy => (
            enzui_core::localization::text("text-your-last-session-is-still-being-restored-cf9f94"),
            enzui_core::localization::text("text-your-open-tabs-won-t-be-remembered-8547ce"),
            ToastTone::Info,
        ),
        Kind::Superseded => (
            enzui_core::localization::text("text-your-last-session-wasn-t-restored-206c73"),
            enzui_core::localization::text("text-a-file-was-opened-first-restore-it-517db9"),
            ToastTone::Info,
        ),
        Kind::DurabilityUncertain => (
            enzui_core::localization::text("text-your-session-may-not-have-been-saved-9a1910"),
            enzui_core::localization::text("text-text-will-try-again-when-your-tabs-0d8634"),
            ToastTone::Warning,
        ),
        Kind::PublishOverBound(_) => (
            enzui_core::localization::text("text-too-many-tabs-to-remember-ca483d"),
            enzui_core::localization::text("text-close-some-tabs-and-your-session-will-6d42ea"),
            ToastTone::Warning,
        ),
        Kind::PublishRevisionExhausted => (
            enzui_core::localization::text("text-your-session-can-t-be-saved-any-619169"),
            enzui_core::localization::text("text-restart-text-to-save-your-open-tabs-779e1b"),
            ToastTone::Error,
        ),
        Kind::CustodyLost => (
            enzui_core::localization::text(
                "text-your-session-file-was-changed-by-something-545167",
            ),
            enzui_core::localization::text("text-your-open-tabs-won-t-be-remembered-8ab7d9"),
            ToastTone::Error,
        ),
        Kind::FutureVersion => (
            enzui_core::localization::text("text-your-last-session-is-from-a-newer-2192ab"),
            crate::i18n::domain_text(NOT_REMEMBERED),
            ToastTone::Error,
        ),
        Kind::Replaced | Kind::Refused(Refusal::Replaced | Refusal::Unstable) => (
            enzui_core::localization::text("text-your-last-session-changed-while-it-was-d87f97"),
            crate::i18n::domain_text(NOT_REMEMBERED),
            ToastTone::Error,
        ),
        Kind::Refused(Refusal::TooLarge | Refusal::TooManyEntries | Refusal::PathBudget) => (
            enzui_core::localization::text("text-your-last-session-is-too-large-to-643e69"),
            crate::i18n::domain_text(NOT_REMEMBERED),
            ToastTone::Error,
        ),
        Kind::Refused(Refusal::Malformed | Refusal::ReadFailed) => (
            enzui_core::localization::text("text-your-last-session-couldn-t-be-read-e63a30"),
            crate::i18n::domain_text(NOT_REMEMBERED),
            ToastTone::Error,
        ),
        Kind::Refused(Refusal::NotRegularFile | Refusal::OpenRefused) => (
            enzui_core::localization::text("text-your-last-session-couldn-t-be-opened-f7020d"),
            crate::i18n::domain_text(NOT_REMEMBERED),
            ToastTone::Error,
        ),
    }
}
