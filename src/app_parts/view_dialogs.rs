//! TextApp status bar, find bar and dialog views.

use super::*;

use iced::widget::column;

impl TextApp {
    // Status bar via `enz_status_bar_strip`: item ids are focus-restore targets,
    // the language pill anchors its menu with `anchored_overlay`, and forwarded
    // runtime tokens set the band height, text size and padding.
    pub(crate) fn view_status_bar(&self, tab: &Tab) -> Element<'_, Message> {
        let sp = self.spacing();

        // A paged read-only tab has no caret and no complete buffer, so it reports
        // the mode and source size; the viewer publishes its own visible range.
        let pos_readout = match tab.buffer.large() {
            Some(doc) => enzui_core::localized!("text-0a856ac7b5b4", "mode" =>
                if doc.is_binary_preview() {
                    enzui_core::localization::text("text-binary-preview-9be483")
                } else {
                    enzui_core::localization::text("text-read-only-72bb90")
                },
                "size" => large_doc::format_bytes(doc.byte_len)
            ),
            // Not plain text: the caret is real, but the tab cannot be edited.
            None if tab.decoded_lossily => {
                enzui_core::localized!("text-read-only-ln-p0-col-p1-fa6ab3", "p0" => self.model.cursor_line, "p1" => self.model.cursor_col)
            }
            None => {
                enzui_core::localized!("text-ln-p0-col-p1-b0505d", "p0" => self.model.cursor_line, "p1" => self.model.cursor_col)
            }
        };

        let mut strip = enz_status_bar_strip()
            .sizing(self.themes.theme().sizing)
            .typography(self.themes.theme().typography)
            .spacing_scale(sp)
            // Markdown's only preview switch must remain reachable on compact
            // windows; the complete readout can yield to its footer actions.
            .leading_priority(if tab.is_markdown() { 1 } else { 2 })
            .trailing_priority(if tab.is_markdown() { 2 } else { 1 })
            .trailing_atomic(true)
            // Embedded: no paint of its own, ink resolved against
            // `palette.background`. `Color::TRANSPARENT` would resolve ink against
            // black and pick light ink on a light theme.
            .embedded();

        strip = strip.push_leading(StatusBarItem::text(pos_readout));
        if self.model.settings.show_counts && !tab.is_read_only() {
            // Read-only: `text_stats_ready` never starts a scan. Large documents
            // get counts from `Message::StatsCompleted` and edit deltas.
            let label = match tab.text_stats_ready() {
                Some(s) => {
                    enzui_core::localized!("text-p0-chars-p1-words-p2-lines-c76241", "p0" => s.chars, "p1" => s.words, "p2" => s.lines)
                }
                // Only a buffer whose counts were deferred reports counting.
                None if tab.stats_pending() => {
                    enzui_core::localized!("text-counting-p0-lines-c30674", "p0" => tab.total_lines())
                }
                None => {
                    let s = tab.text_stats();
                    enzui_core::localized!("text-p0-chars-p1-words-p2-lines-c76241", "p0" => s.chars, "p1" => s.words, "p2" => s.lines)
                }
            };
            strip = strip.push_leading(StatusBarItem::separator());
            strip = strip.push_leading(StatusBarItem::text(label));
        }

        // Tab-indent mode is shown only for an editable tab.
        if self.tab_indent_mode && !tab.is_read_only() {
            strip = strip.push_leading(StatusBarItem::separator());
            strip = strip.push_leading(StatusBarItem::text(enzui_core::localization::text(
                "text-tab-indents-cb786f",
            )));
        }

        let le_label = tab.line_ending.display_label();
        let lang_label = tab
            .display_language()
            .unwrap_or_else(|| enzui_core::localization::text("text-plain-text-7b5cb3"));

        let lang_popup: Option<Element<'_, Message>> = if self.model.language_menu_open {
            let mut menu = enz_menu().width(160.0).id_prefix("text-app-language-menu");
            for label in enzui_widgets::syntax::Language::OVERRIDE_LABELS {
                menu = menu.push_item(EnzMenuItem::command(
                    *label,
                    crate::i18n::domain_text(label),
                    Message::SetLanguage((*label).to_owned()),
                ));
            }
            Some(menu.into())
        } else {
            None
        };

        if tab.is_markdown() {
            strip = strip.push_trailing(
                StatusBarItem::action(
                    crate::i18n::domain_text(if tab.preview_visible() {
                        "Edit"
                    } else {
                        "Preview"
                    }),
                    Message::ToggleMarkdownPreview {
                        tab: tab.id.clone(),
                        version: tab.snapshot_version(),
                    },
                )
                .id(iced::advanced::widget::Id::new("text-app-markdown-preview")),
            );
        }

        // `StatusBarItem::Action` has no disabled state, so pills a paged tab
        // cannot honour stay operable and refuse with the keyboard route's
        // explanation (`refuse_capability`).
        strip = strip
            .push_trailing(
                StatusBarItem::action(
                    enzui_core::localization::text("text-stats-b4a736"),
                    Message::OpenStats,
                )
                .id(stats_button_id()),
            )
            .push_trailing(
                StatusBarItem::action(le_label, Message::CycleLineEnding).id(line_ending_pill_id()),
            )
            .push_trailing(
                StatusBarItem::action(
                    crate::i18n::domain_text(&lang_label),
                    Message::ToggleLanguageMenu,
                )
                .id(language_pill_id())
                .anchored_overlay(move |lang_trigger| {
                    anchored_overlay(lang_trigger, lang_popup)
                        .dismiss_on_outside_press(|| Message::DismissPopovers)
                        .placement(
                            Placement::new(Side::Top)
                                .gap(4.0)
                                .alignment(PlacementAlignment::End),
                        )
                        .into()
                }),
            );

        strip.into()
    }

    pub(crate) fn overflow_menu_actions() -> Vec<ToolbarAction> {
        ToolbarAction::ALL.to_vec()
    }

    pub(crate) fn view_find_bar(&self) -> Element<'_, Message> {
        let s = &self.model.search;
        // A tab switch can put a paged read-only tab under an open bar; it has no
        // match set, so no match state may be claimed.
        let searchable = self
            .model
            .active_tab()
            .supports(large_doc::Capability::Find);
        let has_matches = searchable && !s.matches.is_empty();
        let counter = if searchable {
            s.counter_label()
        } else {
            crate::i18n::domain_text(large_doc::UNSEARCHABLE_COUNTER)
        };

        use enzui_widgets::find_replace_bar::enz_find_option;

        let scope: Element<Message> = enzui_widgets::combo_box::enz_combo_box(
            "",
            search::ReplaceScope::ALL
                .iter()
                .map(|scope| scope.label())
                .collect(),
            Some(s.replace_scope.label()),
            |_| Message::NoOp,
        )
        .aria_label(enzui_core::localization::text("text-replace-scope-eae133"))
        .size(enzui_widgets::text_field::TextFieldSize::Sm)
        .width(Length::Fixed(176.0))
        .on_select_index(Message::SetReplaceScope)
        .id_prefix("text-replace-scope")
        .into_element();

        enz_find_replace_bar(
            &s.query,
            &s.replacement,
            counter,
            s.replace_open,
            |event| match event {
                FindReplaceEvent::QueryChanged(value) => Message::FindQueryChanged(value),
                FindReplaceEvent::ReplacementChanged(value) => Message::ReplaceValueChanged(value),
                FindReplaceEvent::Previous => Message::FindPrev,
                FindReplaceEvent::Next => Message::FindNext,
                FindReplaceEvent::ToggleReplace => Message::ToggleReplaceBar,
                FindReplaceEvent::Replace => Message::ReplaceCurrent,
                FindReplaceEvent::ReplaceAll => Message::ReplaceAll,
                FindReplaceEvent::Close => Message::CloseFind,
            },
        )
        // Stable ids: focus tasks and focus-return gates address these by id.
        .query_id(find_field_id())
        .replacement_id(replace_field_id())
        .replacement_width(Length::Fill)
        .close_id(find_close_id())
        .can_navigate(has_matches)
        .can_replace(has_matches)
        .option(enz_find_option(
            "case-sensitive",
            &enzui_core::localization::text("text-match-case-619880"),
            s.options.case_sensitive,
            Message::ToggleCaseSensitive,
        ))
        .option(enz_find_option(
            "whole-word",
            &enzui_core::localization::text("text-whole-word-9a99f7"),
            s.options.whole_word,
            Message::ToggleWholeWord,
        ))
        .option(enz_find_option(
            "regex",
            &enzui_core::localization::text("text-regular-expression-9f896c"),
            s.options.regex,
            Message::ToggleRegex,
        ))
        .replace_scope(scope)
        .announcement(if searchable && !s.error.is_empty() {
            Announcement::Alert
        } else {
            Announcement::Status
        })
        .into_element()
    }

    pub(crate) fn view_goto_bar(&self) -> Element<'_, Message> {
        let pal = self.palette();
        let sp = self.spacing();
        let input: Element<Message> = enz_text_field(
            "",
            enzui_core::localization::text("text-line-number-6e2a7a"),
            &self.model.goto_query,
            Message::GotoQueryChanged,
        )
        .input_purpose(iced::advanced::input_method::Purpose::Number)
        .on_submit(Message::GotoSubmit)
        .aria_label(enzui_core::localization::text("text-line-number-6e2a7a"))
        .id(goto_field_id())
        .into();
        // Content-fit width for a line number; no sizing token covers inputs.
        let input = container(input).width(Length::Fixed(160.0));
        let go = enz_button(
            enzui_core::localization::text("text-go-6cc851"),
            Message::GotoSubmit,
        )
        .kind(ButtonKind::Secondary)
        // The default height, next to the field: Xs was a 22 px button beside
        // a 32 px field.
        .size(ButtonSize::Md);
        let close = enz_toolbar_icon("action.close", Message::CloseGoto)
            .label(enzui_core::localization::text(
                "text-close-go-to-line-62dc25",
            ))
            .id(goto_close_id());
        let rtl = enzui_core::placement::ambient_direction()
            == enzui_core::placement::Direction::RightToLeft;
        let controls = row![input, go]
            .mirrored(rtl)
            .spacing(sp.sm())
            .align_y(iced::Alignment::Center);
        let r = row![
            controls,
            iced::widget::Space::new().width(Length::Fill),
            close
        ]
        .mirrored(rtl)
        .spacing(sp.sm())
        .align_y(iced::Alignment::Center)
        .height(Length::Fixed(BAR_HEIGHT));
        container(r)
            .width(Length::Fill)
            .padding(Padding::from([sp.sm(), sp.md()]))
            .style(bar_style(pal))
            .into()
    }

    pub(crate) fn view_stats_dialog(&self, tab: &Tab) -> Element<'_, Message> {
        use enzui_widgets::modal_surface::{CLOSE_CONTROL_SIZE, close_inset};

        let pal = self.palette();
        let sp = self.spacing();
        let ty = self.typography();
        let header_inset = close_inset(enzui_theme::text_defaults::shape_radius(|s| s.dialog));
        let heading = || -> Element<'_, Message> {
            container(
                enz_text(enzui_core::localization::text(
                    "text-document-statistics-308052",
                ))
                .size(TextSize::Title3)
                .color(pal.text)
                .heading(HeadingLevel::Page),
            )
            .height(Length::Shrink.min(CLOSE_CONTROL_SIZE))
            .align_y(iced::Alignment::Center)
            .into()
        };
        // A tab switch can put a paged document under an open dialog; explain the
        // refusal instead of rendering zero counts.
        if !tab.supports(large_doc::Capability::Stats) {
            let body = column![
                heading(),
                iced::widget::Space::new().height(Length::Fixed(sp.sm())),
                enz_text(large_doc::capability_refused_sentence(
                    large_doc::Capability::Stats,
                    &tab.title(),
                ))
                .size(TextSize::Body)
                .color(pal.text_muted)
                .wrapping(iced::widget::text::Wrapping::Word),
            ]
            .spacing(sp.sm());
            return enz_modal_surface(body.into())
                .width(Length::Fixed(380.0))
                .padding(sp.lg())
                .padding_top(header_inset)
                .on_dismiss(|| Message::CloseStats)
                .close_label(enzui_core::localization::text(
                    "text-close-document-statistics-3fc676",
                ))
                .into_element();
        }
        // The `Selection` group reads only worker-counted values; a large
        // selection substring is still large.
        let s = tab.document_stats();
        let row_of = |k: String, v: String| -> Element<Message> {
            enz_labeled_content(k)
                .value(v)
                // Wide enough for "Characters (no spaces)", which elided at 160.
                .label_width(200.0)
                .typography(ty)
                // Values are short; stacking would only make the dialog taller.
                .auto_compact(false)
                .into_element()
        };
        // No form inset: `enz_modal_surface` already pads the dialog.
        let stats_form = enz_form_layout(vec![
            row_of(
                enzui_core::localization::text("text-characters-6fac39"),
                crate::i18n::number(s.chars),
            ),
            row_of(
                enzui_core::localization::text("text-characters-no-spaces-d81184"),
                crate::i18n::number(s.chars_no_spaces),
            ),
            row_of(
                enzui_core::localization::text("text-words-0a8be8"),
                crate::i18n::number(s.words),
            ),
            row_of(
                enzui_core::localization::text("text-lines-3b26a5"),
                crate::i18n::number(s.lines),
            ),
            row_of(
                enzui_core::localization::text("text-bytes-4d8000"),
                crate::i18n::number(s.bytes),
            ),
            row_of(
                enzui_core::localization::text("text-reading-time-7d2e3e"),
                s.reading_time_label(),
            ),
        ])
        .spacing_scale(sp)
        .form_padding(0.0);

        // Shown only with a selection (O(1) anchor test). Values come only from
        // a worker count for this exact selection; the view never calls
        // `Content::selection()`, which copies the selection on every repaint.
        // An outstanding count shows `Counting…`, not the previous selection's.
        let selection_group: Option<Element<Message>> = tab.has_selection().then(|| {
            let ready = tab.selection_stats_ready();
            let value = |pick: fn(&doc_stats::Stats) -> String| -> String {
                ready.as_ref().map_or_else(
                    || enzui_core::localization::text("text-counting-f323a6"),
                    pick,
                )
            };
            let sel_form = enz_form_layout(vec![
                row_of(
                    enzui_core::localization::text("text-characters-6fac39"),
                    value(|s| crate::i18n::number(s.chars)),
                ),
                row_of(
                    enzui_core::localization::text("text-characters-no-spaces-d81184"),
                    value(|s| crate::i18n::number(s.chars_no_spaces)),
                ),
                row_of(
                    enzui_core::localization::text("text-words-0a8be8"),
                    value(|s| crate::i18n::number(s.words)),
                ),
                row_of(
                    enzui_core::localization::text("text-lines-3b26a5"),
                    value(|s| crate::i18n::number(s.lines)),
                ),
                row_of(
                    enzui_core::localization::text("text-bytes-4d8000"),
                    value(|s| crate::i18n::number(s.bytes)),
                ),
                row_of(
                    enzui_core::localization::text("text-reading-time-7d2e3e"),
                    value(super::doc_stats::Stats::reading_time_label),
                ),
            ])
            .spacing_scale(sp)
            .form_padding(0.0);
            column![
                enz_text(enzui_core::localization::text("text-selection-ca4067"))
                    .size(TextSize::Title3)
                    .color(pal.text)
                    // A section inside the dialog, whose title is the Page rung.
                    .heading(HeadingLevel::Section),
                iced::widget::Space::new().height(Length::Fixed(sp.sm())),
                sel_form,
            ]
            .spacing(sp.sm())
            .into()
        });

        let mut items: Vec<Element<Message>> = vec![
            heading(),
            iced::widget::Space::new()
                .height(Length::Fixed(sp.sm()))
                .into(),
            stats_form.into(),
        ];
        if let Some(group) = selection_group {
            items.push(
                iced::widget::Space::new()
                    .height(Length::Fixed(sp.md()))
                    .into(),
            );
            items.push(group);
        }
        let body = column(items).spacing(sp.sm());
        let _ = pal;
        // Between `dialog_width_sm` (360) and `dialog_width_md` (460): one is too
        // cramped for the rows, the other too wide. 420 leaves the value column
        // room for "7 minutes 35 seconds" beside the 200 px labels.
        enz_modal_surface(body.into())
            .width(Length::Fixed(420.0))
            .height(Length::Shrink)
            .padding(sp.lg())
            .padding_top(header_inset)
            .on_dismiss(|| Message::CloseStats)
            .close_label(enzui_core::localization::text(
                "text-close-document-statistics-3fc676",
            ))
            .into_element()
    }
    // Built from `ALL_BINDINGS` via `shortcut_dialog_entries()`, so it cannot
    // drift from the dispatcher.
    pub(crate) fn view_shortcuts_dialog(&self) -> Element<'_, Message> {
        enz_shortcuts_dialog::<Message>(true, keybindings::shortcut_dialog_entries())
            .title(enzui_core::localization::text(
                "text-keyboard-shortcuts-59cdaa",
            ))
            .into_surface(Some(Box::new(|| Message::CloseShortcuts)))
    }

    // Save → Retry, Discard → close without clearing (recovered edits may
    // reappear), Cancel → keep the window open.
    pub(crate) fn view_recovery_discard_failure(&self) -> Element<'_, Message> {
        enz_save_changes_dialog(
            enzui_core::localization::text("text-couldn-t-clear-crash-recovery-498c6e"),
            enzui_core::localization::text("text-the-crash-recovery-file-could-not-be-152203"),
            Message::RecoveryDiscardEvent,
        )
        .detail(enzui_core::localization::text(
            "text-close-without-clearing-it-and-your-recovered-933bc5",
        ))
        .save_label(enzui_core::localization::text("text-retry-942087"))
        .discard_label(enzui_core::localization::text(
            "text-close-without-clearing-e3b401",
        ))
        .cancel_label(enzui_core::localization::text(
            "text-keep-window-open-796ae0",
        ))
        .id_prefix("text-app-recovery-discard")
        .into_body()
    }

    pub(crate) fn view_window_close_confirm(&self) -> Element<'_, Message> {
        let names: Vec<String> = self
            .close_guard
            .pending_items()
            .map(|item| format!("• {}", item.name))
            .collect();
        let (save, discard) = if names.len() > 1 {
            (
                enzui_core::localization::text("text-save-all-d3c54e"),
                enzui_core::localization::text("text-discard-all-e5d95b"),
            )
        } else {
            (
                enzui_core::localization::text("text-save-1509f5"),
                enzui_core::localization::text("text-discard-eb1a70"),
            )
        };
        let list = names.join("\n");
        enz_save_changes_dialog(
            enzui_core::localization::text("text-save-changes-before-closing-6239f0"),
            enzui_core::localization::text(
                "text-the-following-documents-have-unsaved-changes-fd5702",
            ),
            |event| match event {
                SaveChangesEvent::Action(SaveChangesAction::SaveAll) => Message::WindowCloseSaveAll,
                SaveChangesEvent::Action(SaveChangesAction::Discard) => Message::WindowCloseDiscard,
                SaveChangesEvent::Action(SaveChangesAction::Cancel) => Message::WindowCloseCancel,
                SaveChangesEvent::FocusNext => {
                    Message::DialogEvent(enzui_widgets::modal_host::ModalHostEvent::FocusNext)
                }
                SaveChangesEvent::FocusPrevious => {
                    Message::DialogEvent(enzui_widgets::modal_host::ModalHostEvent::FocusPrevious)
                }
            },
        )
        .detail(list)
        .save_label(save)
        .discard_label(discard)
        .saving(self.close_guard.is_saving())
        .id_prefix(crate::dialog_focus::WINDOW_CLOSE_ID_PREFIX)
        .into_body()
    }

    pub(crate) fn view_workspace_remove_confirm(
        &self,
        root: &std::path::Path,
    ) -> Element<'_, Message> {
        let name = self
            .workspaces
            .workspaces
            .iter()
            .find(|workspace| workspace.root == root)
            .map_or_else(
                || enzui_core::localization::text("text-this-workspace-3afd67"),
                |workspace| workspace.name.clone(),
            );
        enz_confirm_dialog(
            enzui_core::localization::text("text-remove-workspace-686606"),
            enzui_core::localized!("text-remove-name-from-the-sidebar-this-deletes-702f98", "name" => name),
            &self.workspace_remove_confirm_state,
            Message::WorkspaceRemoveEvent,
        )
        .confirm_label(enzui_core::localization::text("text-remove-c3812f"))
        .cancel_label(enzui_core::localization::text("text-cancel-19766e"))
        .destructive(true)
        .id_prefix("text-app-remove-workspace")
        .into_surface()
    }

    pub(crate) fn view_close_confirm(&self, id: &str) -> Element<'_, Message> {
        let name = self.model.tabs.iter().find(|t| t.id == id).map_or_else(
            || enzui_core::localization::text("text-this-document-3d8782"),
            |t| crate::safe_text::bounded_inline(&t.title()),
        );
        enz_confirm_dialog(
            enzui_core::localization::text("text-unsaved-changes-a710c2"),
            enzui_core::localized!("text-name-has-unsaved-changes-close-without-saving-090358", "name" => name),
            &self.close_confirm_state,
            Message::CloseConfirmEvent,
        )
        .confirm_label(enzui_core::localization::text("text-close-anyway-bb84f7"))
        .cancel_label(enzui_core::localization::text("text-cancel-19766e"))
        .destructive(true)
        // Alert dismissal: Escape and the scrim do not discard.
        .alert(true)
        .id_prefix("text-app-close-confirm")
        .into_surface()
    }
}
