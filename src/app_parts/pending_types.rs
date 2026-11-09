//! TextApp pending-work payload types.

use super::*;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CloseDrain {
    pub(crate) window: iced::window::Id,
    pub(crate) deadline: std::time::Instant,
}

#[derive(Clone)]
pub(crate) struct PendingLoad {
    pub(crate) path: PathBuf,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) token: large_doc::LoadToken,
    pub(crate) reason: LoadReason,
}

#[derive(Clone)]
pub(crate) struct PendingWorkspaceRead {
    pub(crate) generation: u64,
    pub(crate) path: PathBuf,
}

pub(crate) struct RetryPayload<T> {
    pub(crate) value: std::sync::Arc<std::sync::Mutex<Option<T>>>,
}

impl<T> Clone for RetryPayload<T> {
    fn clone(&self) -> Self {
        Self {
            value: std::sync::Arc::clone(&self.value),
        }
    }
}

impl<T> std::fmt::Debug for RetryPayload<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetryPayload")
            .finish_non_exhaustive()
    }
}

impl<T> RetryPayload<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            value: std::sync::Arc::new(std::sync::Mutex::new(Some(value))),
        }
    }

    pub(crate) fn take(&self) -> Option<T> {
        self.value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}
