use super::*;
use enzui_core::activation::ApplicationIntent;

#[test]
fn desktop_uri_intents_preserve_local_names_and_reject_remote_or_malformed_uris() {
    crate::test_fixture::with_locale(|| {
        let paths = desktop::local_paths(&ApplicationIntent::Open(vec![
            "file:///tmp/Text%20Gr%C3%BC%C3%9Fe.txt".into(),
            "https://example.org/private".into(),
            "file://remote/tmp/file.txt".into(),
            "file:///tmp/bad%00name".into(),
        ]));
        assert_eq!(paths, vec![std::path::PathBuf::from("/tmp/Text Grüße.txt")]);
        use std::os::unix::ffi::OsStringExt;
        let name = std::ffi::OsString::from_vec(b"native-\xff.txt".to_vec());
        let request =
            enzui_core::activation::launch_request([name.clone()], std::path::Path::new("/tmp"));
        assert_eq!(
            desktop::local_paths(&request.intent),
            vec![std::path::Path::new("/tmp").join(name)]
        );
    });
}

#[test]
fn desktop_open_waits_for_modal_completion_without_changing_dirty_text() {
    crate::test_fixture::with_locale(|| {
        let mut app = TextApp::for_test();
        let _ = app.update(Message::EditorAction(text_editor::Action::Edit(
            text_editor::Edit::Paste(std::sync::Arc::new("unsaved text".into())),
        )));
        let before = app.model.active_tab().snapshot();
        let tabs = app.model.tabs.len();
        let _ = app.update(Message::OpenSettings);
        app.queue_desktop_paths([std::path::PathBuf::from("/nonexistent/activation.txt")]);
        let _ = app.flush_desktop_opens();
        assert_eq!(app.model.tabs.len(), tabs);
        assert_eq!(app.model.active_tab().snapshot(), before);
        assert!(app.model.active_tab().dirty);
        let _ = app.update(Message::Desktop(desktop::DesktopEvent::Paths(vec![
            std::path::PathBuf::from("/nonexistent/dropped.txt"),
        ])));
        assert_eq!(app.model.active_tab().snapshot(), before);
        assert_eq!(app.model.tabs.len(), tabs);
        let _ = app.update(Message::CloseSettings);
        assert_eq!(app.model.tabs.len(), tabs);
        if let Some(key) = app.settings_presentation_key() {
            let _ = app.update(Message::SettingsExitSettled(key));
        }
        assert_eq!(app.model.tabs.len(), tabs + 2);
        assert!(
            app.model
                .tabs
                .iter()
                .any(|tab| tab.snapshot() == before && tab.dirty)
        );
    });
}

#[test]
fn a_desktop_search_choice_focuses_reopens_or_finds_in_the_document() {
    crate::test_fixture::with_locale(|| {
        let dir = crate::test_support::TempDir::new();
        let first = dir.write("first plan.txt", b"alpha needle omega");
        let second = dir.write("second.txt", b"second");
        let mut app = TextApp::for_test();
        assert!(app.open_path(&first));
        assert!(app.open_path(&second));
        let _ = app.update(Message::NoOp);
        let search = |app: &TextApp, text: &str| app.desktop.search_ids(text);
        assert_eq!(search(&app, "PLAN"), [first.to_str().unwrap().to_owned()]);

        let open = |app: &mut TextApp, id: &str| {
            let task = app.apply_search_choice(crate::desktop_search::choice(
                enzui_core::activation::dbus::search::SearchRequest::Open {
                    id: id.to_owned(),
                    query: "plan".into(),
                },
            ));
            let _ = crate::task_driver::drive_tasks(app, task);
            let task = app.update(Message::NoOp);
            let _ = crate::task_driver::drive_tasks(app, task);
        };
        let tabs = app.model.tabs.len();
        open(&mut app, first.to_str().unwrap());
        assert_eq!(app.model.tabs.len(), tabs);
        assert_eq!(
            app.model.active_tab().file_path.as_deref(),
            Some(first.as_path())
        );

        let closing = app.model.active_tab_id.clone();
        let _ = app.update(Message::CloseTab(closing));
        assert!(search(&app, "plan").is_empty());
        open(&mut app, first.to_str().unwrap());
        assert_eq!(
            app.model.active_tab().file_path.as_deref(),
            Some(first.as_path())
        );
        assert_eq!(app.model.active_tab().snapshot(), "alpha needle omega");
        assert_eq!(search(&app, "plan"), [first.to_str().unwrap().to_owned()]);

        let task = app.apply_search_choice(crate::desktop_search::choice(
            enzui_core::activation::dbus::search::SearchRequest::Search {
                query: "needle".into(),
            },
        ));
        let _ = crate::task_driver::drive_tasks(&mut app, task);
        assert!(app.model.search.open);
        assert_eq!(app.model.search.query, "needle");
    });
}
