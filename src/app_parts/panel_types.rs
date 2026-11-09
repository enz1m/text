//! TextApp workspace panel and worker-pool types.

use super::*;

#[derive(Debug, Clone)]
pub(crate) enum WorkspaceCommand {
    Remove,
}

pub(crate) type WorkspaceContextMenu =
    enzui_widgets::context_menu_host::ContextMenu<PathBuf, WorkspaceCommand>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveAsFinish {
    Cancelled,
    Failed,
    InFlight,
}

pub(crate) const BLOCKING_WORKERS: NonZeroUsize =
    NonZeroUsize::new(4).expect("worker count is positive");
pub(crate) const BLOCKING_QUEUE: NonZeroUsize =
    NonZeroUsize::new(12).expect("queue capacity is positive");
pub(crate) const CLOSE_DRAIN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
pub(crate) const CLOSE_DRAIN_POLL: std::time::Duration = std::time::Duration::from_millis(25);
pub(crate) const BLOCKING_RETRY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);

pub(crate) fn app_blocking_owner() -> BlockingTaskOwner {
    BlockingTaskOwner::new("text-app-blocking", BLOCKING_WORKERS, BLOCKING_QUEUE)
        .unwrap_or_else(|_| BlockingTaskOwner::unavailable(BLOCKING_QUEUE))
}
