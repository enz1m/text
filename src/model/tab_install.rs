//! Tab buffer installation.

use super::*;

impl Tab {
    // Install `text` as both the buffer and the snapshot, invalidating derived
    // caches.
    //
    // Used by the async open path and every whole-buffer writer. Reserves a
    // version first, so an exhausted counter returns before `virtual_editor`,
    // `content` or the document change.
    pub(crate) fn install_buffer(&mut self, text: String) {
        // One reservation for the whole install, taken before the `Content` swap:
        // at `snapshot_version == MAX` the tab is not latched yet, and a nested
        // refusal would leave `Content` ahead of the document.
        let Ok(reserved) = self.reserve_snapshot_version() else {
            return;
        };
        self.virtual_editor = None;
        self.content = text_editor::Content::with_text(&text);
        // Small documents are counted here so the first view is exact; large
        // ones are counted off-thread (`Message::StatsCompleted`).
        let eager = text.len() <= Self::EAGER_STATS_BYTES;
        // Inline swap: `set_snapshot` would reserve a second time.
        self.document = TextStore::from_string(text);
        self.commit_snapshot_version(reserved);
        self.stats_pending = !eager;
        if eager {
            let stats = crate::stats::text_stats_snapshot(self.document.current());
            let document = crate::doc_stats::document_stats_snapshot(self.document.current());
            let version = self.snapshot_version;
            self.text_stats_cache.set(Some((version, stats)));
            self.doc_stats_cache.set(Some((version, document)));
        }
        self.loading = false;
    }

    // Install a persistent document prepared by the background open worker.
    //
    // Performs no document-sized scan or allocation in the reducer; only the
    // bounded visible+overscan projection is built.
    pub(crate) fn install_viewport_document(&mut self, document: TextStore) {
        self.install_viewport_document_with_policy(document, EditableTextViewportPolicy::default());
    }

    pub(crate) fn install_viewport_document_with_policy(
        &mut self,
        document: TextStore,
        policy: EditableTextViewportPolicy,
    ) {
        // Reservation first: a refusal leaves the tab exactly as on entry. A
        // reload must mint a fresh tab identity via a new `LoadToken`.
        let Ok(reserved) = self.reserve_snapshot_version() else {
            return;
        };
        let eager = document.current().len() <= Self::EAGER_STATS_BYTES;
        self.virtual_editor = None;
        self.document = document;
        self.commit_snapshot_version(reserved);
        let accessibility = self.build_virtual_text_document();
        self.virtual_editor = Some(VirtualEditorState::with_policy(policy, accessibility));
        let outcome = self.materialize_editor_window(0, Some(0));
        debug_assert_eq!(
            outcome,
            FrameOutcome::Materialized,
            "the admitted editor frame must materialize"
        );
        self.stats_pending = !eager;
        if eager {
            let stats = crate::stats::text_stats_snapshot(self.document.current());
            let document = crate::doc_stats::document_stats_snapshot(self.document.current());
            let version = self.snapshot_version;
            self.text_stats_cache.set(Some((version, stats)));
            self.doc_stats_cache.set(Some((version, document)));
        }
        self.loading = false;
    }
}

#[cfg(test)]
impl Tab {
    pub(crate) fn install_viewport_buffer_with_policy(
        &mut self,
        text: String,
        policy: EditableTextViewportPolicy,
    ) {
        self.install_viewport_document_with_policy(TextStore::from_string(text), policy);
    }

    pub(crate) fn install_viewport_buffer(&mut self, text: String) {
        self.install_viewport_buffer_with_policy(text, EditableTextViewportPolicy::default());
    }
}
