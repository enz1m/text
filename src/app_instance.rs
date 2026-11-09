//! One instance per user that also answers the desktop's Search.
//!
//! The owner holds the `org.freedesktop.Application` service itself, so the
//! Search provider is served on the bus name and object path the installed
//! declaration names, and a chosen result wakes the same subscription as a
//! forwarded launch. Launch semantics match
//! `enzui_widgets::single_instance::SingleInstance`: a later launch forwards
//! its files and exits; without a session bus the app runs on its own.

use std::time::Instant;

use enzui_core::activation::dbus::search::{SearchInbox, SearchProvider, SearchRequest};
use enzui_core::activation::dbus::{
    ApplicationService, ApplicationServiceError, ApplicationStart, ApplicationWake,
};
use enzui_core::activation::{
    ActivationBroker, ActivationContext, ApplicationIntent, InboundActivationToken,
    launch_request_from_env,
};
use iced::{Subscription, Task, window};

const DRAIN_LIMIT: usize = 16;

#[expect(
    clippy::large_enum_variant,
    reason = "built once in `main` and moved into the application"
)]
pub(crate) enum InstanceStart {
    Owner(AppInstance, ApplicationIntent),
    Forwarded,
}

pub(crate) struct AppInstance {
    service: Option<ApplicationService>,
    broker: ActivationBroker,
    searches: Option<SearchInbox>,
}

// What one wake delivered: launches to open, Search choices to show, and the
// task that raises the window for each.
pub(crate) struct Drained<M> {
    pub(crate) launches: Vec<ApplicationIntent>,
    pub(crate) searches: Vec<SearchRequest>,
    pub(crate) raise: Task<M>,
}

impl AppInstance {
    pub(crate) fn start(application_id: &str) -> InstanceStart {
        use enzui_core::activation::dbus::ApplicationServiceError as Error;
        let intent = launch_request_from_env().intent;
        let mut token = InboundActivationToken::from_env().ok().flatten();
        match ApplicationService::connect_or_forward_with_token(
            application_id,
            Some(&intent),
            &mut token,
        ) {
            Ok(ApplicationStart::Reused) => InstanceStart::Forwarded,
            Ok(ApplicationStart::Owner(service)) => {
                InstanceStart::Owner(Self::with_service(Some(service)), intent)
            }
            // A refused forward must never start a second owner of the same identity.
            Err(error @ (Error::Forward(_) | Error::NameNotAcquired(_))) => {
                log::warn!("the running instance did not take this launch: {error}");
                InstanceStart::Forwarded
            }
            Err(error) => {
                log::warn!("running on its own: {error}");
                InstanceStart::Owner(Self::standalone(), intent)
            }
        }
    }

    pub(crate) fn standalone() -> Self {
        Self::with_service(None)
    }

    fn with_service(service: Option<ApplicationService>) -> Self {
        Self {
            service,
            broker: ActivationBroker::default(),
            searches: None,
        }
    }

    // Serves `provider` beside the application endpoint; a standalone
    // instance has no bus to answer on and serves nothing.
    pub(crate) fn serve_search(
        &mut self,
        provider: SearchProvider,
    ) -> Result<(), ApplicationServiceError> {
        if let Some(service) = &self.service {
            self.searches = Some(service.serve_search(provider)?);
        }
        Ok(())
    }

    pub(crate) fn subscription<M>(&self, on_wake: fn(ApplicationWake) -> M) -> Subscription<M>
    where
        M: Send + 'static,
    {
        match &self.service {
            // `map` takes only zero-sized closures; the constructor rides
            // along as subscription data instead.
            Some(service) => service
                .wake_subscription(1)
                .with(on_wake)
                .map(|(on_wake, wake)| on_wake(wake)),
            None => Subscription::none(),
        }
    }

    // Raises `window` (the most recently opened one when `None`) once per
    // activation, with the caller's activation token when it sent one.
    pub(crate) fn drain<M>(&mut self, window: Option<window::Id>) -> Drained<M>
    where
        M: Send + 'static,
    {
        let Some(service) = self.service.as_mut() else {
            return Drained {
                launches: Vec::new(),
                searches: Vec::new(),
                raise: Task::none(),
            };
        };
        let now = Instant::now();
        let mut contexts = Vec::new();
        let launches = service
            .drain(&mut self.broker, now, DRAIN_LIMIT)
            .into_iter()
            .map(|call| {
                contexts.push(call.context);
                call.intent
            })
            .collect();
        let searches = self
            .searches
            .as_ref()
            .map(|inbox| inbox.drain(&mut self.broker, now, DRAIN_LIMIT))
            .unwrap_or_default()
            .into_iter()
            .map(|activation| {
                contexts.push(activation.context);
                activation.request
            })
            .collect();
        let raise = Task::batch(contexts.into_iter().map(|context| raise(window, context)));
        Drained {
            launches,
            searches,
            raise,
        }
    }
}

fn raise<M: Send + 'static>(
    window: Option<window::Id>,
    mut context: Option<ActivationContext>,
) -> Task<M> {
    let target = match window {
        Some(window) => Task::done(Some(window)),
        None => window::latest(),
    };
    target.then(move |window| match (window, context.take()) {
        (Some(window), Some(context)) => context.activate(window, Instant::now()).discard(),
        (Some(window), None) => window::gain_focus(window),
        (None, _) => Task::none(),
    })
}
