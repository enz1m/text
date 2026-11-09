//! Desktop activation and copy-only native file drops share the bounded open route.

use enzui_core::activation::ApplicationIntent;
use enzui_widgets::file_drop::{DragAction, FileDrop};
use iced::{Subscription, Task};
use std::collections::VecDeque;
use std::path::PathBuf;

use crate::app_instance::AppInstance;
use crate::desktop_search::{SearchChoice, SearchIndex};
use crate::{Message, TextApp};

pub(crate) type DropKey = (String, crate::large_doc::LoadToken);

const MAX_PENDING_FILES: usize = 64;

#[derive(Clone)]
pub(crate) enum DesktopEvent {
    Wake,
    WindowOpened(iced::window::Id),
    WindowClosed(iced::window::Id),
    WindowTagged(iced::window::Id, String, bool),
    Dropped {
        key: DropKey,
        files: FileDrop,
    },
    Retained {
        key: DropKey,
        files: FileDrop,
        copies: Result<enzui_core::drop_files::CopiedFiles, String>,
    },
    Paths(Vec<PathBuf>),
}

impl std::fmt::Debug for DesktopEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Wake => "Wake",
            Self::WindowOpened(_) => "WindowOpened",
            Self::WindowClosed(_) => "WindowClosed",
            Self::WindowTagged(..) => "WindowTagged",
            Self::Dropped { .. } => "Dropped",
            Self::Retained { .. } => "Retained",
            Self::Paths(_) => "Paths",
        })
    }
}

pub(crate) struct Desktop {
    pub(crate) instance: AppInstance,
    search: SearchIndex,
    home: Option<PathBuf>,
    pub(crate) activity: Option<enzui_activity::Publisher>,
    activity_job: Option<enzui_activity::Job>,
    activity_pending_tag: Option<String>,
    activity_token: Option<String>,
    window: Option<iced::window::Id>,
    pending: VecDeque<PathBuf>,
    drops: Vec<PendingDrop>,
}

struct PendingDrop {
    receipt: Option<FileDrop>,
    copies: Option<enzui_core::drop_files::CopiedFiles>,
    targets: Vec<(String, crate::large_doc::LoadToken, PathBuf)>,
}

impl Default for Desktop {
    fn default() -> Self {
        Self {
            instance: AppInstance::standalone(),
            search: SearchIndex::default(),
            home: std::env::var_os("HOME").map(PathBuf::from),
            activity: None,
            activity_job: None,
            activity_pending_tag: None,
            activity_token: None,
            window: None,
            pending: VecDeque::new(),
            drops: Vec::new(),
        }
    }
}

impl Desktop {
    #[cfg(test)]
    pub(crate) fn search_ids(&self, text: &str) -> Vec<String> {
        self.search.ids(text)
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        self.instance
            .subscription(|_| Message::Desktop(DesktopEvent::Wake))
    }
}

pub(crate) fn local_paths(intent: &ApplicationIntent) -> Vec<PathBuf> {
    match intent {
        ApplicationIntent::Open(uris) => uris
            .iter()
            .filter_map(|uri| enzui_clipboard::payload::validate_file_uri(uri).ok())
            .collect(),
        _ => Vec::new(),
    }
}

impl TextApp {
    fn admit_desktop_drop(
        &mut self,
        receipt: FileDrop,
        paths: Vec<PathBuf>,
        copies: Option<enzui_core::drop_files::CopiedFiles>,
    ) -> Task<Message> {
        let pending = self
            .desktop
            .drops
            .iter()
            .map(|drop| drop.targets.len())
            .sum::<usize>();
        if self.desktop_input_blocked() || paths.is_empty() {
            return Task::none();
        }
        if paths.len() > MAX_PENDING_FILES.saturating_sub(pending) {
            self.picker.failure = Some(enzui_core::localization::text(
                "text-too-many-files-were-offered-open-the-d6f7a9",
            ));
            return Task::none();
        }
        let mut targets = Vec::new();
        let mut tasks = Vec::new();
        for path in &paths {
            tasks.push(self.begin_open(path, None));
            let Some(tab) = self
                .model
                .tabs
                .iter()
                .find(|tab| tab.file_path.as_ref() == Some(path))
            else {
                return Task::batch(tasks);
            };
            targets.push((tab.id.clone(), tab.load_token, path.clone()));
        }
        // The source keeps its files until every admitted load owns its document.
        self.desktop.drops.push(PendingDrop {
            receipt: Some(receipt),
            copies,
            targets,
        });
        Task::batch(tasks)
    }

    // Answers the desktop's Search from the documents this instance has open.
    pub(crate) fn serve_desktop_search(&mut self) {
        self.refresh_desktop_search();
        let provider = self.desktop.search.provider();
        if let Err(error) = self.desktop.instance.serve_search(provider) {
            log::warn!("Search provider unavailable: {error}");
        }
    }

    // A chosen document takes the forwarded-launch route, which focuses its
    // tab when it is open; Find opens the bar on the active document.
    pub(crate) fn apply_search_choice(&mut self, choice: SearchChoice) -> Task<Message> {
        match choice {
            SearchChoice::Open(path) => {
                self.queue_desktop_paths([path]);
                Task::none()
            }
            SearchChoice::Find(query) => {
                let open = self.reduce_find_open_find();
                Task::batch([open, self.reduce_find_find_query_changed(query)])
            }
        }
    }

    pub(crate) fn refresh_desktop_search(&mut self) {
        let home = self.desktop.home.as_deref();
        self.desktop
            .search
            .refresh(&self.model.tabs, &self.model.active_tab_id, home);
    }

    pub(crate) fn desktop_input_blocked(&self) -> bool {
        self.a_dialog_is_open() || self.settings_is_closing() || self.picker.is_pending()
    }

    pub(crate) fn sync_desktop_activity(&mut self) {
        let busy = self.blocking_tasks.dispatcher().snapshot().live() > 0;
        if busy && self.desktop.activity_job.is_none() {
            self.desktop.activity_job = self.desktop.activity.as_ref().and_then(|publisher| {
                publisher
                    .start_after(
                        enzui_activity::Status {
                            window_token: self.desktop.activity_token.clone(),
                            ..enzui_activity::Status::running(enzui_core::localization::text(
                                "text-processing-documents-8ace85",
                            ))
                        },
                        enzui_activity::Visibility::WhenUnfocused,
                        std::time::Duration::from_millis(800),
                    )
                    .ok()
            });
        } else if !busy {
            // The worker aggregate has no single result; withdraw it without inventing success.
            self.desktop.activity_job = None;
        }
    }

    pub(crate) fn queue_desktop_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if self.desktop.pending.len() == MAX_PENDING_FILES {
                self.picker.failure = Some(enzui_core::localization::text(
                    "text-too-many-files-were-offered-open-the-d6f7a9",
                ));
                break;
            }
            self.desktop.pending.push_back(path);
        }
    }

    pub(crate) fn flush_desktop_opens(&mut self) -> Task<Message> {
        if self.desktop_input_blocked() {
            return Task::none();
        }
        let paths = self.desktop.pending.drain(..).collect::<Vec<_>>();
        Task::batch(paths.into_iter().map(|path| self.begin_open(&path, None)))
    }

    pub(crate) fn settle_desktop_drops(&mut self) {
        self.desktop.drops.retain_mut(|pending| {
            let mut loading = false;
            for (id, token, path) in &pending.targets {
                let Some(tab) = self.model.tabs.iter().find(|tab| {
                    &tab.id == id
                        && &tab.load_token == token
                        && tab.file_path.as_ref() == Some(path)
                }) else {
                    return false;
                };
                loading |= tab.loading;
            }
            if loading {
                return true;
            }
            if let Some(copies) = &pending.copies {
                copies.preserve();
            }
            if let Some(receipt) = pending.receipt.take() {
                receipt.finish();
            }
            false
        });
    }

    pub(crate) fn reduce_desktop(&mut self, event: DesktopEvent) -> Task<Message> {
        match event {
            DesktopEvent::Wake => {
                let drained = self.desktop.instance.drain(self.desktop.window);
                let mut tasks: Vec<Task<Message>> = drained
                    .searches
                    .into_iter()
                    .map(|request| self.apply_search_choice(crate::desktop_search::choice(request)))
                    .collect();
                tasks.push(drained.raise);
                let raise = Task::batch(tasks);
                for intent in drained.launches {
                    let paths = local_paths(&intent);
                    if let ApplicationIntent::Open(uris) = &intent
                        && paths.len() != uris.len()
                    {
                        self.picker.failure = Some(enzui_core::localization::text(
                            "text-only-local-files-can-be-opened-in-4f3054",
                        ));
                    }
                    self.queue_desktop_paths(paths);
                }
                raise
            }
            DesktopEvent::WindowOpened(window) => {
                if self.desktop.window.is_none() {
                    self.desktop.window = Some(window);
                    self.picker.set_window(Some(window));
                    self.desktop.activity_token = None;
                    self.desktop.activity_pending_tag = enzui_activity::new_window_token().ok();
                    if let Some(tag) = self.desktop.activity_pending_tag.clone() {
                        return iced::window::set_toplevel_tag(window, tag.clone()).map(
                            move |queued| {
                                Message::Desktop(DesktopEvent::WindowTagged(
                                    window,
                                    tag.clone(),
                                    queued,
                                ))
                            },
                        );
                    }
                }
                Task::none()
            }
            DesktopEvent::WindowTagged(window, tag, queued) => {
                if self.desktop.window == Some(window)
                    && self.desktop.activity_pending_tag.as_deref() == Some(&tag)
                {
                    self.desktop.activity_token = queued.then_some(tag);
                    self.desktop.activity_pending_tag = None;
                    if let Some(job) = &self.desktop.activity_job {
                        let _ = job.set_window_token(self.desktop.activity_token.clone());
                    }
                }
                Task::none()
            }
            DesktopEvent::WindowClosed(window) => {
                self.picker.parent_closed(window);
                if self.desktop.window == Some(window) {
                    self.print.abort_for_close();
                    self.print_retry = None;
                    self.print_preparing = false;
                    self.desktop.window = None;
                    self.desktop.activity_token = None;
                    self.desktop.activity_pending_tag = None;
                    if let Some(job) = &self.desktop.activity_job {
                        let _ = job.set_window_token(None);
                    }
                }
                Task::none()
            }
            DesktopEvent::Paths(paths) => {
                self.queue_desktop_paths(paths);
                Task::none()
            }
            DesktopEvent::Retained { key, files, copies } => {
                if key != self.desktop_drop_key() || self.desktop_input_blocked() {
                    return Task::none();
                }
                match copies {
                    Ok(copies) => {
                        self.admit_desktop_drop(files, copies.paths().to_vec(), Some(copies))
                    }
                    Err(error) => {
                        self.picker.failure = Some(error);
                        Task::none()
                    }
                }
            }
            DesktopEvent::Dropped { key, files } => {
                if key != self.desktop_drop_key()
                    || self.desktop_input_blocked()
                    || files
                        .action()
                        .is_some_and(|action| action != DragAction::Copy)
                {
                    return Task::none();
                }
                let paths = files.paths().to_vec();
                if files.is_temporary() {
                    let Some(storage) = enzui_core::persist::XdgResolver::from_env()
                        .resolve(
                            enzui_core::persist::PersistenceKind::State,
                            "text-app",
                            "imports",
                        )
                        .ok()
                        .flatten()
                    else {
                        return Task::none();
                    };
                    let task = self.blocking_tasks.submit(0, move |_| {
                        enzui_core::drop_files::copy_files(
                            &paths,
                            Some(&storage),
                            256 * 1024 * 1024,
                            &enzui_core::cancellation::CancelToken::new(),
                        )
                        .map_err(|error| error.to_string())
                    });
                    return Task::perform(task, move |settlement| {
                        Message::Desktop(DesktopEvent::Retained {
                            key,
                            files,
                            copies: settlement
                                .into_result()
                                .map_err(|error| error.to_string())
                                .and_then(std::convert::identity),
                        })
                    });
                }
                self.admit_desktop_drop(files, paths, None)
            }
        }
    }

    pub(crate) fn desktop_drop_key(&self) -> DropKey {
        let tab = self.model.active_tab();
        (tab.id.clone(), tab.load_token)
    }
}
