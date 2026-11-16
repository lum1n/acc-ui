//! The session model: ACC events in, regions out.
//!
//! A [`SessionView`] folds a session's events into what a session screen
//! shows, instead of a top-to-bottom transcript:
//!
//! | Region | From |
//! |---|---|
//! | [`Header`] | session start, state, title, usage, quota |
//! | Now ([`Activity`]) | the items running right now |
//! | Plan ([`PlanView`]) | the agent's to-do list |
//! | Needs you ([`Ask`]) | approvals and questions waiting on the user |
//! | Files ([`FileEntry`]) | files read and changed, with their edits |
//! | Commands ([`CommandEntry`]) | shell commands: kind, outcome, test counts, output tails |
//! | Subagents ([`SubagentEntry`]) | other agents' runs and their reports |
//! | Turns ([`Turn`]) | each prompt and the items it led to |
//!
//! Feed it every event, in order, with [`SessionView::apply_json`]. Each call
//! returns the [`Changes`] it made, so a renderer redraws only those.

mod ask;
mod command;
pub mod event;
mod model;
mod output;
mod restore;
mod shell;
mod translate;

use std::collections::{HashMap, HashSet};

use serde::Serialize;

pub use ask::{AnswerDraft, Question, QuestionOption, encode_answers};
pub use command::{CommandKind, Diagnostics, TestCounts};
use event::{Detail, FileChangeKind, ItemKind, ItemStatus, RunState, ToolKind};
pub use event::{Event, ImageRef};
pub use model::{
    Activity, ActivityKind, Ask, CommandEntry, CommandOutcome, Digest, DigestPart, FileEdit, FileEntry,
    HEADLINE_CHARS, Header, ItemView, OUTPUT_TAIL_LINES, PlanView, SubagentEntry, Turn, TurnOutcome,
};
pub use shell::unwrap_shell;

/// Longest title taken from a prompt.
const TITLE_CHARS: usize = 80;
/// What ACC shows for a prompt that is only images.
const IMAGE_PLACEHOLDER: &str = "[image]";

/// Notices kept; older ones are dropped.
const MAX_NOTICES: usize = 50;

/// What one call changed. Ids are item ids; files are keyed by path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Changes {
    pub header: bool,
    pub now: bool,
    pub plan: bool,
    pub asks: bool,
    pub notices: bool,
    pub files: Vec<String>,
    pub commands: Vec<String>,
    pub subagents: Vec<String>,
    pub turns: Vec<usize>,
    pub items: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn push<T: PartialEq>(list: &mut Vec<T>, v: T) {
        if !list.contains(&v) {
            list.push(v);
        }
    }
}

/// Everything a [`SessionView`] holds, for golden tests and for apps that
/// take the whole state at once.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    pub header: Header,
    pub now: Vec<Activity>,
    pub plan: Option<PlanView>,
    pub asks: Vec<Ask>,
    pub files: Vec<FileEntry>,
    pub commands: Vec<CommandEntry>,
    pub subagents: Vec<SubagentEntry>,
    pub turns: Vec<Turn>,
    pub notices: Vec<String>,
    pub items: Vec<ItemView>,
}

/// A session's live state, built from its ACC events.
#[derive(Debug, Clone, Default)]
pub struct SessionView {
    header: Header,
    /// The client set the root; commands don't override it.
    root_fixed: bool,
    /// The harness named the session; prompts don't override it.
    titled: bool,
    plan: Option<PlanView>,
    asks: Vec<Ask>,
    files: Vec<FileEntry>,
    file_index: HashMap<String, usize>,
    commands: Vec<CommandEntry>,
    command_index: HashMap<String, usize>,
    subagents: Vec<SubagentEntry>,
    subagent_index: HashMap<String, usize>,
    turns: Vec<Turn>,
    notices: Vec<String>,
    items: Vec<ItemView>,
    item_index: HashMap<String, usize>,
    /// Every item of each turn, subagents' included, as indices into
    /// `items`, for the digests.
    turn_items: Vec<Vec<usize>>,
    /// Reads already counted, as (item id, path), so an item completed
    /// twice reads its files once.
    reads_seen: HashSet<(String, String)>,
    /// Commands recognised with neither a `cwd` nor a root, whose relative
    /// reads [`SessionView::set_root`] resolves later.
    unresolved: HashSet<String>,
    /// The latest time any event reported, ms since the Unix epoch.
    last_seen_ms: Option<u64>,
    /// Rebuilding from a transcript: a new prompt ends the turn before it
    /// normally, rather than cutting it short.
    restoring: bool,
}

impl SessionView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the session's working directory, which the client knows from
    /// how it started the session. Paths under it are shown relative.
    /// Commands seen before without a `cwd` have their relative reads
    /// resolved against it now.
    pub fn set_root(&mut self, root: impl Into<String>) {
        self.header.root = Some(root.into());
        self.root_fixed = true;
        for f in &mut self.files {
            f.display = display_path(self.header.root.as_deref(), &f.path);
        }
        let mut ch = Changes::default();
        let mut pending: Vec<usize> = self
            .unresolved
            .drain()
            .filter_map(|id| self.item_index.get(&id).copied())
            .collect();
        pending.sort_unstable();
        let mut turns = Vec::new();
        for i in pending {
            let item = self.items[i].clone();
            self.upsert_command(&item, &mut ch);
            self.record_command_reads(&item, &mut ch);
            Changes::push(&mut turns, item.turn);
        }
        for t in turns {
            self.refresh_digest(t, &mut ch);
        }
    }

    /// Applies one event as ACC serialises it (one line of `acc run
    /// --json`, or an event's JSON from the bindings). Unknown event types
    /// change nothing; JSON that isn't an event is an error.
    pub fn apply_json(&mut self, json: &str) -> Result<Changes, serde_json::Error> {
        Ok(self.apply(serde_json::from_str(json)?))
    }

    /// Applies one event.
    pub fn apply(&mut self, event: Event) -> Changes {
        let mut ch = Changes::default();
        match event {
            Event::SessionStarted {
                harness,
                native_id,
                model,
            } => {
                self.header.harness = non_empty(harness);
                self.header.native_id = non_empty(native_id);
                if model.is_some() {
                    self.header.model = model;
                }
                ch.header = true;
            }
            Event::StateChanged {
                state,
                message,
                observed_at_ms,
            } => {
                self.header.state = state;
                self.header.state_message = message;
                ch.header = true;
                self.seen(observed_at_ms, &mut ch);
            }
            Event::TitleChanged { title } => {
                if !title.trim().is_empty() {
                    self.header.title = Some(title);
                    self.titled = true;
                    ch.header = true;
                }
            }
            Event::Usage(u) => {
                self.header.usage = Some(u);
                ch.header = true;
            }
            Event::Quota(q) => {
                self.header.quota = Some(q);
                ch.header = true;
            }
            Event::Notice { message } => {
                self.notices.push(message);
                if self.notices.len() > MAX_NOTICES {
                    self.notices.remove(0);
                }
                ch.notices = true;
            }
            Event::Plan(p) => {
                self.plan = Some(PlanView::new(p.entries, p.explanation));
                ch.plan = true;
            }
            Event::ItemStarted {
                id,
                kind,
                parent,
                detail,
                started_at_ms,
                images,
            } => {
                let first = !self.item_index.contains_key(&id);
                self.item_started(id.clone(), kind, parent, detail, started_at_ms, &mut ch);
                if first && !images.is_empty() {
                    self.item_images(&id, images, &mut ch);
                }
            }
            Event::ItemDelta { id, text } => self.item_delta(&id, &text, &mut ch),
            Event::ItemUpdated { id, detail } => {
                self.item_detail(&id, detail, None, None, &mut ch);
            }
            Event::ItemCompleted {
                id,
                status,
                detail,
                completed_at_ms,
            } => {
                self.item_detail(&id, detail, Some(status), completed_at_ms, &mut ch);
                self.seen(completed_at_ms, &mut ch);
                self.clear_asks(&mut ch, |a| a.item_id.as_deref() == Some(id.as_str()));
            }
            Event::PermissionRequested {
                request_id,
                title,
                options,
                kind,
                item_id,
                detail,
                native,
            } => {
                let questions = ask::questions(&native);
                let plan = ask::plan(kind, &native);
                let turn = item_id
                    .as_deref()
                    .and_then(|i| self.item(i))
                    .map_or_else(|| self.current_turn(), |i| i.turn);
                self.asks.retain(|a| a.request_id != request_id);
                self.asks.push(Ask {
                    request_id,
                    kind,
                    title,
                    options,
                    item_id,
                    subject: detail,
                    questions,
                    plan,
                    turn,
                });
                ch.asks = true;
            }
            Event::PermissionResolved { request_id } => {
                self.clear_asks(&mut ch, |a| a.request_id == request_id);
            }
            Event::TurnDiff { diff, .. } => {
                let t = self.open_turn(&mut ch);
                self.turns[t].diff = Some(diff);
                Changes::push(&mut ch.turns, t);
            }
            Event::TurnCompleted => {
                let outcome = match self.header.state {
                    RunState::Error => TurnOutcome::Failed,
                    RunState::Waiting => TurnOutcome::Waiting,
                    _ => TurnOutcome::Done,
                };
                self.close_turn(outcome, &mut ch);
                self.clear_asks(&mut ch, |_| true);
                ch.now = true;
            }
            Event::Exited { code } => {
                self.header.exited = true;
                self.header.exit_code = code;
                self.header.state = RunState::Exited;
                ch.header = true;
                self.close_turn(TurnOutcome::Interrupted, &mut ch);
                self.clear_asks(&mut ch, |_| true);
                ch.now = true;
            }
            Event::Unknown => {}
        }
        ch
    }

    /// Records that the client answered a request, since harnesses only
    /// report requests they withdraw themselves.
    pub fn answered(&mut self, request_id: &str) -> Changes {
        let mut ch = Changes::default();
        self.clear_asks(&mut ch, |a| a.request_id == request_id);
        ch
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn plan(&self) -> Option<&PlanView> {
        self.plan.as_ref()
    }

    pub fn asks(&self) -> &[Ask] {
        &self.asks
    }

    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    pub fn commands(&self) -> &[CommandEntry] {
        &self.commands
    }

    pub fn subagents(&self) -> &[SubagentEntry] {
        &self.subagents
    }

    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    pub fn items(&self) -> &[ItemView] {
        &self.items
    }

    pub fn item(&self, id: &str) -> Option<&ItemView> {
        self.item_index.get(id).map(|&i| &self.items[i])
    }

    /// A file by path, as [`Changes::files`] names it.
    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.file_index.get(path).map(|&i| &self.files[i])
    }

    /// A command by item id, as [`Changes::commands`] names it.
    pub fn command(&self, item_id: &str) -> Option<&CommandEntry> {
        self.command_index.get(item_id).map(|&i| &self.commands[i])
    }

    /// A subagent by item id, as [`Changes::subagents`] names it.
    pub fn subagent(&self, item_id: &str) -> Option<&SubagentEntry> {
        self.subagent_index.get(item_id).map(|&i| &self.subagents[i])
    }

    /// A turn by index, as [`Changes::turns`] names it.
    pub fn turn(&self, index: usize) -> Option<&Turn> {
        self.turns.get(index)
    }

    /// A waiting ask by request id.
    pub fn ask(&self, request_id: &str) -> Option<&Ask> {
        self.asks.iter().find(|a| a.request_id == request_id)
    }

    /// What the agent and its subagents are doing right now, oldest first.
    pub fn now(&self) -> Vec<Activity> {
        self.items
            .iter()
            .filter(|i| i.status == ItemStatus::Running)
            .filter_map(|i| self.activity(i))
            .collect()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            header: self.header.clone(),
            now: self.now(),
            plan: self.plan.clone(),
            asks: self.asks.clone(),
            files: self.files.clone(),
            commands: self.commands.clone(),
            subagents: self.subagents.clone(),
            turns: self.turns.clone(),
            notices: self.notices.clone(),
            items: self.items.clone(),
        }
    }

    // Items

    fn item_started(
        &mut self,
        id: String,
        kind: ItemKind,
        parent: Option<String>,
        detail: Detail,
        started_at_ms: Option<u64>,
        ch: &mut Changes,
    ) {
        if self.item_index.contains_key(&id) {
            self.item_detail(&id, detail, None, None, ch);
            return;
        }
        // A prompt in the main conversation starts a turn; anything else
        // joins the open one, or its subagent's.
        let turn = match parent.as_deref().and_then(|p| self.item(p)) {
            Some(p) => p.turn,
            None if parent.is_none() && kind == ItemKind::UserMessage => self.new_turn(ch),
            None => self.open_turn(ch),
        };
        // Reasoning joins the turn once it has text: Codex sends empty
        // reasoning items when summaries are off, and they show nothing.
        if parent.is_none() && kind != ItemKind::Reasoning {
            let t = &mut self.turns[turn];
            t.items.push(id.clone());
            if kind == ItemKind::AgentMessage {
                // The message it replaces as the narrative folds away.
                if t.narrative_item.is_some() {
                    t.steps += 1;
                }
                t.narrative_item = Some(id.clone());
                t.narrative = Some(String::new());
            } else if kind != ItemKind::UserMessage {
                t.steps += 1;
            }
            Changes::push(&mut ch.turns, turn);
        }
        self.turn_items[turn].push(self.items.len());
        if let Some(p) = parent.as_deref()
            && let Some(&s) = self.subagent_index.get(p)
        {
            self.subagents[s].items += 1;
            Changes::push(&mut ch.subagents, p.to_owned());
        }
        self.item_index.insert(id.clone(), self.items.len());
        self.items.push(ItemView {
            id: id.clone(),
            kind,
            parent,
            turn,
            status: ItemStatus::Running,
            text: String::new(),
            detail: Detail::Text,
            output: String::new(),
            images: Vec::new(),
            started_at_ms,
            completed_at_ms: None,
        });
        Changes::push(&mut ch.items, id.clone());
        ch.now = true;
        self.seen(started_at_ms, ch);
        self.item_detail(&id, detail, None, None, ch);
    }

    /// Images a message carried (ACC's refs); a prompt's go on its turn.
    fn item_images(&mut self, id: &str, images: Vec<ImageRef>, ch: &mut Changes) {
        let Some(&i) = self.item_index.get(id) else {
            return;
        };
        let item = &mut self.items[i];
        item.images = images;
        if item.kind == ItemKind::UserMessage && item.parent.is_none() {
            let t = &mut self.turns[item.turn];
            t.prompt.get_or_insert_with(String::new);
            t.prompt_images.extend(item.images.iter().cloned());
            Changes::push(&mut ch.turns, item.turn);
        }
    }

    fn item_delta(&mut self, id: &str, text: &str, ch: &mut Changes) {
        let Some(&i) = self.item_index.get(id) else {
            return;
        };
        let item = &mut self.items[i];
        match item.detail {
            Detail::Command { .. } => {
                item.output.push_str(text);
                let output = item.output.clone();
                self.update_command_output(id, &output, ch);
            }
            // ACC's stand-in for an image-only prompt, when it sent refs.
            Detail::Text if text == IMAGE_PLACEHOLDER && item.text.is_empty() && !item.images.is_empty() => {}
            Detail::Text => {
                let was_blank = item.text.trim().is_empty();
                item.text.push_str(text);
                Changes::push(&mut ch.items, id.to_owned());
                let (kind, parent, turn) = (item.kind, item.parent.is_none(), item.turn);
                if kind == ItemKind::Reasoning && parent && was_blank && !text.trim().is_empty() {
                    self.reasoning_shown(i, ch);
                }
                let t = &mut self.turns[turn];
                if t.narrative_item.as_deref() == Some(id) {
                    t.narrative.get_or_insert_with(String::new).push_str(text);
                    Changes::push(&mut ch.turns, turn);
                }
                if kind == ItemKind::UserMessage && parent {
                    let t = &mut self.turns[turn];
                    t.prompt.get_or_insert_with(String::new).push_str(text);
                    Changes::push(&mut ch.turns, turn);
                    if !self.titled && turn == 0 {
                        self.header.title =
                            first_line(t.prompt.as_deref().unwrap_or("")).map(|l| shorten(&l, TITLE_CHARS));
                        ch.header = true;
                    }
                }
            }
            _ => {}
        }
    }

    /// Reasoning item `i` got its first text: it joins its turn's items,
    /// in order, as a step.
    fn reasoning_shown(&mut self, i: usize, ch: &mut Changes) {
        let item = &self.items[i];
        let t = &mut self.turns[item.turn];
        let at = t
            .items
            .iter()
            .position(|other| self.item_index.get(other).is_some_and(|&o| o > i))
            .unwrap_or(t.items.len());
        t.items.insert(at, item.id.clone());
        t.steps += 1;
        Changes::push(&mut ch.turns, item.turn);
    }

    /// A new detail, from a start, update or completion. `status` is set
    /// on completion.
    fn item_detail(
        &mut self,
        id: &str,
        detail: Detail,
        status: Option<ItemStatus>,
        completed_at_ms: Option<u64>,
        ch: &mut Changes,
    ) {
        let Some(&i) = self.item_index.get(id) else {
            return;
        };
        // A completion without a detail keeps the one it had.
        if detail != Detail::Text || self.items[i].detail == Detail::Text {
            if let Some(images) = tool_images(&detail) {
                self.items[i].images = images;
            }
            self.items[i].detail = detail;
        }
        if let Some(s) = status {
            self.items[i].status = s;
            self.items[i].completed_at_ms = completed_at_ms;
            ch.now = true;
        } else if self.items[i].status == ItemStatus::Running {
            ch.now = true;
        }
        Changes::push(&mut ch.items, id.to_owned());
        let item = self.items[i].clone();
        match &item.detail {
            Detail::Command { .. } => {
                self.upsert_command(&item, ch);
                if status == Some(ItemStatus::Completed) {
                    self.record_command_reads(&item, ch);
                }
            }
            Detail::Subagent { .. } => self.upsert_subagent(&item, ch),
            Detail::FileChange { files, .. } if status == Some(ItemStatus::Completed) => {
                for f in files {
                    self.record_edit(&item, f, ch);
                }
                self.bump_parent(&item, |s| s.edits += 1, ch);
            }
            Detail::Tool { tool, paths, .. }
                if tool == "imageView" && status == Some(ItemStatus::Completed) =>
            {
                for p in paths {
                    self.record_read(&item, p, ch);
                }
            }
            Detail::Tool {
                kind: ToolKind::Read,
                paths,
                title,
                ..
            } if status == Some(ItemStatus::Completed) => {
                let paths = if paths.is_empty() && title.starts_with('/') {
                    std::slice::from_ref(title)
                } else {
                    paths.as_slice()
                };
                for p in paths {
                    self.record_read(&item, p, ch);
                }
            }
            _ => {}
        }
        self.refresh_digest(item.turn, ch);
    }

    // Commands

    fn upsert_command(&mut self, item: &ItemView, ch: &mut Changes) {
        let Detail::Command {
            command,
            cwd,
            exit_code,
            ..
        } = &item.detail
        else {
            return;
        };
        if !self.root_fixed
            && self.header.root.is_none()
            && let Some(c) = cwd
        {
            self.header.root = Some(c.clone());
            for f in &mut self.files {
                f.display = display_path(Some(c), &f.path);
            }
            ch.header = true;
        }
        let output = item.command_output().unwrap_or_default();
        let (output_tail, output_lines) = tail(output);
        let display = unwrap_shell(command);
        let base = cwd.as_deref().or(self.header.root.as_deref());
        if base.is_none() {
            self.unresolved.insert(item.id.clone());
        } else {
            self.unresolved.remove(&item.id);
        }
        let mut rec = command::recognise(&display, base);
        if item.status != ItemStatus::Running {
            rec.read_output(output);
        }
        // Claude Code marks a Bash call that exits non-zero as an error, so
        // one that completed exited 0, unless a pipe or `;` hid the status.
        let zero_if_completed =
            self.header.harness.as_deref() == Some("claude") && command::is_simple(&display);
        let (outcome, outcome_inferred) =
            CommandOutcome::judge(item.status, *exit_code, rec.verdict, zero_if_completed);
        let entry = CommandEntry {
            item_id: item.id.clone(),
            parent: item.parent.clone(),
            turn: item.turn,
            headline: headline(&display),
            display,
            command: command.clone(),
            cwd: cwd.clone(),
            status: item.status,
            exit_code: *exit_code,
            outcome,
            outcome_inferred,
            kind: rec.kind,
            tool: rec.tool,
            reads: rec.reads.into_iter().map(|r| r.path).collect(),
            tests: rec.tests,
            diagnostics: rec.diagnostics,
            output_tail,
            output_lines,
            started_at_ms: item.started_at_ms,
            completed_at_ms: item.completed_at_ms,
        };
        match self.command_index.get(&item.id) {
            Some(&c) => self.commands[c] = entry,
            None => {
                self.command_index.insert(item.id.clone(), self.commands.len());
                self.commands.push(entry);
                self.bump_parent(item, |s| s.commands += 1, ch);
            }
        }
        Changes::push(&mut ch.commands, item.id.clone());
    }

    /// The files a completed command showed, unless it failed.
    fn record_command_reads(&mut self, item: &ItemView, ch: &mut Changes) {
        if item.status == ItemStatus::Completed
            && let Some(c) = self.command(&item.id)
            && c.outcome != CommandOutcome::Failed
        {
            for p in c.reads.clone() {
                self.record_read(item, &p, ch);
            }
        }
    }

    fn update_command_output(&mut self, id: &str, output: &str, ch: &mut Changes) {
        if let Some(&c) = self.command_index.get(id) {
            let (t, n) = tail(output);
            self.commands[c].output_tail = t;
            self.commands[c].output_lines = n;
            Changes::push(&mut ch.commands, id.to_owned());
        }
    }

    // Subagents

    fn upsert_subagent(&mut self, item: &ItemView, ch: &mut Changes) {
        let Detail::Subagent {
            agent,
            description,
            prompt,
            status,
            output,
            model,
            background,
            ..
        } = &item.detail
        else {
            return;
        };
        // Coordination calls (Codex's `wait`) name no agent; they show in
        // Now, not as a run of their own.
        if is_coordination(&item.detail) {
            return;
        }
        let entry = SubagentEntry {
            item_id: item.id.clone(),
            parent: item.parent.clone(),
            turn: item.turn,
            agent: agent.clone(),
            description: description.clone(),
            prompt: prompt.clone(),
            model: model.clone(),
            status: item.status,
            harness_status: status.clone(),
            background: *background,
            report: output.clone(),
            items: 0,
            commands: 0,
            edits: 0,
        };
        match self.subagent_index.get(&item.id) {
            Some(&s) => {
                let old = &self.subagents[s];
                self.subagents[s] = SubagentEntry {
                    items: old.items,
                    commands: old.commands,
                    edits: old.edits,
                    // A later update can drop what an earlier one said.
                    background: old.background || entry.background,
                    model: entry.model.clone().or_else(|| old.model.clone()),
                    ..entry
                };
            }
            None => {
                self.subagent_index.insert(item.id.clone(), self.subagents.len());
                self.subagents.push(entry);
            }
        }
        Changes::push(&mut ch.subagents, item.id.clone());
    }

    fn bump_parent(&mut self, item: &ItemView, f: impl FnOnce(&mut SubagentEntry), ch: &mut Changes) {
        if let Some(p) = &item.parent
            && let Some(&s) = self.subagent_index.get(p)
        {
            f(&mut self.subagents[s]);
            Changes::push(&mut ch.subagents, p.clone());
        }
    }

    // Files

    fn file_mut(&mut self, path: &str) -> &mut FileEntry {
        let i = match self.file_index.get(path) {
            Some(&i) => i,
            None => {
                self.file_index.insert(path.to_owned(), self.files.len());
                self.files.push(FileEntry {
                    path: path.to_owned(),
                    display: display_path(self.header.root.as_deref(), path),
                    old_path: None,
                    change: None,
                    added: 0,
                    removed: 0,
                    reads: 0,
                    edits: Vec::new(),
                    turns: Vec::new(),
                });
                self.files.len() - 1
            }
        };
        &mut self.files[i]
    }

    fn record_edit(&mut self, item: &ItemView, diff: &event::FileDiff, ch: &mut Changes) {
        if diff.path.is_empty() || self.edit_recorded(&item.id, &diff.path) {
            return;
        }
        let f = self.file_mut(&diff.path);
        f.change = Some(match (f.change, diff.kind) {
            // Created this session and edited since: still new.
            (Some(FileChangeKind::Add), FileChangeKind::Update) => FileChangeKind::Add,
            (_, k) => k,
        });
        if diff.old_path.is_some() {
            f.old_path.clone_from(&diff.old_path);
        }
        f.added += diff.added;
        f.removed += diff.removed;
        f.edits.push(FileEdit {
            item_id: item.id.clone(),
            turn: item.turn,
            kind: diff.kind,
            added: diff.added,
            removed: diff.removed,
            hunks: diff.hunks.clone(),
        });
        Changes::push(&mut f.turns, item.turn);
        Changes::push(&mut ch.files, diff.path.clone());
    }

    fn edit_recorded(&self, item_id: &str, path: &str) -> bool {
        self.file_index
            .get(path)
            .is_some_and(|&i| self.files[i].edits.iter().any(|e| e.item_id == item_id))
    }

    fn record_read(&mut self, item: &ItemView, path: &str, ch: &mut Changes) {
        if !self.reads_seen.insert((item.id.clone(), path.to_owned())) {
            return;
        }
        let f = self.file_mut(path);
        f.reads += 1;
        Changes::push(&mut f.turns, item.turn);
        Changes::push(&mut ch.files, path.to_owned());
    }

    // Asks

    fn clear_asks(&mut self, ch: &mut Changes, done: impl Fn(&Ask) -> bool) {
        let before = self.asks.len();
        self.asks.retain(|a| !done(a));
        if self.asks.len() != before {
            ch.asks = true;
        }
    }

    // Turns

    fn current_turn(&self) -> usize {
        self.turns.len().saturating_sub(1)
    }

    /// The open turn, starting one if there is none (a turn the harness
    /// started itself, or a session attached mid-turn).
    fn open_turn(&mut self, ch: &mut Changes) -> usize {
        match self.turns.last() {
            Some(t) if t.open => t.index,
            _ => self.new_turn(ch),
        }
    }

    fn new_turn(&mut self, ch: &mut Changes) -> usize {
        let outcome = if self.restoring {
            TurnOutcome::Done
        } else {
            TurnOutcome::Interrupted
        };
        self.close_turn(outcome, ch);
        let index = self.turns.len();
        self.turns.push(Turn {
            index,
            prompt: None,
            prompt_images: Vec::new(),
            narrative: None,
            narrative_item: None,
            items: Vec::new(),
            steps: 0,
            digest: Digest::default(),
            outcome: TurnOutcome::Running,
            open: true,
            // Harnesses report working just before the prompt item.
            started_at_ms: match self.header.state {
                RunState::Working { .. } => self.last_seen_ms,
                _ => None,
            },
            ended_at_ms: None,
            diff: None,
        });
        self.turn_items.push(Vec::new());
        self.header.turns = self.turns.len();
        ch.header = true;
        Changes::push(&mut ch.turns, index);
        index
    }

    fn close_turn(&mut self, outcome: TurnOutcome, ch: &mut Changes) {
        if let Some(t) = self.turns.last_mut()
            && t.open
        {
            t.open = false;
            t.outcome = outcome;
            t.ended_at_ms = self.last_seen_ms.max(t.started_at_ms);
            Changes::push(&mut ch.turns, t.index);
        }
    }

    /// Notes a time an event reported. The open turn starts at the first
    /// one after it opened.
    fn seen(&mut self, ms: Option<u64>, ch: &mut Changes) {
        let Some(ms) = ms else { return };
        self.last_seen_ms = self.last_seen_ms.max(Some(ms));
        if let Some(t) = self.turns.last_mut()
            && t.open
            && t.started_at_ms.is_none()
        {
            t.started_at_ms = Some(ms);
            Changes::push(&mut ch.turns, t.index);
        }
    }

    /// Counts what turn `turn` did, from its items.
    fn refresh_digest(&mut self, turn: usize, ch: &mut Changes) {
        let mut d = Digest::default();
        let mut changed: HashMap<&str, FileChangeKind> = HashMap::new();
        let mut read: HashSet<&str> = HashSet::new();
        for &i in &self.turn_items[turn] {
            let item = &self.items[i];
            let completed = item.status == ItemStatus::Completed;
            match &item.detail {
                Detail::FileChange { files, .. } if completed => {
                    for f in files {
                        d.lines_added += f.added;
                        d.lines_removed += f.removed;
                        let kind = changed.entry(&f.path).or_insert(f.kind);
                        // Created this turn and edited since: still new.
                        if *kind != FileChangeKind::Add || f.kind == FileChangeKind::Delete {
                            *kind = f.kind;
                        }
                    }
                }
                Detail::Command { .. } => {
                    d.commands += 1;
                    let Some(c) = self.command_index.get(&item.id).map(|&c| &self.commands[c]) else {
                        continue;
                    };
                    if c.outcome == CommandOutcome::Failed {
                        d.commands_failed += 1;
                    } else if completed {
                        read.extend(c.reads.iter().map(String::as_str));
                    }
                    if completed && c.tests.is_some() {
                        d.tests = c.tests;
                    }
                }
                Detail::Subagent { .. } if !is_coordination(&item.detail) => d.subagents += 1,
                Detail::Tool {
                    kind: ToolKind::Read,
                    paths,
                    title,
                    ..
                } => {
                    if completed {
                        if !paths.is_empty() {
                            read.extend(paths.iter().map(String::as_str));
                        } else if title.starts_with('/') {
                            read.insert(title);
                        }
                    }
                }
                Detail::Tool { tool, .. } if !is_bookkeeping(tool) => d.tools += 1,
                _ => {}
            }
        }
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        d.files_changed = count(changed.len());
        d.files_created = count(changed.values().filter(|k| **k == FileChangeKind::Add).count());
        d.files_deleted = count(changed.values().filter(|k| **k == FileChangeKind::Delete).count());
        d.files_read = count(read.iter().filter(|p| !changed.contains_key(*p)).count());
        if self.turns[turn].digest != d {
            self.turns[turn].digest = d;
            Changes::push(&mut ch.turns, turn);
        }
    }

    // Now

    fn activity(&self, item: &ItemView) -> Option<Activity> {
        let root = self.header.root.as_deref();
        let (kind, target) = match (&item.detail, item.kind) {
            (_, ItemKind::UserMessage) => return None,
            (Detail::Text, ItemKind::Reasoning) => (ActivityKind::Thinking, String::new()),
            (Detail::Text, _) => (ActivityKind::Writing, String::new()),
            (Detail::Command { command, .. }, _) => {
                let c = self.command(&item.id);
                let line = || first_line(&unwrap_shell(command)).unwrap_or_default();
                match c.map(|c| c.kind) {
                    Some(CommandKind::Test) => (ActivityKind::Testing, line()),
                    Some(CommandKind::Build) => (ActivityKind::Building, line()),
                    Some(CommandKind::Lint) => (ActivityKind::Checking, line()),
                    Some(CommandKind::Search | CommandKind::List) => (ActivityKind::Searching, line()),
                    Some(CommandKind::Read) if c.is_some_and(|c| !c.reads.is_empty()) => (
                        ActivityKind::Reading,
                        c.map(|c| {
                            c.reads
                                .iter()
                                .map(|p| display_path(root, p))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default(),
                    ),
                    Some(CommandKind::Read) => (ActivityKind::Reading, line()),
                    _ => (ActivityKind::Running, line()),
                }
            }
            (Detail::FileChange { files, .. }, _) => (
                ActivityKind::Editing,
                files
                    .iter()
                    .map(|f| display_path(root, &f.path))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            (
                d @ Detail::Subagent {
                    agent, description, ..
                },
                _,
            ) => {
                if is_coordination(d) {
                    (ActivityKind::WaitingForSubagents, String::new())
                } else {
                    let who = agent.as_deref().map(|a| format!("{a}: ")).unwrap_or_default();
                    (ActivityKind::Delegating, format!("{who}{description}"))
                }
            }
            (
                Detail::Tool {
                    tool,
                    kind,
                    title,
                    paths,
                    ..
                },
                _,
            ) => {
                let target = match paths.first() {
                    Some(p) if title.is_empty() || title == p => display_path(root, p),
                    _ if title.starts_with('/') => display_path(root, title),
                    _ => first_line(title).unwrap_or_default(),
                };
                (tool_activity(tool, *kind), target)
            }
            (Detail::Unknown, _) => (ActivityKind::Working, String::new()),
        };
        Some(Activity {
            item_id: item.id.clone(),
            parent: item.parent.clone(),
            kind,
            target,
            since_ms: item.started_at_ms,
        })
    }
}

fn tool_activity(tool: &str, kind: ToolKind) -> ActivityKind {
    match kind {
        ToolKind::Read => return ActivityKind::Reading,
        ToolKind::Edit => return ActivityKind::Editing,
        ToolKind::Fetch => return ActivityKind::Fetching,
        ToolKind::Shell => return ActivityKind::Running,
        ToolKind::Mcp => return ActivityKind::CallingTool,
        ToolKind::Other => {}
    }
    let t = tool.to_ascii_lowercase();
    if t.contains("todo") || t.contains("plan") {
        ActivityKind::Planning
    } else if t == "toolsearch" {
        ActivityKind::Working
    } else if t.contains("grep") || t.contains("glob") || t.contains("search") || t == "ls" {
        ActivityKind::Searching
    } else if t.contains("fetch") {
        ActivityKind::Fetching
    } else {
        ActivityKind::Working
    }
}

/// Tools that manage the agent's own work rather than act on the project.
/// They don't count in digests.
fn is_bookkeeping(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    t.contains("todo") || t.contains("plan") || t == "toolsearch" || t == "skill"
}

/// A subagent-shaped call that names no agent, child or task: waiting on
/// or messaging agents already running.
fn is_coordination(detail: &Detail) -> bool {
    matches!(
        detail,
        Detail::Subagent {
            agent: None,
            child: None,
            prompt: None,
            ..
        }
    )
}

fn display_path(root: Option<&str>, path: &str) -> String {
    root.map(|r| r.trim_end_matches('/'))
        .filter(|r| !r.is_empty())
        .and_then(|r| path.strip_prefix(r))
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
        .to_owned()
}

fn first_line(s: &str) -> Option<String> {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
}

/// `s` cut at a word boundary to at most `max` characters, with an ellipsis.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!(
        "{}…",
        cut.trim_end_matches(|c: char| c.is_whitespace() || c == ',' || c == '.')
    )
}

/// The first non-empty line of a command, cut to [`HEADLINE_CHARS`], with
/// `…` when lines or characters were left out.
fn headline(display: &str) -> String {
    let first = first_line(display).unwrap_or_default();
    let more_lines = display.trim().lines().count() > 1;
    if first.chars().count() > HEADLINE_CHARS {
        let cut: String = first.chars().take(HEADLINE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else if more_lines {
        format!("{first} …")
    } else {
        first
    }
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// The last [`OUTPUT_TAIL_LINES`] lines of `output`, and its line count.
fn tail(output: &str) -> (String, usize) {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.len().saturating_sub(OUTPUT_TAIL_LINES);
    (lines[start..].join("\n"), lines.len())
}

/// The image a Codex `imageView` looked at or an `imageGeneration` made,
/// by path. `None` for other details.
fn tool_images(detail: &Detail) -> Option<Vec<ImageRef>> {
    let Detail::Tool {
        tool, input, paths, ..
    } = detail
    else {
        return None;
    };
    let path = match tool.as_str() {
        "imageView" => paths
            .first()
            .cloned()
            .or_else(|| input.get("path")?.as_str().map(str::to_owned)),
        "imageGeneration" => ["savedPath", "path"].iter().find_map(|k| {
            input
                .get(*k)?
                .as_str()
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
        }),
        _ => return None,
    };
    Some(
        path.map(|p| ImageRef {
            path: Some(p),
            ..ImageRef::default()
        })
        .into_iter()
        .collect(),
    )
}
