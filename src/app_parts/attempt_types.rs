//! Worker attempt structs.

use super::*;

#[derive(Clone)]
pub(crate) struct RecoveryPublishAttempt {
    pub(crate) generation: u64,
    pub(crate) captured: std::collections::HashMap<String, u64>,
    pub(crate) leases: Vec<RecoveryLease>,
}

#[derive(Clone)]
pub(crate) struct PrintAttempt {
    pub(crate) generation: u64,
    pub(crate) snapshot: enzui_core::chunked_text::TextSnapshot,
    pub(crate) title: String,
}

#[derive(Clone)]
pub(crate) struct PendingSearch {
    pub(crate) generation: u64,
    pub(crate) from: Option<usize>,
    pub(crate) query: String,
    pub(crate) options: search::SearchOptions,
    pub(crate) document: enzui_core::chunked_text::TextSnapshot,
    pub(crate) cancel: std::sync::Arc<worker::Cancel>,
    pub(crate) binding: search::MatchBinding,
}

#[derive(Clone)]
pub(crate) struct PendingStats {
    pub(crate) tab: String,
    pub(crate) generation: u64,
    pub(crate) document: enzui_core::chunked_text::TextSnapshot,
}

#[derive(Clone)]
pub(crate) struct PendingSelectionStats {
    pub(crate) tab: String,
    pub(crate) key: crate::model::SelectionStatsKey,
    pub(crate) selection: crate::model::SelectionStatsJob,
}

#[derive(Clone)]
pub(crate) struct SaveRequest {
    pub(crate) tab: String,
    pub(crate) generation: u64,
    pub(crate) destination: PathBuf,
    pub(crate) ending: line_ending::LineEnding,
    pub(crate) document: enzui_core::chunked_text::TextSnapshot,
    pub(crate) rebind: Option<PathBuf>,
    pub(crate) replaces: atomic_save::Replaces,
    pub(crate) guard_attempt: Option<SaveAttemptToken>,
}
