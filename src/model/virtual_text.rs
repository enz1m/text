//! Lazy virtual-text ownership and reducer integration for the chunked editor.
//!
//! The application's `TextStore` is the sole byte authority. Accessibility
//! reads use a persistent snapshot, while every mutation is admitted and
//! answered on Text's normal update thread.

use super::*;
use enzui_core::virtual_text::{
    CompositionId, MutationKind, MutationReply, MutationRequest, VirtualDocumentId,
    VirtualRevision, VirtualSelection as AccessibleSelection, VirtualTextBudget,
    VirtualTextCancellation, VirtualTextDocument, VirtualTextError,
};

// Document-coordinate selection for a viewport editor. `anchor_byte` is kept
// even when it is outside the native projection; `active_byte` is the caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct VirtualSelection {
    pub(super) anchor_byte: Option<usize>,
    pub(super) active_byte: usize,
}

// App-owned state for a dense editable document. The native editor contains
// only `materialized`; the complete authoritative text remains in `document`.
pub(super) struct VirtualEditorState {
    pub(super) cache: EditableTextViewportCache,
    pub(super) accessibility: std::sync::Arc<VirtualTextDocument>,
    pub(super) accessibility_selection: AccessibleSelection,
    pub(super) composition: Option<CompositionId>,
    pub(super) composition_ticket: Option<(
        CompositionId,
        enzui_core::virtual_text::MutationTicket,
        VirtualTextCancellation,
    )>,
    pub(super) materialized: std::ops::Range<usize>,
    pub(super) first_visible: usize,
    pub(super) selection: VirtualSelection,
    pub(super) projected: Option<text_editor::Cursor>,
    pub(super) undo: std::collections::VecDeque<VirtualHistoryEntry>,
    pub(super) redo: std::collections::VecDeque<VirtualHistoryEntry>,
    pub(super) undo_bytes: usize,
    // Laid-out editor offset an accessibility mutation must restore before the
    // next frame, because cosmic scrolls to its caret on layout.
    pub(super) scroll_restore: Option<iced::Vector>,
    pub(super) scroll_to_line: Option<(usize, f32)>,
}

impl VirtualEditorState {
    pub(super) fn with_policy(
        policy: EditableTextViewportPolicy,
        accessibility: std::sync::Arc<VirtualTextDocument>,
    ) -> Self {
        Self {
            cache: EditableTextViewportCache::new(policy),
            accessibility,
            accessibility_selection: AccessibleSelection::default(),
            composition: None,
            composition_ticket: None,
            materialized: 0..0,
            first_visible: 0,
            selection: VirtualSelection::default(),
            projected: None,
            undo: std::collections::VecDeque::new(),
            redo: std::collections::VecDeque::new(),
            undo_bytes: 0,
            scroll_restore: None,
            scroll_to_line: None,
        }
    }
}

impl Tab {
    fn virtual_document_id(&self) -> VirtualDocumentId {
        fn lane(bytes: &[u8], mut hash: u64) -> u64 {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
            hash
        }

        let bytes = self.id.as_bytes();
        VirtualDocumentId(
            (u128::from(lane(bytes, 0xcbf2_9ce4_8422_2325)) << 64)
                | u128::from(lane(bytes, 0x8422_2325_cbf2_9ce4)),
        )
    }

    fn accessible_selection(&self) -> AccessibleSelection {
        let selection = self
            .virtual_editor
            .as_ref()
            .map_or_else(VirtualSelection::default, |state| state.selection);
        let snapshot = self.document.current();
        AccessibleSelection {
            anchor: selection
                .anchor_byte
                .and_then(|byte| snapshot.character_of_byte(byte))
                .and_then(|value| u64::try_from(value).ok()),
            focus: snapshot
                .character_of_byte(selection.active_byte)
                .unwrap_or_else(|| snapshot.character_count()) as u64,
        }
    }

    fn selection_from_accessible(
        &self,
        selection: AccessibleSelection,
    ) -> Option<VirtualSelection> {
        let snapshot = self.document.current();
        let focus = usize::try_from(selection.focus).ok()?;
        let active_byte = snapshot.byte_of_character(focus)?;
        let anchor_byte = match selection.anchor {
            Some(anchor) => Some(snapshot.byte_of_character(usize::try_from(anchor).ok()?)?),
            None => None,
        };
        Some(VirtualSelection {
            anchor_byte,
            active_byte,
        })
    }

    pub(super) fn build_virtual_text_document(&self) -> std::sync::Arc<VirtualTextDocument> {
        let snapshot = self.document.snapshot();
        let mut store = TextStore::default();
        store.restore(snapshot);
        let document = std::sync::Arc::new(VirtualTextDocument::from_chunked(
            self.virtual_document_id(),
            VirtualRevision(self.snapshot_version),
            store,
            VirtualTextBudget::default(),
        ));
        let _ = document.confirm_selection(
            self.virtual_document_id(),
            VirtualRevision(self.snapshot_version),
            self.accessible_selection(),
        );
        document
    }

    pub(crate) fn virtual_text_document(&self) -> Option<std::sync::Arc<VirtualTextDocument>> {
        self.virtual_editor
            .as_ref()
            .map(|state| std::sync::Arc::clone(&state.accessibility))
    }

    // Publish reducer-owned bytes and selection without reconstructing the
    // provider. At most one document generation may cross this boundary.
    pub(crate) fn publish_virtual_text_state(&mut self) -> Result<(), VirtualTextError> {
        self.apply_virtual_scroll();
        // The frame after the mutation is what moved the editor.
        if let Some(offset) = self
            .virtual_editor
            .as_mut()
            .and_then(|state| state.scroll_restore.take())
            && self.content.scroll_metrics().offset != offset
        {
            self.content
                .perform(text_editor::Action::ScrollTo { offset });
        }
        let Some(provider) = self.virtual_text_document() else {
            return Ok(());
        };
        let summary = provider.summary()?;
        let document = self.virtual_document_id();
        let selection = self.accessible_selection();
        if summary.document != document {
            return Err(VirtualTextError::WrongDocument);
        }
        if summary.revision.0 == self.snapshot_version {
            provider.confirm_selection(document, summary.revision, selection)?;
        } else if summary.revision.0.checked_add(1) == Some(self.snapshot_version) {
            provider.confirm_mutation(
                document,
                summary.revision,
                VirtualRevision(self.snapshot_version),
                selection,
                self.document.snapshot(),
            )?;
        } else if summary.revision.0 < self.snapshot_version {
            // One reducer turn can advance the generation more than once (a
            // refused or drifted delta bumps twice). Refusing would pin the
            // provider on a dead generation forever, so resync forward.
            provider.resync_forward(
                document,
                summary.revision,
                VirtualRevision(self.snapshot_version),
                selection,
                self.document.snapshot(),
            )?;
        } else {
            // The provider is ahead; rewinding a published generation would
            // revalidate an AT's older revision.
            return Err(VirtualTextError::RevisionSequence {
                previous: summary.revision,
                proposed: VirtualRevision(self.snapshot_version),
            });
        }
        if let Some(state) = &mut self.virtual_editor {
            state.accessibility_selection = selection;
        }
        Ok(())
    }

    fn refuse(requested: VirtualRevision, current: VirtualRevision) -> MutationReply {
        MutationReply::Stale { requested, current }
    }

    // Projecting an off-window selection moves cosmic's caret, which the editor
    // follows on layout. Requests that ask to be revealed carry
    // `ViewportEffect::Reveal` and take the reveal route instead.
    fn preserving_editor_scroll<R>(&mut self, apply: impl FnOnce(&mut Self) -> R) -> R {
        let before = self.content.scroll_metrics().offset;
        let outcome = apply(self);
        if self.content.scroll_metrics().offset != before {
            self.content
                .perform(text_editor::Action::ScrollTo { offset: before });
        }
        // Caret-follow happens in the next layout, so
        // `publish_virtual_text_state` replays the restore on the turn after it.
        if let Some(state) = &mut self.virtual_editor {
            state.scroll_restore = Some(before);
        }
        outcome
    }

    pub(crate) fn apply_virtual_text_request(
        &mut self,
        provider: &std::sync::Arc<VirtualTextDocument>,
        request: &MutationRequest,
    ) -> MutationReply {
        self.preserving_editor_scroll(|tab| tab.apply_virtual_text_request_inner(provider, request))
    }

    fn apply_virtual_text_request_inner(
        &mut self,
        provider: &std::sync::Arc<VirtualTextDocument>,
        request: &MutationRequest,
    ) -> MutationReply {
        let current = VirtualRevision(self.snapshot_version);
        if request.document() != self.virtual_document_id() {
            return MutationReply::Refused;
        }
        if request.revision() != current {
            return Self::refuse(request.revision(), current);
        }
        if request.is_cancelled() {
            return MutationReply::Refused;
        }
        if self.is_read_only() && !matches!(request.mutation(), MutationKind::SetSelection(_)) {
            return MutationReply::Refused;
        }

        match request.mutation() {
            MutationKind::SetSelection(selection) => {
                let Some(selection_bytes) = self.selection_from_accessible(*selection) else {
                    return MutationReply::Refused;
                };
                if let Some(state) = &mut self.virtual_editor {
                    state.selection = selection_bytes;
                } else {
                    return MutationReply::Refused;
                }
                self.project_virtual_selection();
                if provider
                    .confirm_selection(request.document(), current, *selection)
                    .is_err()
                {
                    return MutationReply::Refused;
                }
                if let Some(state) = &mut self.virtual_editor {
                    state.accessibility_selection = *selection;
                }
                MutationReply::Accepted {
                    revision: current,
                    selection: *selection,
                }
            }
            MutationKind::Replace { range, value } => {
                self.apply_virtual_text_replacement(provider, request, range.clone(), value)
            }
            MutationKind::CommitComposition { id, range, value } => {
                let reply =
                    self.apply_virtual_text_replacement(provider, request, range.clone(), value);
                if matches!(reply, MutationReply::Accepted { .. })
                    && provider.finish_composition_commit(*id, reply).is_ok()
                    && let Some(state) = &mut self.virtual_editor
                {
                    state.composition = None;
                }
                reply
            }
        }
    }

    fn apply_virtual_text_replacement(
        &mut self,
        provider: &std::sync::Arc<VirtualTextDocument>,
        request: &MutationRequest,
        characters: std::ops::Range<u64>,
        value: &str,
    ) -> MutationReply {
        let Some(start_character) = usize::try_from(characters.start).ok() else {
            return MutationReply::Refused;
        };
        let Some(end_character) = usize::try_from(characters.end).ok() else {
            return MutationReply::Refused;
        };
        let Some(start) = self.document.current().byte_of_character(start_character) else {
            return MutationReply::Refused;
        };
        let Some(end) = self.document.current().byte_of_character(end_character) else {
            return MutationReply::Refused;
        };
        // Refuse a mutation whose successor version would overflow.
        if start > end || self.authority_exhausted() {
            return MutationReply::Refused;
        }
        if self.snapshot_version == u64::MAX {
            self.mark_authority_exhausted();
            return MutationReply::Refused;
        }

        let previous_document = self.document.snapshot();
        let previous_version = self.snapshot_version;
        let previous_selection = self
            .virtual_editor
            .as_ref()
            .map_or_else(VirtualSelection::default, |state| state.selection);
        let first_visible = self
            .virtual_editor
            .as_ref()
            .map_or(0, |state| state.first_visible);
        let first_line = self.document.current().line_column_of_byte(start).0;
        let checkpoint = VirtualHistoryEntry {
            document: previous_document.clone(),
            caret_byte: previous_selection.active_byte,
            cost: end.saturating_sub(start).saturating_add(value.len()).max(1),
        };

        if self.document.replace(start..end, value.to_owned()).is_err() {
            return MutationReply::Refused;
        }
        if self.overlong_line_after_edit(start, value.len()).is_some() {
            self.document.restore(previous_document);
            return MutationReply::Refused;
        }

        self.snapshot_version = previous_version + 1;
        self.line_count_cache.set(None);
        self.text_stats_cache.set(None);
        self.doc_stats_cache.set(None);
        self.language_cache.replace(None);
        self.stats_pending = true;
        let active_byte = start.saturating_add(value.len());
        if let Some(state) = &mut self.virtual_editor {
            state
                .cache
                .invalidate_from(self.snapshot_version, first_line);
            state.selection = VirtualSelection {
                anchor_byte: None,
                active_byte,
            };
        }
        if !matches!(
            self.materialize_editor_window(first_visible, None),
            FrameOutcome::Materialized
        ) {
            self.document.restore(previous_document);
            self.snapshot_version = previous_version;
            if let Some(state) = &mut self.virtual_editor {
                state.selection = previous_selection;
                state.cache.clear();
                state.materialized = 0..0;
            }
            let _ = self.materialize_editor_window(first_visible, None);
            return MutationReply::Refused;
        }

        let selection = self.accessible_selection();
        if provider
            .confirm_mutation(
                request.document(),
                request.revision(),
                VirtualRevision(self.snapshot_version),
                selection,
                self.document.snapshot(),
            )
            .is_err()
        {
            self.document.restore(previous_document);
            self.snapshot_version = previous_version;
            if let Some(state) = &mut self.virtual_editor {
                state.selection = previous_selection;
                state.cache.clear();
                state.materialized = 0..0;
            }
            let _ = self.materialize_editor_window(first_visible, None);
            return MutationReply::Refused;
        }
        self.push_virtual_undo(checkpoint);
        if let Some(state) = &mut self.virtual_editor {
            state.accessibility_selection = selection;
        }
        MutationReply::Accepted {
            revision: VirtualRevision(self.snapshot_version),
            selection,
        }
    }

    pub(crate) fn begin_virtual_composition(&mut self) -> Result<(), VirtualTextError> {
        let Some(provider) = self.virtual_text_document() else {
            return Ok(());
        };
        if self
            .virtual_editor
            .as_ref()
            .is_some_and(|state| state.composition.is_some())
        {
            return Ok(());
        }
        let summary = provider.summary()?;
        let range = summary
            .selection
            .range()
            .unwrap_or(summary.selection.focus..summary.selection.focus);
        let id = provider.begin_composition(
            summary.document,
            summary.revision,
            range,
            &VirtualTextCancellation::default(),
        )?;
        if let Some(state) = &mut self.virtual_editor {
            state.composition = Some(id);
        }
        Ok(())
    }

    pub(crate) fn update_virtual_composition(
        &mut self,
        preedit: &str,
    ) -> Result<(), VirtualTextError> {
        self.begin_virtual_composition()?;
        let Some(state) = &self.virtual_editor else {
            return Ok(());
        };
        let Some(id) = state.composition else {
            return Ok(());
        };
        state
            .accessibility
            .update_composition(id, preedit, &VirtualTextCancellation::default())
    }

    pub(crate) fn enqueue_virtual_composition_commit(&mut self) -> Result<(), VirtualTextError> {
        self.begin_virtual_composition()?;
        let Some(state) = &mut self.virtual_editor else {
            return Ok(());
        };
        let Some(id) = state.composition else {
            return Ok(());
        };
        let cancellation = VirtualTextCancellation::default();
        let ticket = state
            .accessibility
            .enqueue_composition_commit(id, &cancellation)?;
        state.composition_ticket = Some((id, ticket, cancellation));
        Ok(())
    }

    pub(crate) fn clear_finished_virtual_composition(&mut self) {
        let Some(state) = &mut self.virtual_editor else {
            return;
        };
        if state
            .composition_ticket
            .as_ref()
            .is_some_and(|(_, ticket, _)| ticket.try_recv().is_some())
        {
            state.composition_ticket = None;
        }
    }

    pub(crate) fn cancel_virtual_composition(&mut self) {
        let Some(state) = &mut self.virtual_editor else {
            return;
        };
        if let Some((_, _, cancellation)) = state.composition_ticket.take() {
            cancellation.cancel();
        }
        if let Some(id) = state.composition.take() {
            let _ = state.accessibility.cancel_composition(id);
        }
    }
}
