//! TextApp event subscriptions.

use super::*;

impl TextApp {
    pub(crate) const FOREGROUND_SEARCH_BYTES: usize = 1 << 20;

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let mut subs: Vec<Subscription<Message>> = vec![
            #[cfg(feature = "center")]
            self.center_subscription(),
            iced::event::listen_with(self.event_classifier()),
            a11y::subscription(),
            self.desktop.subscription(),
            self.themes
                .subscription()
                .map(|event| Message::Theme(Box::new(event))),
        ];

        let picker_wake = self.picker.completion_subscription();
        subs.push(picker_wake.map(|_| Message::PickerTick));
        subs.push(
            self.picker
                .location_subscription()
                .map(|_| Message::PickerTick),
        );
        if self.picker.location_needs_poll() {
            subs.push(iced::time::every(LOCATION_POLL_INTERVAL).map(|_| Message::PickerTick));
        }

        // Timer-driven, never `iced::window::frames()`: a frame subscription
        // that outlives gestures turns redraws into a self-sustaining busy loop
        // (see `FILE_WATCH_PROBE_INTERVAL`). Only path-backed tabs have watchers.
        if self.model.tabs.iter().any(|t| t.watcher.is_some()) {
            subs.push(iced::time::every(FILE_WATCH_PROBE_INTERVAL).map(|_| Message::FileWatchTick));
        }

        // Exists only while revision debt is unpublished or a write is in
        // flight, so a clean editor never wakes on the 3-second timer.
        if self.recovery_store.is_enabled()
            && (self.recovery_has_debt() || self.recovery_inflight.is_some())
        {
            subs.push(iced::time::every(RECOVERY_DEBOUNCE).map(|_| Message::RecoveryTick));
        }

        // The core Print portal exposes no completion signal to app code, so a
        // bounded poll runs only while a print is live.
        if self.print.is_busy() {
            subs.push(iced::time::every(PRINT_POLL_INTERVAL).map(|_| Message::PrintTick));
        }
        // A non-error print outcome dismisses itself after the toast dwell.
        if let Some(key) = self
            .print
            .status_key()
            .filter(|_| self.print.status().is_some_and(print::PrintStatus::expires))
        {
            subs.push(
                iced::time::every(PRINT_STATUS_DWELL)
                    .with(key)
                    .map(|_| Message::DismissPrintStatus),
            );
        }
        if self.close_drain.is_some() {
            subs.push(iced::time::every(CLOSE_DRAIN_POLL).map(|_| Message::CloseDrainTick));
        }
        if self.blocking_retry_pending && !self.blocking_retry_suspended {
            subs.push(
                iced::time::every(BLOCKING_RETRY_INTERVAL).map(|_| Message::BlockingRetryTick),
            );
        }

        // Selection-statistics debounce: a timer, never `frames()`, and only
        // while an open Statistics dialog lacks counts for the live selection.
        if self.model.stats_open && self.model.active_tab().wants_selection_stats() {
            subs.push(
                iced::time::every(SELECTION_STATS_DEBOUNCE)
                    .map(|_| Message::SelectionStatsSettleTick),
            );
        }

        Subscription::batch(subs)
    }
}
