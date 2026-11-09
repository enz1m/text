//! TextApp session handoff types.

use super::*;

type SessionWriteOutcome = (session::SessionStore, bool, session::SessionSnapshot, u64);

#[derive(Clone)]
pub(crate) struct SessionWriteHandoff {
    // The u64 is the projection revision at dispatch, so settlement stamps a
    // `PublishOverBound` refusal with it rather than the newer live revision.
    pub(crate) state: std::sync::Arc<std::sync::Mutex<Option<SessionWriteOutcome>>>,
}

impl std::fmt::Debug for SessionWriteHandoff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionWriteHandoff")
            .finish_non_exhaustive()
    }
}

impl SessionWriteHandoff {
    pub(crate) fn pending(
        store: session::SessionStore,
        snapshot: session::SessionSnapshot,
        dispatch_revision: u64,
    ) -> Self {
        Self {
            state: std::sync::Arc::new(std::sync::Mutex::new(Some((
                store,
                false,
                snapshot,
                dispatch_revision,
            )))),
        }
    }

    pub(crate) fn execute(&self, operation: impl FnOnce(&mut session::SessionStore) -> bool) {
        let Some((mut store, _, snapshot, dispatch_revision)) = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&mut store)));
        let persisted = match &outcome {
            Ok(persisted) => *persisted,
            Err(_) => false,
        };
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((store, persisted, snapshot, dispatch_revision));
        if let Err(payload) = outcome {
            std::panic::resume_unwind(payload);
        }
    }

    pub(crate) fn take(self) -> Option<SessionWriteOutcome> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

// Bounded typed debt for an over-bound refusal: an exact session-projection
// revision and a bounded summary. Retains no tab or workspace content.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OverBoundReducerDebt {
    // Equal to the current projection revision iff no projection-relevant
    // mutation has advanced the counter since the refusal.
    pub(crate) revision_at_refusal: u64,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the refusal's detail, which tests inspect")
    )]
    pub(crate) summary: session::PublishOverBoundSummary,
}
