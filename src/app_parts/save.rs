//! TextApp save paths.

use super::*;

impl TextApp {
    pub(crate) fn start_save_as(&mut self) -> bool {
        // Refuse before launching the portal, so its completion never sees a
        // read-only tab.
        let active = self.model.active_tab_id.clone();
        if self.refuse_write_if_read_only(&active) {
            return false;
        }
        let suggested = self
            .model
            .active_tab()
            .file_path
            .as_ref()
            .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
            .or_else(|| Some(suggested_document_name(self.model.active_tab())));
        match self.picker.start_save(suggested.as_deref()) {
            Ok(()) => true,
            Err(outcome) => {
                let _ = self.finish_pick_save(outcome);
                false
            }
        }
    }

    // Returns `true` and records the refusal sentence when the tab must not be
    // written: a read-only tab holds no complete buffer.
    pub(crate) fn refuse_write_if_read_only(&mut self, id: &str) -> bool {
        let Some(tab) = self.model.tabs.iter().find(|tab| tab.id == id) else {
            return false;
        };
        if !tab.is_read_only() {
            return false;
        }
        let path = tab
            .file_path
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from(tab.title()));
        self.picker.failure = Some(
            if tab.decoded_lossily
                || tab
                    .buffer
                    .large()
                    .is_some_and(super::large_doc::LargeDocument::is_binary_preview)
            {
                enzui_core::localized!("text-p0-isn-t-plain-text-so-it-20ec13", "p0" => crate::safe_text::bounded_inline(&tab.title()))
            } else {
                large_doc::save_refused_sentence(&path)
            },
        );
        true
    }

    // Returns `true` and publishes the refusal sentence when `capability` must
    // not run on the active tab. The view disables the same set through
    // [`Tab::supports`](crate::model::Tab::supports).
    pub(crate) fn refuse_capability(&mut self, capability: large_doc::Capability) -> bool {
        let id = self.model.active_tab_id.clone();
        let Some(tab) = self.model.tabs.iter().find(|tab| tab.id == id) else {
            return false;
        };
        if tab.supports(capability) {
            return false;
        }
        let title = tab.title();
        self.picker.failure = Some(
            if tab.decoded_lossily
                || tab
                    .buffer
                    .large()
                    .is_some_and(super::large_doc::LargeDocument::is_binary_preview)
            {
                enzui_core::localized!("text-p0-is-unavailable-p1-isn-t-plain-f673fe", "p0" => capability.label(), "p1" => crate::safe_text::bounded_inline(&title))
            } else {
                large_doc::capability_refused_sentence(capability, &title)
            },
        );
        true
    }

    pub(crate) fn refuse_replace_if_read_only(&mut self) -> bool {
        self.refuse_capability(large_doc::Capability::Replace)
    }

    // Returns `true` when the active tab's `snapshot_version` or
    // `cursor_epoch` is exhausted; the caller must then leave the content,
    // selection, dirty flag and history untouched. Each refusal republishes
    // the terminal sentence so screen readers announce it. `_action` is
    // currently unused: every action on an exhausted tab is refused.
    pub(crate) fn refuse_if_authority_exhausted_on_action(
        &mut self,
        _action: &iced::widget::text_editor::Action,
    ) -> bool {
        if !self.model.active_tab().authority_exhausted() {
            return false;
        }
        self.publish_authority_exhausted_sentence();
        true
    }

    pub(crate) fn refuse_if_authority_exhausted(&mut self) -> bool {
        if !self.model.active_tab().authority_exhausted() {
            return false;
        }
        self.publish_authority_exhausted_sentence();
        true
    }

    pub(crate) fn publish_authority_exhausted_sentence(&mut self) {
        self.picker.failure = Some(crate::i18n::domain_text(
            model::TAB_AUTHORITY_EXHAUSTED_MESSAGE,
        ));
    }

    pub(crate) fn save_tab(&mut self, id: &str) -> SaveDispatch {
        if self.refuse_write_if_read_only(id) {
            return SaveDispatch::Failed;
        }
        let Some(index) = self.model.tabs.iter().position(|tab| tab.id == id) else {
            return SaveDispatch::Failed;
        };
        self.save_retry_blocked.remove(id);
        let Some(path) = self.model.tabs[index].file_path.clone() else {
            return SaveDispatch::Failed;
        };
        if self.refuse_unacknowledged_change(index, &path) {
            return SaveDispatch::Failed;
        }
        // Temp sibling + `fsync` + `rename`, never `O_TRUNC`, streamed from an
        // O(1) snapshot on a worker; the buffer version travels with the bytes.
        self.dispatch_streaming_save(index, &path, None)
    }

    // Refuse after an external change, even when clean: writing the old bytes
    // would revert the external edit until the user refreshes or dismisses it.
    fn refuse_unacknowledged_change(&mut self, index: usize, path: &std::path::Path) -> bool {
        if !self.model.tabs[index].externally_changed {
            return false;
        }
        self.picker.failure = Some(
            enzui_core::localized!("text-p0-changed-on-disk-refresh-or-dismiss-ecd72b", "p0" => crate::safe_text::bounded_path(path)),
        );
        true
    }

    // The revision an in-place save may replace: the one the tab loaded, last
    // saved, or the user acknowledged by dismissing a change. Without one, the
    // save may only create a file that is still absent.
    fn in_place_replaces(&self, index: usize, path: &std::path::Path) -> atomic_save::Replaces {
        self.model.tabs[index]
            .watcher
            .as_ref()
            .filter(|watcher| watcher.path() == path)
            .and_then(enzui_core::file_watch::FileWatcher::baseline)
            .map_or(
                atomic_save::Replaces::Absent,
                atomic_save::Replaces::Revision,
            )
    }

    // One writer per tab, so an older generation can never rename over a newer
    // one. A request during a save is queued and the completion saves the
    // latest buffer.
    pub(crate) fn dispatch_streaming_save(
        &mut self,
        index: usize,
        path: &std::path::Path,
        rebind: Option<std::path::PathBuf>,
    ) -> SaveDispatch {
        if self.model.tabs[index].saving || self.save_destinations.contains_key(path) {
            self.model.tabs[index].save_queued = Some(rebind);
            return SaveDispatch::InFlight;
        }
        let replaces = if rebind.is_some() {
            atomic_save::Replaces::Chosen
        } else if self.refuse_unacknowledged_change(index, path) {
            return SaveDispatch::Failed;
        } else {
            self.in_place_replaces(index, path)
        };
        let request = SaveRequest {
            tab: self.model.tabs[index].id.clone(),
            generation: self.model.tabs[index].snapshot_version(),
            destination: path.to_path_buf(),
            ending: self.model.tabs[index].line_ending,
            document: self.model.tabs[index].snapshot_handle(),
            rebind,
            replaces,
            guard_attempt: self.close_save_attempt.take(),
        };
        self.submit_save_request(index, request)
    }

    pub(crate) fn submit_save_request(
        &mut self,
        index: usize,
        request: SaveRequest,
    ) -> SaveDispatch {
        if self.save_destinations.contains_key(&request.destination) {
            self.model.tabs[index].save_queued = Some(request.rebind.clone());
            self.save_debt
                .insert(self.model.tabs[index].id.clone(), request);
            return SaveDispatch::InFlight;
        }
        let worker_request = request.clone();
        let output_request = request.clone();
        let failure_request = request.clone();
        let generation = request.generation;
        #[cfg(test)]
        let carried_faults = atomic_save::fault::carry();
        let worker_localizer = enzui_core::localization::current();
        let task = self.blocking_tasks.submit(generation, move |_| {
            worker_localizer.scope(|| {
                #[cfg(test)]
                let _faults = atomic_save::fault::install(carried_faults);
                let bytes = match worker_request.ending {
                    line_ending::LineEnding::Crlf => {
                        atomic_save::DocumentBytes::Crlf(&worker_request.document)
                    }
                    line_ending::LineEnding::Cr => {
                        atomic_save::DocumentBytes::Cr(&worker_request.document)
                    }
                    line_ending::LineEnding::Lf | line_ending::LineEnding::Mixed => {
                        atomic_save::DocumentBytes::ChunkedExact(&worker_request.document)
                    }
                };
                atomic_save::write_document_with(
                    &worker_request.destination,
                    &bytes,
                    worker_request.replaces,
                )
                .map(|_mode| {
                    std::fs::metadata(&worker_request.destination)
                        .ok()
                        .as_ref()
                        .map(FileIdentity::from_metadata)
                })
                .map_err(|error| match error {
                    atomic_save::WriteError::ExternalRevision(found) => {
                        document_failure::DocumentFailure::from_external_revision(
                            &worker_request.destination,
                            found,
                        )
                    }
                    atomic_save::WriteError::Save(error) => {
                        document_failure::DocumentFailure::from_user_file(
                            &worker_request.destination,
                            error,
                        )
                    }
                })
            })
        });
        self.deferred_tasks
            .push(Task::perform(task, move |settlement| {
                match settlement.into_result() {
                    Ok(result) => WorkDone::Save {
                        tab: output_request.tab,
                        generation: output_request.generation,
                        destination: output_request.destination,
                        rebind: output_request.rebind,
                        guard_attempt: output_request.guard_attempt,
                        result,
                    }
                    .message(),
                    Err(error) => Message::BlockingWorkFailed(BlockingWorkFailure::Save {
                        tab: failure_request.tab,
                        generation: failure_request.generation,
                        destination: failure_request.destination,
                        rebind: failure_request.rebind,
                        guard_attempt: failure_request.guard_attempt,
                        error,
                    }),
                }
            }));
        self.save_destinations.insert(
            request.destination.clone(),
            self.model.tabs[index].id.clone(),
        );
        self.save_inflight
            .insert(self.model.tabs[index].id.clone(), request);
        self.model.tabs[index].saving = true;
        SaveDispatch::InFlight
    }

    // `generation` is the buffer version written; the tab becomes clean only if
    // it has not been edited since.
    pub(crate) fn finish_save(
        &mut self,
        id: &str,
        generation: u64,
        destination: std::path::PathBuf,
        rebind: Option<std::path::PathBuf>,
        result: Result<Option<FileIdentity>, document_failure::DocumentFailure>,
    ) -> SaveOutcome {
        self.save_destinations.remove(&destination);
        self.save_inflight.remove(id);
        self.save_debt.remove(id);
        let Some(index) = self.model.tabs.iter().position(|tab| tab.id == id) else {
            // The tab closed during the write; the write itself completed.
            return SaveOutcome::Failed;
        };
        self.model.tabs[index].saving = false;
        let save_as_target = rebind.clone();

        let outcome = match result {
            Ok(saved_identity) => {
                let rebinding = rebind.is_some();
                if let Some(path) = rebind {
                    // Save As rebinds only on success: a tab that could not be
                    // written must still point at the file it came from.
                    let tab = &mut self.model.tabs[index];
                    tab.file_path = Some(path);
                    tab.install_watcher(std::sync::Arc::clone(&self.file_watch_backend));
                }
                if rebinding {
                    self.advance_session_projection_revision();
                }
                // An edit during the write keeps the tab dirty and blocks a
                // waiting close guard.
                let current = self.model.tabs[index].snapshot_version() == generation;
                if current {
                    self.model.tabs[index].dirty = false;
                    self.model.tabs[index].externally_changed = false;
                }
                if let Some(identity) = saved_identity
                    && let Some(watcher) = &mut self.model.tabs[index].watcher
                {
                    watcher.acknowledge(identity);
                }
                if current {
                    SaveOutcome::Saved
                } else {
                    self.model.tabs[index].dirty = true;
                    SaveOutcome::Failed
                }
            }
            Err(error) => {
                if let Some(found) = error.external_revision() {
                    // Raises the change notice; Refresh reloads `found`, Dismiss
                    // makes it the revision the next save may replace.
                    let tab = &mut self.model.tabs[index];
                    tab.externally_changed = true;
                    if let Some(watcher) = &mut tab.watcher {
                        match found {
                            Some(identity) => watcher.acknowledge(identity),
                            None => watcher.mark_deleted(),
                        }
                    }
                    let summary = enzui_core::localized!("text-p0-changed-on-disk-refresh-or-dismiss-ecd72b", "p0" => crate::safe_text::bounded_path(&destination));
                    self.publish_document_failure(&error, summary);
                } else {
                    let summary = save_as_target
                        .as_deref()
                        .map_or_else(|| error.save_summary(), |path| error.save_as_summary(path));
                    self.publish_document_failure(&error, summary);
                }
                self.model.tabs[index].dirty = true;
                SaveOutcome::Failed
            }
        };

        // Run the queued save against the current buffer.
        if let Some(rebind) = self.model.tabs[index].save_queued.take()
            && let Some(path) = rebind
                .clone()
                .or_else(|| self.model.tabs[index].file_path.clone())
        {
            let _ = self.dispatch_streaming_save(index, &path, rebind);
        }
        // Whatever the bounded queue refused earlier can go now.
        self.pump_pending_work();

        outcome
    }
}

fn suggested_document_name(tab: &Tab) -> String {
    let snapshot = tab.document();
    let mut stem = String::new();
    let mut byte = 0;
    // Bound scanning as well as output for huge documents with leading whitespace.
    while byte < snapshot.len().min(4096) && stem.chars().count() < 10 {
        let Some(c) = snapshot.char_at(byte) else {
            break;
        };
        byte += c.len_utf8();
        if c.is_whitespace() {
            if !stem.is_empty() {
                break;
            }
        } else if c.is_alphanumeric() || matches!(c, '-' | '_') {
            stem.push(c);
        }
    }
    if stem.is_empty() {
        stem.push_str("Untitled");
    }
    format!("{stem}.txt")
}
