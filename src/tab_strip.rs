use crate::{
    Element, EnzMenuItem, Length, Message, Task, TextApp, WorkspaceAccentPalette, enz_menu,
    enz_toolbar_icon, tab_list_button_id,
};
use enzui_widgets::document_tabs::{TabEvent, enz_document_tabs};
use enzui_widgets::scroll_view::{ScrollGeometryCell, publish_geometry_of};
use iced::widget::{Space, container, mouse_area, responsive, row, stack};

const LABEL: &str = "Text documents";
const CELL_WIDTH: f32 = 180.0;
const ACTION_WIDTH: f32 = enzui_widgets::titlebar_tab::DEFAULT_ROW_LIST_RESERVE;
const GAP: f32 = 4.0;
const MINIMUM_TAB_WIDTH: f32 = 120.0;
const PEEK_FRACTION: f32 = 1.0 / 3.0;
pub(crate) const MINIMUM_WIDTH: f32 =
    MINIMUM_TAB_WIDTH * (1.0 + PEEK_FRACTION) + 2.0 * (ACTION_WIDTH + GAP);

fn tab_extent(viewport: f32, count: usize) -> f32 {
    if viewport < MINIMUM_TAB_WIDTH * (1.0 + PEEK_FRACTION) || count as f32 * CELL_WIDTH <= viewport
    {
        return viewport.clamp(1.0, CELL_WIDTH);
    }
    let whole = (viewport / CELL_WIDTH)
        .floor()
        .max(1.0)
        .min(count.saturating_sub(1) as f32);
    (viewport / (whole + PEEK_FRACTION)).clamp(MINIMUM_TAB_WIDTH, 220.0)
}

#[derive(Default)]
pub(crate) struct TabStripState {
    pub offset: f32,
    pub geometry: ScrollGeometryCell,
    revealed: Option<(String, u32, usize, usize)>,
}

impl TabStripState {
    pub(crate) fn request_reveal(&mut self) {
        self.revealed = None;
    }
}

impl TextApp {
    pub(crate) fn view_scroll_tabs(&self) -> Element<'_, Message> {
        responsive(move |size| -> Element<'_, Message> {
            let count = self.model.tabs.len();
            let show_new = size.width >= 96.0;
            let new_reserve = if show_new { ACTION_WIDTH + GAP } else { 0.0 };
            let overflow = count as f32 * CELL_WIDTH > size.width - new_reserve;
            let list_reserve = if overflow { ACTION_WIDTH + GAP } else { 0.0 };
            let available = (size.width - new_reserve - list_reserve).max(0.0);
            let extent = if overflow {
                available
            } else {
                available.min(count as f32 * CELL_WIDTH)
            };
            let width = tab_extent(extent, count);
            let accent_palette = WorkspaceAccentPalette::for_mode(&self.palette());
            let strip = enz_document_tabs(
                &self.model.doc_tabs,
                LABEL,
                move |_| width,
                move |item, ctx| {
                    let id = item.key.as_str().to_owned();
                    let tab = self.model.tabs.iter().find(|tab| tab.id == id);
                    let accent = tab
                        .and_then(|tab| tab.workspace_root.as_deref())
                        .and_then(|root| self.workspaces.accent_for(root))
                        .map(|accent| accent_palette.resolve(accent));
                    let cell = enzui_widgets::document_tab::enz_document_tab(
                        item.title.clone(),
                        Message::SwitchTab(id.clone()),
                    )
                    .on_close(Message::CloseTab(id.clone()))
                    .selected(ctx.selected)
                    .focused(ctx.focused)
                    .embedded(true)
                    .id(id.clone())
                    .set_position(ctx.position, ctx.count)
                    .modified(tab.is_some_and(|tab| tab.dirty))
                    .favicon_size(10.0)
                    .close_visible(width >= 100.0)
                    .tab_min_width((width - 2.0).max(1.0))
                    .tab_max_width((width - 2.0).max(1.0))
                    .fill_width(true)
                    .workspace_accent(accent);
                    let bounded = container(cell)
                        .height(enzui_widgets::window_controls::DEFAULT_CONTROL_SIZE)
                        .width(width)
                        .padding(iced::Padding::ZERO.right(2.0));
                    let described = enzui_widgets::tooltip::enz_tooltip(
                        bounded.into(),
                        Self::tab_description(&self.model, &id, &item.title),
                        iced::widget::tooltip::Position::Bottom,
                    );
                    mouse_area(
                        container(described)
                            .height(Length::Fill)
                            .align_y(iced::Alignment::Center),
                    )
                    .on_scroll(Message::TabStripWheel)
                    .into()
                },
            )
            .extent(extent)
            .scroll_offset(self.tab_strip.offset)
            .overscan(2)
            .hide_scrollbar(true)
            .height(Length::Fill)
            .on_scroll(Message::TabStripScrolled)
            .on_event(Message::TabStripEvent)
            .into_element();
            let strip = publish_geometry_of(strip, self.tab_strip.geometry.clone());
            let edges = self.tab_strip.geometry.get().map_or(
                enzui_widgets::scroll_fade_edge::ScrollFadeEdges::NONE,
                |geo| enzui_widgets::scroll_fade_edge::ScrollFadeEdges::from_geometry(&geo),
            );
            // Fades are siblings so their changing edge layers cannot reset scroll custody.
            let fade = enzui_widgets::scroll_fade_edge::scroll_fade_edges(
                Space::new().width(Length::Fill).height(Length::Fill),
                edges,
            )
            .color(self.palette().background);
            let strip = mouse_area(stack![strip, fade]).on_scroll(Message::TabStripWheel);
            let mut band = row![].spacing(GAP).height(Length::Fill);
            if extent >= 28.0 {
                band = band.push(container(strip).width(if overflow {
                    Length::Fill
                } else {
                    Length::Fixed(extent)
                }));
            }
            if overflow {
                band = band.push(self.view_document_list());
            }
            if show_new {
                band = band.push(
                    enz_toolbar_icon("plus", Message::NewTab)
                        .label(enzui_core::localization::text("text-new-tab-1e08fd"))
                        .id(iced::advanced::widget::Id::new("text-app-new-tab")),
                );
            }
            band.width(Length::Fill)
                .align_y(iced::Alignment::Center)
                .into()
        })
        .into()
    }

    fn view_document_list(&self) -> Element<'_, Message> {
        let trigger = enz_toolbar_icon("chevron-down", Message::ToggleTabList)
            .label(crate::i18n::domain_text("Open documents"))
            .id(tab_list_button_id())
            .expanded(self.model.tab_list_open)
            .relinquish_focus_while(self.model.tab_list_open)
            .reclaim_focus_on_return((!self.model.tab_list_open).then(tab_list_button_id));
        let popup = self.model.tab_list_open.then(|| {
            let mut menu = enz_menu()
                .width(240.0)
                .label(crate::i18n::domain_text("Open documents"))
                .id_prefix("text-document-list")
                .active_maybe(Some(enzui_core::menu_model::EntryId::from(
                    self.model.active_tab_id.clone(),
                )))
                .roving_self(true)
                .focus_return_to(tab_list_button_id());
            for item in self.model.doc_tabs.visual_order() {
                menu = menu.push_item(EnzMenuItem::command(
                    enzui_core::menu_model::EntryId::from(item.key.as_str()),
                    item.title.clone(),
                    Message::SwitchTab(item.key.as_str().to_owned()),
                ));
            }
            menu.into()
        });
        enzui_widgets::anchored_overlay::dismissible_anchored_menu(
            trigger,
            popup,
            enzui_core::placement::Placement::new(enzui_core::placement::Side::Bottom).gap(GAP),
            || Message::DismissPopovers,
        )
    }

    pub(crate) fn reduce_tab_strip_event(&mut self, event: TabEvent) -> Task<Message> {
        match event {
            TabEvent::Activated(key) => self.reduce_tab_switch_tab(key.as_str().to_owned()),
            TabEvent::CloseRequested(key, _) => self.reduce_tab_close_tab(key.as_str().to_owned()),
            TabEvent::FocusMoved(key) => {
                self.model.doc_tabs.focus(&key);
                self.scroll_tab_key_into_view(&key)
            }
            TabEvent::NewRequested => self.reduce_tab_new_tab(),
            TabEvent::ContextRequested(..) => Task::none(),
        }
    }

    pub(crate) fn reduce_tab_strip_wheel(
        &mut self,
        delta: iced::mouse::ScrollDelta,
    ) -> Task<Message> {
        let amount = match delta {
            iced::mouse::ScrollDelta::Lines { x, y } => -if x == 0.0 { y } else { x } * 40.0,
            iced::mouse::ScrollDelta::Pixels { x, y } => -if x == 0.0 { y } else { x },
        };
        if !amount.is_finite() {
            return Task::none();
        }
        let Some(geo) = self.tab_strip.geometry.get() else {
            return Task::none();
        };
        let maximum = (geo.content_bounds.width - geo.bounds.width).max(0.0);
        self.tab_strip.offset = (self.tab_strip.offset + amount).clamp(0.0, maximum);
        self.tab_strip_scroll_task()
    }

    fn tab_strip_scroll_task(&self) -> Task<Message> {
        iced::advanced::widget::operate(enzui_widgets::document_tabs::reveal_scroll_operation(
            LABEL,
            self.tab_strip.offset,
            enzui_core::document_tabs::Orientation::Horizontal,
        ))
    }

    fn scroll_tab_key_into_view(
        &mut self,
        key: &enzui_core::document_tabs::DocumentKey,
    ) -> Task<Message> {
        let Some(geo) = self.tab_strip.geometry.get() else {
            return Task::none();
        };
        let width = tab_extent(geo.bounds.width, self.model.tabs.len());
        let metrics = enzui_core::document_tabs::StripMetrics::new(&self.model.doc_tabs, |_| width);
        if let Some(offset) = enzui_widgets::document_tabs::reveal_offset(
            &metrics,
            key,
            self.tab_strip.offset,
            geo.bounds.width,
        ) {
            let offset = if let Some(index) = metrics
                .session_keys()
                .iter()
                .position(|candidate| candidate == key)
            {
                let lead = metrics.session_offset_of(index);
                let lower = (lead + width - geo.bounds.width).max(0.0);
                let upper = lead.min(metrics.max_scroll_offset(geo.bounds.width));
                let peek = (geo.bounds.width - width).clamp(0.0, width * PEEK_FRACTION);
                if offset > self.tab_strip.offset {
                    (offset + peek).min(upper)
                } else {
                    (offset - peek).max(lower)
                }
            } else {
                offset
            };
            self.tab_strip.offset = offset;
            return self.tab_strip_scroll_task();
        }
        Task::none()
    }

    pub(crate) fn reveal_active_tab(&mut self) -> Task<Message> {
        let Some(geo) = self.tab_strip.geometry.get() else {
            return Task::none();
        };
        let position = self
            .model
            .tabs
            .iter()
            .position(|tab| tab.id == self.model.active_tab_id)
            .unwrap_or(0);
        let stamp = (
            self.model.active_tab_id.clone(),
            geo.bounds.width.to_bits(),
            position,
            self.model.tabs.len(),
        );
        if self.tab_strip.revealed.as_ref() == Some(&stamp) {
            return Task::none();
        }
        self.tab_strip.revealed = Some(stamp);
        let Some(key) = self.model.doc_tabs.selected().cloned() else {
            return Task::none();
        };
        self.model.doc_tabs.focus(&key);
        self.scroll_tab_key_into_view(&key)
    }
}

#[cfg(test)]
mod tests {}
