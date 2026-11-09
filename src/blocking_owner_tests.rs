use crate::task_driver::drive_tasks;
use std::time::Duration;

use crate::{Message, TextApp};

struct ReleaseOnDrop(Option<std::sync::mpsc::SyncSender<()>>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.try_send(());
        }
    }
}

struct LiveJob {
    release: ReleaseOnDrop,
    receipt: Option<enzui_core::blocking_task::BlockingTask<()>>,
    expired: std::sync::Arc<std::sync::atomic::AtomicBool>,
    dispatcher: enzui_core::blocking_task::BlockingTaskDispatcher,
}

impl LiveJob {
    fn new(app: &TextApp) -> Self {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        use std::time::Instant;

        let dispatcher = app.blocking_tasks.dispatcher();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let mut held = Self {
            release: ReleaseOnDrop(Some(release_tx)),
            receipt: None,
            expired: Arc::new(AtomicBool::new(false)),
            dispatcher,
        };
        let expired = std::sync::Arc::clone(&held.expired);
        let deadline = Instant::now() + Duration::from_secs(2);
        held.receipt = Some(held.dispatcher.submit(9001, move |_| {
            let _ = ready_tx.try_send(());
            if release_rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_err()
            {
                expired.store(true, Ordering::SeqCst);
            }
        }));
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("held worker starts");
        assert_eq!(held.dispatcher.snapshot().running, 1);
        held
    }

    fn finish(mut self) {
        self.release
            .0
            .take()
            .unwrap()
            .try_send(())
            .expect("worker still held");
        if let Some(task) = self.receipt.take() {
            assert_eq!(
                iced_test::futures::futures::executor::block_on(task).into_result(),
                Ok(())
            );
        }
        wait_for_empty(&self.dispatcher);
        assert!(!self.expired.load(std::sync::atomic::Ordering::SeqCst));
    }
}

fn wait_for_empty(dispatcher: &enzui_core::blocking_task::BlockingTaskDispatcher) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while dispatcher.snapshot().live() != 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(dispatcher.snapshot().live(), 0);
}

#[test]
fn close_waits_for_live_owner_before_emitting_the_exact_close_and_exit() {
    crate::test_fixture::with_locale(|| {
        for receipt_state in ["retained", "dropped", "cancelled"] {
            let mut app = TextApp::for_test();
            let mut held = LiveJob::new(&app);
            let dispatcher = held.dispatcher.clone();
            match receipt_state {
                "dropped" => drop(held.receipt.take()),
                "cancelled" => {
                    held.receipt.as_ref().unwrap().cancellation().cancel();
                    let settlement = iced_test::futures::futures::executor::block_on(
                        held.receipt.take().unwrap(),
                    );
                    assert_eq!(
                        settlement.into_result(),
                        Err(enzui_core::blocking_task::BlockingTaskError::Cancelled)
                    );
                }
                _ => {}
            }
            assert_eq!(dispatcher.snapshot().live(), 1);
            let window = iced::window::Id::unique();
            let close = app.update(Message::CloseRequested(
                enzui_core::close_guard::CloseRequest::for_window(window),
            ));
            let (closes, exits) = drive_tasks(&mut app, close);
            assert!(closes.is_empty());
            assert_eq!(exits, 0);
            assert!(app.close_drain.is_some());
            assert_eq!(dispatcher.snapshot().live(), 1);
            let repeat = app.update(Message::CloseRequested(
                enzui_core::close_guard::CloseRequest::for_window(iced::window::Id::unique()),
            ));
            assert_eq!(drive_tasks(&mut app, repeat), (Vec::new(), 0));
            assert_eq!(app.close_drain.unwrap().window, window);

            held.finish();
            let tick = app.update(Message::CloseDrainTick);
            assert_eq!(drive_tasks(&mut app, tick), (vec![window], 1));
            assert!(app.close_drain.is_none());
            let late = app.update(Message::CloseDrainTick);
            assert_eq!(drive_tasks(&mut app, late), (Vec::new(), 0));
        }
    });
}

#[test]
fn live_owner_deadline_veto_requires_an_explicit_new_close_request() {
    crate::test_fixture::with_locale(|| {
        let mut app = TextApp::for_test();
        let held = LiveJob::new(&app);
        let first = iced::window::Id::unique();
        let close = app.update(Message::CloseRequested(
            enzui_core::close_guard::CloseRequest::for_window(first),
        ));
        assert_eq!(drive_tasks(&mut app, close), (Vec::new(), 0));
        app.close_drain.as_mut().unwrap().deadline = std::time::Instant::now();
        let tick = app.update(Message::CloseDrainTick);
        assert_eq!(drive_tasks(&mut app, tick), (Vec::new(), 0));
        assert!(app.close_drain.is_none());
        assert!(app.blocking_retry_suspended);
        assert_eq!(
            app.picker.failure.as_deref(),
            Some("Close was cancelled because durable work did not settle. Retry Close.")
        );
        assert_eq!(held.dispatcher.snapshot().live(), 1);
        held.finish();
        let late = app.update(Message::CloseDrainTick);
        assert_eq!(drive_tasks(&mut app, late), (Vec::new(), 0));
        let retry_window = iced::window::Id::unique();
        let retry = app.update(Message::CloseRequested(
            enzui_core::close_guard::CloseRequest::for_window(retry_window),
        ));
        assert_eq!(drive_tasks(&mut app, retry), (vec![retry_window], 1));
    });
}
