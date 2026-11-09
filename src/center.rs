//! Text's enzui-center functions, behind the `center` feature.
//!
//! Document-level only: list, read (bounded), open a new document, switch,
//! and insert text at the caret of the active document. Reads are sensitive
//! (names and contents are the user's); insertion accepts one-shot consent
//! only. Every mutation is an ordinary [`Message`] through [`TextApp::update`],
//! so read-only, large-document and line-ending rules apply exactly as they
//! do to typing.

use std::sync::Arc;

use enzui_agent::center::{
    AppFunction, AppFunctionRegistry, AppMetadata, CenterFailure, CenterHost, CenterOutcome,
    CenterProvision, CenterTurn, ConnectorEvent, ConsentScope, FunctionRisk, FunctionSpec,
    object_schema,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

pub(crate) const CENTER_APP_ID: &str = "io.github.enzui.Text";
pub(crate) const READ_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const INSERT_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CenterCall {
    ListDocuments,
    ReadDocument { document_id: Option<String> },
    NewDocument,
    SwitchDocument { document_id: String },
    InsertText { text: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    #[serde(default)]
    document_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentArgs {
    document_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InsertArgs {
    text: String,
}

fn document_id_result() -> Value {
    object_schema(json!({"document_id": {"type": "string"}}), &["document_id"])
}

/// The agent catalogue, always in English: an installed app's runtime
/// catalogue must match its packaged (English) manifest byte for byte, and its
/// descriptor hashes (which saved permissions are bound to) must not change
/// with the session's language.
pub(crate) fn registry() -> AppFunctionRegistry<CenterCall> {
    crate::i18n::for_locales(vec![enzui_core::localization::Locale::default()])
        .scope(localized_registry)
}

fn localized_registry() -> AppFunctionRegistry<CenterCall> {
    let once_or_session = vec![ConsentScope::Once, ConsentScope::Session];
    let list = FunctionSpec::new(
        "list_documents",
        enzui_core::localization::text("text-list-documents-ba70e8"),
        enzui_core::localization::text("text-lists-the-open-documents-id-title-unsaved-81b34f"),
        object_schema(json!({}), &[]),
        object_schema(
            json!({"documents": {"type": "array", "items": object_schema(json!({
                "document_id": {"type": "string"},
                "title": {"type": "string"},
                "dirty": {"type": "boolean"},
                "active": {"type": "boolean"}
            }), &["document_id", "title", "dirty", "active"])}}),
            &["documents"],
        ),
        FunctionRisk::SensitiveRead,
        once_or_session.clone(),
    );
    let read = FunctionSpec::new(
        "read_document",
        enzui_core::localization::text("text-read-a-document-ff9c31"),
        enzui_core::localization::text("text-returns-the-start-of-a-document-s-183a89"),
        object_schema(
            json!({"document_id": {"type": "string", "maxLength": 128}}),
            &[],
        ),
        object_schema(
            json!({
                "document_id": {"type": "string"},
                "title": {"type": "string"},
                "text": {"type": "string"},
                "truncated": {"type": "boolean"}
            }),
            &["document_id", "title", "text", "truncated"],
        ),
        FunctionRisk::SensitiveRead,
        once_or_session.clone(),
    );
    let new_document = FunctionSpec::new(
        "new_document",
        enzui_core::localization::text("text-new-document-1b2684"),
        enzui_core::localization::text("text-opens-a-new-empty-document-and-makes-52542e"),
        object_schema(json!({}), &[]),
        document_id_result(),
        FunctionRisk::Mutating,
        once_or_session.clone(),
    );
    let switch = FunctionSpec::new(
        "switch_document",
        enzui_core::localization::text("text-switch-document-537b4c"),
        enzui_core::localization::text("text-makes-the-document-with-this-id-the-0f2948"),
        object_schema(
            json!({"document_id": {"type": "string", "maxLength": 128}}),
            &["document_id"],
        ),
        document_id_result(),
        FunctionRisk::Mutating,
        once_or_session,
    );
    let insert = FunctionSpec::new(
        "insert_text",
        enzui_core::localization::text("text-insert-text-62a09d"),
        enzui_core::localization::text("text-inserts-text-at-the-caret-of-the-4827d2"),
        object_schema(
            json!({"text": {"type": "string", "maxLength": INSERT_TEXT_BYTES}}),
            &["text"],
        ),
        object_schema(
            json!({
                "document_id": {"type": "string"},
                "inserted_bytes": {"type": "integer"}
            }),
            &["document_id", "inserted_bytes"],
        ),
        FunctionRisk::Mutating,
        vec![ConsentScope::Once],
    );
    AppFunctionRegistry::from_functions(vec![
        AppFunction::new(list, |_| CenterCall::ListDocuments).expect("list_documents spec"),
        AppFunction::typed(read, |args: ReadArgs| CenterCall::ReadDocument {
            document_id: args.document_id,
        })
        .expect("read_document spec"),
        AppFunction::new(new_document, |_| CenterCall::NewDocument).expect("new_document spec"),
        AppFunction::typed(switch, |args: DocumentArgs| CenterCall::SwitchDocument {
            document_id: args.document_id,
        })
        .expect("switch_document spec"),
        AppFunction::typed(insert, |args: InsertArgs| CenterCall::InsertText {
            text: args.text,
        })
        .expect("insert_text spec"),
    ])
    .expect("the text registry is valid")
}

pub(crate) fn metadata() -> AppMetadata {
    AppMetadata {
        id: CENTER_APP_ID.into(),
        name: enzui_core::localization::text("text-text-71988c"),
        description: enzui_core::localization::text("text-plain-text-and-code-editor-68d8bc"),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

fn text_prefix(tab: &model::Tab, limit: usize) -> (String, bool) {
    let document = tab.document();
    let end = limit.min(document.len());
    let mut text = String::with_capacity(end);
    document.for_each_chunk_in(0..end, |chunk| text.push_str(chunk));
    while !text.is_char_boundary(text.len()) {
        text.pop();
    }
    (text, end < document.len())
}

impl TextApp {
    pub(crate) fn start_center(&mut self) {
        let Some(provision) = CenterProvision::discover(CENTER_APP_ID) else {
            return;
        };
        match CenterHost::spawn(provision, metadata(), registry()) {
            Ok(host) => self.center = Some(host),
            Err(error) => eprintln!("text-app: center connector disabled: {error}"),
        }
    }

    pub(crate) fn center_subscription(&self) -> Subscription<Message> {
        self.center
            .as_ref()
            .map_or_else(Subscription::none, |host| {
                host.subscription(Message::Center)
            })
    }

    pub(crate) fn shutdown_center(&mut self) {
        if let Some(host) = self.center.take() {
            host.shutdown();
        }
    }

    pub(crate) fn center_event(&mut self, event: ConnectorEvent) -> Task<Message> {
        let Some(host) = self.center.as_mut() else {
            return Task::none();
        };
        let CenterTurn::Call(call, request) = host.receive_with_ends(event).turn else {
            return Task::none();
        };
        if !host.is_live(&call) {
            return Task::none();
        }
        let (task, outcome) = self.apply_center_call(request);
        if let Some(host) = self.center.as_mut() {
            host.answer(call, outcome);
        }
        task
    }

    fn center_document(&self, id: Option<&str>) -> Result<&model::Tab, CenterFailure> {
        let id = id.unwrap_or(&self.model.active_tab_id);
        self.model
            .tabs
            .iter()
            .find(|tab| tab.id == id)
            .ok_or_else(|| {
                CenterFailure::new(
                    "not_found",
                    enzui_core::localization::text("text-64482cc53371"),
                )
            })
    }

    fn apply_center_call(&mut self, request: CenterCall) -> (Task<Message>, CenterOutcome) {
        match request {
            CenterCall::ListDocuments => {
                let documents: Vec<Value> = self
                    .model
                    .tabs
                    .iter()
                    .map(|tab| {
                        json!({
                            "document_id": tab.id,
                            "title": tab.title(),
                            "dirty": tab.dirty,
                            "active": tab.id == self.model.active_tab_id,
                        })
                    })
                    .collect();
                (Task::none(), Ok(json!({ "documents": documents })))
            }
            CenterCall::ReadDocument { document_id } => {
                let outcome = self.center_document(document_id.as_deref()).map(|tab| {
                    let (text, truncated) = text_prefix(tab, READ_TEXT_BYTES);
                    json!({
                        "document_id": tab.id,
                        "title": tab.title(),
                        "text": text,
                        "truncated": truncated,
                    })
                });
                (Task::none(), outcome)
            }
            CenterCall::NewDocument => {
                let before: Vec<String> = self.model.tabs.iter().map(|t| t.id.clone()).collect();
                let task = self.update(Message::NewTab);
                let opened = self
                    .model
                    .tabs
                    .iter()
                    .find(|tab| !before.contains(&tab.id))
                    .map(|tab| tab.id.clone());
                let outcome = opened
                    .map(|document_id| json!({ "document_id": document_id }))
                    .ok_or_else(|| {
                        CenterFailure::new(
                            "unavailable",
                            enzui_core::localization::text("text-6b99dd587216"),
                        )
                    });
                (task, outcome)
            }
            CenterCall::SwitchDocument { document_id } => {
                if let Err(failure) = self.center_document(Some(&document_id)) {
                    return (Task::none(), Err(failure));
                }
                let task = self.update(Message::SwitchTab(document_id.clone()));
                let outcome = if self.model.active_tab_id == document_id {
                    Ok(json!({ "document_id": document_id }))
                } else {
                    Err(CenterFailure::new(
                        "unavailable",
                        enzui_core::localization::text("text-7d1d4661bf01"),
                    ))
                };
                (task, outcome)
            }
            CenterCall::InsertText { text } => {
                if text.len() > INSERT_TEXT_BYTES {
                    return (
                        Task::none(),
                        Err(CenterFailure::new(
                            "invalid_arguments",
                            enzui_core::localization::text("text-6dd298fb20a8"),
                        )),
                    );
                }
                let tab = self.model.active_tab();
                if !tab.supports(large_doc::Capability::Edit) {
                    return (
                        Task::none(),
                        Err(CenterFailure::new(
                            "read_only",
                            enzui_core::localization::text("text-1408fd0f3303"),
                        )),
                    );
                }
                let document_id = tab.id.clone();
                if text.is_empty() {
                    return (
                        Task::none(),
                        Ok(json!({ "document_id": document_id, "inserted_bytes": 0 })),
                    );
                }
                let before = tab.document().len();
                let task = self.update(Message::EditorAction(text_editor::Action::Edit(
                    text_editor::Edit::Paste(Arc::new(text)),
                )));
                let tab = self.model.active_tab();
                let outcome = if tab.id == document_id && tab.document().len() > before {
                    Ok(json!({
                        "document_id": document_id,
                        "inserted_bytes": tab.document().len() - before,
                    }))
                } else {
                    Err(CenterFailure::new(
                        "unavailable",
                        enzui_core::localization::text("text-49a3e4d3bd79"),
                    ))
                };
                (task, outcome)
            }
        }
    }
}
