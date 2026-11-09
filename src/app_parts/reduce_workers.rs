//! TextApp worker completion reduce methods.

use super::*;

impl TextApp {
    pub(crate) fn reduce_workers_virtual_input_method(
        &mut self,
        event: iced::advanced::input_method::Event,
    ) -> Task<Message> {
        if self.model.active_tab().is_read_only() {
            return Task::none();
        }
        // An IME commit mints fresh snapshot/cursor authority, so refuse before
        // that mint once authority is exhausted.
        if self.refuse_if_authority_exhausted() {
            return Task::none();
        }
        self.handle_virtual_input_method(event);
        Task::none()
    }
    // Intercepted by `update` above (which skips the session /
    // doc-tab passes for it); handled here too so the reducer stays
    // total and a direct `update_inner` call in a test behaves.
    pub(crate) fn reduce_workers_search_completed(
        &mut self,
        generation: u64,
        from: Option<usize>,
        result: Option<Result<search::Scan, String>>,
        binding: search::MatchBinding,
    ) -> Task<Message> {
        self.search_scan = None;
        self.search_attempt = None;
        self.search_retry = None;
        // Reject a completion whose scan targeted a tab or snapshot version that
        // is no longer live, even if a route failed to bump the generation;
        // queued work still pumps.
        let (active_id, active_version) = {
            let active = self.model.active_tab();
            (active.id.clone(), active.snapshot_version())
        };
        let binding_lives =
            binding.tab_id == active_id && binding.snapshot_version == active_version;
        if binding_lives && let Some(result) = result {
            let _accepted = self.model.search.accept(generation, result, from, binding);
        }
        self.pump_pending_work();
        Task::none()
    }
    pub(crate) fn reduce_workers_stats_completed(
        &mut self,
        tab: String,
        generation: u64,
        text: stats::TextStats,
        document: doc_stats::Stats,
    ) -> Task<Message> {
        self.stats_attempts.remove(&tab);
        self.stats_retry.remove(&tab);
        self.stats_retry_blocked.remove(&tab);
        if let Some(index) = self.model.tabs.iter().position(|t| t.id == tab)
            && !self.model.tabs[index].accept_stats(generation, text, document)
        {
            // Stale: the buffer was edited while it was being
            // counted. Exactly one replacement pass, against the
            // buffer as it is now.
            let _ = self.spawn_stats(&tab);
        }
        self.pump_pending_work();
        Task::none()
    }
    pub(crate) fn reduce_workers_selection_stats_completed(
        &mut self,
        tab: String,
        key: crate::model::SelectionStatsKey,
        stats: doc_stats::Stats,
    ) -> Task<Message> {
        self.selection_stats_attempts.remove(&tab);
        self.selection_stats_retry.remove(&tab);
        self.selection_stats_retry_blocked.remove(&tab);
        // `accept_selection_stats` drops a stale count. No replacement starts
        // here, which would copy the selection per drag event; the next settle
        // tick starts exactly one.
        if let Some(index) = self.model.tabs.iter().position(|t| t.id == tab) {
            let _accepted = self.model.tabs[index].accept_selection_stats(key, stats);
        }
        self.pump_pending_work();
        Task::none()
    }
    // The debounce tick runs only while an open Statistics dialog lacks counts
    // for the live selection. It copies the selection to a worker only after
    // two consecutive ticks see the same generation.
    pub(crate) fn reduce_workers_selection_stats_settle_tick(&mut self) -> Task<Message> {
        let tab_id = self.model.active_tab_id.clone();
        let key = self.model.active_tab().selection_stats_key();
        if !self.model.active_tab().wants_selection_stats() {
            self.selection_settle_probe = None;
            return Task::none();
        }
        if self.selection_settle_probe.as_ref() == Some(&(tab_id.clone(), key)) {
            self.selection_settle_probe = None;
            self.spawn_selection_stats();
        } else {
            self.selection_settle_probe = Some((tab_id, key));
        }
        Task::none()
    }
}
