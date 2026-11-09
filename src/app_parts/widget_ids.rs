//! TextApp widget id and focus-restore helpers.

use super::*;

pub(crate) fn stats_button_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.status.stats")
}

pub(crate) fn language_pill_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.status.language")
}

pub(crate) fn line_ending_pill_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.status.line-ending")
}

pub(crate) fn settings_invoker_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.titlebar.overflow")
}

pub(crate) fn tab_list_button_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.titlebar.tab-list")
}

pub(crate) fn editor_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.editor")
}

pub(crate) fn find_field_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.find.query")
}

pub(crate) fn replace_field_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.find.replacement")
}

pub(crate) fn find_close_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.find.close")
}

pub(crate) fn goto_close_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.goto.close")
}

// `enz_text_field` has no focus-on-mount, so `Message::OpenGoto` focuses
// this id.
pub(crate) fn goto_field_id() -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::new("text-app.goto.query")
}

// Uses [`enzui_widgets::modal_host::restore_focus_operation`], which is a
// no-op when `id` is not in the rendered tree.
pub(crate) fn restore_focus_task(id: iced::advanced::widget::Id) -> Task<Message> {
    match restore_focus_operation_for(&id) {
        Some(op) => iced::advanced::widget::operate(op),
        None => Task::none(),
    }
}

pub(crate) fn restore_focus_operation_for(
    id: &iced::advanced::widget::Id,
) -> Option<Box<dyn iced::advanced::widget::Operation<Message>>> {
    use enzui_core::modal::DismissCause;
    use enzui_widgets::modal_host::{ModalHostEvent, restore_focus_operation};
    restore_focus_operation::<Message>(ModalHostEvent::Dismissed(DismissCause::Escape), Some(id))
}
