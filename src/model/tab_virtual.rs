//! Tab virtual editor selection.

use super::*;

impl Tab {
    pub(crate) fn sync_virtual_selection_from_native(&mut self) {
        let Some(state) = &self.virtual_editor else {
            return;
        };
        let origin = state.materialized.start;
        let cursor = self.content.cursor();
        // An off-frame anchor is clamped to the frame edge for paint. If the
        // native cursor still equals the projection, cosmic reported nothing and
        // reading it back would replace the global selection with the clamp.
        if state.projected == Some(cursor) {
            return;
        }
        let position_byte = |position: text_editor::Position| {
            self.document
                .current()
                .byte_of_line_column(position.line.saturating_add(origin), position.index)
        };
        let selection = if let (Some(anchor), Some(range)) =
            (cursor.selection, self.content.selection_range())
        {
            let caret_before_anchor =
                (cursor.position.line, cursor.position.index) <= (anchor.line, anchor.index);
            let (active, anchor) = if caret_before_anchor {
                (range.start, range.end)
            } else {
                (range.end, range.start)
            };
            VirtualSelection {
                anchor_byte: Some(position_byte(anchor)),
                active_byte: position_byte(active),
            }
        } else {
            VirtualSelection {
                anchor_byte: None,
                active_byte: position_byte(cursor.position),
            }
        };
        let Some(state) = self.virtual_editor.as_mut() else {
            return;
        };
        state.selection = selection;
        state.projected = Some(cursor);
    }

    pub(crate) fn project_virtual_selection(&mut self) {
        let Some(state) = &self.virtual_editor else {
            return;
        };
        if state.materialized.is_empty() {
            return;
        }
        let materialized = state.materialized.clone();
        let global = state.selection;
        let first_byte = self
            .document
            .current()
            .line_start(materialized.start)
            .unwrap_or(0);
        let last_line = materialized.end.saturating_sub(1);
        let last_byte = self
            .document
            .current()
            .line_end(last_line)
            .unwrap_or_else(|| self.document.current().len());
        let local_position = |byte: usize| {
            let byte = byte.clamp(first_byte, last_byte);
            let (line, column) = self.document.current().line_column_of_byte(byte);
            text_editor::Position {
                line: line.saturating_sub(materialized.start),
                index: column,
            }
        };
        self.content.move_to(text_editor::Cursor {
            position: local_position(global.active_byte),
            selection: global.anchor_byte.map(local_position),
        });
        let settled = self.content.cursor();
        self.virtual_editor
            .as_mut()
            .expect("checked above")
            .projected = Some(settled);
    }

    #[must_use]
    pub(crate) fn editor_scroll_metrics(&self) -> text_editor::ScrollMetrics {
        let metrics = self.content.scroll_metrics();
        self.editor_scroll_projection()
            .and_then(|projection| projection.apply(metrics))
            .unwrap_or(metrics)
    }

    pub(crate) fn editor_scroll_projection(
        &self,
    ) -> Option<iced::advanced::text::editor::scrollbar::Projection> {
        self.virtual_editor.as_ref().and_then(|state| {
            iced::advanced::text::editor::scrollbar::Projection::new(
                state.materialized.start,
                self.document.current().line_count(),
                self.content.line_count().max(1),
            )
        })
    }

    // Never panics. A window the bounded cache cannot represent leaves the
    // previous projection and the document unchanged and returns
    // [`FrameOutcome::Refused`].
    pub(crate) fn materialize_editor_window(
        &mut self,
        first_visible: usize,
        caret_byte: Option<usize>,
    ) -> FrameOutcome {
        let Some(state) = &mut self.virtual_editor else {
            return FrameOutcome::Materialized;
        };
        let mut visible_lines = EDITABLE_VIEWPORT_LINES;
        let frame: EditableTextFrame = loop {
            match state.cache.materialize(
                self.document.current(),
                self.snapshot_version,
                first_visible,
                visible_lines,
            ) {
                Ok(frame) => break frame,
                Err(MaterializeError::ByteBudget { .. } | MaterializeError::LineBudget { .. })
                    if visible_lines > 1 =>
                {
                    visible_lines = visible_lines.div_ceil(2);
                }
                Err(MaterializeError::ByteBudget { .. } | MaterializeError::LineBudget { .. })
                    if state.cache.policy().overscan_lines > 0 =>
                {
                    let mut policy = state.cache.policy();
                    policy.overscan_lines = 0;
                    state.cache = EditableTextViewportCache::new(policy);
                }
                Err(MaterializeError::LineTooLarge { line, bytes, limit }) => {
                    return FrameOutcome::Refused(EditorFrameRefusal::LineTooLarge {
                        line,
                        bytes,
                        limit,
                    });
                }
                Err(_) => return FrameOutcome::Refused(EditorFrameRefusal::Budget),
            }
        };
        state.first_visible = frame.materialized.start.saturating_add(frame.visible.start);
        state.materialized = frame.materialized.clone();
        if let Some(byte) = caret_byte {
            state.selection = VirtualSelection {
                anchor_byte: None,
                active_byte: self.document.current().floor_char_boundary(byte),
            };
        }
        self.content = text_editor::Content::with_text(&frame.editor_text());
        // A brand-new native buffer has no projection yet; anything recorded
        // for the previous frame describes a different buffer.
        if let Some(state) = &mut self.virtual_editor {
            state.projected = None;
        }
        self.project_virtual_selection();
        FrameOutcome::Materialized
    }

    // Rebuild the native projection from the authoritative document after a
    // refused edit, clamping a global caret that the rollback shortened past.
    // The document is the source of truth; the native buffer is a view of it.
    pub(crate) fn restore_editor_projection(&mut self) -> FrameOutcome {
        let Some(state) = &mut self.virtual_editor else {
            return FrameOutcome::Materialized;
        };
        let length = self.document.current().len();
        state.selection = VirtualSelection {
            anchor_byte: None,
            active_byte: self.document.current().floor_char_boundary(
                state
                    .selection
                    .active_byte
                    .min(state.selection.anchor_byte.unwrap_or(usize::MAX))
                    .min(length),
            ),
        };
        let first = state.first_visible;
        let caret = state.selection.active_byte;
        state.materialized = 0..0;
        state.cache.clear();
        self.materialize_editor_window(first, Some(caret))
    }

    // Synchronize a click, drag, or native in-frame motion back into the
    // document-coordinate cursor.
    //
    // Must run after every native action in viewport mode, or a later
    // recenter restores a stale caret.
    pub(crate) fn sync_virtual_selection(&mut self) {
        self.sync_virtual_selection_from_native();
    }

    pub(crate) fn virtual_selection_range(&self) -> Option<std::ops::Range<usize>> {
        let state = self.virtual_editor.as_ref()?;
        let anchor = state.selection.anchor_byte?;
        let active = state.selection.active_byte;
        (anchor != active).then(|| anchor.min(active)..anchor.max(active))
    }

    // Materialize the selected bytes from the authoritative store. This is a
    // user-requested clipboard boundary, never a frame-time flatten.
    pub(crate) fn virtual_selected_text(&self) -> Option<String> {
        self.document
            .current()
            .slice_to_string(self.virtual_selection_range()?)
    }

    // Refusal is atomic: the write is checked and rolled back in O(1) before
    // any other state moves, leaving version, caches, selection, projection and
    // history untouched.
    pub(crate) fn replace_virtual_selection(&mut self, replacement: &str) -> VirtualWriteOutcome {
        let Some(range) = self.virtual_selection_range() else {
            return VirtualWriteOutcome::Unchanged;
        };
        // Reservation first: exhaustion refuses before any document, history or
        // selection change. The rollback below covers only over-long lines.
        let Ok(reserved) = self.reserve_snapshot_version() else {
            return VirtualWriteOutcome::AuthorityExhausted;
        };
        let first_line = self.document.current().line_column_of_byte(range.start).0;
        let checkpoint = VirtualHistoryEntry {
            document: self.document.snapshot(),
            caret_byte: range.end,
            cost: range.len().saturating_add(replacement.len()).max(1),
        };
        // The pre-write generation, taken in O(1). Restoring it shares every
        // unchanged piece, so a refusal costs neither bytes nor a copy.
        let rollback = self.document.snapshot();
        if self
            .document
            .replace(range.clone(), replacement.to_owned())
            .is_err()
        {
            // Dropping `reserved` undoes the reservation; nothing was advanced.
            return VirtualWriteOutcome::Unchanged;
        }
        // Checked before commit, history push and recenter, so the rollback is
        // the whole undo.
        if let Some((line, bytes, limit)) =
            self.overlong_line_after_edit(range.start, replacement.len())
        {
            self.document.restore(rollback);
            return VirtualWriteOutcome::RefusedOverlongLine { line, bytes, limit };
        }
        self.commit_snapshot_version(reserved);
        self.stats_pending = true;
        let active = range.start.saturating_add(replacement.len());
        if let Some(state) = &mut self.virtual_editor {
            state
                .cache
                .invalidate_from(self.snapshot_version, first_line);
            state.selection = VirtualSelection {
                anchor_byte: None,
                active_byte: active,
            };
        }
        self.push_virtual_undo(checkpoint);
        self.materialize_editor_window(
            first_line.saturating_sub(EDITABLE_VIEWPORT_LINES / 2),
            None,
        );
        VirtualWriteOutcome::Applied
    }

    // Cut the complete global selection and return the bytes destined for the
    // clipboard. The mutation and returned text share one authoritative range.
    //
    // A cut across lines joins the first line's prefix to the last line's
    // suffix, so it can exceed the paragraph limit exactly as a replacement
    // can; a refused cut copies nothing.
    pub(crate) fn cut_virtual_selection(&mut self) -> VirtualCutOutcome {
        let Some(copied) = self.virtual_selected_text() else {
            return VirtualCutOutcome::Nothing;
        };
        match self.replace_virtual_selection("") {
            VirtualWriteOutcome::Applied => VirtualCutOutcome::Cut(copied),
            VirtualWriteOutcome::Unchanged => VirtualCutOutcome::Nothing,
            VirtualWriteOutcome::RefusedOverlongLine { line, bytes, limit } => {
                VirtualCutOutcome::RefusedOverlongLine { line, bytes, limit }
            }
            // A cut is not line-oriented, so the indent bound cannot refuse it.
            VirtualWriteOutcome::RefusedIndentScope { .. } => VirtualCutOutcome::Nothing,
            // A refused reservation copies and mutates nothing, so cut never
            // degrades to copy.
            VirtualWriteOutcome::AuthorityExhausted => VirtualCutOutcome::Nothing,
        }
    }

    pub(crate) fn perform_virtual_selection_edit(
        &mut self,
        action: &text_editor::Action,
    ) -> Option<VirtualWriteOutcome> {
        self.virtual_selection_range()?;
        let replacement = match action {
            text_editor::Action::Edit(text_editor::Edit::Insert(character)) => {
                character.to_string()
            }
            text_editor::Action::Edit(text_editor::Edit::Paste(text)) => text.as_str().to_owned(),
            text_editor::Action::Edit(text_editor::Edit::Enter) => "\n".to_owned(),
            text_editor::Action::Edit(text_editor::Edit::Backspace | text_editor::Edit::Delete) => {
                String::new()
            }
            // Performed on the authoritative store, one bounded region at a time,
            // never on the visible slice.
            text_editor::Action::Edit(
                edit @ (text_editor::Edit::Indent | text_editor::Edit::Unindent),
            ) => {
                let indent = matches!(edit, text_editor::Edit::Indent);
                return Some(self.indent_virtual_selection(indent));
            }
            _ => return None,
        };
        Some(self.replace_virtual_selection(&replacement))
    }

    // Runs on the authoritative store as one persistent replacement and
    // remaps anchor and caret so the same text stays selected; stop width and
    // per-line rules match cosmic-text. A selection over more than
    // [`EDITABLE_INDENT_LINES`] lines or [`EDITABLE_INDENT_BYTES`] bytes is
    // refused visibly.
    pub(crate) fn indent_virtual_selection(&mut self, indent: bool) -> VirtualWriteOutcome {
        let Some(range) = self.virtual_selection_range() else {
            return VirtualWriteOutcome::Unchanged;
        };
        // Reservation first: refused before any O(lines) work or mutation.
        let Ok(reserved) = self.reserve_snapshot_version() else {
            return VirtualWriteOutcome::AuthorityExhausted;
        };
        let (selection, first_line, last_line, region) = {
            let snapshot = self.document.current();
            let first_line = snapshot.line_column_of_byte(range.start).0;
            let last_line = snapshot.line_column_of_byte(range.end).0;
            let start = snapshot.line_start(first_line).unwrap_or(0);
            let end = snapshot
                .line_end(last_line)
                .unwrap_or_else(|| snapshot.len());
            (
                self.virtual_editor
                    .as_ref()
                    .map(|state| state.selection)
                    .unwrap_or_default(),
                first_line,
                last_line,
                start..end,
            )
        };
        let lines = last_line.saturating_sub(first_line).saturating_add(1);
        let bytes = region.len();
        if lines > EDITABLE_INDENT_LINES || bytes > EDITABLE_INDENT_BYTES {
            return VirtualWriteOutcome::RefusedIndentScope {
                lines,
                bytes,
                line_limit: EDITABLE_INDENT_LINES,
                byte_limit: EDITABLE_INDENT_BYTES,
            };
        }
        let Some(source) = self.document.current().slice_to_string(region.clone()) else {
            return VirtualWriteOutcome::Unchanged;
        };

        let mut rewritten = String::with_capacity(bytes.saturating_add(lines * EDITABLE_TAB_WIDTH));
        let mut edits = Vec::with_capacity(lines);
        let mut line_start = region.start;
        for (position, line) in source.split('\n').enumerate() {
            if position > 0 {
                rewritten.push('\n');
            }
            let (cut, insert) = indent_edit(line, indent);
            let removed = line_start.saturating_add(cut.start)..line_start.saturating_add(cut.end);
            edits.push((removed, insert));
            let (Some(prefix), Some(suffix)) = (line.get(..cut.start), line.get(cut.end..)) else {
                return VirtualWriteOutcome::Unchanged;
            };
            rewritten.push_str(prefix);
            for _ in 0..insert {
                rewritten.push(' ');
            }
            rewritten.push_str(suffix);
            line_start = line_start.saturating_add(line.len()).saturating_add(1);
        }
        // Both endpoints map from the same pre-edit coordinates; mutating one
        // per line would skip cumulative Unindent deltas.
        let anchor = selection
            .anchor_byte
            .map(|byte| map_line_edit_endpoint(byte, &edits));
        let active = map_line_edit_endpoint(selection.active_byte, &edits);
        if rewritten == source {
            return VirtualWriteOutcome::Unchanged;
        }

        let checkpoint = VirtualHistoryEntry {
            document: self.document.snapshot(),
            caret_byte: selection.active_byte,
            cost: bytes.saturating_add(rewritten.len()).max(1),
        };
        let rollback = self.document.snapshot();
        let written = rewritten.len();
        if self.document.replace(region.clone(), rewritten).is_err() {
            // Reservation dropped without commit.
            return VirtualWriteOutcome::Unchanged;
        }
        if let Some((line, line_bytes, limit)) =
            self.overlong_line_after_edit(region.start, written)
        {
            self.document.restore(rollback);
            return VirtualWriteOutcome::RefusedOverlongLine {
                line,
                bytes: line_bytes,
                limit,
            };
        }
        self.commit_snapshot_version(reserved);
        self.stats_pending = true;
        let version = self.snapshot_version;
        let length = self.document.current().len();
        if let Some(state) = &mut self.virtual_editor {
            state.cache.invalidate_from(version, first_line);
            state.selection = VirtualSelection {
                anchor_byte: anchor.map(|byte| byte.min(length)),
                active_byte: active.min(length),
            };
        }
        self.push_virtual_undo(checkpoint);
        let caret_line = self
            .document
            .current()
            .line_column_of_byte(active.min(length))
            .0;
        self.materialize_editor_window(
            caret_line.saturating_sub(EDITABLE_VIEWPORT_LINES / 2),
            None,
        );
        VirtualWriteOutcome::Applied
    }

    // Perform navigation whose target can cross a materialization boundary.
    // Shift navigation extends the app-owned anchor instead of asking cosmic
    // to preserve an endpoint it cannot represent outside its local buffer.
    pub(crate) fn perform_virtual_navigation(&mut self, action: &text_editor::Action) -> bool {
        let Some(state) = &self.virtual_editor else {
            return false;
        };
        if matches!(action, text_editor::Action::SelectAll) {
            let end = self.document.current().len();
            self.virtual_editor
                .as_mut()
                .expect("checked above")
                .selection = VirtualSelection {
                anchor_byte: Some(0),
                active_byte: end,
            };
            let line = self.document.current().line_count().saturating_sub(1);
            self.materialize_editor_window(line.saturating_sub(EDITABLE_VIEWPORT_LINES), None);
            return true;
        }

        let (motion, selecting) = match action {
            text_editor::Action::Move(motion) => (*motion, false),
            text_editor::Action::Select(motion) => (*motion, true),
            _ => return false,
        };
        let old = state.selection;
        if !selecting && let Some(anchor) = old.anchor_byte {
            let moves_left = matches!(
                motion,
                text_editor::Motion::Left
                    | text_editor::Motion::Up
                    | text_editor::Motion::WordLeft
                    | text_editor::Motion::Home
                    | text_editor::Motion::PageUp
                    | text_editor::Motion::DocumentStart
            );
            let active = if moves_left {
                anchor.min(old.active_byte)
            } else {
                anchor.max(old.active_byte)
            };
            self.virtual_editor
                .as_mut()
                .expect("checked above")
                .selection = VirtualSelection {
                anchor_byte: None,
                active_byte: active,
            };
            let line = self.document.current().line_column_of_byte(active).0;
            if !self
                .virtual_editor
                .as_ref()
                .expect("checked above")
                .materialized
                .contains(&line)
            {
                self.materialize_editor_window(
                    line.saturating_sub(EDITABLE_VIEWPORT_LINES / 2),
                    None,
                );
            } else {
                self.project_virtual_selection();
            }
            return true;
        }

        use text_editor::Motion;
        let snapshot = self.document.current();
        let (line, column) = snapshot.line_column_of_byte(old.active_byte);
        let last_line = snapshot.line_count().saturating_sub(1);
        let target = match motion {
            Motion::Up => snapshot.byte_of_line_column(line.saturating_sub(1), column),
            Motion::Down => {
                snapshot.byte_of_line_column(line.saturating_add(1).min(last_line), column)
            }
            Motion::PageUp => {
                snapshot.byte_of_line_column(line.saturating_sub(EDITABLE_VIEWPORT_LINES), column)
            }
            Motion::PageDown => snapshot.byte_of_line_column(
                line.saturating_add(EDITABLE_VIEWPORT_LINES).min(last_line),
                column,
            ),
            Motion::Home => snapshot.line_start(line).unwrap_or(0),
            Motion::End => snapshot.line_end(line).unwrap_or(snapshot.len()),
            Motion::DocumentStart => 0,
            Motion::DocumentEnd => snapshot.len(),
            Motion::Left => snapshot.floor_char_boundary(old.active_byte.saturating_sub(1)),
            Motion::Right => snapshot
                .char_at(old.active_byte)
                .map_or(old.active_byte, |character| {
                    old.active_byte.saturating_add(character.len_utf8())
                }),
            Motion::WordLeft => {
                let mut byte = old.active_byte;
                while snapshot.char_before(byte).is_some_and(char::is_whitespace) {
                    byte = snapshot.floor_char_boundary(byte.saturating_sub(1));
                }
                while snapshot
                    .char_before(byte)
                    .is_some_and(|c| !c.is_whitespace())
                {
                    byte = snapshot.floor_char_boundary(byte.saturating_sub(1));
                }
                byte
            }
            Motion::WordRight => {
                let mut byte = old.active_byte;
                while let Some(character) = snapshot.char_at(byte).filter(|c| !c.is_whitespace()) {
                    byte = byte.saturating_add(character.len_utf8());
                }
                while let Some(character) = snapshot.char_at(byte).filter(|c| c.is_whitespace()) {
                    byte = byte.saturating_add(character.len_utf8());
                }
                byte
            }
        };
        let anchor = selecting.then(|| old.anchor_byte.unwrap_or(old.active_byte));
        self.virtual_editor
            .as_mut()
            .expect("checked above")
            .selection = VirtualSelection {
            anchor_byte: anchor,
            active_byte: target,
        };
        let target_line = self.document.current().line_column_of_byte(target).0;
        let represented = self
            .virtual_editor
            .as_ref()
            .expect("checked above")
            .materialized
            .contains(&target_line);
        if represented {
            self.project_virtual_selection();
        } else {
            self.materialize_editor_window(
                target_line.saturating_sub(EDITABLE_VIEWPORT_LINES / 2),
                None,
            );
        }
        true
    }

    pub(crate) fn virtual_edit_checkpoint(
        &self,
        action: &text_editor::Action,
    ) -> Option<VirtualHistoryEntry> {
        (self.virtual_editor.is_some()
            && matches!(
                action,
                text_editor::Action::Edit(edit)
                    if !matches!(edit, text_editor::Edit::Undo | text_editor::Edit::Redo)
            ))
        .then(|| VirtualHistoryEntry {
            document: self.document.snapshot(),
            caret_byte: self.caret_byte(),
            cost: 0,
        })
    }

    pub(crate) fn commit_virtual_edit(
        &mut self,
        mut checkpoint: VirtualHistoryEntry,
        changed_bytes: usize,
    ) {
        if self.virtual_editor.is_none() {
            return;
        }
        checkpoint.cost = changed_bytes.max(1);
        self.push_virtual_undo(checkpoint);
    }

    pub(crate) fn push_virtual_undo(&mut self, checkpoint: VirtualHistoryEntry) {
        let Some(state) = &mut self.virtual_editor else {
            return;
        };
        state.undo_bytes = state.undo_bytes.saturating_add(checkpoint.cost);
        state.undo.push_back(checkpoint);
        state.redo.clear();
        while state.undo.len() > EDITABLE_UNDO_OPERATIONS || state.undo_bytes > EDITABLE_UNDO_BYTES
        {
            let Some(oldest) = state.undo.pop_front() else {
                break;
            };
            state.undo_bytes = state.undo_bytes.saturating_sub(oldest.cost);
        }
    }

    // Handle virtual undo/redo against persistent generations. `None` means
    // this is not a virtual-history action; `Some` says whether bytes moved.
    //
    // An exhausted version refuses before either stack is popped, leaving
    // history, dirty flag, selection and document unchanged.
    pub(crate) fn perform_virtual_history(&mut self, action: &text_editor::Action) -> Option<bool> {
        let undo = matches!(action, text_editor::Action::Edit(text_editor::Edit::Undo));
        let redo = matches!(action, text_editor::Action::Edit(text_editor::Edit::Redo));
        if self.virtual_editor.is_none() || (!undo && !redo) {
            return None;
        }

        // Reserve before popping so a refusal leaves the stacks intact.
        let Ok(reserved) = self.reserve_snapshot_version() else {
            return Some(false);
        };

        let mut current = VirtualHistoryEntry {
            document: self.document.snapshot(),
            caret_byte: self.caret_byte(),
            cost: 1,
        };
        let restored = {
            let state = self.virtual_editor.as_mut()?;
            if undo {
                let restored = state.undo.pop_back();
                if let Some(entry) = &restored {
                    state.undo_bytes = state.undo_bytes.saturating_sub(entry.cost);
                    current.cost = entry.cost;
                    state.redo.push_back(current);
                }
                restored
            } else {
                let restored = state.redo.pop_back();
                if let Some(entry) = &restored {
                    current.cost = entry.cost;
                    state.undo_bytes = state.undo_bytes.saturating_add(current.cost);
                    state.undo.push_back(current);
                }
                restored
            }
        };
        let changed = restored.is_some();
        if let Some(restored) = restored {
            self.document.restore(restored.document);
            self.commit_snapshot_version(reserved);
            self.stats_pending = true;
            let line = self
                .document
                .current()
                .line_column_of_byte(restored.caret_byte)
                .0;
            let first = line.saturating_sub(EDITABLE_VIEWPORT_LINES / 2);
            self.materialize_editor_window(first, Some(restored.caret_byte));
        }
        // An empty stack drops the reservation without commit.
        Some(changed)
    }

    // Scroll an editable viewport using wheel or whole-document scrollbar coordinates.
    //
    // The native editor holds only a window, so this moves within it or
    // rebuilds it around the new view; the native scroll is applied after the
    // next layout (see [`Self::apply_virtual_scroll`]).
    pub(crate) fn scroll_virtual_view(&mut self, action: &text_editor::Action) -> bool {
        let Some(state) = &self.virtual_editor else {
            return false;
        };
        let total = self.document.current().line_count();
        let window = state.materialized.clone();
        let current = self.virtual_view_top().unwrap_or(window.start);
        let metrics = self.content.scroll_metrics();
        let row = metrics.content.height / self.content.line_count().max(1) as f32;
        let first = match action {
            text_editor::Action::Scroll { lines } => current.saturating_add_signed(*lines as isize),
            text_editor::Action::ScrollTo { offset } if row > 0.0 => {
                (offset.y.max(0.0) / row) as usize
            }
            _ => return false,
        }
        .min(total.saturating_sub(1));
        // The rows the view shows, from real geometry once there is some.
        let rows = if metrics.viewport.height > 0.0 && row > 0.0 {
            (metrics.viewport.height / row).ceil() as usize
        } else {
            EDITABLE_VIEWPORT_LINES / 2
        };
        let fits = first >= window.start && first + rows <= window.end;
        if !fits {
            self.materialize_editor_window(first, None);
        }
        if let Some(state) = &mut self.virtual_editor {
            let x = match action {
                text_editor::Action::ScrollTo { offset } => offset.x,
                _ => metrics.offset.x,
            };
            state.scroll_to_line = Some((first.saturating_sub(state.materialized.start), x));
            state.scroll_restore = None;
        }
        true
    }

    #[must_use]
    pub(crate) fn virtual_view_top(&self) -> Option<usize> {
        let state = self.virtual_editor.as_ref()?;
        Some(
            state.materialized.start
                + state.scroll_to_line.map_or_else(
                    || self.content.scroll_metrics().first_line,
                    |(line, _)| line,
                ),
        )
    }

    // Scroll the native editor to the line [`Self::scroll_virtual_view`]
    // chose, once it has real geometry. Runs on the turn after a frame.
    pub(super) fn apply_virtual_scroll(&mut self) {
        let Some((line, x)) = self
            .virtual_editor
            .as_ref()
            .and_then(|state| state.scroll_to_line)
        else {
            return;
        };
        let metrics = self.content.scroll_metrics();
        if metrics.viewport.height <= 0.0 {
            return;
        }
        self.content.perform(text_editor::Action::ScrollToLine {
            line,
            offset: iced::Vector::new(x, 0.0),
        });
        if let Some(state) = &mut self.virtual_editor {
            state.scroll_to_line = None;
        }
    }

    pub(crate) fn prepare_editor_action(&mut self, action: &text_editor::Action) {
        let Some(state) = &self.virtual_editor else {
            return;
        };
        let current = self.editor_cursor().position.line;
        let total = self.document.current().line_count();
        let materialized = state.materialized.clone();
        let target = match action {
            text_editor::Action::Move(text_editor::Motion::DocumentStart)
            | text_editor::Action::Select(text_editor::Motion::DocumentStart) => Some(0),
            text_editor::Action::Move(text_editor::Motion::DocumentEnd)
            | text_editor::Action::Select(text_editor::Motion::DocumentEnd) => {
                Some(total.saturating_sub(1))
            }
            text_editor::Action::Move(text_editor::Motion::PageDown)
            | text_editor::Action::Select(text_editor::Motion::PageDown) => Some(
                current
                    .saturating_add(EDITABLE_VIEWPORT_LINES)
                    .min(total.saturating_sub(1)),
            ),
            text_editor::Action::Move(text_editor::Motion::PageUp)
            | text_editor::Action::Select(text_editor::Motion::PageUp) => {
                Some(current.saturating_sub(EDITABLE_VIEWPORT_LINES))
            }
            text_editor::Action::ScrollTo { offset } => {
                let local = self.content.scroll_metrics();
                let row_height =
                    (local.content.height / self.content.line_count().max(1) as f32).max(1.0);
                Some(((offset.y / row_height).max(0.0) as usize).min(total.saturating_sub(1)))
            }
            text_editor::Action::Move(text_editor::Motion::Down)
            | text_editor::Action::Select(text_editor::Motion::Down)
                if current.saturating_add(2) >= materialized.end =>
            {
                Some(current)
            }
            text_editor::Action::Move(text_editor::Motion::Up)
            | text_editor::Action::Select(text_editor::Motion::Up)
                if current <= materialized.start.saturating_add(1) =>
            {
                Some(current)
            }
            text_editor::Action::Edit(text_editor::Edit::Backspace)
                if self.content.cursor().position.line == 0 && current > 0 =>
            {
                Some(current)
            }
            text_editor::Action::Edit(text_editor::Edit::Delete)
                if self.content.cursor().position.line.saturating_add(1)
                    >= self.content.line_count()
                    && current.saturating_add(1) < total =>
            {
                Some(current)
            }
            // A scroll may leave the global caret off-frame; restore it before an
            // edit instead of writing at the projection edge.
            text_editor::Action::Edit(_) if !materialized.contains(&current) => Some(current),
            _ => None,
        };
        if let Some(target) = target {
            let first = match action {
                text_editor::Action::Move(text_editor::Motion::DocumentEnd)
                | text_editor::Action::Select(text_editor::Motion::DocumentEnd) => {
                    total.saturating_sub(EDITABLE_VIEWPORT_LINES)
                }
                text_editor::Action::Move(text_editor::Motion::PageDown)
                | text_editor::Action::Select(text_editor::Motion::PageDown) => current,
                text_editor::Action::Move(text_editor::Motion::PageUp)
                | text_editor::Action::Select(text_editor::Motion::PageUp)
                | text_editor::Action::ScrollTo { .. } => target,
                _ => target.saturating_sub(EDITABLE_VIEWPORT_LINES / 2),
            };
            self.materialize_editor_window(first, None);
        }
    }
}

#[cfg(test)]
impl Tab {
    #[must_use]
    pub(crate) fn virtual_history_depth(&self) -> Option<(usize, usize)> {
        self.virtual_editor
            .as_ref()
            .map(|state| (state.undo.len(), state.redo.len()))
    }
}
