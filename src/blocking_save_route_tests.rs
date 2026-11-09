use crate::test_support::TempDir;
use crate::{Message, TextApp};
use iced_test::futures::futures::{executor::block_on, future};
use std::cell::Cell;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

const BOUND: Duration = Duration::from_secs(2);

struct PublicationHold {
    release: mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
    expired: Arc<AtomicBool>,
}

impl PublicationHold {
    fn new(root: &Path) -> Self {
        let parent = File::open(root).expect("open private publication parent");
        parent.lock().expect("hold publication lock");
        let (release, released) = mpsc::channel();
        let expired = Arc::new(AtomicBool::new(false));
        let timeout = Arc::clone(&expired);
        let worker = std::thread::spawn(move || {
            if matches!(
                released.recv_timeout(BOUND),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                timeout.store(true, Ordering::SeqCst);
            }
            parent.unlock().expect("release publication lock");
        });
        Self {
            release,
            worker: Some(worker),
            expired,
        }
    }

    fn release(&mut self) {
        let _ = self.release.send(());
        if let Some(worker) = self.worker.take() {
            worker.join().expect("publication watchdog");
        }
    }

    fn assert_live(&self) {
        assert!(!self.expired.load(Ordering::SeqCst));
    }
}

impl Drop for PublicationHold {
    fn drop(&mut self) {
        self.release();
    }
}

fn has_staged_bytes(root: &Path, expected: &[u8]) -> bool {
    std::fs::read_dir("/proc/self/fd")
        .expect("inspect own descriptors")
        .filter_map(Result::ok)
        .any(|entry| {
            let Ok(target) = std::fs::read_link(entry.path()) else {
                return false;
            };
            if target.parent() != Some(root) {
                return false;
            }
            let Ok(before) = std::fs::metadata(entry.path()) else {
                return false;
            };
            if !before.is_file() || before.nlink() != 0 || before.len() != expected.len() as u64 {
                return false;
            }
            let Ok(file) = File::open(entry.path()) else {
                return false;
            };
            let Ok(held) = file.metadata() else {
                return false;
            };

            if (before.dev(), before.ino()) != (held.dev(), held.ino()) {
                return false;
            }
            let mut bytes = Vec::new();
            file.take(expected.len() as u64 + 1)
                .read_to_end(&mut bytes)
                .expect("read owned stage");
            bytes == expected
        })
}

async fn outputs(task: iced::Task<Message>, saved: Rc<Cell<bool>>) -> Vec<Message> {
    let Some(mut stream) = iced_test::runtime::task::into_stream(task) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    let deadline = Instant::now() + BOUND;
    let mut actions = 0;
    while let Some(action) = future::poll_fn(|context| {
        assert!(Instant::now() < deadline);
        let next = stream.as_mut().poll_next(context);
        if next.is_pending() {
            context.waker().wake_by_ref();
        }
        next
    })
    .await
    {
        actions += 1;
        assert!(actions <= 64);
        if let iced_test::runtime::Action::Output(message) = action {
            if matches!(&message, Message::SaveCompleted { .. }) {
                saved.set(true);
            }
            messages.push(message);
        }
    }
    messages
}

#[test]
fn real_save_stays_pending_while_same_executor_updates_the_app() {
    crate::test_fixture::with_locale(|| {
        let scratch = TempDir::new();
        let path = scratch.0.join("document.txt");
        let old = "original on disk";
        let new = "edited text ♥\nsecond line\n";
        std::fs::write(&path, old).expect("seed document");
        let mut app = TextApp::for_test();
        assert!(app.open_path(&path));
        let tab = app.model.active_tab_id.clone();
        app.model.active_tab_mut().set_snapshot(new);
        app.model.active_tab_mut().dirty = true;
        let generation = app.model.active_tab().snapshot_version();
        assert_eq!(app.blocking_tasks.dispatcher().snapshot().live(), 0);
        let before_tabs = app.model.tabs.len();
        let polling_thread = std::thread::current().id();
        let saved = Rc::new(Cell::new(false));
        let mut hold = PublicationHold::new(&scratch.0);
        let task = app.update(Message::SaveFile);
        hold.assert_live();
        let saving = outputs(task, Rc::clone(&saved));
        let progress = async {
            let deadline = Instant::now() + BOUND;
            future::poll_fn(|context| {
                hold.assert_live();
                assert!(Instant::now() < deadline);
                assert!(!saved.get());
                if has_staged_bytes(&scratch.0, new.as_bytes()) {
                    std::task::Poll::Ready(())
                } else {
                    context.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
            })
            .await;
            assert_eq!(std::thread::current().id(), polling_thread);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
            assert_eq!(app.blocking_tasks.dispatcher().snapshot().running, 1);
            let task = app.update(Message::NewTab);
            assert_eq!(app.model.tabs.len(), before_tabs + 1);
            let messages = outputs(task, Rc::new(Cell::new(false))).await;
            assert!(!saved.get());
            hold.assert_live();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
            hold.release();
            messages
        };
        let (saved_messages, progress_messages) = block_on(future::join(saving, progress));
        hold.assert_live();
        assert!(saved.get());
        let completions = saved_messages
            .iter()
            .filter(|message| matches!(message, Message::SaveCompleted { .. }))
            .count();
        assert_eq!(completions, 1);
        for message in &saved_messages {
            if let Message::SaveCompleted {
                tab: result_tab,
                generation: result_generation,
                destination,
                result,
                ..
            } = message
            {
                assert_eq!(result_tab, &tab);
                assert_eq!(*result_generation, generation);
                assert_eq!(destination, &path);
                assert!(result.is_ok());
            }
        }
        for message in saved_messages.into_iter().chain(progress_messages) {
            let task = app.update(message);
            assert_eq!(
                crate::task_driver::drive_tasks(&mut app, task),
                (Vec::new(), 0)
            );
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), new);
        let saved_tab = app
            .model
            .tabs
            .iter()
            .find(|candidate| candidate.id == tab)
            .unwrap();
        assert!(!saved_tab.dirty && !saved_tab.saving);
    });
}
