use super::*;
use iced::advanced::widget::Operation;
use iced::advanced::widget::operation::Outcome;
use iced_test::core::mouse;
use iced_test::core::shell::Waker;
use iced_test::core::window;
use iced_test::runtime::Action;
use iced_test::runtime::user_interface::{Cache, UserInterface};

fn new_renderer() -> iced_test::renderer::Renderer {
    use iced::advanced::renderer::Headless;
    iced_test::futures::futures::executor::block_on(
        <iced_test::renderer::Renderer as Headless>::new(
            iced::advanced::renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer")
}

fn widget_ops(task: Task<Message>) -> Vec<Box<dyn Operation>> {
    match iced_test::runtime::task::into_stream(task) {
        Some(stream) => {
            let actions = iced_test::futures::futures::executor::block_on(async move {
                use iced_test::futures::futures::StreamExt;
                stream.take(1).collect::<Vec<_>>().await
            });
            actions
                .into_iter()
                .filter_map(|a| match a {
                    Action::Widget(op) => Some(op),
                    _ => None,
                })
                .collect()
        }
        None => Vec::new(),
    }
}

type Ui<'a> = UserInterface<'a, Message, iced::Theme, iced_test::renderer::Renderer>;

fn run_op(ui: &mut Ui<'_>, renderer: &iced_test::renderer::Renderer, mut op: Box<dyn Operation>) {
    loop {
        ui.operate(renderer, {
            let o: &mut dyn Operation = op.as_mut();
            o
        });
        match op.finish() {
            Outcome::Chain(next) => op = next,
            _ => break,
        }
    }
}

pub(crate) struct Driver {
    renderer: iced_test::renderer::Renderer,
    cache: Cache,
    size: iced::Size,
    cursor: mouse::Cursor,
}

impl Driver {
    pub(crate) fn new() -> Self {
        Driver {
            renderer: new_renderer(),
            cache: Cache::default(),
            size: iced::Size::new(1400.0, 900.0),
            cursor: mouse::Cursor::Unavailable,
        }
    }

    pub(crate) fn point_at(&mut self, p: iced::Point) {
        self.cursor = mouse::Cursor::Available(p);
    }

    pub(crate) fn apply(&mut self, app: &mut TextApp, messages: Vec<Message>) {
        let mut ops: Vec<Box<dyn Operation>> = Vec::new();
        for m in messages {
            ops.extend(widget_ops(app.update(m)));
        }
        if !ops.is_empty() {
            self.operate(app, ops);
        }
    }

    fn operate(&mut self, app: &TextApp, ops: impl IntoIterator<Item = Box<dyn Operation>>) {
        let cache = std::mem::take(&mut self.cache);
        let mut ui = UserInterface::build(app.view(), self.size, cache, &mut self.renderer);
        for op in ops {
            run_op(&mut ui, &self.renderer, op);
        }
        self.cache = ui.into_cache();
    }

    pub(crate) fn feed(&mut self, app: &mut TextApp, events: Vec<iced::Event>) {
        let classifier = app.event_classifier();
        let mut messages: iced::advanced::shell::Bus<Message> = iced::advanced::shell::Bus::new();
        let statuses;
        {
            let cache = std::mem::take(&mut self.cache);
            let mut ui = UserInterface::build(app.view(), self.size, cache, &mut self.renderer);
            let (_, observed) = ui.update(
                &window::Headless,
                &Waker::noop(),
                &events,
                self.cursor,
                &mut self.renderer,
                &mut messages,
            );
            statuses = observed;
            self.cache = ui.into_cache();
        }

        messages.extend(
            events
                .into_iter()
                .zip(statuses)
                .filter_map(|(event, status)| {
                    classifier(event, status, iced::window::Id::unique())
                }),
        );
        self.apply(app, messages.into_iter().collect());
    }

    pub(crate) fn click_at(&mut self, app: &mut TextApp, p: iced::Point) {
        self.point_at(p);

        for event in iced_test::simulator::click() {
            self.feed(app, vec![event]);
        }
    }

    pub(crate) fn edit_menu_action(&mut self, app: &mut TextApp, key: &str) {
        self.point_at(EDITOR_POINT);
        for event in [
            mouse::Event::ButtonPressed(mouse::Button::Right),
            mouse::Event::ButtonReleased(mouse::Button::Right),
        ] {
            self.feed(app, vec![iced::Event::Mouse(event)]);
        }
        let id = enzui_core::identity::derive_id("enzui-edit-menu", key);
        let point = self
            .nodes(app)
            .into_iter()
            .find(|node| node.id.as_ref() == Some(&id))
            .expect("native editor menu command")
            .bounds
            .center();
        self.click_at(app, point);
    }

    pub(crate) fn layout(&mut self, app: &TextApp) {
        use iced::advanced::graphics::core::theme::Base;
        let cache = std::mem::take(&mut self.cache);
        let mut ui = UserInterface::build(app.view(), self.size, cache, &mut self.renderer);
        let theme = app.themes.theme().iced_theme();
        let base = theme.base();
        ui.draw(
            &mut self.renderer,
            &theme,
            &iced::advanced::renderer::Style {
                text_color: base.text_color,
            },
            self.cursor,
        );
        self.cache = ui.into_cache();
    }

    pub(crate) fn nodes(&mut self, app: &TextApp) -> Vec<enzui_core::collect::CollectedNode> {
        use enzui_core::collect::SemanticsCollector;
        let cache = std::mem::take(&mut self.cache);
        let mut ui = UserInterface::build(app.view(), self.size, cache, &mut self.renderer);
        let mut collector = SemanticsCollector::new();
        ui.operate(&self.renderer, {
            let op: &mut dyn Operation = &mut collector;
            op
        });
        self.cache = ui.into_cache();
        collector.into_collected()
    }
}

const EDITOR_POINT: iced::Point = iced::Point::new(700.0, 450.0);

fn drive_boot(driver: &mut Driver, app: &mut TextApp, task: Task<Message>) {
    crate::task_driver::drive_tasks_observing(app, task, |app, action| match action {
        Action::Widget(op) => driver.operate(app, [op]),
        Action::Window(iced_test::runtime::window::Action::SetToplevelTag(_, _, result)) => {
            let _ = result.send(false);
        }
        _ => {}
    });
}

#[test]
fn explicit_startup_file_accepts_keyboard_input_after_load() {
    let dir = crate::test_support::TempDir::new();
    let path = dir.write("startup.txt", b"fixture");
    let env = BootEnv::from_parts(None, None, None, [path.into_os_string()].into_iter(), None);
    for window_before_load in [false, true] {
        let (mut app, boot) = TextApp::boot_at(&env);
        let mut driver = Driver::new();
        let window = Message::Desktop(desktop::DesktopEvent::WindowOpened(window::Id::unique()));
        if window_before_load {
            let opened = app.update(window.clone());
            drive_boot(&mut driver, &mut app, opened);
        }
        drive_boot(&mut driver, &mut app, boot);
        if !window_before_load {
            let opened = app.update(window);
            drive_boot(&mut driver, &mut app, opened);
        }

        assert!(driver.nodes(&app).iter().any(|node| {
            node.id.as_ref() == Some(&editor_id())
                && node.semantics.role == enzui_core::semantics::Role::TextInput
                && node.semantics.focused
        }));
        driver.feed(&mut app, iced_test::simulator::typewrite("x").collect());
        assert_eq!(app.model.active_tab().document().to_string(), "xfixture");
    }
}

#[test]
fn startup_load_respects_the_open_find_field() {
    let dir = crate::test_support::TempDir::new();
    let path = dir.write("startup.txt", b"fixture");
    let env = BootEnv::from_parts(None, None, None, [path.into_os_string()].into_iter(), None);
    let (mut app, boot) = TextApp::boot_at(&env);
    let mut driver = Driver::new();
    let opened = app.update(Message::Desktop(desktop::DesktopEvent::WindowOpened(
        window::Id::unique(),
    )));
    drive_boot(&mut driver, &mut app, opened);
    driver.apply(&mut app, vec![Message::OpenFind]);
    drive_boot(&mut driver, &mut app, boot);

    driver.feed(&mut app, iced_test::simulator::typewrite("x").collect());
    assert_eq!(app.model.search.query, "x");
    assert_eq!(app.model.active_tab().document().to_string(), "fixture");
}
