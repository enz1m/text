//! TextApp boot disclosure helper.

use super::*;

pub(crate) fn recovery_boot_disclosure(
    report: &RecoveryBootReport,
    origin: &recovery::Origin,
) -> Option<String> {
    let mut msgs: Vec<String> = Vec::new();
    if report.restored > 0 {
        msgs.push(enzui_core::localized!("text-recovered-p0-unsaved-buffer-s-from-a-d646e6", "p0" => report.restored));
    }
    if !report.conflicts.is_empty() {
        msgs.push(enzui_core::localized!("text-p0-changed-on-disk-since-then-review-849459", "p0" => report.conflicts.join(", ")));
    }
    if !report.deleted.is_empty() {
        msgs.push(enzui_core::localized!("text-p0-was-deleted-since-then-save-to-706197", "p0" => report.deleted.join(", ")));
    }
    match origin {
        recovery::Origin::RecoveredFromCorrupt => {
            msgs.push(enzui_core::localization::text(
                "text-a-corrupt-recovery-file-was-set-aside-1ea7c3",
            ));
        }
        recovery::Origin::QuarantineRefused => {
            msgs.push(enzui_core::localization::text(
                "text-a-suspicious-recovery-file-could-not-be-06fa2c",
            ));
        }
        recovery::Origin::Loaded { dropped } if *dropped > 0 => {
            msgs.push(enzui_core::localized!("text-dropped-unreadable-recovery-record-s-were-quarantined-9ab1da", "dropped" => dropped));
        }
        recovery::Origin::Unavailable => {
            msgs.push(enzui_core::localization::text(
                "text-recovery-is-unavailable-this-session-its-folder-7b049c",
            ));
        }
        recovery::Origin::ReadOnlyFutureVersion => {
            msgs.push(enzui_core::localization::text(
                "text-recovery-data-from-a-newer-text-editor-b7b1d7",
            ));
        }
        recovery::Origin::Missing | recovery::Origin::Loaded { .. } => {}
    }
    if msgs.is_empty() {
        None
    } else {
        Some(msgs.join("; "))
    }
}
