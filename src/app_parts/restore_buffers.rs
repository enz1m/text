//! Recovered-buffer restore.

use super::*;

// Records are classified with [`recovery::classify`]: a disk-changed buffer
// is flagged `externally_changed`, and a deleted-path buffer is disclosed. A
// record whose document already has a tab upgrades that tab in place.
pub(crate) fn restore_recovered_buffers(
    model: &mut Model,
    records: Vec<recovery::RecoveryRecord>,
) -> RecoveryBootReport {
    let mut restored = 0usize;
    let mut conflicts: Vec<String> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();
    let mut claimed = std::collections::HashSet::new();
    for record in records {
        // The watcher baseline keeps the full identity: it is the revision an
        // in-place save may replace, and the fingerprint omits the change time.
        let disk_identity = record
            .original_path
            .as_deref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|metadata| FileIdentity::from_metadata(&metadata));
        let disk = disk_identity.map(recovery::DiskFingerprint::from_identity);
        let decision = recovery::classify(record, disk);
        let is_conflict = matches!(decision, recovery::RecoveryDecision::Conflict { .. });
        let is_deleted = matches!(decision, recovery::RecoveryDecision::Deleted { .. });
        let rec = match decision {
            recovery::RecoveryDecision::RestoreUntitled(r)
            | recovery::RecoveryDecision::RestoreClean(r)
            | recovery::RecoveryDecision::Deleted { record: r } => r,
            recovery::RecoveryDecision::Conflict { record, .. } => record,
        };
        let ending = match rec.line_ending.as_str() {
            "CRLF" => crate::line_ending::LineEnding::Crlf,
            "CR" => crate::line_ending::LineEnding::Cr,
            "Mixed" => crate::line_ending::LineEnding::Mixed,
            _ => crate::line_ending::LineEnding::Lf,
        };
        // Reusing an existing tab's id keeps the document identity stable.
        let (index, id) = match existing_tab_for(model, &rec, &claimed) {
            Some(index) => (index, model.tabs[index].id.clone()),
            None => (model.tabs.len(), recovered_tab_id(model, &rec.document_id)),
        };
        let mut tab = Tab::with_text(id, rec.content.as_str());
        tab.dirty = true;
        tab.file_path.clone_from(&rec.original_path);
        tab.line_ending = ending;
        tab.language_override.clone_from(&rec.language_override);
        if tab.file_path.is_some() {
            tab.install_watcher(std::sync::Arc::new(FsBackend));
            if let Some(watcher) = &mut tab.watcher {
                match disk_identity {
                    Some(identity) => watcher.acknowledge(identity),
                    None => watcher.mark_deleted(),
                }
            }
        }
        if index < model.tabs.len() {
            model.tabs[index] = tab;
        } else {
            model.tabs.push(tab);
        }
        let name_of = |rec: &recovery::RecoveryRecord| {
            rec.original_path
                .as_deref()
                .map(crate::safe_text::bounded_path)
        };
        if is_conflict {
            model.tabs[index].externally_changed = true;
            if let Some(name) = name_of(&rec) {
                conflicts.push(name);
            }
        } else if is_deleted && let Some(name) = name_of(&rec) {
            deleted.push(name);
        }
        claimed.insert(index);
        restore_caret_selection(&mut model.tabs[index].content, &rec.caret);
        model.tabs[index].pending_scroll =
            (rec.caret.scroll_offset > 0.5).then_some(rec.caret.scroll_offset);
        restored += 1;
    }
    RecoveryBootReport {
        restored,
        conflicts,
        deleted,
    }
}
