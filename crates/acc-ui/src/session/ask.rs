//! What a waiting request asks, and answering it.
//!
//! A question's `native` payload carries its questions (Claude's
//! `AskUserQuestion` and MCP elicitation forms, Codex's `requestUserInput`,
//! opencode's question tool), in the shape `acc_core::question::parse`
//! reads. A plan approval is Claude's `ExitPlanMode` permission, whose input
//! holds the plan.
//!
//! [`AnswerDraft`] holds the user's picks and text while they answer, and
//! encodes them the way every ACC driver decodes them
//! (`acc_core::question::decode`): send its [`option_id`](AnswerDraft::option_id)
//! as `Command::AnswerPermission`'s `option_id`, then call
//! [`SessionView::answered`](super::SessionView::answered).

use serde::Serialize;
use serde_json::Value;

use super::event::AskKind;
use super::model::Ask;

/// One question in a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Question {
    /// The harness's id for it (Codex's question id, a form field's name).
    /// Claude keys `AskUserQuestion` answers by the question text instead.
    pub id: Option<String>,
    pub text: String,
    /// A short label for it, when the harness gives one.
    pub header: Option<String>,
    /// Suggested answers; empty when only text answers it.
    pub options: Vec<QuestionOption>,
    /// More than one option may be picked.
    pub multi_select: bool,
    /// The user may type their own answer, besides or instead of the options.
    pub allows_text: bool,
    /// The answer is a secret (a password or token): don't show or keep it.
    pub secret: bool,
    /// The request can't be answered while this question has no answer.
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

/// The questions in a request's `native` payload, in order.
pub(super) fn questions(native: &Value) -> Vec<Question> {
    let Some(qs) = native.get("questions").and_then(Value::as_array) else {
        return Vec::new();
    };
    // Forms (MCP elicitation) take only their choices where they have any;
    // Claude's `AskUserQuestion` always takes text too.
    let form = native.get("requested_schema").is_some() || native.get("requestedSchema").is_some();
    qs.iter()
        .filter(|q| q.is_object())
        .map(|q| {
            let options: Vec<QuestionOption> = q
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|o| {
                    Some(QuestionOption {
                        label: text(o, "label")?,
                        description: text(o, "description"),
                    })
                })
                .collect();
            let allows_text = match q.get("isOther").and_then(Value::as_bool) {
                Some(other) => other || options.is_empty(),
                None => !form || options.is_empty(),
            };
            Question {
                id: text(q, "id"),
                text: text(q, "question").unwrap_or_default(),
                header: text(q, "header"),
                multi_select: q.get("multiSelect").and_then(Value::as_bool) == Some(true),
                allows_text,
                secret: q.get("isSecret").and_then(Value::as_bool) == Some(true),
                required: q.get("required").and_then(Value::as_bool) != Some(false),
                options,
            }
        })
        .collect()
}

/// The plan an approval is about: Claude's `ExitPlanMode` input.
pub(super) fn plan(kind: AskKind, native: &Value) -> Option<String> {
    if kind != AskKind::Permission {
        return None;
    }
    text(native, "plan")
}

/// A non-empty string field.
fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

/// Answers to every question, one list per question, as an `option_id`
/// (`acc_core::question::encode`).
pub fn encode_answers(answers: &[Vec<String>]) -> String {
    serde_json::to_string(answers).unwrap_or_default()
}

/// The user's answers to a request's questions while they make them.
///
/// Picking an option of a single-choice question replaces the earlier pick
/// and any typed text; typing text replaces the pick. On a multi-choice
/// question picks toggle and text is one more answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnswerDraft {
    request_id: String,
    questions: Vec<Question>,
    /// Picked option indexes per question, in option order.
    picked: Vec<Vec<usize>>,
    text: Vec<String>,
}

impl AnswerDraft {
    /// An empty draft for `ask`'s questions.
    pub fn new(ask: &Ask) -> Self {
        let n = ask.questions.len();
        Self {
            request_id: ask.request_id.clone(),
            questions: ask.questions.clone(),
            picked: vec![Vec::new(); n],
            text: vec![String::new(); n],
        }
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    /// Picks (or, on a multi-choice question, toggles) option `option` of
    /// question `question`. False when there is no such option.
    pub fn pick(&mut self, question: usize, option: usize) -> bool {
        let Some(q) = self.questions.get(question) else {
            return false;
        };
        if option >= q.options.len() {
            return false;
        }
        let picked = &mut self.picked[question];
        if q.multi_select {
            match picked.binary_search(&option) {
                Ok(i) => {
                    picked.remove(i);
                }
                Err(i) => picked.insert(i, option),
            }
        } else {
            *picked = vec![option];
            self.text[question].clear();
        }
        true
    }

    /// Picks the option labelled `label`, as [`pick`](Self::pick) does.
    pub fn pick_label(&mut self, question: usize, label: &str) -> bool {
        let option = self
            .questions
            .get(question)
            .and_then(|q| q.options.iter().position(|o| o.label == label));
        option.is_some_and(|o| self.pick(question, o))
    }

    /// Sets question `question`'s typed answer. False when it takes no text.
    pub fn set_text(&mut self, question: usize, text: &str) -> bool {
        let Some(q) = self.questions.get(question) else {
            return false;
        };
        if !q.allows_text {
            return false;
        }
        if !q.multi_select && !text.trim().is_empty() {
            self.picked[question].clear();
        }
        text.clone_into(&mut self.text[question]);
        true
    }

    /// The picked option indexes of question `question`, in option order.
    pub fn picked(&self, question: usize) -> &[usize] {
        self.picked.get(question).map_or(&[], Vec::as_slice)
    }

    pub fn text(&self, question: usize) -> &str {
        self.text.get(question).map_or("", String::as_str)
    }

    /// Question `question`'s answers as they will be sent: the picked
    /// labels, then the typed text.
    pub fn answer(&self, question: usize) -> Vec<String> {
        let Some(q) = self.questions.get(question) else {
            return Vec::new();
        };
        let mut out: Vec<String> = self.picked[question]
            .iter()
            .map(|&i| q.options[i].label.clone())
            .collect();
        let text = self.text[question].trim();
        if !text.is_empty() {
            out.push(text.to_owned());
        }
        out
    }

    pub fn is_answered(&self, question: usize) -> bool {
        !self.answer(question).is_empty()
    }

    /// The required questions still without an answer.
    pub fn missing(&self) -> Vec<usize> {
        (0..self.questions.len())
            .filter(|&i| self.questions[i].required && !self.is_answered(i))
            .collect()
    }

    /// Every required question has an answer, and there is a question.
    pub fn is_complete(&self) -> bool {
        !self.questions.is_empty() && self.missing().is_empty()
    }

    /// The `option_id` that answers the request, once
    /// [complete](Self::is_complete).
    pub fn option_id(&self) -> Option<String> {
        self.is_complete().then(|| {
            let answers: Vec<Vec<String>> = (0..self.questions.len()).map(|i| self.answer(i)).collect();
            encode_answers(&answers)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ask(native: Value) -> Ask {
        Ask {
            request_id: "r1".into(),
            kind: AskKind::Question,
            title: String::new(),
            options: Vec::new(),
            item_id: None,
            subject: None,
            questions: questions(&native),
            plan: None,
            turn: 0,
        }
    }

    #[test]
    fn reads_claude_questions() {
        let qs = questions(&json!({"questions": [
            {"question": "Which DB?", "header": "DB", "multiSelect": false,
             "options": [{"label": "Postgres", "description": "big"}, {"label": "SQLite", "description": ""}]},
            {"question": "Features?", "multiSelect": true, "options": [{"label": "A"}, {"label": "B"}]},
        ]}));
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].header.as_deref(), Some("DB"));
        assert_eq!(qs[0].options[0].description.as_deref(), Some("big"));
        assert_eq!(qs[0].options[1].description, None);
        assert!(qs[0].allows_text && qs[0].required && !qs[0].secret);
        assert!(qs[1].multi_select);
    }

    #[test]
    fn reads_codex_and_form_flags() {
        let codex = questions(&json!({"questions": [
            {"id": "q1", "question": "Which DB?", "isOther": false, "isSecret": false,
             "options": [{"label": "Postgres", "description": ""}]},
            {"id": "q2", "question": "Token?", "isOther": false, "isSecret": true, "options": null},
        ]}));
        assert!(!codex[0].allows_text);
        assert_eq!(codex[0].id.as_deref(), Some("q1"));
        assert!(codex[1].allows_text && codex[1].secret, "no options means text");

        let form = questions(&json!({"requested_schema": {}, "questions": [
            {"id": "env", "question": "Environment", "options": [{"label": "prod"}], "required": true},
            {"id": "note", "question": "Note", "options": [], "required": false},
        ]}));
        assert!(
            !form[0].allows_text,
            "a form's choice field takes only its choices"
        );
        assert!(form[1].allows_text && !form[1].required);
    }

    #[test]
    fn plan_only_on_permissions() {
        let native = json!({"plan": "1. Do it"});
        assert_eq!(plan(AskKind::Permission, &native).as_deref(), Some("1. Do it"));
        assert_eq!(plan(AskKind::Question, &native), None);
        assert_eq!(plan(AskKind::Permission, &json!({"plan": " "})), None);
        assert!(questions(&json!({"command": "ls"})).is_empty());
    }

    #[test]
    fn draft_single_multi_and_text() {
        let a = ask(json!({"questions": [
            {"question": "Which DB?", "options": [{"label": "Postgres"}, {"label": "SQLite"}]},
            {"question": "Features?", "multiSelect": true, "options": [{"label": "A"}, {"label": "B"}]},
        ]}));
        let mut d = AnswerDraft::new(&a);
        assert!(!d.is_complete());
        assert_eq!(d.option_id(), None);
        assert_eq!(d.missing(), [0, 1]);

        assert!(d.pick(0, 0));
        assert!(d.pick(0, 1), "a single choice replaces the pick");
        assert_eq!(d.picked(0), [1]);
        assert!(d.set_text(0, "MySQL"));
        assert!(d.picked(0).is_empty(), "text replaces a single pick");
        assert!(d.pick_label(0, "SQLite"));
        assert_eq!(d.text(0), "", "a pick replaces text");

        assert!(d.pick(1, 1));
        assert!(d.pick(1, 0));
        assert_eq!(d.picked(1), [0, 1], "in option order");
        assert!(d.set_text(1, " C "));
        assert_eq!(d.answer(1), ["A", "B", "C"]);
        assert!(d.pick(1, 1), "toggles off");

        assert!(!d.pick(1, 5));
        assert!(!d.pick(7, 0));
        assert!(d.is_complete());
        assert_eq!(d.option_id().as_deref(), Some(r#"[["SQLite"],["A","C"]]"#));
    }

    #[test]
    fn draft_respects_text_and_required() {
        let a = ask(json!({"questions": [
            {"id": "q1", "question": "Which DB?", "isOther": false, "options": [{"label": "Postgres"}]},
            {"id": "q2", "question": "Note", "required": false, "options": []},
        ]}));
        let mut d = AnswerDraft::new(&a);
        assert!(!d.set_text(0, "MySQL"));
        assert!(d.pick(0, 0));
        assert!(d.is_complete(), "the optional question can stay empty");
        assert_eq!(d.option_id().as_deref(), Some(r#"[["Postgres"],[]]"#));
        assert!(!AnswerDraft::new(&ask(json!({}))).is_complete());
    }
}
