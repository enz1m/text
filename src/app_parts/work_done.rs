//! TextApp completed-work type.

use super::*;

#[derive(Clone)]
pub(crate) enum WorkDone {
    Search {
        generation: u64,
        from: Option<usize>,
        result: Option<Result<search::Scan, String>>,
        binding: search::MatchBinding,
    },
    Stats {
        tab: String,
        generation: u64,
        text: stats::TextStats,
        document: doc_stats::Stats,
    },
    Save {
        tab: String,
        generation: u64,
        destination: std::path::PathBuf,
        rebind: Option<std::path::PathBuf>,
        guard_attempt: Option<SaveAttemptToken>,
        result: Result<Option<FileIdentity>, document_failure::DocumentFailure>,
    },
    SelectionStats {
        tab: String,
        key: crate::model::SelectionStatsKey,
        stats: doc_stats::Stats,
    },
}

impl WorkDone {
    pub(crate) fn message(self) -> Message {
        match self {
            Self::Search {
                generation,
                from,
                result,
                binding,
            } => Message::SearchCompleted {
                generation,
                from,
                result,
                binding,
            },
            Self::Stats {
                tab,
                generation,
                text,
                document,
            } => Message::StatsCompleted {
                tab,
                generation,
                text,
                document,
            },
            Self::Save {
                tab,
                generation,
                destination,
                rebind,
                guard_attempt,
                result,
            } => Message::SaveCompleted {
                tab,
                generation,
                destination,
                rebind,
                guard_attempt,
                result,
            },
            Self::SelectionStats { tab, key, stats } => {
                Message::SelectionStatsCompleted { tab, key, stats }
            }
        }
    }
}
