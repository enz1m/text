//! TextApp blocking work failure type.

use super::*;

impl BlockingWorkFailure {
    pub(crate) fn error(&self) -> BlockingTaskError {
        match self {
            Self::Search { error, .. }
            | Self::Stats { error, .. }
            | Self::SelectionStats { error, .. }
            | Self::Save { error, .. }
            | Self::Print { error, .. }
            | Self::Load { error, .. }
            | Self::Page { error, .. } => *error,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum BlockingWorkFailure {
    Search {
        generation: u64,
        from: Option<usize>,
        error: BlockingTaskError,
    },
    Stats {
        tab: String,
        generation: u64,
        error: BlockingTaskError,
    },
    SelectionStats {
        tab: String,
        key: crate::model::SelectionStatsKey,
        error: BlockingTaskError,
    },
    Save {
        tab: String,
        generation: u64,
        destination: PathBuf,
        rebind: Option<PathBuf>,
        guard_attempt: Option<SaveAttemptToken>,
        error: BlockingTaskError,
    },
    Print {
        generation: u64,
        error: BlockingTaskError,
    },
    Load {
        tab: String,
        path: PathBuf,
        workspace: Option<PathBuf>,
        token: large_doc::LoadToken,
        reason: LoadReason,
        error: BlockingTaskError,
    },
    Page {
        tab: String,
        token: large_doc::LoadToken,
        operation: RetryPayload<enzui_core::paged_text::PagedTextOperation>,
        error: BlockingTaskError,
    },
}
