//! A [`SessionView`] from what ACC saved, for history and for picking a
//! session up without its events.

use serde::Deserialize;
use serde_json::Value;

use super::event::{AskKind, AskOption, Detail, Event, ItemKind, ItemStatus, Quota, Usage};
use super::{Changes, SessionView, TurnOutcome};

/// One item of ACC's `Transcript` (`acc_core::transcript::Item`).
#[derive(Debug, Deserialize)]
struct StoredItem {
    id: String,
    #[serde(default)]
    kind: ItemKind,
    #[serde(default)]
    status: ItemStatus,
    #[serde(default)]
    text: String,
    #[serde(default)]
    detail: Detail,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    started_at_ms: Option<u64>,
    #[serde(default)]
    completed_at_ms: Option<u64>,
}

/// A pending request in a session snapshot (`acc_core::session::Permission`).
#[derive(Debug, Deserialize)]
struct StoredAsk {
    request_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    options: Vec<AskOption>,
    #[serde(default)]
    kind: AskKind,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    detail: Option<Detail>,
}

impl SessionView {
    /// Builds a view from what ACC saved: a session snapshot
    /// (`Session::snapshot`), a `Transcript`, or a bare array of its items.
    ///
    /// Transcripts keep items, not turns, so turns are rebuilt from the
    /// prompts: each top-level user message starts one. The last turn stays
    /// open while any item still runs. A snapshot also gives the harness,
    /// working directory, title, model, usage, quota and pending requests.
    pub fn from_transcript(json: &str) -> Result<Self, serde_json::Error> {
        let v: Value = serde_json::from_str(json)?;
        let items = v
            .get("transcript")
            .and_then(|t| t.get("items"))
            .or_else(|| v.get("items"))
            .unwrap_or(&v);
        let mut items: Vec<StoredItem> = serde_json::from_value(items.clone())?;
        items.sort_by_key(|i| i.seq);

        let mut view = SessionView {
            restoring: true,
            ..SessionView::default()
        };
        let spec = v.get("spec");
        let text = |key: &str| {
            spec.and_then(|s| s.get(key))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        if let Some(cwd) = text("cwd") {
            view.set_root(cwd);
        }
        view.header.harness = text("harness");
        view.header.model = text("model");
        view.header.native_id = text("native_id");
        if let Some(title) = text("title") {
            view.apply(Event::TitleChanged { title });
        }

        for item in items {
            let running = item.status == ItemStatus::Running;
            view.apply(Event::ItemStarted {
                id: item.id.clone(),
                kind: item.kind,
                parent: item.parent,
                detail: item.detail.clone(),
                started_at_ms: item.started_at_ms,
            });
            if !item.text.is_empty() {
                view.apply(Event::ItemDelta {
                    id: item.id.clone(),
                    text: item.text,
                });
            }
            if !running {
                view.apply(Event::ItemCompleted {
                    id: item.id,
                    status: item.status,
                    detail: item.detail,
                    completed_at_ms: item.completed_at_ms,
                });
            }
        }
        if !view.items.iter().any(|i| i.status == ItemStatus::Running) {
            view.close_turn(TurnOutcome::Done, &mut Changes::default());
        }

        if let Some(u) = v.get("usage").and_then(|u| Usage::deserialize(u).ok()) {
            view.apply(Event::Usage(u));
        }
        if let Some(q) = v.get("quota").and_then(|q| Quota::deserialize(q).ok()) {
            view.apply(Event::Quota(q));
        }
        if let Some(asks) = v.get("permissions").and_then(Value::as_array) {
            for a in asks {
                if let Ok(a) = StoredAsk::deserialize(a) {
                    view.apply(Event::PermissionRequested {
                        request_id: a.request_id,
                        title: a.title,
                        options: a.options,
                        kind: a.kind,
                        item_id: a.item_id,
                        detail: a.detail,
                    });
                }
            }
        }
        if v.get("exited").and_then(Value::as_bool) == Some(true) {
            view.apply(Event::Exited { code: None });
        }
        view.restoring = false;
        Ok(view)
    }
}
