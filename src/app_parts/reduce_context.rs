//! Context menu and accent reducers.

use super::*;

impl TextApp {
    pub(crate) fn reduce_context_workspace_context(
        &mut self,
        root: PathBuf,
        at: iced::Point,
    ) -> Task<Message> {
        let Some(epoch) = self.workspace_context_epoch.checked_add(1) else {
            return Task::none();
        };
        self.workspace_context_epoch = epoch;
        let restore = workspace_context::restore_target(&root);
        self.workspace_context_menu.open_restoring(
            root,
            InvocationKind::Pointer(ContextPoint::new(at.x, at.y)),
            vec![
                ContextMenuItem::separator(),
                ContextMenuItem::command(
                    WorkspaceCommand::Remove,
                    enzui_core::localization::text("text-remove-workspace-9de7ea"),
                )
                .icon("folder-minus")
                .destructive(true),
            ],
            restore,
        );
        Task::none()
    }

    pub(crate) fn view_workspace_colors(&self) -> Element<'_, Message> {
        let root = self.workspace_context_menu.target().cloned();
        let selected = root
            .as_ref()
            .and_then(|root| self.workspaces.accent_for(root));
        let epoch = self.workspace_context_epoch;
        let chooser = enzui_widgets::workspace_accent_chooser::enz_workspace_accent_chooser(
            WorkspaceAccentPalette::for_mode(&self.palette()),
            selected,
            move |accent| {
                root.as_ref()
                    .map_or(Message::NoOp, |root| Message::WorkspaceAccentChosen {
                        root: root.clone(),
                        epoch,
                        accent,
                    })
            },
        )
        .columns(4)
        .id_prefix(format!("text-workspace-colors-{epoch}"));
        iced::widget::container(
            iced::widget::column![
                enz_text(enzui_core::localization::text(
                    "text-workspace-colour-a2c102"
                ))
                .size(TextSize::Caption),
                chooser.into_element(),
            ]
            .spacing(self.spacing().sm()),
        )
        .padding(self.spacing().sm())
        .into()
    }

    pub(crate) fn choose_workspace_accent(
        &mut self,
        root: PathBuf,
        epoch: u64,
        accent: Option<WorkspaceAccentKey>,
    ) -> Task<Message> {
        if epoch != self.workspace_context_epoch
            || self.workspace_context_menu.target() != Some(&root)
        {
            return Task::none();
        }
        let mut changed = self.workspaces.set_accent(&root, accent);
        for tab in &mut self.model.tabs {
            if tab.workspace_root.is_none()
                && tab
                    .file_path
                    .as_deref()
                    .and_then(|path| self.workspaces.containing_root(path))
                    == Some(root.as_path())
            {
                tab.workspace_root = Some(root.clone());
                changed = true;
            }
        }
        if changed {
            self.advance_session_projection_revision();
        }
        let (_, focus) = self
            .workspace_context_menu
            .update(&ContextMenuEvent::Dismissed(
                enzui_core::menu_model::CloseCause::Activated,
            ));
        focus
    }

    pub(crate) fn reduce_context_settings_saved(
        &mut self,
        generation: u64,
        result: Result<bool, BlockingTaskError>,
    ) -> Task<Message> {
        self.settings_saved(generation, result)
    }
    pub(crate) fn reduce_context_workspace_context_event(
        &mut self,
        event: ContextMenuEvent,
    ) -> Task<Message> {
        let (chosen, focus) = self.workspace_context_menu.update(&event);
        if let Some((root, command)) = chosen {
            match command {
                WorkspaceCommand::Remove => {
                    self.pending_workspace_remove = Some(root);
                    self.workspace_remove_confirm_state = ConfirmState::new(None);
                }
            }
        }
        focus
    }
    pub(crate) fn reduce_context_workspace_remove_event(
        &mut self,
        event: ConfirmDialogEvent,
    ) -> Task<Message> {
        if let ConfirmDialogEvent::Resolved(cause) = event {
            let confirmed = enzui_core::confirm::ConfirmDecision::from_cause(cause).is_confirmed();
            if confirmed && let Some(root) = self.pending_workspace_remove.clone() {
                if self.workspaces.active == workspace::ActiveWorkspace::Named(root.clone())
                    && !self.switch_workspace(workspace::ActiveWorkspace::OpenFiles)
                {
                    return self
                        .close_stack_dialog(crate::dialog_focus::DialogFocusKind::WorkspaceRemove);
                }
                if self.workspaces.remove(&root) {
                    self.advance_session_projection_revision();
                }
            }
            return self.close_stack_dialog(crate::dialog_focus::DialogFocusKind::WorkspaceRemove);
        }
        Task::none()
    }
}
