//! Tab construction and versions.

use super::*;

impl Tab {
    pub(crate) fn new(id: String) -> Self {
        Self {
            id,
            content: text_editor::Content::new(),
            document: TextStore::default(),
            virtual_editor: None,
            dirty: false,
            preview: crate::markdown_preview::MarkdownPreview::default(),
            file_path: None,
            workspace_root: None,
            line_ending: LineEnding::Lf,
            detected_line_ending: LineEnding::Lf,
            language_override: None,
            snapshot_version: 0,
            stats_pending: false,
            stats_in_flight: false,
            saving: false,
            save_queued: None,
            line_count_cache: std::cell::Cell::new(None),
            text_stats_cache: std::cell::Cell::new(None),
            doc_stats_cache: std::cell::Cell::new(None),
            cursor_epoch: 0,
            selection_stats_landed: None,
            selection_stats_in_flight: false,
            language_cache: std::cell::RefCell::new(None),
            watcher: None,
            externally_changed: false,
            loading: false,
            decoded_lossily: false,
            opened_from: None,
            buffer: crate::large_doc::DocumentBuffer::Editable,
            load_token: crate::large_doc::LoadToken::default(),
            pending_scroll: None,
            authority_exhausted: false,
        }
    }

    pub(crate) fn with_text(id: String, text: &str) -> Self {
        Self {
            id,
            content: text_editor::Content::with_text(text),
            document: TextStore::from_string(text.to_owned()),
            virtual_editor: None,
            dirty: false,
            preview: crate::markdown_preview::MarkdownPreview::default(),
            file_path: None,
            workspace_root: None,
            line_ending: LineEnding::Lf,
            detected_line_ending: LineEnding::Lf,
            language_override: None,
            snapshot_version: 0,
            stats_pending: false,
            stats_in_flight: false,
            saving: false,
            save_queued: None,
            line_count_cache: std::cell::Cell::new(None),
            text_stats_cache: std::cell::Cell::new(None),
            doc_stats_cache: std::cell::Cell::new(None),
            cursor_epoch: 0,
            selection_stats_landed: None,
            selection_stats_in_flight: false,
            language_cache: std::cell::RefCell::new(None),
            watcher: None,
            externally_changed: false,
            loading: false,
            decoded_lossily: false,
            opened_from: None,
            buffer: crate::large_doc::DocumentBuffer::Editable,
            load_token: crate::large_doc::LoadToken::default(),
            pending_scroll: None,
            authority_exhausted: false,
        }
    }

    // Whether this tab refuses edits and writes (large read-only mode); every
    // guard consults this predicate.
    pub(crate) fn is_read_only(&self) -> bool {
        self.buffer.is_read_only() || self.decoded_lossily
    }

    // Whether this tab's buffer mode supports an editor capability; the view
    // disables exactly what this refuses.
    pub(crate) fn supports(&self, capability: crate::large_doc::Capability) -> bool {
        use crate::large_doc::Capability;
        let writes = matches!(
            capability,
            Capability::Edit | Capability::Save | Capability::Replace | Capability::LineEnding
        );
        !(self.decoded_lossily && writes) && self.buffer.supports(capability)
    }

    // Install a file watcher for this tab's path using the given backend.
    // Call on first open and whenever `file_path` changes (Save As).
    pub(crate) fn install_watcher(&mut self, backend: std::sync::Arc<dyn FileWatchBackend>) {
        if let Some(path) = &self.file_path {
            self.watcher = Some(FileWatcher::from_arc(backend, path.clone()));
            self.externally_changed = false;
        }
    }

    pub(crate) fn title(&self) -> String {
        self.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(
                || enzui_core::localization::text("text-untitled-f59ab8"),
                |n| n.to_string_lossy().to_string(),
            )
    }

    pub(crate) fn language(&self) -> Option<&'static str> {
        crate::stats::detect_language(self.file_path.as_deref())
    }

    pub(crate) fn syntax_language(&self) -> enzui_widgets::syntax::Language {
        // Resolved every frame, so cached against the buffer version.
        if let Some((version, override_label, language)) = self.language_cache.borrow().as_ref()
            && *version == self.snapshot_version
            && override_label.as_deref() == self.language_override.as_deref()
        {
            return *language;
        }
        let ovr = match self.language_override.as_deref() {
            Some("Auto") | None => None,
            other => other,
        };
        // A file that is not text is shown, never coloured as code, unless
        // the user picks a language for it.
        if self.decoded_lossily && ovr.is_none() {
            return enzui_widgets::syntax::Language::Plain;
        }
        // Only the sniff prefix is materialized.
        let snapshot = self.document.current();
        let sniff = snapshot
            .slice_to_string(0..snapshot.len().min(64 * 1024))
            .unwrap_or_default();
        let language =
            enzui_widgets::syntax::Language::resolve(self.file_path.as_deref(), &sniff, ovr);
        self.language_cache.replace(Some((
            self.snapshot_version,
            self.language_override.clone(),
            language,
        )));
        language
    }

    pub(crate) fn display_language(&self) -> Option<String> {
        if let Some(ovr) = self.language_override.as_deref()
            && ovr != "Auto"
        {
            return Some(ovr.to_owned());
        }
        // A known extension names the file; otherwise the label is whatever
        // the colours come from, so the two never disagree.
        enzui_widgets::syntax::Language::display_label(self.file_path.as_deref())
            .map(std::borrow::ToOwned::to_owned)
            .or_else(|| self.language().map(std::borrow::ToOwned::to_owned))
            .or_else(|| match self.syntax_language() {
                enzui_widgets::syntax::Language::Plain => None,
                detected => Some(detected.label().to_owned()),
            })
    }

    // Select a line ending. Re-selecting the active value is a no-op: a radio
    // button must not rewrite the document or manufacture unsaved changes.
    pub(crate) fn set_line_ending(&mut self, ending: LineEnding) {
        if self.line_ending != ending {
            self.line_ending = ending;
            self.dirty = true;
        }
    }

    // Toggle the line ending between LF and CRLF, marking the tab dirty.
    //
    // `Cr` cycles to LF, since the settings surface exposes only LF and CRLF.
    pub(crate) fn cycle_line_ending(&mut self) {
        let next = match self.line_ending {
            LineEnding::Lf | LineEnding::Mixed => LineEnding::Crlf,
            LineEnding::Crlf | LineEnding::Cr => LineEnding::Lf,
        };
        self.set_line_ending(next);
    }

    pub(crate) const EAGER_STATS_BYTES: usize = 1 << 20;

    pub(crate) fn snapshot_version(&self) -> u64 {
        self.snapshot_version
    }

    // Whether this tab's `snapshot_version` or `cursor_epoch` counter has
    // reached its ceiling. Once true, every path that would mint a version
    // (edit, install, write) refuses the action rather than reuse a generation
    // with different bytes; existing document, selection and save state stay.
    #[must_use]
    pub(crate) fn authority_exhausted(&self) -> bool {
        self.authority_exhausted
    }

    pub(crate) fn mark_authority_exhausted(&mut self) {
        self.authority_exhausted = true;
    }

    // Restore the native `Content` from the authoritative document.
    //
    // iced's `text_editor` mutates its `Content` before the `EditDelta` reaches
    // the reducer, so a refused delta leaves the UI showing bytes the document
    // never saw and a later Save would write different bytes. Call on refusal
    // paths only; cost is linear in document length. Preserves the dirty flag
    // and file path.
    pub(crate) fn restore_content_from_document(&mut self) {
        let text = self.document.current().to_string();
        self.content = text_editor::Content::with_text(&text);
        if self.virtual_editor.is_some() {
            let (first, caret) = {
                let state = self.virtual_editor.as_mut().expect("just checked");
                state.cache.clear();
                state.materialized = 0..0;
                (state.first_visible, state.selection.active_byte)
            };
            let _ = self.materialize_editor_window(first, Some(caret));
        }
    }

    // Reserve the successor snapshot version without mutating anything.
    //
    // Mutation routes reserve, write only on success, then
    // [`commit`](Self::commit_snapshot_version) or drop the reservation. At the
    // ceiling, or once [`Self::authority_exhausted`] is latched, this returns
    // [`AuthorityExhausted`] and latches the tab, leaving `snapshot_version`
    // unchanged so the caller aborts before moving a byte.
    pub(crate) fn reserve_snapshot_version(&mut self) -> Result<u64, AuthorityExhausted> {
        if self.authority_exhausted {
            return Err(AuthorityExhausted);
        }
        match self.snapshot_version.checked_add(1) {
            Some(next) => Ok(next),
            None => {
                self.authority_exhausted = true;
                Err(AuthorityExhausted)
            }
        }
    }

    // Publish a reserved snapshot version and invalidate every derived cache,
    // so a stale worker completion cannot answer for the new bytes.
    //
    // Call only after the mutation succeeded. `reserved` is written verbatim;
    // pair each reserve with exactly one commit or drop.
    pub(crate) fn commit_snapshot_version(&mut self, reserved: u64) {
        self.snapshot_version = reserved;
        self.line_count_cache.set(None);
        self.text_stats_cache.set(None);
        self.doc_stats_cache.set(None);
        self.language_cache.replace(None);
    }
}
