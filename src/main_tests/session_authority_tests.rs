use super::*;

fn settle_update(app: &mut TextApp, message: Message) {
    let task = app.update(message);
    assert_eq!(crate::task_driver::drive_tasks(app, task), (Vec::new(), 0));
    assert!(!app.session_write_inflight);
}

fn sandbox() -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    std::env::temp_dir().join(format!(
        "text-app-authority-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[test]
fn a_destination_replaced_under_the_store_is_not_overwritten() {
    crate::test_fixture::with_locale(|| {
        let dir = sandbox();
        std::fs::create_dir_all(&dir).expect("sandbox");
        let one = dir.join("one.txt");
        let two = dir.join("two.txt");
        std::fs::write(&one, "one").expect("one");
        std::fs::write(&two, "two").expect("two");
        let session = dir.join("session.list");

        let mut app = TextApp::test_with_session(settings::Settings::default(), session.clone());
        assert!(app.open_path(&one));
        settle_update(&mut app, Message::Resized(800.0, 600.0));
        assert!(
            std::fs::read_to_string(&session)
                .expect("first publication landed")
                .contains("one.txt")
        );

        std::fs::remove_file(&session).expect("unlink");
        std::fs::write(&session, "FOREIGN").expect("plant replacement");

        assert!(app.open_path(&two));
        settle_update(&mut app, Message::NoOp);
        assert_eq!(
            std::fs::read_to_string(&session).expect("replacement still present"),
            "FOREIGN"
        );
        let _ = std::fs::remove_dir_all(dir);
    });
}

#[test]
fn reducer_publish_over_bound_records_bounded_typed_debt_with_no_snapshot_and_a_fresh_projection_publishes()
 {
    crate::test_fixture::with_locale(|| {
        let dir = sandbox();
        std::fs::create_dir_all(&dir).expect("sandbox");
        let session = dir.join("session.list");
        let mut app = TextApp::test_with_session(settings::Settings::default(), session.clone());

        app.session_restore_request =
            Some(session::SessionRequest::for_path(session.clone(), None));

        let long = std::path::PathBuf::from(format!(
            "/{}",
            "a".repeat(session::MAX_SESSION_PATH_BYTES + 100)
        ));
        let tab_id = app.model.fresh_tab_id();
        let mut tab = model::Tab::new(tab_id.clone());
        tab.file_path = Some(long);
        app.model.add_tab(tab);

        let _task = app.update(Message::NoOp);

        assert!(!app.session_write_inflight);
        assert!(app.session_write_debt.is_none());
        assert!(!session.exists() || std::fs::metadata(&session).map_or(0, |m| m.len()) == 0);

        let over_bound = app
            .session_over_bound_debt
            .as_ref()
            .expect("over-bound preflight recorded a typed identity");
        assert_eq!(
            over_bound.revision_at_refusal,
            app.session_projection_revision
        );
        assert_eq!(over_bound.summary.tabs, 1);
        assert_eq!(
            over_bound.summary.reason,
            session::SessionRefusal::PathBudget
        );
        assert!(over_bound.summary.aggregate_path_bytes > session::MAX_SESSION_PATH_BYTES as u64);

        assert!(std::mem::size_of::<OverBoundReducerDebt>() <= 64);

        assert!(app.session_retry_blocked);
        let debt_kind = app
            .session_debt
            .as_ref()
            .map(session::SessionDebt::kind)
            .expect("banner debt is set from the preflight refusal");
        assert!(matches!(
            debt_kind,
            session::SessionDebtKind::PublishOverBound(_)
        ));

        let bounded = dir.join("kept.txt");
        if let Some(tab) = app.model.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.file_path = Some(bounded);
        }

        let task = app.update(Message::NewTab);
        for message in drive_local(task) {
            let follow_up = app.update(message);
            for cascade in drive_local(follow_up) {
                drop(app.update(cascade));
            }
        }

        assert!(app.session_over_bound_debt.is_none());
        assert!(app.session_debt.is_none());
        assert!(!app.session_retry_blocked);
        assert!(app.session_write_debt.is_none());
        let on_disk = std::fs::read_to_string(&session).expect("session written to disk");
        assert!(on_disk.contains("kept.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    });
}

fn drive_local(task: Task<Message>) -> Vec<Message> {
    let Some(stream) = iced_test::runtime::task::into_stream(task) else {
        return Vec::new();
    };
    iced_test::futures::futures::executor::block_on(async move {
        use iced_test::futures::futures::StreamExt;
        stream
            .filter_map(|action| async move {
                match action {
                    iced_test::runtime::Action::Output(message) => Some(message),
                    _ => None,
                }
            })
            .collect()
            .await
    })
}
