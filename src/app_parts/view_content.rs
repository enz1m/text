//! TextApp content and editor views.

use super::*;

use iced::widget::column;

impl TextApp {
    pub(crate) fn view_content_pane(&self) -> Element<'_, Message> {
        // Size transitions reserve editor height; wrappers stay mounted while hidden.
        let center: Vec<Element<Message>> = vec![
            motion(
                self.view_find_bar(),
                self.model.search.open,
                MotionEffect::Size(MotionAxis::Vertical),
            )
            .into(),
            motion(
                self.view_goto_bar(),
                self.model.goto_open,
                MotionEffect::Size(MotionAxis::Vertical),
            )
            .into(),
            self.view_editor_row(),
            container(self.view_status_bar(self.model.active_tab()))
                .padding(Padding::from([0.0, 0.0]).bottom(EDITOR_INTERNAL_PAD))
                .width(Length::Fill)
                .into(),
        ];

        column(center)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub(crate) fn view_editor_row(&self) -> Element<'_, Message> {
        let tab = self.model.active_tab();
        let palette = self.model.settings.editor_theme.palette(&self.palette());
        let mut document_theme = self.themes.theme().clone();
        document_theme.palette = palette.clone();
        let body: Element<Message> = if tab.preview_visible() {
            let snapshot = tab
                .preview
                .source
                .as_ref()
                .expect("visible preview has a generation");
            let blocks = std::sync::Arc::clone(&snapshot.blocks);
            let version = snapshot.version;
            let font_size = self.model.settings.font_size.px();
            let document =
                iced::widget::lazy((tab.id.clone(), version, font_size.to_bits()), move |_| {
                    enzui_widgets::markdown::enz_markdown_blocks::<Message>(blocks.as_ref().clone())
                        .body_size(font_size)
                        .into_element()
                });
            let owner = tab.id.clone();
            let preview_version = version;
            enz_scroll_view(document)
                .id(iced::advanced::widget::Id::from(format!(
                    "text-preview-{}",
                    tab.id
                )))
                .inset(self.spacing().lg())
                .height(Length::Fill)
                .backdrop(palette.background)
                .initial_offset(iced::widget::scrollable::AbsoluteOffset {
                    x: 0.0,
                    y: tab.preview.scroll,
                })
                .on_scroll(move |viewport| Message::MarkdownPreviewScrolled {
                    tab: owner.clone(),
                    version: preview_version,
                    offset: viewport.absolute_offset().y,
                })
                .into()
        } else {
            self.view_editor_row_inner(
                self.model.settings.show_line_numbers,
                self.model.settings.word_wrap,
                self.model.settings.font_size.px(),
                // The gutter's own inset on each side of the numbers: `md`
                // leaves the numbers a clear column (about 17 px to the text
                // with the editor's own pad) instead of touching it.
                self.spacing().md(),
            )
        };
        let body = container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| iced::widget::container::Style {
                background: Some(palette.background.into()),
                ..Default::default()
            });
        iced::widget::themer(Some(document_theme.iced_theme()), body).into()
    }

    pub(crate) fn view_editor_row_inner(
        &self,
        show_line_numbers: bool,
        word_wrap: bool,
        font_px: f32,
        pad_x: f32,
    ) -> Element<'_, Message> {
        use enzui_widgets::line_number_gutter::editor_gutter;
        use enzui_widgets::text_measure::{MeasureTypography, TextMeasureKind};

        let pal = self.model.settings.editor_theme.palette(&self.palette());
        let tab = self.model.active_tab();
        let theme = self.themes.theme();

        // Large read-only mode: the shared paged viewer materializes only visible
        // rows plus bounded overscan. Wrap, line numbers, highlighting and the
        // editor scrollbar do not apply to it.
        if let Some(doc) = tab.buffer.large() {
            let _ = (word_wrap, show_line_numbers, pad_x, font_px);
            // A terminal failure replaces the viewer with one line rather than
            // presenting cached rows as current; reason and retry are in the banner.
            if doc.load_error().is_some() {
                return container(
                    enz_text(enzui_core::localization::text(
                        "text-no-text-is-displayed-reading-this-document-5b37b4",
                    ))
                    .size(TextSize::Body)
                    .color(pal.text_muted)
                    .wrapping(iced::widget::text::Wrapping::Word),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(self.spacing().md())
                .into();
            }
            // A conditional host wrapper would replace the viewer subtree and its focus state.
            let viewer = match doc.byte_preview() {
                Some((state, viewport)) => enzui_widgets::paged_text_viewer::enz_paged_byte_viewer(
                    state,
                    viewport,
                    Message::PagedNavigate,
                ),
                None => enzui_widgets::paged_text_viewer::enz_paged_text_viewer(
                    doc.state(),
                    doc.viewport(),
                    Message::PagedNavigate,
                ),
            };
            return viewer
                .label(enzui_core::localized!("text-5c6a8781369d", "name" => crate::safe_text::bounded_inline(&tab.title()), "mode" =>
                    if doc.is_binary_preview() {
                        enzui_core::localization::text("text-115961becea8")
                    } else {
                        enzui_core::localization::text("text-be7208bc1b8f")
                    }
                ))
                .font_from_theme(theme)
                .typography(theme.typography)
                .text_size_px(font_px)
                .backdrop(pal.background)
                .viewport_state(doc.viewport_state())
                .into_element();
        }

        let wrapping = if word_wrap {
            enzui_widgets::text_area::WrapMode::Word
        } else {
            enzui_widgets::text_area::WrapMode::None
        };

        // Gutter wrapping and width must use the family painted by the editor.
        let gutter_typo = MeasureTypography::new(enzui_widgets::enz_text::TextSize::Body)
            .kind(TextMeasureKind::Monospace)
            .pin_size_px(font_px)
            .font_from_theme(theme);

        let gutter = show_line_numbers.then(|| {
            editor_gutter(tab.total_lines(), gutter_typo, pad_x)
                .line_offset(tab.editor_line_origin())
                .color(pal.text_subtle)
        });

        let font_size = font_px;

        // Language comes from the extension, then a content heuristic, unless
        // manually overridden.
        let language = tab.syntax_language();
        // Each match range costs a renderer prefix walk; limit paint to the viewport,
        // rather than supplying the document-wide search result set.
        let row_px = EDITOR_LINE_HEIGHT * font_px;
        let line_origin = tab.editor_line_origin();
        // Matches paint only while bound to the active tab's
        // `(id, snapshot_version)`; a stale binding paints nothing.
        let find_paint = if self
            .model
            .search
            .binding_matches(&tab.id, tab.snapshot_version())
        {
            crate::match_ink::match_paint(
                &self.model.search,
                tab.document(),
                line_origin,
                visible_line_window(&tab.content, line_origin, tab.total_lines(), row_px),
                &pal,
            )
        } else {
            crate::match_ink::MatchPaint::default()
        };
        let crate::match_ink::MatchPaint {
            fills: find_highlights,
            runs: find_runs,
        } = find_paint;

        let drag_tab = tab.id.clone();
        let mut tokens = enzui_widgets::text_area::TextAreaTokens::from_palette(&pal);
        tokens.background = pal.background;
        tokens.text = pal.text;
        tokens.muted = pal.text_subtle;
        tokens.selection = pal.surface_selected;
        tokens.padding = EDITOR_INTERNAL_PAD;
        let editor = enzui_widgets::text_area::enz_text_area(&tab.content, Message::EditorAction)
            .id(editor_id())
            .aria_label(enzui_core::localized!("text-p0-document-1eac49", "p0" => crate::safe_text::bounded_inline(&tab.title())))
            .variant(enzui_widgets::text_area::TextAreaVariant::Plain)
            .tokens(tokens)
            .read_only(tab.is_read_only())
            // The widget already applied the drag; replaying it here would move the
            // selection twice. The owner is carried because tabs may switch first.
            .on_drag_update(move |cursor| Message::EditorDragUpdated {
                tab: drag_tab.clone(),
                cursor,
            })
            .font_role(enzui_theme::FontRole::Monospace)
            .font_from_theme(theme)
            .text_size_px(font_size)
            .line_height(EDITOR_LINE_HEIGHT)
            .wrap(wrapping)
            .fill_height()
            .placeholder(enzui_core::localization::text("text-start-typing-or-open-a-file-40d894"))
            .background_highlights(find_highlights)
            .highlight_with::<enzui_widgets::match_ink::MatchInkHighlighter>(
                enzui_widgets::match_ink::MatchInkSettings::new(language, find_runs),
                enzui_widgets::match_ink::format_for_editor_ink,
            );
        // A focused editor indents with Tab / Shift+Tab when there is a selection
        // or Tab-indent mode is on; otherwise Tab traverses. In viewport mode the
        // clipboard commands reroute to the chunked selection after the indent
        // check, since cosmic copies only its local frame.
        let is_viewport_editor = tab.uses_viewport_editor();
        let tab_indent_mode = self.tab_indent_mode;
        let capture_tab = tab.has_selection() || tab_indent_mode;
        let editor = editor.key_binding(move |press| {
            if let Some(binding) = editor_redo_binding(&press) {
                return Some(binding);
            }
            if let Some(binding) = editor_indent_binding(&press, capture_tab) {
                return Some(binding);
            }
            // Escape leaves Tab-indent mode so the next Tab traverses (WCAG 2.1.2).
            if tab_indent_mode
                && matches!(press.status, text_editor::Status::Focused { .. })
                && matches!(
                    press.key,
                    keyboard::Key::Named(keyboard::key::Named::Escape)
                )
                && !press.modifiers.control()
                && !press.modifiers.alt()
                && !press.modifiers.logo()
            {
                return Some(text_editor::Binding::Custom(Message::SetTabIndentMode(
                    false,
                )));
            }
            match text_editor::Binding::from_key_press(press) {
                Some(text_editor::Binding::Copy) if is_viewport_editor => {
                    Some(text_editor::Binding::Custom(Message::VirtualCopy))
                }
                Some(text_editor::Binding::Cut) if is_viewport_editor => {
                    Some(text_editor::Binding::Custom(Message::VirtualCut))
                }
                other => other,
            }
        });
        let editor = if let Some(gutter) = gutter {
            editor.gutter(gutter)
        } else {
            editor
        };
        // The field's prefix value must not advertise whole-document SetValue.
        // Its native menu overlays remain outside this semantic boundary.
        let editor_element: Element<Message> = enzui_widgets::decorative::decorative(editor).into();

        let scrollbar = enz_editor_scrollbar(editor_element, &tab.content, Message::EditorAction)
            .horizontal(!word_wrap);
        let scrollbar = if let Some(projection) = tab.editor_scroll_projection() {
            scrollbar.projection(projection)
        } else if tab.uses_viewport_editor() {
            scrollbar.metrics(tab.editor_scroll_metrics())
        } else {
            scrollbar
        };
        let editor_with_scrollbar: Element<Message> = scrollbar.into();
        // Only the chunked viewport needs lazy text; it reuses the id from
        // `sync_virtual_text_binding`.
        let editor_with_scrollbar: Element<Message> = if tab.uses_viewport_editor() {
            enzui_widgets::virtual_text_surface::enz_virtual_text_surface(
                editor_with_scrollbar,
                self.virtual_text_surface.clone(),
                enzui_core::localized!("text-p0-document-1eac49", "p0" => crate::safe_text::bounded_inline(&tab.title())),
            )
            .read_only(tab.is_read_only())
            .focus_id(editor_id())
            .on_input_method(Message::VirtualInputMethod)
            .into()
        } else {
            // The bounded prefix is disclosure only; full-value replacement
            // belongs to the app-owned document provider.
            let mut semantics =
                enzui_core::semantics::Semantics::new(enzui_core::semantics::Role::TextInput)
                    .label(enzui_core::localized!("text-p0-document-1eac49", "p0" => crate::safe_text::bounded_inline(&tab.title())))
                    .read_only(tab.is_read_only());
            let value = enzui_widgets::text_area::semantics_value(
                &tab.content,
                enzui_widgets::text_area::SEMANTICS_VALUE_BUDGET,
            );
            if !value.is_empty() {
                semantics = semantics.value(value);
            }
            enzui_widgets::text_field_semantics::TextFieldSemantics::new(
                editor_with_scrollbar,
                editor_id(),
                Some(editor_id()),
                semantics,
            )
            .into()
        };

        // No inset between editor and gutter; padding would expose the pane
        // surface as a grey card.
        let editor_with_padding: Element<Message> = container(editor_with_scrollbar)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        editor_with_padding
    }
}
