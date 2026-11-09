use std::collections::VecDeque;
use std::time::Duration;

use crate::{Message, TextApp};
use iced::Task;

pub(crate) fn drive_tasks(
    app: &mut TextApp,
    initial: Task<Message>,
) -> (Vec<iced::window::Id>, usize) {
    drive_tasks_observing(app, initial, |_, _| {})
}

pub(crate) fn update_driven(app: &mut TextApp, message: Message) -> (Vec<iced::window::Id>, usize) {
    let task = app.update(message);
    drive_tasks(app, task)
}

pub(crate) fn drive_tasks_observing(
    app: &mut TextApp,
    initial: Task<Message>,
    mut observe: impl FnMut(&TextApp, iced_test::runtime::Action<Message>),
) -> (Vec<iced::window::Id>, usize) {
    let mut tasks = VecDeque::from([initial]);
    let mut closes = Vec::new();
    let mut exits = 0;
    let mut turns = 0;
    while let Some(task) = tasks.pop_front() {
        turns += 1;
        assert!(turns <= 64, "task route exceeded its bounded turn budget");
        let Some(mut stream) = iced_test::runtime::task::into_stream(task) else {
            continue;
        };
        let mut messages = Vec::new();
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut actions = 0;
        loop {
            assert!(std::time::Instant::now() < deadline, "task did not settle");
            let action = match stream.as_mut().poll_next(&mut context) {
                std::task::Poll::Ready(Some(action)) => action,
                std::task::Poll::Ready(None) => break,
                std::task::Poll::Pending => {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
            };
            actions += 1;
            assert!(actions <= 64, "task exceeded its action budget");
            // Observers must service or release request-owned senders before the next poll.
            match action {
                iced_test::runtime::Action::Output(message) => messages.push(message),
                action => {
                    match &action {
                        iced_test::runtime::Action::Window(
                            iced_test::runtime::window::Action::Close(window),
                        ) => closes.push(*window),
                        iced_test::runtime::Action::Exit => exits += 1,
                        _ => {}
                    }
                    observe(app, action);
                }
            }
        }
        for message in messages {
            tasks.push_back(app.update(message));
        }
    }
    (closes, exits)
}
