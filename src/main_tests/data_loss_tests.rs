use super::*;
use crate::test_support::TempDir;

fn dirty(app: &mut TextApp, text: &str) {
    app.model.active_tab_mut().set_snapshot(text);
    app.model.active_tab_mut().dirty = true;
}

fn finish(app: &mut TextApp, task: Task<Message>) -> usize {
    let (closes, exits) = crate::task_driver::drive_tasks(app, task);
    let mut destroyed = closes.len() + exits;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while destroyed == 0 && app.close_drain.is_some() {
        assert!(std::time::Instant::now() < deadline);
        let task = app.update(Message::CloseDrainTick);
        let (closes, exits) = crate::task_driver::drive_tasks(app, task);
        destroyed += closes.len() + exits;
        std::thread::yield_now();
    }
    destroyed
}

#[test]
fn cancelling_close_keeps_edits_and_save_all_reaches_each_dirty_document() {
    crate::test_fixture::with_locale(|| {
        let dir = TempDir::new();
        let paths = [dir.write("one", b"old one"), dir.write("two", b"old two")];
        let mut app = TextApp::for_test();
        for path in &paths {
            assert!(app.open_path(path));
            dirty(&mut app, "Grüße 日本語\n");
        }
        let window = iced::window::Id::unique();
        let request = classify_event(
            iced::Event::Window(iced::window::Event::CloseRequested),
            iced::event::Status::Ignored,
            window,
        )
        .unwrap();
        let task = app.update(request);
        assert_eq!(finish(&mut app, task), 0);
        let _ = app.update(Message::WindowCloseCancel);
        assert!(app.model.tabs.iter().filter(|t| t.dirty).count() == 2);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"old one");
        let task = app.update(Message::CloseRequested(
            enzui_core::close_guard::CloseRequest::for_window(window),
        ));
        assert_eq!(finish(&mut app, task), 0);
        let task = app.update(Message::WindowCloseSaveAll);
        assert!(finish(&mut app, task) > 0);
        assert!(app.model.tabs.iter().all(|t| !t.dirty));
        for path in paths {
            assert_eq!(std::fs::read_to_string(path).unwrap(), "Grüße 日本語\n");
        }
    });
}

#[test]
fn failed_save_all_keeps_the_window_and_every_unsaved_document() {
    crate::test_fixture::with_locale(|| {
        let dir = TempDir::new();
        let mut app = TextApp::for_test();
        let first = app.model.active_tab_mut();
        first.file_path = Some(dir.0.join("missing/one"));
        first.set_snapshot("first draft");
        first.dirty = true;
        let later_path = dir.write("later", b"precious");
        let mut later = Tab::with_text(app.model.fresh_tab_id(), "later draft");
        later.file_path = Some(later_path.clone());
        later.dirty = true;
        app.model.add_tab(later);
        let window = iced::window::Id::unique();
        let task = app.update(Message::CloseRequested(
            enzui_core::close_guard::CloseRequest::for_window(window),
        ));
        assert_eq!(finish(&mut app, task), 0);
        let task = app.update(Message::WindowCloseSaveAll);
        assert_eq!(finish(&mut app, task), 0);
        assert!(app.model.tabs.iter().all(|t| t.dirty));
        assert!(app.picker.failure.is_some());
        assert_eq!(std::fs::read(later_path).unwrap(), b"precious");
    });
}

#[test]
fn failed_or_uncertain_publication_never_marks_a_document_saved() {
    crate::test_fixture::with_locale(|| {
        for published in [false, true] {
            let dir = TempDir::new();
            let path = dir.write("precious", b"old");
            let mut app = TextApp::for_test();
            assert!(app.open_path(&path));
            dirty(&mut app, "complete new bytes");
            if published {
                crate::atomic_save::fault::arm_published_failure();
            } else {
                crate::atomic_save::fault::arm();
            }
            let _ = crate::task_driver::update_driven(&mut app, Message::SaveFile);
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                if published {
                    "complete new bytes"
                } else {
                    "old"
                }
            );
            assert!(app.model.active_tab().dirty);
            assert_eq!(app.model.active_tab().snapshot(), "complete new bytes");
            assert!(app.picker.failure.is_some());
        }
    });
}

#[test]
fn save_as_cancel_and_rebind_preserve_unicode_endings_and_original_bytes() {
    crate::test_fixture::with_locale(|| {
        use crate::line_ending::LineEnding;
        use crate::picker::tests::ScriptedBackend;
        use enzui_widgets::file_chooser::PortalResponse;
        for (ending, encoded) in [
            (LineEnding::Lf, "é\n日本語\n"),
            (LineEnding::Crlf, "é\r\n日本語\r\n"),
            (LineEnding::Cr, "é\r日本語\r"),
        ] {
            let dir = TempDir::new();
            let original = dir.write("original", b"original disk bytes");
            let target = dir.0.join("new.txt");
            let mut app = TextApp::for_test();
            assert!(app.open_path(&original));
            dirty(&mut app, "é\n日本語\n");
            app.model.active_tab_mut().line_ending = ending;
            for response in [PortalResponse::Cancelled, PortalResponse::Success] {
                app.picker = picker::Picker::with_backend(Box::new(ScriptedBackend::replying(
                    response,
                    vec![format!("file://{}", target.display())],
                )));
                let _ = app.update(Message::SaveFileAs);
                let _ = crate::task_driver::update_driven(&mut app, Message::PickerTick);
                assert_eq!(std::fs::read(&original).unwrap(), b"original disk bytes");
                assert!(app.picker.failure.is_none());
                if response == PortalResponse::Cancelled {
                    assert!(app.model.active_tab().dirty);
                    assert!(!target.exists());
                    assert_eq!(app.model.active_tab().file_path.as_ref(), Some(&original));
                }
            }
            assert_eq!(app.model.active_tab().file_path.as_ref(), Some(&target));
            assert!(!app.model.active_tab().dirty);
            assert_eq!(std::fs::read(&target).unwrap(), encoded.as_bytes());
            dirty(&mut app, "next");
            let _ = crate::task_driver::update_driven(&mut app, Message::SaveFile);
            assert_eq!(std::fs::read(target).unwrap(), b"next");
            assert_eq!(std::fs::read(original).unwrap(), b"original disk bytes");
        }
    });
}

#[test]
fn external_replacement_is_not_overwritten_and_symlink_saves_keep_the_link() {
    crate::test_fixture::with_locale(|| {
        let dir = TempDir::new();
        let real = dir.write("real", b"original");
        let link = dir.0.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut app = TextApp::for_test();
        assert!(app.open_path(&link));
        dirty(&mut app, "through link");
        let _ = crate::task_driver::update_driven(&mut app, Message::SaveFile);
        assert!(!app.model.active_tab().dirty);
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "through link");
        dirty(&mut app, "unsaved draft");
        std::fs::write(&real, "external replacement").unwrap();
        app.model.active_tab_mut().externally_changed = true;
        let _ = crate::task_driver::update_driven(&mut app, Message::SaveFile);
        assert!(app.model.active_tab().dirty);
        assert_eq!(app.model.active_tab().snapshot(), "unsaved draft");
        assert_eq!(
            std::fs::read_to_string(real).unwrap(),
            "external replacement"
        );
    });
}

#[test]
fn a_save_before_the_watch_poll_keeps_external_revisions_until_acknowledged() {
    crate::test_fixture::with_locale(|| {
        let dir = TempDir::new();
        let path = dir.write("note", b"loaded revision");
        let mut app = TextApp::for_test();
        assert!(app.open_path(&path));
        let refused = |app: &mut TextApp, draft: &str, on_disk: Option<&str>| {
            app.picker.failure = None;
            let _ = crate::task_driver::update_driven(app, Message::SaveFile);
            let tab = app.model.active_tab();
            assert!(tab.dirty && tab.externally_changed);
            assert_eq!(tab.snapshot(), draft);
            assert!(app.picker.failure.is_some());
            assert_eq!(std::fs::read_to_string(&path).ok().as_deref(), on_disk);
        };

        dirty(&mut app, "draft one");
        let replacement = dir.write("replacement", b"replaced by another program");
        std::fs::rename(&replacement, &path).unwrap();
        refused(&mut app, "draft one", Some("replaced by another program"));
        refused(&mut app, "draft one", Some("replaced by another program"));
        let _ = crate::task_driver::update_driven(&mut app, Message::FileWatchDismiss);
        let _ = crate::task_driver::update_driven(&mut app, Message::SaveFile);
        assert!(!app.model.active_tab().dirty);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "draft one");

        dirty(&mut app, "draft two");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, b", then appended in place").unwrap();
        drop(file);
        refused(
            &mut app,
            "draft two",
            Some("draft one, then appended in place"),
        );
        let _ = crate::task_driver::update_driven(&mut app, Message::FileWatchDismiss);

        std::fs::remove_file(&path).unwrap();
        refused(&mut app, "draft two", None);
        let _ = crate::task_driver::update_driven(&mut app, Message::FileWatchDismiss);
        let _ = crate::task_driver::update_driven(&mut app, Message::SaveFile);
        assert!(!app.model.active_tab().dirty);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "draft two");
    });
}
