use super::*;

fn sandbox() -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    std::env::temp_dir().join(format!(
        "text-app-3020-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

struct Fixture {
    dir: std::path::PathBuf,
    session: std::path::PathBuf,
    one: std::path::PathBuf,
    two: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = sandbox();
    std::fs::create_dir_all(&dir).expect("sandbox");
    let one = dir.join("one.txt");
    let two = dir.join("two.txt");
    std::fs::write(&one, "one").expect("one");
    std::fs::write(&two, "two").expect("two");
    let session = dir.join("session.list");
    let mut seed = TextApp::test_with_session(settings::Settings::default(), session.clone());
    assert!(seed.open_path(&one));
    assert!(seed.open_path(&two));
    let task = seed.update(Message::Resized(800.0, 600.0));
    assert_eq!(
        crate::task_driver::drive_tasks(&mut seed, task),
        (Vec::new(), 0)
    );
    assert!(!seed.session_write_inflight);
    assert!(
        std::fs::read_to_string(&session)
            .expect("session written")
            .contains("\"version\": 3")
    );
    Fixture {
        dir,
        session,
        one,
        two,
    }
}

fn pending_app(fx: &Fixture) -> TextApp {
    let mut app =
        TextApp::test_with_session(settings::Settings::default(), fx.dir.join("live.list"));
    make_pending(&mut app);
    app
}

fn make_pending(app: &mut TextApp) {
    let baseline = session::projection(&app.model, &app.workspaces);
    app.session_restore_gen = 0;
    app.session_restore_pending = Some(SessionRestorePending {
        cli_opened: false,
        baseline,
        debt: None,
    });
}

fn feed_completion(app: &mut TextApp, fx: &Fixture) {
    let restore = session::SessionStore::read(fx.session.clone(), None);
    let _ = app.update(Message::SessionRestoreCompleted {
        generation: 0,
        result: Ok(restore),
    });
}

fn has_tab(app: &TextApp, path: &std::path::Path) -> bool {
    app.model
        .tabs
        .iter()
        .any(|tab| tab.file_path.as_deref() == Some(path))
}

#[test]
fn cli_document_load_and_edit_keep_session_authority_without_restoring_saved_tabs() {
    crate::test_fixture::with_locale(|| {
        for missing in [false, true] {
            let fx = fixture();
            let source = if missing {
                fx.dir.join("missing.list")
            } else {
                fx.session.clone()
            };
            let mut app = pending_app(&fx);
            app.session_restore_pending
                .as_mut()
                .expect("pending restore")
                .cli_opened = true;
            assert!(app.open_path(&fx.one));
            app.model
                .active_tab_mut()
                .install_buffer("CLI edits must survive".to_owned());
            app.model.active_tab_mut().dirty = true;
            let task = app.update(Message::NoOp);
            assert_eq!(
                crate::task_driver::drive_tasks(&mut app, task),
                (Vec::new(), 0)
            );
            assert!(app.session_restore_pending.is_some());
            let task = app.update(Message::SessionRestoreCompleted {
                generation: 0,
                result: Ok(session::SessionStore::read(source.clone(), None)),
            });
            assert_eq!(
                crate::task_driver::drive_tasks(&mut app, task),
                (Vec::new(), 0)
            );
            assert!(app.session_debt.is_none());
            assert!(app.view_session_notification().is_none());
            assert!(has_tab(&app, &fx.one));
            assert!(!has_tab(&app, &fx.two));
            assert_eq!(app.model.active_tab().snapshot(), "CLI edits must survive");
            assert!(app.model.active_tab().dirty);
            assert!(
                std::fs::read_to_string(&source)
                    .expect("session remembered")
                    .contains("one.txt")
            );
            let _ = std::fs::remove_dir_all(fx.dir);
        }
    });
}

#[test]
fn new_tab_supersedes_pending_restore() {
    crate::test_fixture::with_locale(|| {
        let fx = fixture();
        let mut app =
            TextApp::test_with_session(settings::Settings::default(), fx.dir.join("live.list"));
        assert!(app.open_path(&fx.one));
        make_pending(&mut app);
        let _ = app.update(Message::NewTab);
        feed_completion(&mut app, &fx);
        assert!(app.model.active_tab().file_path.is_none());
        assert!(!has_tab(&app, &fx.two));
        assert!(app.session_restore_pending.is_none());
        let _ = std::fs::remove_dir_all(fx.dir);
    });
}

#[test]
fn a_source_replaced_before_worker_settlement_refuses_restore() {
    crate::test_fixture::with_locale(|| {
        let fx = fixture();
        let mut app = pending_app(&fx);
        let source = fx.session.clone();
        let _reset = session::set_restore_after_read_seam(move || {
            std::fs::remove_file(&source).expect("unlink");
            std::fs::write(&source, "REPLACED").expect("replacement");
        });
        let restore = session::SessionStore::read(fx.session.clone(), None);
        let _ = app.update(Message::SessionRestoreCompleted {
            generation: 0,
            result: Ok(restore),
        });
        assert!(!has_tab(&app, &fx.one) && !has_tab(&app, &fx.two));
        assert_eq!(
            app.session_debt.as_ref().map(session::SessionDebt::kind),
            Some(session::SessionDebtKind::Replaced)
        );
        assert!(app.session_restore_pending.is_none());
        assert!(!app.session_store.discard_open_tabs());
        assert_eq!(
            std::fs::read_to_string(&fx.session).expect("replacement present"),
            "REPLACED"
        );
        let _ = std::fs::remove_dir_all(fx.dir);
    });
}

#[test]
fn a_document_argument_on_a_fresh_profile_boots_without_a_session_retry_notice() {
    crate::test_fixture::with_locale(|| {
        use std::os::unix::fs::PermissionsExt;
        let root = sandbox();
        let data = root.join("data");
        std::fs::create_dir_all(&data).expect("data home");
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        let note = root.join("note.txt");
        std::fs::write(&note, "command-line document").expect("note");
        let env = BootEnv {
            xdg_config_home: Some(root.join("config")),
            xdg_data_home: Some(data.clone()),
            home: None,
            cli_args: vec![note.clone()],
            runtime_fallback: None,
        };
        let (mut app, task) = TextApp::boot_at(&env);
        assert_eq!(
            crate::task_driver::drive_tasks(&mut app, task),
            (Vec::new(), 0)
        );
        assert!(app.session_restore_pending.is_none());
        assert!(app.session_debt.is_none());
        assert!(app.view_session_notification().is_none());
        assert_eq!(
            app.model.active_tab().file_path.as_deref(),
            Some(note.as_path())
        );
        assert_eq!(app.model.active_tab().snapshot(), "command-line document");
        let second = root.join("second.txt");
        std::fs::write(&second, "second document").expect("second");
        assert!(app.open_path(&second));
        let task = app.update(Message::Resized(800.0, 600.0));
        assert_eq!(
            crate::task_driver::drive_tasks(&mut app, task),
            (Vec::new(), 0)
        );
        let session = std::fs::read_to_string(data.join("text-app/session.list"))
            .expect("session written to the fresh profile");
        assert!(session.contains("note.txt") && session.contains("second.txt"));
        assert!(app.session_debt.is_none());
        let _ = std::fs::remove_dir_all(root);
    });
}
