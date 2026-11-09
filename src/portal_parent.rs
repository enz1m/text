//! Owned portal parents retained through request settlement.

use enzui_core::portal::parent::ParentHandle;
use enzui_widgets::file_chooser::ChooserParent;

pub(crate) struct ParentLease {
    window: iced::window::Id,
    handle: ParentHandle,
    export: Option<enzui_wayland::ParentExport>,
}

impl std::fmt::Debug for ParentLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParentLease")
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl ParentLease {
    pub(crate) fn x11(window: iced::window::Id, handle: ParentHandle) -> Self {
        Self {
            window,
            handle,
            export: None,
        }
    }

    pub(crate) fn wayland(window: iced::window::Id, export: enzui_wayland::ParentExport) -> Self {
        Self {
            window,
            handle: export.parent_handle(),
            export: Some(export),
        }
    }

    pub(crate) fn window(&self) -> iced::window::Id {
        self.window
    }

    pub(crate) fn handle(&self) -> ParentHandle {
        self.handle.clone()
    }

    pub(crate) fn is_alive(&self) -> bool {
        !self.handle.is_none()
            && self
                .export
                .as_ref()
                .is_none_or(enzui_wayland::ParentExport::is_alive)
    }

    pub(crate) fn is_live(&self, window: Option<iced::window::Id>) -> bool {
        window == Some(self.window) && self.is_alive()
    }

    pub(crate) fn into_chooser(self) -> ChooserParent {
        match self.export {
            Some(export) => ChooserParent::Wayland {
                window: self.window,
                export,
            },
            None => ChooserParent::X11 {
                window: self.window,
                handle: self.handle,
            },
        }
    }
}

pub(crate) type ParentHandoff = enzui_parent_lease::Handoff<ParentLease>;
