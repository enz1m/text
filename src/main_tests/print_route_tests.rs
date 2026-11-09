use super::*;

#[test]
fn unsupported_print_characters_are_refused_without_submitting_or_changing_the_document() {
    crate::test_fixture::with_locale(|| {
        let mut app = TextApp::for_test();
        app.load_active_buffer("Grüße 世界");
        let before = app.model.active_tab().snapshot();
        let spool = NeverSpool;
        app.print = crate::print::PrintState::with_spool(Box::new(spool));
        let _ = app.update(Message::PrintDocument);
        let generation = app.print.current_generation().expect("print generation");
        let prepared = crate::print::prepare_document(&before, "document");
        assert_eq!(
            prepared.as_ref().expect_err("unsupported characters"),
            &crate::print::PrintRenderError::UnsupportedCharacters
        );
        let _ = app.update(Message::PrintPrepared {
            generation,
            prepared,
        });

        assert_eq!(app.model.active_tab().snapshot(), before);
        assert_eq!(
            app.print.status(),
            Some(&crate::print::PrintStatus::UnsupportedCharacters)
        );
        assert!(
            app.print
                .status()
                .expect("print refusal")
                .banner()
                .contains("cannot be printed accurately")
        );
    });
}

struct NeverSpool;

impl crate::print::PrintSpool for NeverSpool {
    fn start(
        &mut self,
        _: enzui_core::portal::print::PrintRequest,
    ) -> Result<(), enzui_core::portal::PortalOutcome<enzui_core::portal::print::PrintReceipt>>
    {
        panic!("an unsupported document reached the print spool")
    }
    fn poll(
        &mut self,
    ) -> Option<enzui_core::portal::PortalOutcome<enzui_core::portal::print::PrintReceipt>> {
        None
    }
    fn cancel(&mut self, _: enzui_core::portal::CancelReason) {}
    fn settle(&mut self) {}
    fn is_live(&self) -> bool {
        false
    }
}

#[test]
fn parented_print_rejects_stale_and_duplicate_authority_and_settles_before_close() {
    crate::test_fixture::with_locale(|| {
        use crate::portal_parent::{ParentHandoff, ParentLease};
        use enzui_core::portal::parent::ParentHandle;
        use std::sync::{Arc, Mutex};

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut app = TextApp::for_test();
        app.print = crate::print::PrintState::with_spool(Box::new(RecordingSpool {
            log: Arc::clone(&log),
            request: None,
        }));
        let window = iced::window::Id::unique();
        app.picker.set_window(Some(window));
        let first = app.print.begin().expect("first print");
        let prepared = crate::print::prepare_document("sealed text", "document").expect("prepared");
        let duplicate = prepared.clone();
        let _ = app.finish_print_prepare(first, Ok(prepared));
        assert!(duplicate.take().is_none());
        app.picker.parent_closed(window);
        app.print.abort_for_close();
        let parent = || ParentHandoff::new(ParentLease::x11(window, ParentHandle::x11(42)));
        let prepared =
            || crate::print::prepare_document("sealed text", "document").expect("prepared");
        let _ = app.finish_print_parent(first, prepared(), Some(parent()));
        assert!(log.lock().expect("log").is_empty());
        assert!(!app.print.is_busy());

        let old_parent = parent();
        let window = iced::window::Id::unique();
        app.picker.set_window(Some(window));
        let parent = || ParentHandoff::new(ParentLease::x11(window, ParentHandle::x11(44)));
        let current = app.print.begin().expect("changed parent");
        let _ = app.finish_print_parent(current, prepared(), Some(old_parent));
        assert_eq!(app.print.status(), Some(&crate::print::PrintStatus::Failed));
        let current = app.print.begin().expect("replacement print");
        let _ = app.finish_print_parent(first, prepared(), Some(parent()));
        assert_eq!(app.print.current_generation(), Some(current));
        let _ = app.finish_print_parent(current, prepared(), None);
        assert_eq!(app.print.status(), Some(&crate::print::PrintStatus::Failed));
        assert!(log.lock().expect("log").is_empty());

        let current = app.print.begin().expect("parented print");
        let prepared = prepared();
        let parent = parent();
        let _ = app.finish_print_parent(current, prepared.clone(), Some(parent.clone()));
        let _ = app.finish_print_parent(current, prepared, Some(parent));
        assert_eq!(*log.lock().expect("log"), vec!["start"]);
        assert!(app.print.has_admitted_inflight());
        app.print.abort_for_close();
        app.print.abort_for_close();
        assert_eq!(
            *log.lock().expect("log"),
            vec!["start", "parent closed", "settle"]
        );
        assert!(!app.print.is_busy());

        let current = app.print.begin().expect("completed print");
        let prepared = crate::print::prepare_document("text", "document").expect("prepared");
        let parent = ParentHandoff::new(ParentLease::x11(window, ParentHandle::x11(43)));
        let _ = app.finish_print_parent(current, prepared, Some(parent));
        app.print.poll();
        assert_eq!(
            app.print.status(),
            Some(&crate::print::PrintStatus::Submitted)
        );
        assert!(!app.print.is_busy());
        assert_eq!(
            *log.lock().expect("log"),
            vec!["start", "parent closed", "settle", "start", "settle"]
        );
    });
}

struct RecordingSpool {
    log: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
    request: Option<enzui_core::portal::print::PrintRequest>,
}

impl crate::print::PrintSpool for RecordingSpool {
    fn start(
        &mut self,
        request: enzui_core::portal::print::PrintRequest,
    ) -> Result<(), enzui_core::portal::PortalOutcome<enzui_core::portal::print::PrintReceipt>>
    {
        assert!(self.request.is_none());
        assert!(request.token().is_none());
        assert_eq!(
            request.format(),
            enzui_core::portal::print::PrintFormat::PostScript
        );
        let seals = rustix::fs::fcntl_get_seals(request.document()).expect("sealed document");
        assert!(seals.contains(
            rustix::fs::SealFlags::WRITE
                | rustix::fs::SealFlags::GROW
                | rustix::fs::SealFlags::SHRINK
                | rustix::fs::SealFlags::SEAL
        ));
        self.log.lock().expect("log").push("start");
        self.request = Some(request);
        Ok(())
    }
    fn poll(
        &mut self,
    ) -> Option<enzui_core::portal::PortalOutcome<enzui_core::portal::print::PrintReceipt>> {
        self.request.as_ref().map(|_| {
            enzui_core::portal::PortalOutcome::Completed(enzui_core::portal::print::PrintReceipt {
                token: enzui_core::portal::print::PrintToken::from_raw(1),
            })
        })
    }
    fn cancel(&mut self, reason: enzui_core::portal::CancelReason) {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(
                if reason == enzui_core::portal::CancelReason::ParentClosed {
                    "parent closed"
                } else {
                    "application"
                },
            );
    }
    fn settle(&mut self) {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push("settle");
        self.request = None;
    }
    fn is_live(&self) -> bool {
        self.request.is_some()
    }
}
