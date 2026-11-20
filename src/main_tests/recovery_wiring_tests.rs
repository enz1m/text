use super::*;
use crate::model::Model;
use crate::recovery::{CaretMeta, DiskFingerprint, RecoveryRecord, RecoveryStore};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn sandbox(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "text-app-recovery-wire-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("sandbox");
    dir
}

fn enabled_app(dir: &Path) -> TextApp {
    let mut app = TextApp::for_test();
    let (_records, store, _origin) = RecoveryStore::open(dir.join("recovery.json"));
    app.recovery_store = store;

    app.recovery_gen = app.recovery_store.published_generation();
    app
}

fn dirty_active(app: &mut TextApp, text: &str) -> (String, u64) {
    let tab = app.model.active_tab_mut();
    tab.set_snapshot(text);
    tab.dirty = true;
    tab.loading = false;
    (tab.id.clone(), tab.snapshot_version())
}

#[test]
fn discard_retires_recovery_on_disk_and_survives_a_stale_completion() {
    crate::test_fixture::with_locale(|| {
        let dir = sandbox("discard");
        let mut app = enabled_app(&dir);
        let (id, rev) = dirty_active(&mut app, "abandon me");
        app.recovery_published.insert(id.clone(), rev);

        let rec = RecoveryRecord::new(id.as_str(), "abandon me".to_owned());
        let snap = crate::recovery::RecoverySnapshot::from_records(vec![rec]);
        assert!(app.recovery_store.publish(1, snap).committed());

        app.recovery_gen = app.recovery_store.published_generation();
        let (before, _s, _o) = RecoveryStore::open(dir.join("recovery.json"));
        assert!(!before.is_empty());

        let window = iced::window::Id::unique();
        let task = app.begin_recovery_discard(window);
        let generation = pending_gen(&app);
        assert!(app.recovery_published.contains_key(&id));

        // The discard has already submitted its worker. Drive its actual
        // completion so the test does not race a second writer for this generation.
        let _ = crate::task_driver::drive_tasks(&mut app, task);
        assert_eq!(app.recovery_store.published_generation(), generation);
        assert!(app.recovery_discard.is_none());
        assert!(app.recovery_published.is_empty());

        let (after, _store, _origin) = RecoveryStore::open(dir.join("recovery.json"));
        assert!(after.is_empty());

        let _ = app.update(Message::RecoveryWritten {
            generation: 0,
            result: Ok(true),
        });
        assert!(!app.recovery_published.contains_key(&id));
    });
}

#[test]
fn boot_restores_an_untitled_buffer_with_its_selection() {
    crate::test_fixture::with_locale(|| {
        let mut model = Model::default();
        let before = model.tabs.len();
        let caret = CaretMeta {
            cursor_line: 0,
            cursor_column: 5,
            selection_line: Some(0),
            selection_column: Some(0),
            scroll_offset: 12.0,
        };
        let rec = RecoveryRecord::new("draft-1", "hello world".to_owned()).with_caret(caret);

        let report = restore_recovered_buffers(&mut model, vec![rec]);
        assert_eq!(report.restored, 1);
        assert_eq!(model.tabs.len(), before + 1);

        let tab = model.tabs.last().expect("restored tab");
        assert!(tab.dirty);
        let cursor = tab.content.cursor();
        assert_eq!(cursor.position.line, 0);
        assert_eq!(cursor.position.index, 5);
        assert!(cursor.selection.is_some());
    });
}

#[test]
fn a_disk_changed_path_is_a_conflict() {
    crate::test_fixture::with_locale(|| {
        let dir = sandbox("conflict");
        let real = dir.join("live.txt");
        std::fs::write(&real, "disk changed after the crash").expect("write");
        let stale = DiskFingerprint {
            dev: 1,
            ino: 2,
            mtime_secs: 1,
            mtime_nsec: 0,
            len: 1,
        };
        let mut model = Model::default();
        let rec = RecoveryRecord::new("c-id", "recovered".to_owned())
            .with_path(Some(real.clone()), Some(stale));

        let report = restore_recovered_buffers(&mut model, vec![rec]);
        assert_eq!(report.conflicts.len(), 1);
        let tab = model
            .tabs
            .iter()
            .find(|t| t.file_path.as_deref() == Some(real.as_path()))
            .expect("restored tab");
        assert!(tab.externally_changed);
        assert!(tab.dirty);
        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "disk changed after the crash"
        );
    });
}

use crate::RecoveryDiscard;
use enzui_widgets::save_changes_dialog::{SaveChangesAction, SaveChangesEvent};

fn pending_gen(app: &TextApp) -> u64 {
    match app.recovery_discard {
        Some(RecoveryDiscard::Pending { generation, .. }) => generation,
        other => panic!("expected a pending discard, saw {other:?}"),
    }
}

#[test]
fn a_failed_tombstone_keeps_open_and_retry_settles() {
    crate::test_fixture::with_locale(|| {
        let dir = sandbox("discard-fail");
        let mut app = enabled_app(&dir);
        let (id, rev) = dirty_active(&mut app, "keep me");
        app.recovery_published.insert(id.clone(), rev);

        let window = iced::window::Id::unique();
        let _ = app.begin_recovery_discard(window);
        let gen1 = pending_gen(&app);

        let _ = app.update(Message::RecoveryDiscardSettled {
            generation: gen1,
            result: Ok(false),
        });
        assert!(matches!(
            app.recovery_discard,
            Some(RecoveryDiscard::Failed { .. })
        ));
        assert!(app.a_dialog_is_open());
        assert!(app.recovery_published.contains_key(&id));

        let _ = app.update(Message::RecoveryDiscardEvent(SaveChangesEvent::Action(
            SaveChangesAction::SaveAll,
        )));
        let gen2 = pending_gen(&app);
        assert!(gen2 > gen1);

        let _ = app.update(Message::RecoveryDiscardSettled {
            generation: gen2,
            result: Ok(true),
        });
        assert!(app.recovery_discard.is_none());
        assert!(app.recovery_published.is_empty());
    });
}
