//! Workspace tab management.

use super::*;

impl Model {
    pub(crate) fn active_tab(&self) -> &Tab {
        self.tabs
            .iter()
            .find(|t| t.id == self.active_tab_id)
            .unwrap_or_else(|| self.tabs.first().expect("always at least one tab"))
    }

    pub(crate) fn active_tab_mut(&mut self) -> &mut Tab {
        let id = self.active_tab_id.clone();
        if let Some(idx) = self.tabs.iter().position(|t| t.id == id) {
            &mut self.tabs[idx]
        } else {
            &mut self.tabs[0]
        }
    }

    // Recompute literal or whole-word matches against the active tab without
    // materializing it, binding them to the tab's id and snapshot version.
    //
    // Regex searches belong on the blocking worker instead.
    pub(crate) fn recompute_search(&mut self) {
        let index = self.active_tab_index();
        let Self { tabs, search, .. } = self;
        if search.exhausted {
            return;
        }
        let binding = crate::search::MatchBinding {
            tab_id: tabs[index].id.clone(),
            snapshot_version: tabs[index].snapshot_version(),
        };
        search.recompute_snapshot(tabs[index].document());
        search.rebind(binding);
    }

    // [`Model::recompute_search`], seeded from a caret byte offset.
    pub(crate) fn recompute_search_starting_at(&mut self, caret: usize) {
        let index = self.active_tab_index();
        let Self { tabs, search, .. } = self;
        if search.exhausted {
            return;
        }
        let binding = crate::search::MatchBinding {
            tab_id: tabs[index].id.clone(),
            snapshot_version: tabs[index].snapshot_version(),
        };
        search.recompute_snapshot_starting_at(tabs[index].document(), caret);
        search.rebind(binding);
    }

    pub(crate) fn active_tab_index(&self) -> usize {
        self.tabs
            .iter()
            .position(|t| t.id == self.active_tab_id)
            .unwrap_or(0)
    }

    // Replace the settings (e.g. loaded from disk at boot).
    pub(crate) fn load_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    pub(crate) fn window_title(&self) -> String {
        let tab = self.active_tab();
        let name = crate::safe_text::bounded_inline(&tab.title());
        if tab.dirty {
            enzui_core::localized!("text-name-text-editor-0f86c0", "name" => name)
        } else {
            enzui_core::localized!("text-name-text-editor-5b87f2", "name" => name)
        }
    }

    pub(crate) fn fresh_tab_id(&mut self) -> String {
        let id = format!("tab-{}", self.next_tab_seq);
        self.next_tab_seq += 1;
        id
    }

    pub(crate) fn add_tab(&mut self, mut tab: Tab) -> String {
        if tab.id.is_empty() {
            tab.id = self.fresh_tab_id();
        }
        let id = tab.id.clone();
        self.tabs.push(tab);
        self.active_tab_id.clone_from(&id);
        id
    }

    pub(crate) fn open_paths<I, P>(&mut self, paths: I) -> Vec<PendingOpen>
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        // Only an untouched Untitled tab may be replaced.
        let mut first_slot_taken = self
            .tabs
            .iter()
            .any(|t| t.file_path.is_some() || !t.document().is_empty());
        // `add_tab` activates the newest tab; the first touched tab is
        // restored as active at the end.
        let mut first_id: Option<String> = None;
        let mut pending: Vec<PendingOpen> = Vec::new();
        for p in paths {
            let path: PathBuf = p.into();
            if !path.is_file() {
                continue;
            }
            // An exhausted token source skips the path rather than reuse a
            // wrapped token.
            let Some(token) = self.load_tokens.next() else {
                break;
            };
            let id = if !first_slot_taken {
                let t = self.active_tab_mut();
                t.file_path = Some(path.clone());
                t.loading = true;
                t.load_token = token;
                first_slot_taken = true;
                t.id.clone()
            } else {
                let id = self.fresh_tab_id();
                let mut tab = Tab::new(id.clone());
                tab.file_path = Some(path.clone());
                tab.loading = true;
                tab.load_token = token;
                self.add_tab(tab);
                id
            };
            if first_id.is_none() {
                first_id = Some(id.clone());
            }
            pending.push(PendingOpen {
                tab_id: id,
                path,
                workspace: None,
                token,
            });
        }
        if let Some(id) = first_id {
            self.active_tab_id = id;
        }
        pending
    }

    // Install a completed document load; the only place a tab's buffer mode is
    // decided.
    //
    // Returns the first bounded page read for a large document, or `None` for
    // an editable one. The caller checks the load token first.
    pub(crate) fn install_loaded_document(
        &mut self,
        tab_id: &str,
        path: &Path,
        contents: crate::large_doc::LoadedContents,
        visible_rows: usize,
    ) -> Result<
        Option<enzui_core::paged_text::PagedTextOperation>,
        crate::document_failure::DocumentFailure,
    > {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Ok(None);
        };
        match contents {
            crate::large_doc::LoadedContents::Editable {
                contents,
                detected_line_ending,
            } => {
                let effective = match detected_line_ending {
                    crate::line_ending::LineEnding::Mixed => crate::line_ending::LineEnding::Lf,
                    other => other,
                };
                let tab = &mut self.tabs[index];
                tab.buffer = crate::large_doc::DocumentBuffer::Editable;
                tab.install_buffer(contents);
                tab.file_path = Some(path.to_path_buf());
                tab.detected_line_ending = detected_line_ending;
                tab.line_ending = effective;
                tab.dirty = false;
                tab.externally_changed = false;
                Ok(None)
            }
            crate::large_doc::LoadedContents::EditableViewport {
                document,
                detected_line_ending,
            } => {
                let effective = match detected_line_ending {
                    crate::line_ending::LineEnding::Mixed => crate::line_ending::LineEnding::Lf,
                    other => other,
                };
                let tab = &mut self.tabs[index];
                tab.buffer = crate::large_doc::DocumentBuffer::Editable;
                tab.install_viewport_document(document);
                tab.file_path = Some(path.to_path_buf());
                tab.detected_line_ending = detected_line_ending;
                tab.line_ending = effective;
                tab.dirty = false;
                tab.externally_changed = false;
                Ok(None)
            }
            crate::large_doc::LoadedContents::LargeReadOnly {
                source,
                byte_len,
                reason,
            } => {
                let (doc, operation) = crate::large_doc::LargeDocument::open_because(
                    path.to_path_buf(),
                    source,
                    byte_len,
                    visible_rows,
                    reason,
                )
                .map_err(|detail| {
                    crate::document_failure::DocumentFailure::from_detail(path, detail)
                })?;
                let tab = &mut self.tabs[index];
                // Discarded, not truncated, so no partial text can be mistaken
                // for the complete document.
                tab.install_buffer(String::new());
                tab.buffer = crate::large_doc::DocumentBuffer::LargeReadOnly(Box::new(doc));
                tab.file_path = Some(path.to_path_buf());
                tab.dirty = false;
                tab.externally_changed = false;
                tab.language_override = None;
                tab.detected_line_ending = crate::line_ending::LineEnding::Lf;
                tab.line_ending = crate::line_ending::LineEnding::Lf;
                Ok(Some(operation))
            }
        }
    }

    // Restore the persisted global tab set with each tab's workspace
    // association, re-selecting `active_path`.
    //
    // Buffer mode is not persisted; the returned loads reclassify each file.
    pub(crate) fn restore_persisted_tabs(
        &mut self,
        tabs: &[crate::workspace::PersistedTab],
        active_path: Option<&Path>,
    ) -> Vec<PendingOpen> {
        // Session paths identify documents, so repeated entries and a path
        // already represented by crash recovery must not create another tab.
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<&crate::workspace::PersistedTab> = tabs
            .iter()
            .filter(|tab| seen.insert(tab.path.clone()))
            .collect();
        let paths: Vec<PathBuf> = unique
            .iter()
            .filter(|persisted| {
                !self
                    .tabs
                    .iter()
                    .any(|tab| tab.file_path.as_ref() == Some(&persisted.path))
            })
            .map(|persisted| persisted.path.clone())
            .collect();
        let mut pending = self.open_paths(paths.iter().cloned());
        for persisted in unique {
            if let Some(tab) = self
                .tabs
                .iter_mut()
                .find(|tab| tab.file_path.as_deref() == Some(persisted.path.as_path()))
            {
                tab.workspace_root.clone_from(&persisted.workspace);
            }
            for open in &mut pending {
                if open.path == persisted.path {
                    open.workspace.clone_from(&persisted.workspace);
                }
            }
        }
        if let Some(active_path) = active_path
            && let Some(tab) = self
                .tabs
                .iter()
                .find(|tab| tab.file_path.as_deref() == Some(active_path))
        {
            self.active_tab_id = tab.id.clone();
        }
        self.sync_doc_tabs();
        pending
    }

    // Whether the window is narrow enough to fold titlebar actions into an
    // overflow menu; `false` until the first `Resized`.
    pub(crate) fn is_narrow(&self) -> bool {
        self.window_size.is_some_and(|(w, _)| w <= 640.0)
    }

    pub(crate) fn request_close(&mut self, id: &str) -> bool {
        let dirty = self
            .tabs
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| t.dirty);
        if dirty {
            self.pending_close = Some(id.to_owned());
            false
        } else {
            self.close_tab(id);
            true
        }
    }

    pub(crate) fn close_tab(&mut self, id: &str) {
        let Some(idx) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let was_active = self.active_tab_id == id;
        // A fresh token makes late loads for the closing tab no-ops. When
        // minting is exhausted the current token is kept; a reset value could
        // let a stale completion match a future tab.
        {
            if let Some(token) = self.load_tokens.next() {
                let tab = &mut self.tabs[idx];
                tab.load_token = token;
            }
            let tab = &mut self.tabs[idx];
            if let Some(doc) = tab.buffer.large_mut() {
                doc.close();
            }
        }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            let new_id = self.fresh_tab_id();
            self.tabs.push(Tab::new(new_id.clone()));
            self.active_tab_id = new_id;
            return;
        }
        if was_active {
            let new_idx = if idx == 0 { 0 } else { idx - 1 };
            self.active_tab_id = self.tabs[new_idx].id.clone();
        }
    }

    // The stable [`DocumentKey`] for a tab id; keys are never derived from a
    // position.
    pub(crate) fn doc_key(id: &str) -> DocumentKey {
        DocumentKey::new(id)
    }

    pub(crate) fn sync_doc_tabs(&mut self) {
        let items: Vec<DocumentItem> = self
            .tabs
            .iter()
            .map(|tab| {
                DocumentItem::new(
                    Self::doc_key(&tab.id),
                    crate::safe_text::bounded_inline(&tab.title()),
                )
            })
            .collect();
        // Cannot fail: `reconcile` errors only on duplicate keys, and tab ids
        // are never reused.
        let _ = self.doc_tabs.reconcile(items);
        self.doc_tabs.set_orientation(Orientation::Horizontal);
        let active = Self::doc_key(&self.active_tab_id);
        self.doc_tabs.select(&active);
    }
}
