//! File choosers, OpenLocation and shared request-lifetime portal parenting.
//!
//! Parented requests retain their export until settlement; stale exports are refused.

use crate::portal_parent::{ParentHandoff, ParentLease};
use enzui_core::portal::open_uri::{OpenUri, OpenUriBackend, OpenUriRequest, PendingOpenUri};
#[cfg(test)]
use enzui_widgets::file_chooser::PortalBackend;
use enzui_widgets::file_chooser::{
    CancelReason, ChooserOptions, ChooserSession, FallbackEvent, FileFilter, FileResult,
    PortalOperation, PortalOutcome, PortalParent, PortalWake,
};

pub(crate) const DIALOG_TITLE: &str = "Open a text file";

pub(crate) const SAVE_AS_DIALOG_TITLE: &str = "Save As";

pub(crate) const WORKSPACE_DIALOG_TITLE: &str = "Add Workspace Folder";

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickerOp {
    Open,
    SaveAs,
    Workspace,
}

pub(crate) const AUTHORITY_EXHAUSTED_MESSAGE: &str = "File chooser authority is exhausted; restart the app to open, save as, \
     or add a workspace again.";

type ParentSettlement = enzui_parent_lease::Handoff<
    enzui_core::blocking_task::BlockingTaskSettlement<enzui_wayland::ParentAdoption>,
>;

#[derive(Clone)]
pub(crate) enum ParentEvent {
    Handle {
        generation: u64,
        handle: enzui_widgets::portal_parent::ParentExport,
    },
    Exported {
        generation: u64,
        result: ParentSettlement,
    },
}

// Export handles and lease payloads must not enter application diagnostics.
impl std::fmt::Debug for ParentEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Handle { generation, .. } => f.debug_tuple("Handle").field(generation).finish(),
            Self::Exported { generation, .. } => {
                f.debug_tuple("Exported").field(generation).finish()
            }
        }
    }
}

struct ParentRequest {
    generation: u64,
    window: iced::window::Id,
    intent: ParentIntent,
    cancel: enzui_core::portal::cancel::PortalCancel,
}

enum ParentIntent {
    Chooser {
        op: PickerOp,
        operation: PortalOperation,
        options: ChooserOptions,
    },
    Location(OpenUri),
    Print {
        generation: u64,
        prepared: crate::print::PreparedHandoff,
    },
}

struct ParentedLocation {
    pending: PendingOpenUri,
    parent: ParentLease,
}

pub(crate) struct Picker {
    chooser: ChooserSession<PickerOp>,
    window: Option<iced::window::Id>,
    parent_request: Option<ParentRequest>,
    location_parent: Option<ParentRequest>,
    print_parent: Option<ParentRequest>,
    parent_task: Option<iced::Task<crate::Message>>,
    export_pool: Option<enzui_widgets::export_pool::ExportPoolOwner>,
    completed: Option<(PickerOp, PortalOutcome<FileResult>)>,
    // One export sequence prevents completions from crossing operation boundaries.
    generation: u64,
    authority_exhausted: bool,
    pub failure: Option<String>,
    location_backend: Option<Box<dyn OpenUriBackend>>,
    pending_location: Option<ParentedLocation>,
    location_generation: u64,
}

impl std::fmt::Debug for Picker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picker")
            .field("chooser", &self.chooser)
            .field("failure", &self.failure)
            .finish()
    }
}

impl Default for Picker {
    fn default() -> Self {
        Self::new()
    }
}

impl Picker {
    pub(crate) fn new() -> Self {
        Self {
            chooser: ChooserSession::new().with_fallback(),
            window: None,
            parent_request: None,
            location_parent: None,
            print_parent: None,
            parent_task: None,
            export_pool: None,
            completed: None,
            generation: 0,
            authority_exhausted: false,
            failure: None,
            location_backend: None,
            pending_location: None,
            location_generation: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_backend(backend: Box<dyn PortalBackend>) -> Self {
        Self {
            chooser: ChooserSession::with_backend(backend).with_fallback(),
            window: None,
            parent_request: None,
            location_parent: None,
            print_parent: None,
            parent_task: None,
            export_pool: None,
            completed: None,
            generation: 0,
            authority_exhausted: false,
            failure: None,
            location_backend: None,
            pending_location: None,
            location_generation: 0,
        }
    }

    fn next_generation(&mut self) -> Option<u64> {
        if self.authority_exhausted {
            self.failure = Some(crate::i18n::domain_text(AUTHORITY_EXHAUSTED_MESSAGE));
            return None;
        }
        match self.generation.checked_add(1) {
            Some(next) => {
                self.generation = next;
                Some(next)
            }
            None => {
                self.authority_exhausted = true;
                self.failure = Some(crate::i18n::domain_text(AUTHORITY_EXHAUSTED_MESSAGE));
                None
            }
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.completed.is_some() || self.parent_request.is_some() || self.chooser.is_pending()
    }

    pub(crate) fn start(&mut self) -> Result<(), PortalOutcome<FileResult>> {
        if self.is_pending() {
            return Ok(());
        }
        // `Ok(())` keeps the caller from routing this into `finish_pick`,
        // which would overwrite the exhaustion banner.
        if self.authority_exhausted {
            self.failure = Some(crate::i18n::domain_text(AUTHORITY_EXHAUSTED_MESSAGE));
            return Ok(());
        }
        self.failure = None;

        let options = ChooserOptions::new()
            .title(crate::i18n::domain_text(DIALOG_TITLE))
            .filter(FileFilter::extensions(
                enzui_core::localization::text("text-text-files-c5df15"),
                [
                    "txt", "md", "rs", "toml", "json", "yaml", "yml", "sh", "py", "c", "h", "cpp",
                    "js", "ts", "html", "css", "xml", "log", "conf", "ini",
                ],
            ))
            .filter(FileFilter::all_files(enzui_core::localization::text(
                "text-all-files-866956",
            )));

        self.start_request(PickerOp::Open, PortalOperation::OpenFile, options)
    }

    pub(crate) fn start_workspace(&mut self) -> Result<(), PortalOutcome<FileResult>> {
        if self.is_pending() {
            return Ok(());
        }
        if self.authority_exhausted {
            self.failure = Some(crate::i18n::domain_text(AUTHORITY_EXHAUSTED_MESSAGE));
            return Ok(());
        }
        self.failure = None;
        let options = ChooserOptions::new().title(crate::i18n::domain_text(WORKSPACE_DIALOG_TITLE));
        self.start_request(PickerOp::Workspace, PortalOperation::OpenDirectory, options)
    }

    pub(crate) fn start_save(
        &mut self,
        suggested_name: Option<&str>,
    ) -> Result<(), PortalOutcome<FileResult>> {
        if self.is_pending() {
            return Ok(());
        }
        if self.authority_exhausted {
            self.failure = Some(crate::i18n::domain_text(AUTHORITY_EXHAUSTED_MESSAGE));
            return Ok(());
        }
        self.failure = None;

        let mut options = ChooserOptions::new()
            .title(crate::i18n::domain_text(SAVE_AS_DIALOG_TITLE))
            .filter(FileFilter::extensions(
                enzui_core::localization::text("text-text-files-c5df15"),
                [
                    "txt", "md", "rs", "toml", "json", "yaml", "yml", "sh", "py", "c", "h", "cpp",
                    "js", "ts", "html", "css", "xml", "log", "conf", "ini",
                ],
            ))
            .filter(FileFilter::all_files(enzui_core::localization::text(
                "text-all-files-866956",
            )));
        if let Some(name) = suggested_name.filter(|s| !s.is_empty()) {
            options = options.current_name(name);
        }

        self.start_request(PickerOp::SaveAs, PortalOperation::SaveFile, options)
    }

    pub(crate) fn completion_subscription(&self) -> iced::Subscription<PortalWake> {
        self.chooser.subscription()
    }

    pub(crate) fn fallback_is_open(&self) -> bool {
        self.chooser.is_fallback_open()
    }

    pub(crate) fn view<'a>(
        &'a self,
        content: iced::Element<'a, crate::Message>,
    ) -> iced::Element<'a, crate::Message> {
        self.chooser.view(content, crate::Message::PickerEvent)
    }

    pub(crate) fn handle(&mut self, event: FallbackEvent) -> iced::Task<crate::Message> {
        self.chooser.handle(event, |op, outcome| {
            self.completed = Some((op, outcome));
            crate::Message::PickerTick
        })
    }

    pub(crate) fn pump(&mut self) -> Option<(PickerOp, PortalOutcome<FileResult>)> {
        if self.completed.is_some() {
            return self.completed.take();
        }
        let mut captured = None;
        let _task: iced::Task<()> = self.chooser.pump(|op, outcome| {
            captured = Some((op, outcome));
        });
        captured
    }

    fn start_request(
        &mut self,
        op: PickerOp,
        operation: PortalOperation,
        options: ChooserOptions,
    ) -> Result<(), PortalOutcome<FileResult>> {
        let Some(generation) = self.next_generation() else {
            return Ok(());
        };
        let Some(window) = self.window else {
            #[cfg(test)]
            return self.chooser.start(op, operation, options);
            #[cfg(not(test))]
            return Err(PortalOutcome::ParentUnavailable);
        };
        let request = Self::new_parent_request(
            window,
            ParentIntent::Chooser {
                op,
                operation,
                options,
            },
            generation,
        );
        self.parent_request = Some(request);
        self.queue_parent_task(window, generation);
        Ok(())
    }

    fn new_parent_request(
        window: iced::window::Id,
        intent: ParentIntent,
        generation: u64,
    ) -> ParentRequest {
        ParentRequest {
            generation,
            window,
            intent,
            cancel: enzui_core::portal::cancel::PortalCancel::new(),
        }
    }

    fn queue_parent_task(&mut self, window: iced::window::Id, generation: u64) {
        let task = iced::window::run(window, |window| {
            window.window_handle().ok().map_or(
                enzui_widgets::portal_parent::ParentExport::Unsupported,
                |handle| enzui_widgets::portal_parent::export_parent(&handle.as_raw()),
            )
        })
        .map(move |handle| {
            crate::Message::PickerParent(ParentEvent::Handle { generation, handle })
        });
        let previous = self.parent_task.take().unwrap_or_else(iced::Task::none);
        self.parent_task = Some(iced::Task::batch([previous, task]));
    }

    fn parent_request_for(&self, generation: u64) -> Option<&ParentRequest> {
        [
            &self.parent_request,
            &self.location_parent,
            &self.print_parent,
        ]
        .into_iter()
        .filter_map(Option::as_ref)
        .find(|r| r.generation == generation)
    }

    fn take_parent_request(&mut self, generation: u64) -> Option<ParentRequest> {
        for slot in [
            &mut self.parent_request,
            &mut self.location_parent,
            &mut self.print_parent,
        ] {
            if slot.as_ref().is_some_and(|r| r.generation == generation) {
                return slot.take();
            }
        }
        None
    }

    pub(crate) fn current_window(&self) -> Option<iced::window::Id> {
        self.window
    }

    pub(crate) fn set_window(&mut self, window: Option<iced::window::Id>) {
        self.window = window;
    }

    pub(crate) fn take_parent_task(&mut self) -> iced::Task<crate::Message> {
        self.parent_task.take().unwrap_or_else(iced::Task::none)
    }

    pub(crate) fn parent_event(&mut self, event: ParentEvent) -> iced::Task<crate::Message> {
        use enzui_widgets::portal_parent::ParentExport;
        let (generation, parent) = match event {
            ParentEvent::Handle {
                generation,
                handle: ParentExport::NeedsWaylandExport,
            } => {
                let Some(request) = self.parent_request_for(generation) else {
                    return iced::Task::none();
                };
                let window = request.window;
                let cancel = request.cancel.clone();
                if self.export_pool.is_none() {
                    self.export_pool = Some(
                        enzui_widgets::export_pool::ExportPoolOwner::start().unwrap_or_else(|_| {
                            enzui_widgets::export_pool::ExportPoolOwner::unavailable()
                        }),
                    );
                }
                let pool = self.export_pool.as_ref().expect("export pool installed");
                return enzui_widgets::portal_parent::export_parent_task(
                    &pool.dispatcher(),
                    generation,
                    window,
                    cancel,
                    std::time::Duration::from_secs(2),
                )
                .map(move |result| {
                    crate::Message::PickerParent(ParentEvent::Exported { generation, result })
                });
            }
            ParentEvent::Handle { generation, handle } => {
                let parent = match handle {
                    ParentExport::Ready(handle) => self
                        .parent_request_for(generation)
                        .map(|request| ParentLease::x11(request.window, handle)),
                    _ => None,
                };
                (generation, parent)
            }
            ParentEvent::Exported { generation, result } => {
                let parent = result.take().and_then(|settled| {
                    if settled.generation() != generation {
                        return None;
                    }
                    match settled.into_result().ok()? {
                        enzui_wayland::ParentAdoption::Parented { export, .. } => self
                            .parent_request_for(generation)
                            .map(|request| ParentLease::wayland(request.window, export)),
                        _ => None,
                    }
                });
                (generation, parent)
            }
        };
        let Some(request) = self.take_parent_request(generation) else {
            return iced::Task::none();
        };
        let parent = parent.filter(|parent| parent.is_live(self.window));
        match request.intent {
            ParentIntent::Chooser {
                op,
                operation,
                options,
            } => {
                let current_window = self.window;
                let result = match parent {
                    Some(parent) => self.chooser.start_parented(
                        op,
                        operation,
                        options,
                        parent.into_chooser(),
                        |window| current_window == Some(window),
                    ),
                    None => Err(PortalOutcome::ParentUnavailable),
                };
                if let Err(outcome) = result {
                    self.completed = Some((op, outcome));
                }
                iced::Task::done(crate::Message::PickerTick)
            }
            ParentIntent::Location(uri) => {
                let result = parent
                    .ok_or_else(location_failure)
                    .and_then(|parent| self.open_location(uri, parent));
                if let Err(error) = result {
                    self.failure = Some(error);
                }
                iced::Task::done(crate::Message::PickerTick)
            }
            ParentIntent::Print {
                generation,
                prepared,
            } => iced::Task::done(crate::Message::PrintParented {
                generation,
                prepared,
                parent: parent.map(ParentHandoff::new),
            }),
        }
    }

    pub(crate) fn parent_closed(&mut self, window: iced::window::Id) {
        for slot in [
            &mut self.parent_request,
            &mut self.location_parent,
            &mut self.print_parent,
        ] {
            if slot.as_ref().is_some_and(|r| r.window == window) {
                let request = slot.take().expect("matching parent request");
                request.cancel.cancel(CancelReason::ParentClosed);
                if let ParentIntent::Chooser { op, .. } = request.intent {
                    self.completed =
                        Some((op, PortalOutcome::Cancelled(CancelReason::ParentClosed)));
                }
            }
        }
        if let Some(completed) = self.chooser.parent_closed(window) {
            self.completed = Some(completed);
        }
        if self
            .pending_location
            .as_ref()
            .is_some_and(|request| request.parent.window() == window)
        {
            self.cancel_location(CancelReason::ParentClosed);
        }
        if self.window == Some(window) {
            self.window = None;
        }
    }

    pub(crate) fn start_print_parent(
        &mut self,
        generation: u64,
        prepared: crate::print::PreparedHandoff,
    ) -> bool {
        let Some(window) = self.window else {
            return false;
        };
        let Some(export_generation) = self.next_generation() else {
            return false;
        };
        if let Some(old) = self.print_parent.take() {
            old.cancel.cancel(CancelReason::Application);
        }
        self.print_parent = Some(Self::new_parent_request(
            window,
            ParentIntent::Print {
                generation,
                prepared,
            },
            export_generation,
        ));
        self.queue_parent_task(window, export_generation);
        true
    }

    pub(crate) fn note_outcome(&mut self, outcome: &PortalOutcome<FileResult>) {
        self.failure = enzui_widgets::file_chooser::failure_text(outcome);
    }

    pub(crate) fn dismiss_failure(&mut self) {
        self.failure = None;
    }

    pub(crate) fn start_location(&mut self, path: &std::path::Path) -> Result<(), String> {
        if self.location_parent.is_some() || self.location_is_pending() {
            return Ok(());
        }
        let uri = OpenUri::from_file_path(path).map_err(|_| {
            enzui_core::localization::text("text-the-session-location-could-not-be-opened-73b811")
        })?;
        let window = self.window.ok_or_else(location_failure)?;
        let generation = self.next_generation().ok_or_else(location_failure)?;
        self.location_generation = generation;
        self.location_parent = Some(Self::new_parent_request(
            window,
            ParentIntent::Location(uri),
            generation,
        ));
        self.queue_parent_task(window, generation);
        Ok(())
    }

    fn open_location(&mut self, uri: OpenUri, parent: ParentLease) -> Result<(), String> {
        if !parent.is_live(self.window) {
            return Err(location_failure());
        }
        let window = self.window;
        let backend = self.location_backend_mut().ok_or_else(location_failure)?;
        let pending = OpenUriRequest::new(uri, PortalParent::Window(parent.window()))
            .with_parent_handle(parent.handle())
            .start(&mut **backend, |id| window == Some(id))
            .map_err(|_| location_failure())?;
        self.pending_location = Some(ParentedLocation { pending, parent });
        Ok(())
    }

    fn location_is_pending(&self) -> bool {
        self.pending_location
            .as_ref()
            .is_some_and(|request| request.pending.is_unsettled())
    }

    fn cancel_location(&mut self, reason: CancelReason) {
        if let Some(mut request) = self.pending_location.take() {
            let _ = request.pending.cancel(reason);
            if let Some(backend) = self.location_backend.as_deref_mut() {
                let _ = request.pending.settle(backend);
            }
        }
    }

    pub(crate) fn location_subscription(&self) -> iced::Subscription<PortalWake> {
        if self.location_is_pending()
            && let Some(signal) = self
                .location_backend
                .as_ref()
                .and_then(|backend| backend.open_uri_completion_signal())
        {
            return enzui_widgets::file_chooser::completion_signal_subscription(
                self.location_generation,
                signal,
            );
        }
        iced::Subscription::none()
    }

    pub(crate) fn location_needs_poll(&self) -> bool {
        // Parent-export liveness has no completion stream of its own.
        self.location_is_pending()
    }

    pub(crate) fn pump_location(&mut self) {
        if self
            .pending_location
            .as_ref()
            .is_some_and(|request| !request.parent.is_live(self.window))
        {
            self.cancel_location(CancelReason::ParentClosed);
            return;
        }
        let (Some(request), Some(backend)) = (
            self.pending_location.as_mut(),
            self.location_backend.as_deref_mut(),
        ) else {
            return;
        };
        let Some(outcome) = request.pending.poll(backend) else {
            return;
        };
        let _ = request.pending.settle(backend);
        self.pending_location = None;
        if !matches!(
            outcome,
            PortalOutcome::Completed(()) | PortalOutcome::Cancelled(_)
        ) {
            self.failure = Some(location_failure());
        }
    }

    fn location_backend_mut(&mut self) -> Option<&mut Box<dyn OpenUriBackend>> {
        if self.location_backend.is_none() {
            self.location_backend =
                Some(enzui_core::portal::open_uri::desktop_launcher::host_open_uri_backend());
        }
        self.location_backend.as_mut()
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        for slot in [
            &mut self.parent_request,
            &mut self.location_parent,
            &mut self.print_parent,
        ] {
            if let Some(request) = slot.take() {
                request.cancel.cancel(CancelReason::Application);
            }
        }
        self.cancel_location(CancelReason::Application);
    }
}

fn location_failure() -> String {
    enzui_core::localization::text("text-the-desktop-could-not-open-the-session-51218e")
}

#[cfg(test)]
pub(crate) mod tests {
    use enzui_core::portal::open_uri::OpenUriBackend;
    use std::sync::{Arc, Mutex};

    use enzui_widgets::file_chooser::{
        BackendError, BackendReply, CancelReason, ChooserOptions, ParentHandle, PortalBackend,
        PortalOperation, PortalResponse, RequestId,
    };

    #[derive(Debug, Default)]
    pub(crate) struct StartLog {
        pub names: Vec<Option<String>>,
    }

    pub(crate) struct ScriptedBackend {
        log: Arc<Mutex<StartLog>>,
        reply: Option<BackendReply>,
        start_error: Option<BackendError>,
        next_id: u64,
    }

    impl ScriptedBackend {
        pub(crate) fn silent() -> Self {
            Self {
                log: Arc::new(Mutex::new(StartLog::default())),
                reply: None,
                start_error: None,
                next_id: 0,
            }
        }

        pub(crate) fn replying(response: PortalResponse, uris: Vec<String>) -> Self {
            Self {
                reply: Some(BackendReply { response, uris }),
                ..Self::silent()
            }
        }

        pub(crate) fn log(&self) -> Arc<Mutex<StartLog>> {
            Arc::clone(&self.log)
        }
    }

    impl PortalBackend for ScriptedBackend {
        fn start(&mut self, operation: PortalOperation) -> Result<RequestId, BackendError> {
            self.start_with_options(operation, &ChooserOptions::new(), &ParentHandle::none())
        }

        fn start_with_options(
            &mut self,
            _operation: PortalOperation,
            options: &ChooserOptions,
            _parent: &ParentHandle,
        ) -> Result<RequestId, BackendError> {
            if let Some(e) = self.start_error {
                return Err(e);
            }
            self.next_id += 1;
            let mut log = self.log.lock().unwrap();
            log.names.push(options.name().map(str::to_owned));
            Ok(RequestId::new(self.next_id))
        }

        fn poll_reply(&mut self, _id: RequestId) -> Option<BackendReply> {
            self.reply.take()
        }

        fn close(&mut self, _id: RequestId, _reason: CancelReason) {}

        fn describe(&self) -> String {
            "scripted (in-process, no D-Bus)".to_owned()
        }
    }
    #[test]
    fn closed_parent_and_stale_export_cannot_start_the_replacement_chooser() {
        crate::test_fixture::with_locale(|| {
            use super::{ParentEvent, Picker};
            use enzui_widgets::portal_parent::ParentExport;

            let backend = ScriptedBackend::silent();
            let log = backend.log();
            let mut picker = Picker::with_backend(Box::new(backend));
            let first_window = iced::window::Id::unique();
            picker.set_window(Some(first_window));
            picker.start().expect("first parent export admitted");
            let old_generation = picker.generation;
            picker.parent_closed(first_window);
            assert!(matches!(
                picker.pump(),
                Some((
                    _,
                    super::PortalOutcome::Cancelled(CancelReason::ParentClosed)
                ))
            ));
            picker.set_window(Some(iced::window::Id::unique()));
            picker.start().expect("replacement parent export admitted");
            let _ = picker.parent_event(ParentEvent::Handle {
                generation: old_generation,
                handle: ParentExport::Ready(ParentHandle::x11(1)),
            });
            assert!(log.lock().expect("requests").names.is_empty());
            assert!(picker.parent_request.is_some());
            let _ = picker.parent_event(ParentEvent::Handle {
                generation: picker.generation,
                handle: ParentExport::Ready(ParentHandle::x11(2)),
            });
            assert_eq!(log.lock().expect("requests").names.len(), 1);
            assert!(picker.chooser.is_pending());
        });
    }
    struct LocationBackend(Arc<Mutex<enzui_core::portal::open_uri::MockOpenUriBackend>>);

    impl OpenUriBackend for LocationBackend {
        fn start_open_uri(
            &mut self,
            uri: &super::OpenUri,
            parent: &ParentHandle,
        ) -> Result<RequestId, BackendError> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .start_open_uri(uri, parent)
        }
        fn poll_open_uri(
            &mut self,
            id: RequestId,
        ) -> Option<enzui_core::portal::open_uri::OpenUriReply> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .poll_open_uri(id)
        }
        fn close_open_uri(&mut self, id: RequestId, reason: CancelReason) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .close_open_uri(id, reason);
        }
        fn describe_open_uri(&self) -> String {
            "scripted location".to_owned()
        }
    }

    #[test]
    fn location_parent_is_separate_from_save_as_and_closed_before_a_stale_replacement() {
        crate::test_fixture::with_locale(|| {
            use super::{ParentEvent, Picker};
            use enzui_core::portal::open_uri::MockOpenUriBackend;
            use enzui_widgets::portal_parent::ParentExport;

            let backend = Arc::new(Mutex::new(MockOpenUriBackend::new()));
            let mut picker = Picker::with_backend(Box::new(ScriptedBackend::silent()));
            picker.location_backend = Some(Box::new(LocationBackend(Arc::clone(&backend))));
            let window = iced::window::Id::unique();
            picker.set_window(Some(window));
            let path = std::path::Path::new("/tmp/Text Grüße");
            picker.start_location(path).expect("location admitted");
            let old_generation = picker.location_generation;
            picker
                .start_save(Some("kept.txt"))
                .expect("independent Save As");
            assert!(picker.parent_request.is_some());
            let _ = picker.parent_event(ParentEvent::Handle {
                generation: old_generation,
                handle: ParentExport::Ready(ParentHandle::x11(7)),
            });
            {
                let backend = backend.lock().expect("backend");
                assert_eq!(backend.started.len(), 1);
                assert_eq!(
                    backend.started[0].1.local_file_path().as_deref(),
                    Some(path)
                );
                assert_eq!(backend.started[0].2, ParentHandle::x11(7));
            }
            assert!(picker.pending_location.is_some());
            picker.parent_closed(window);
            assert!(picker.pending_location.is_none());
            assert_eq!(
                backend.lock().expect("backend").closed[0].1,
                CancelReason::ParentClosed
            );
            picker.parent_closed(window);
            assert_eq!(backend.lock().expect("backend").closed.len(), 1);
            let _ = picker.pump();
            picker.set_window(Some(iced::window::Id::unique()));
            picker.start_location(path).expect("replacement location");
            let _ = picker.parent_event(ParentEvent::Handle {
                generation: old_generation,
                handle: ParentExport::Ready(ParentHandle::x11(8)),
            });
            assert!(picker.location_parent.is_some());
            let _ = picker.parent_event(ParentEvent::Handle {
                generation: picker.location_generation,
                handle: ParentExport::Unsupported,
            });
            assert!(picker.failure.is_some());
            assert_eq!(backend.lock().expect("backend").started.len(), 1);
        });
    }
}
