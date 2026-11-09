use super::*;
use iced::widget::text_editor::{Action, Edit, Motion};

fn edit(app: &mut TextApp, action: Action) {
    let _ = app.update(Message::EditorAction(action));
}

#[test]
fn whole_document_selection_and_replace_history_survive_projection_changes() {
    crate::test_fixture::with_locale(|| {
        let original = "café dog e\u{301} 日本語\n".repeat(600);
        for viewport in [false, true] {
            let mut app = TextApp::for_test();
            if viewport {
                app.model
                    .active_tab_mut()
                    .install_viewport_buffer(original.clone());
            } else {
                app.model.active_tab_mut().install_buffer(original.clone());
            }
            if viewport {
                assert!(
                    app.model.active_tab().content.line_count()
                        < app.model.active_tab().document().line_count()
                );
            }
            let mut driver = crate::find_focus_flow_tests::Driver::new();
            driver.layout(&app);
            if !viewport {
                let nodes = driver.nodes(&app);
                let field = nodes
                    .iter()
                    .find(|node| node.id == Some(editor_id()))
                    .expect("bounded document semantics");
                assert!(!field.semantics.editable);
            }
            driver.click_at(&mut app, iced::Point::new(700.0, 450.0));
            driver.edit_menu_action(&mut app, "select-all");
            edit(&mut app, Action::Scroll { lines: 120 });
            let copied = if viewport {
                app.model.active_tab().virtual_selected_text()
            } else {
                app.model.active_tab().content.selection()
            };
            assert_eq!(copied.as_deref(), Some(original.as_str()));
            driver.edit_menu_action(&mut app, "cut");
            assert_eq!(app.model.active_tab().snapshot(), "");
            edit(&mut app, Action::Edit(Edit::Undo));
            assert_eq!(app.model.active_tab().snapshot(), original);
            let _ = app.update(Message::OpenReplace);
            let _ = app.update(Message::FindQueryChanged("dog".into()));
            let _ = app.update(Message::ReplaceValueChanged("世界".into()));
            let _ = app.update(Message::ReplaceAll);
            let replaced = original.replace("dog", "世界");
            assert_eq!(app.model.active_tab().snapshot(), replaced);
            edit(&mut app, Action::Edit(Edit::Undo));
            assert_eq!(app.model.active_tab().snapshot(), original);
            edit(&mut app, Action::Edit(Edit::Redo));
            assert_eq!(app.model.active_tab().snapshot(), replaced);
        }
    });
}

#[test]
fn bounded_edits_refuse_overlong_joins_without_changing_bytes_or_history() {
    crate::test_fixture::with_locale(|| {
        use enzui_core::editable_text_viewport::EditableTextViewportPolicy;
        let paragraph = "é".repeat(1536);
        let original = format!("{paragraph}\n{paragraph}\n");
        for cut in [false, true] {
            let mut app = TextApp::for_test();
            app.model
                .active_tab_mut()
                .install_viewport_buffer_with_policy(
                    original.clone(),
                    EditableTextViewportPolicy {
                        max_line_bytes: 4096,
                        ..Default::default()
                    },
                );
            app.model.active_tab_mut().reveal_document_line(1);
            if cut {
                edit(&mut app, Action::Select(Motion::Left));
                let _ = app.update(Message::VirtualCut);
            } else {
                edit(&mut app, Action::Edit(Edit::Backspace));
            }
            assert_eq!(app.model.active_tab().snapshot(), original);
            assert_eq!(app.model.active_tab().content.text(), original);
            assert_eq!(app.model.active_tab().virtual_history_depth(), Some((0, 0)));
            assert!(!app.model.active_tab().dirty);
            assert!(app.picker.failure.is_some());
            edit(&mut app, Action::Move(Motion::Right));
            edit(&mut app, Action::Edit(Edit::Insert('Z')));
            let changed = app.model.active_tab().snapshot();
            assert_eq!(changed.len(), original.len() + 1);
            assert_eq!(changed.replace('Z', ""), original);
            assert!(app.model.active_tab().dirty);
        }
    });
}

#[test]
fn incremental_unicode_edits_and_search_do_not_flatten_the_document() {
    crate::test_fixture::with_locale(|| {
        let mut app = TextApp::for_test();
        app.model
            .active_tab_mut()
            .install_buffer("café dog\n".repeat(600));
        let flattens = crate::perf_counters::snapshot_flattens();
        for ch in "é日本語".chars() {
            edit(&mut app, Action::Edit(Edit::Insert(ch)));
        }
        edit(&mut app, Action::Edit(Edit::Backspace));
        assert_eq!(crate::perf_counters::snapshot_flattens(), flattens);
        let tab = app.model.active_tab();
        assert_eq!(tab.content.text(), tab.document().to_string());
        assert!(tab.snapshot().starts_with("é日本café"));
        app.model.search.query = "dog".into();
        let materializations = crate::perf_counters::snapshot_materializations();
        app.model.recompute_search();
        assert_eq!(app.model.search.total, 600);
        assert_eq!(
            crate::perf_counters::snapshot_materializations(),
            materializations
        );
    });
}
