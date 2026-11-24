//! TextApp print and file-watch reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_watch_print_tick(&mut self) -> Task<Message> {
        self.print.poll();
        Task::none()
    }
    pub(crate) fn reduce_watch_dismiss_print_status(&mut self) -> Task<Message> {
        self.print.dismiss_status();
        Task::none()
    }
    pub(crate) fn reduce_watch_file_watch_batch_settled(
        &mut self,
        generation: u64,
        retry: RetryPayload<Vec<(String, enzui_core::file_watch::ProbeOp)>>,
        result: Result<Vec<(String, Probe)>, BlockingTaskError>,
    ) -> Task<Message> {
        if generation != self.watcher_generation {
            return Task::none();
        }
        self.watcher_inflight = false;
        match result {
            Ok(probes) => {
                self.watcher_retry = None;
                self.watcher_retry_paths.clear();
                self.watcher_retry_blocked = false;
                for (tab_id, probe) in probes {
                    self.observe_probe(&tab_id, probe);
                }
            }
            Err(BlockingTaskError::Busy) => {
                self.watcher_retry = Some(retry);
                self.watcher_debt = true;
                self.blocking_retry_pending = true;
            }
            Err(_) => {
                self.watcher_retry = Some(retry);
                self.watcher_debt = false;
                self.watcher_retry_blocked = true;
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-file-change-checks-stopped-unexpectedly-retry-refresh-6c1790",
                ));
            }
        }
        Task::none()
    }
    pub(crate) fn watch_visible_state(&self) -> (Vec<bool>, Option<String>) {
        (
            self.model
                .tabs
                .iter()
                .map(|tab| tab.externally_changed)
                .collect(),
            self.picker.failure.clone(),
        )
    }

    pub(crate) fn observe_probe(&mut self, tab_id: &str, probe: Probe) {
        if let Some(tab) = self.model.tabs.iter_mut().find(|tab| tab.id == tab_id)
            && let Some(watcher) = &mut tab.watcher
            && watcher.observe(probe).requires_attention()
        {
            tab.externally_changed = true;
        }
    }
    pub(crate) fn reduce_watch_window_focus(&mut self, focused: bool) -> Task<Message> {
        let regained = focused && !self.window_focused;
        self.window_focused = focused;
        // The unfocused interval may have hidden an outside change for
        // seconds; show it as soon as the user looks again.
        if regained {
            self.begin_watcher_batch()
        } else {
            Task::none()
        }
    }
    pub(crate) fn reduce_watch_file_watch_refresh(&mut self) -> Task<Message> {
        self.watcher_retry_blocked = false;
        self.watcher_retry = None;
        self.watcher_retry_paths.clear();
        // Off-reducer, through the metadata-first classifier: a file grown past
        // the large boundary reloads read-only paged, and a large document
        // reopens a fresh descriptor rather than reading a replaced inode.
        let id = self.model.active_tab_id.clone();
        let Some(tab) = self.model.tabs.iter().find(|t| t.id == id) else {
            return Task::none();
        };
        let Some(path) = tab.file_path.clone() else {
            return Task::none();
        };
        self.begin_load(&id, &path, tab.workspace_root.clone(), LoadReason::Reload)
    }
    pub(crate) fn reduce_watch_file_watch_dismiss(&mut self) -> Task<Message> {
        // Acknowledging the on-disk identity prevents a re-prompt.
        let id = self.model.active_tab_id.clone();
        if let Some(tab) = self.model.tabs.iter_mut().find(|t| t.id == id) {
            tab.externally_changed = false;
            if let Some(watcher) = &mut tab.watcher
                && let Some(baseline) = watcher.baseline()
            {
                watcher.acknowledge(baseline);
            }
        }
        Task::none()
    }
}
