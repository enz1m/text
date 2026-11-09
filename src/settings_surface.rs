use crate::line_ending::LineEnding;
use crate::*;
use enzui_widgets::combo_box::enz_combo_box;
use enzui_widgets::modal_host::{ModalPresentation, ModalPresentationKey, SurfacePlacement};
use enzui_widgets::preferences_group::enz_preferences_group;
use enzui_widgets::preferences_row::enz_preferences_row;
use enzui_widgets::settings_dialog::{
    DialogSize, EnzSettingsPresentation, SettingsPage, enz_settings_dialog,
    enz_settings_presentation_for,
};
use iced::widget::column;

pub(crate) const SETTINGS_ID_PREFIX: &str = "text-app-settings";

pub(crate) const LINE_ENDING_GROUP_ID: &str = "text-app-line-endings";
pub(crate) type SettingsPresentation = ModalPresentation<EnzSettingsPresentation>;

impl TextApp {
    fn editor_theme_choices(&self) -> Element<'_, Message> {
        use crate::editor_theme::EditorTheme;
        // Styled and inset like the preference rows around it (Semibold, on the
        // rows' label edge), so it reads as one more row title.
        let heading_inset =
            enzui_widgets::preferences_row::PreferencesRowTokens::default().padding_x;
        let mut grid = iced::widget::column![
            container(
                enz_text(crate::i18n::domain_text("Editor colours"))
                    .size(TextSize::Body)
                    .weight_rung(enzui_theme::WeightRung::Semibold)
            )
            .padding(iced::Padding::from([0.0, heading_inset]))
        ];
        for choices in EditorTheme::ALL.chunks(4) {
            let mut line = iced::widget::row![].spacing(self.spacing().xs());
            for choice in choices {
                let choice = *choice;
                let palette = choice.palette(&self.palette());
                let swatch = enzui_widgets::surface::enz_surface(
                    container(enz_text("Aa").color(palette.text))
                        .center_x(Length::Fill)
                        .center_y(Length::Fixed(28.0)),
                )
                .fill(palette.background)
                .width(Length::Fill);
                let tile = iced::widget::column![
                    swatch,
                    enz_text(crate::i18n::domain_text(choice.label())).size(TextSize::Caption)
                ]
                .align_x(iced::Alignment::Center)
                .spacing(self.spacing().xs())
                .width(Length::Fill);
                line = line.push(
                    enzui_widgets::list_row_surface::enz_list_row_surface(
                        tile,
                        Message::SetEditorTheme(choice),
                    )
                    .selected(self.model.settings.editor_theme == choice)
                    .selectable(true)
                    .content_height(true)
                    .width(Length::Fill)
                    .label(format!(
                        "{}: {}",
                        crate::i18n::domain_text("Editor theme"),
                        crate::i18n::domain_text(choice.label())
                    )),
                );
            }
            grid = grid.push(line);
        }
        grid.spacing(self.spacing().xs()).width(Length::Fill).into()
    }

    pub(crate) fn sync_settings_presentation(&mut self) {
        if self.model.settings_open {
            let resolved = enz_settings_presentation_for(self.settings_window_size());
            if self.settings_presentation.is_open() {
                if let Some(presentation) = self.settings_presentation.value_mut() {
                    *presentation = resolved;
                }
            } else {
                let _ = self.settings_presentation.present(resolved);
            }
        } else if self.settings_presentation.is_open() {
            let _ = self.settings_presentation.dismiss();
        }

        if self.settings_presentation.is_closing()
            && (self.pending_workspace_remove.is_some()
                || self.close_guard.is_pending()
                || self.model.pending_close.is_some()
                || self.model.stats_open
                || self.model.shortcuts_open)
            && let Some(key) = self.settings_presentation.key()
        {
            let _ = self.settings_presentation.finish_exit(key);
        }
    }

    pub(crate) fn finish_settings_exit(&mut self, key: ModalPresentationKey) {
        let _ = self.settings_presentation.finish_exit(key);
    }

    pub(crate) fn settings_presentation_key(&self) -> Option<ModalPresentationKey> {
        self.settings_presentation.key()
    }

    pub(crate) fn settings_is_closing(&self) -> bool {
        self.settings_presentation.is_closing()
    }

    pub(crate) fn resolved_settings_presentation(&self) -> EnzSettingsPresentation {
        if self.settings_presentation.is_closing() {
            self.settings_presentation
                .value()
                .copied()
                .unwrap_or_else(|| enz_settings_presentation_for(self.settings_window_size()))
        } else {
            enz_settings_presentation_for(self.settings_window_size())
        }
    }

    fn settings_builder_presentation(&self) -> EnzSettingsPresentation {
        if self.settings_presentation.is_closing() {
            self.resolved_settings_presentation()
        } else {
            EnzSettingsPresentation::Adaptive
        }
    }

    pub(crate) fn settings_surface_placement(&self) -> SurfacePlacement {
        match self.resolved_settings_presentation() {
            EnzSettingsPresentation::Sheet => SurfacePlacement::BottomEdge,
            EnzSettingsPresentation::Dialog | EnzSettingsPresentation::Adaptive => {
                SurfacePlacement::Centred
            }
        }
    }

    fn settings_window_size(&self) -> iced::Size {
        self.model.window_size.map_or(
            iced::Size::new(crate::model::DEFAULT_WINDOW_WIDTH, 900.0),
            |(w, h)| iced::Size::new(w, h),
        )
    }
    fn settings_control_row<'a>(
        &'a self,
        label: String,
        control: Element<'a, Message>,
    ) -> Element<'a, Message> {
        enz_preferences_row(label)
            .control(control)
            .accessory_width(176.0)
            .label_min_width(80.0)
            .into_element()
    }

    pub(crate) fn view_settings_dialog(&self) -> Element<'_, Message> {
        let pal = self.palette();
        let sp = self.spacing();
        let ty = self.typography();
        let st = &self.model.settings;
        let toggle_row =
            |title: String, checked: bool, msg: fn(bool) -> Message| -> Element<Message> {
                enz_preferences_row(title)
                    .switch(checked, msg)
                    .switch_hover(false)
                    .into_element()
            };
        let toggle_row_enabled = |title: String,
                                  checked: bool,
                                  enabled: bool,
                                  msg: fn(bool) -> Message|
         -> Element<Message> {
            enz_preferences_row(title)
                .enabled(enabled)
                .switch(checked, msg)
                .switch_hover(false)
                .into_element()
        };
        let toggle_row_desc = |title: String,
                               desc: String,
                               checked: bool,
                               msg: fn(bool) -> Message|
         -> Element<Message> {
            enz_preferences_row(title)
                .subtitle(desc)
                .switch(checked, msg)
                .switch_hover(false)
                .into_element()
        };

        let font_select = enz_combo_box(
            "",
            FONT_SCALES
                .iter()
                .map(|(label, _)| crate::i18n::domain_text(label))
                .collect(),
            FONT_SCALES
                .iter()
                .find(|(_, scale)| *scale == st.font_size)
                .map(|(label, _)| crate::i18n::domain_text(label)),
            |_| Message::NoOp,
        )
        .aria_label(enzui_core::localization::text("text-font-size-6d784d"))
        .width(Length::Fixed(176.0))
        .on_select_index(|index| {
            Message::SetFontSize(
                FONT_SCALES
                    .get(index)
                    .map_or(FontSizeScale::Medium, |(_, scale)| *scale),
            )
        })
        .id_prefix("text-app-font-size")
        .into_element();
        let font_row = self.settings_control_row(
            enzui_core::localization::text("text-font-size-6d784d"),
            font_select,
        );
        let endings = [LineEnding::Lf, LineEnding::Crlf];
        let ending_select = enz_combo_box(
            "",
            endings
                .iter()
                .map(|ending| ending.display_label())
                .collect(),
            Some(
                self.model
                    .active_tab()
                    .line_ending
                    .written_as()
                    .display_label(),
            ),
            |_| Message::NoOp,
        )
        .aria_label(enzui_core::localization::text("text-line-endings-c948fb"))
        .width(Length::Fixed(176.0))
        .on_select_index(|index| {
            Message::SetLineEnding(if index == 1 {
                LineEnding::Crlf
            } else {
                LineEnding::Lf
            })
        })
        .enabled(
            self.model
                .active_tab()
                .supports(large_doc::Capability::LineEnding),
        )
        .id(iced::advanced::widget::Id::new(LINE_ENDING_GROUP_ID))
        .into_element();
        let encoding_row = column![
            self.settings_control_row(
                enzui_core::localization::text("text-line-endings-c948fb"),
                ending_select
            ),
            self.settings_control_row(
                enzui_core::localization::text("text-encoding-4d25a4"),
                enz_text("UTF-8")
                    .size(TextSize::Body)
                    .color(pal.text_muted)
                    .into()
            ),
        ]
        .spacing(sp.sm());

        let group = enz_preferences_group(
            column![
                toggle_row_desc(
                    enzui_core::localization::text("text-show-line-numbers-d03262"),
                    enzui_core::localization::text(
                        "text-display-a-left-gutter-with-1-based-fdd2bd"
                    ),
                    st.show_line_numbers,
                    |_| Message::ToggleLineNumbers,
                ),
                toggle_row_enabled(
                    enzui_core::localization::text("text-word-wrap-9e210f"),
                    st.word_wrap,
                    self.model
                        .active_tab()
                        .supports(large_doc::Capability::WordWrap),
                    |_| Message::ToggleWordWrap,
                ),
                toggle_row_desc(
                    enzui_core::localization::text("text-show-live-counts-3ceeee"),
                    enzui_core::localization::text(
                        "text-character-word-and-line-counts-in-the-cbbeab"
                    ),
                    st.show_counts,
                    |_| Message::ToggleCounts,
                ),
                toggle_row(
                    enzui_core::localization::text("text-restore-previous-session-f688f5"),
                    st.restore_session,
                    |_| { Message::ToggleRestoreSession }
                ),
                font_row,
                self.editor_theme_choices(),
                encoding_row,
            ]
            .spacing(sp.sm()),
        )
        .spacing_scale(sp)
        .typography_scale(ty);

        let _ = (pal, ty);
        let window = self.settings_window_size();
        // Shared-host surfaces must branch explicitly because `into_surface` ignores presentation.
        let builder = enz_settings_dialog::<Message>(true, window)
            .title(enzui_core::localization::text("text-settings-74a883"))
            // The framework's dialog title, as every other app's settings use.
            .title_typography(ty)
            .size(DialogSize::Compact)
            .fit_content(true)
            .presentation(self.settings_builder_presentation())
            .page(
                SettingsPage::new(
                    "general",
                    enzui_core::localization::text("text-general-c910d4"),
                ),
                group,
            )
            .page_id_prefix(SETTINGS_ID_PREFIX);
        let on_done: Box<dyn Fn() -> Message> = Box::new(|| Message::CloseSettings);
        match builder.resolved_presentation() {
            EnzSettingsPresentation::Sheet => builder.into_sheet_surface(Some(on_done)),
            EnzSettingsPresentation::Dialog | EnzSettingsPresentation::Adaptive => {
                builder.into_surface(Some(on_done))
            }
        }
    }
}
