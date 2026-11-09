//! Session restore and workspace reducers.

use super::*;

impl TextApp {
    pub(crate) fn reduce_session_session_restore_completed(
        &mut self,
        generation: u64,
        result: Result<session::SessionRestore, BlockingTaskError>,
    ) -> Task<Message> {
        self.apply_session_restore(generation, result)
    }
    pub(crate) fn reduce_session_session_restore_open_location(&mut self) -> Task<Message> {
        if let Some(debt) = self.session_debt.as_ref() {
            let location = debt
                .location()
                .parent()
                .unwrap_or_else(|| std::path::Path::new("/"));
            if let Err(text) = self.picker.start_location(location) {
                self.picker.failure = Some(text);
            }
        }
        Task::none()
    }
    pub(crate) fn reduce_session_add_workspace(&mut self) -> Task<Message> {
        if let Err(outcome) = self.picker.start_workspace() {
            self.finish_workspace_pick(outcome);
        }
        Task::none()
    }
    pub(crate) fn reduce_session_workspace_select(&mut self, root: PathBuf) -> Task<Message> {
        let _ = self.switch_workspace(workspace::ActiveWorkspace::Named(root.clone()));
        // The row body toggles expansion like the chevron, but only the body
        // also activates the workspace. The first expand reads its directory
        // off the update thread.
        match self.workspaces.begin_toggle(&root) {
            Some(pending) => self.begin_workspace_read(pending),
            None => {
                self.workspace_debt.remove(&root);
                self.workspace_retry_blocked.remove(&root);
                Task::none()
            }
        }
    }
    pub(crate) fn reduce_session_workspace_sidebar_scrolled(
        &mut self,
        offset: f32,
    ) -> Task<Message> {
        // The model keeps the offset finite and non-negative. It is persisted
        // by the close drain, not on every wheel event.
        self.model.settings.workspace_sidebar_scroll_offset =
            enzui_core::metrics::FiniteNonNeg::new_or(offset, 0.0).get();
        Task::none()
    }
    pub(crate) fn reduce_session_workspace_open_file(
        &mut self,
        workspace: PathBuf,
        path: PathBuf,
    ) -> Task<Message> {
        // Switching workspace never requires saving or closing dirty tabs.
        if self.workspaces.active != workspace::ActiveWorkspace::Named(workspace.clone()) {
            let _ = self.switch_workspace(workspace::ActiveWorkspace::Named(workspace.clone()));
        }
        // The selection is the immediate click feedback; the read is
        // off-thread.
        self.workspaces.select_file(&path);
        if self.sidebar_presentation().is_overlay() {
            self.nav.dismiss_overlay();
        }
        self.begin_open(&path, Some(workspace))
    }
}
