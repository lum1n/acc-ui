//! The host side: the MCP server script, the shell that installs it, and
//! the shell that delivers an answer to a waiting `acc_ui_ask`.
//!
//! Every piece is a POSIX shell script a client runs over its own link to
//! the host (an SSH exec, a local shell). Nothing here talks to the host
//! itself, and ACC isn't involved: the client passes the server to ACC as
//! one of the session's MCP servers.
//!
//! ```text
//! client ── setup_script() ──▶ host: ~/.acc-ui/bin/acc-ui-mcp-<hash>.sh, path printed
//! client ── mcp_server(path, session) ──▶ ACC session spec
//! agent  ── acc_ui_ask(spec) ──▶ server waits for ~/.acc-ui/<session>/<spec id>/answer.json
//! client ── answer_script(session, id, answer) ──▶ host: answer.json written, call returns
//! ```

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use acc_ui::{schema, valid_id};

use crate::submit::json_in_tag;

/// Name of the MCP server, as the agent sees it.
pub const SERVER_NAME: &str = "acc_ui";
/// Tool that shows a component and returns at once.
pub const RENDER_TOOL: &str = "acc_ui_render";
/// Tool that shows a component and waits for the user's answer.
pub const ASK_TOOL: &str = "acc_ui_ask";
/// Environment variable naming the session's answer directory.
pub const SESSION_ENV: &str = "ACC_UI_SESSION";

/// How each result text of the tools starts, so a client can tell them
/// apart in a transcript.
pub mod result_marker {
    pub const RENDERED: &str = "acc-ui: shown";
    pub const ANSWERED: &str = "acc-ui: answered";
    pub const DISMISSED: &str = "acc-ui: dismissed";
    pub const TIMED_OUT: &str = "acc-ui: timed out";
    pub const ERROR: &str = "acc-ui: error";
}

const TEMPLATE: &str = include_str!("server.sh");

/// The MCP server script, ready to write to the host and run with `sh`.
pub fn server_script() -> String {
    use result_marker::*;
    let tools = tools_list().to_string();
    // Texts go into JSON strings inside shell double quotes: no `"`, `\`,
    // `$` or backticks in them.
    let texts = [
        (
            "@RENDERED@",
            format!("{RENDERED}. The user sees the component; don't repeat its content."),
        ),
        (
            "@TIMED_OUT@",
            format!(
                "{TIMED_OUT}. The user hasn't answered yet. Carry on without it or end your turn; \
                 an answer given later arrives as a user message holding \
                 <acc-ui-submit id=...>{{values}}</acc-ui-submit>."
            ),
        ),
        (
            "@ERR_SPEC_OBJECT@",
            format!("{ERROR}: spec must be a JSON object, not a string."),
        ),
        (
            "@ERR_SPEC_ID@",
            format!("{ERROR}: acc_ui_ask needs spec.id, 1-64 of A-Z a-z 0-9 . _ -, unique in this session."),
        ),
        (
            "@ERR_ID_USED@",
            format!("{ERROR}: this spec.id was already used in this session; ask again with a new id."),
        ),
        (
            "@ERR_DIR@",
            format!("{ERROR}: can't create the answer directory."),
        ),
        ("@ERR_ANSWER@", format!("{ERROR}: the answer file is malformed.")),
    ];
    let mut s = TEMPLATE.replace("@VERSION@", env!("CARGO_PKG_VERSION"));
    for (k, v) in texts {
        debug_assert!(!v.contains(['"', '\\', '$', '`']), "{k}");
        s = s.replace(k, &v);
    }
    // The tool list goes into single quotes, not a heredoc in `$(...)`,
    // which bash 3.2 (macOS's sh) misparses when the text holds a quote.
    s.replace("@TOOLS@", &tools.replace('\'', "'\\''"))
}

/// The `tools/list` result the server answers with. The spec schema goes
/// with `acc_ui_render` only; `acc_ui_ask` takes the same spec and points
/// there, so the agent reads the schema once a turn instead of twice.
pub fn tools_list() -> Value {
    let schema = tool_input_schema();
    let ask_schema = json!({
        "type": "object",
        "properties": {
            "spec": {
                "type": "object",
                "description": "An acc-ui/1 spec, exactly as acc_ui_render's input schema describes \
                    it, with an id unique in the session.",
            }
        },
        "required": ["spec"],
    });
    json!({
        "tools": [
            {
                "name": RENDER_TOOL,
                "title": "Show a component",
                "description": "Show the user a rich component described by an acc-ui/1 spec: \
                    cards, key/value lists, tables, badges, progress, checklists, callouts, \
                    file/PR/commit links, charts, Mermaid diagrams, images, markdown, HTML, \
                    and buttons, chips or forms whose answers come back as a user message. \
                    Returns at once. Use it for structured results and status; don't repeat \
                    its content in text.",
                "inputSchema": schema,
                "annotations": { "readOnlyHint": true, "openWorldHint": false },
            },
            {
                "name": ASK_TOOL,
                "title": "Ask with a component",
                "description": "Show the user an acc-ui/1 spec with inputs (a form, buttons \
                    with submit actions, chips) and wait for their answer, returned as JSON \
                    values. Give the spec an id unique in the session. Use it for choices and \
                    structured input you need before carrying on.",
                "inputSchema": ask_schema,
                "annotations": { "readOnlyHint": true, "openWorldHint": false },
            }
        ]
    })
}

/// A shell script that installs the server on the host and prints its
/// absolute path. The path holds a hash of the script, so a new version
/// installs beside the old one and an unchanged one isn't rewritten.
pub fn setup_script() -> String {
    let script = server_script();
    let hash = short_hash(&script);
    format!(
        "set -eu\n\
         d=\"${{ACC_UI_DIR:-$HOME/.acc-ui}}/bin\"\n\
         f=\"$d/acc-ui-mcp-{hash}.sh\"\n\
         if [ ! -f \"$f\" ]; then\n\
         \x20 mkdir -p \"$d\"\n\
         \x20 t=\"$f.tmp.$$\"\n\
         \x20 cat >\"$t\" <<'ACC_UI_SERVER_{hash}'\n\
         {script}\
         ACC_UI_SERVER_{hash}\n\
         \x20 chmod 755 \"$t\"\n\
         \x20 mv -f \"$t\" \"$f\"\n\
         fi\n\
         printf '%s\\n' \"$f\"\n"
    )
}

/// How the client should launch the server: pass this to ACC as one of the
/// session's MCP servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The server launch for a session. `script_path` is what
/// [`setup_script`] printed; `session` names the session's answer
/// directory and must be a valid id (see [`answer_script`]).
pub fn mcp_server(script_path: &str, session: &str) -> Result<McpServer, String> {
    if !valid_id(session) {
        return Err(format!("session {session:?} is not 1-64 of A-Z a-z 0-9 . _ -"));
    }
    Ok(McpServer {
        name: SERVER_NAME.to_owned(),
        command: "sh".to_owned(),
        args: vec![script_path.to_owned()],
        env: vec![(SESSION_ENV.to_owned(), session.to_owned())],
    })
}

/// The user's response to an `acc_ui_ask`.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// Submitted values.
    Values(Map<String, Value>),
    /// Closed without answering.
    Dismissed,
}

/// The tool result an answer becomes, as the server returns it.
pub fn answer_result(id: &str, answer: &Answer) -> Value {
    use result_marker::*;
    match answer {
        Answer::Values(values) => json!({
            "content": [{
                "type": "text",
                "text": format!(
                    "{ANSWERED}: <acc-ui-submit id=\"{id}\">{}</acc-ui-submit>",
                    json_in_tag(values)
                ),
            }],
            "structuredContent": { "id": id, "values": values },
        }),
        Answer::Dismissed => json!({
            "content": [{
                "type": "text",
                "text": format!("{DISMISSED}: the user closed {id:?} without answering."),
            }],
        }),
    }
}

/// A shell script that answers the waiting `acc_ui_ask` for spec `id` in
/// `session`. It writes a temporary file and renames it into place, so the
/// server never reads half an answer. An answer written just before the
/// call reaches the server still counts. Answering a call that already
/// ended leaves the file for nobody; send [`crate::submit_prompt`] then.
pub fn answer_script(session: &str, id: &str, answer: &Answer) -> Result<String, String> {
    for (what, v) in [("session", session), ("id", id)] {
        if !valid_id(v) {
            return Err(format!("{what} {v:?} is not 1-64 of A-Z a-z 0-9 . _ -"));
        }
    }
    let body = answer_result(id, answer).to_string();
    let mut delim = format!("ACC_UI_ANSWER_{}", short_hash(&body));
    while body.contains(&delim) {
        delim.push('X');
    }
    Ok(format!(
        "set -eu\n\
         umask 077\n\
         d=\"${{ACC_UI_DIR:-$HOME/.acc-ui}}/{session}/{id}\"\n\
         mkdir -p \"$d\"\n\
         t=\"$d/.answer.$$\"\n\
         cat >\"$t\" <<'{delim}'\n\
         {body}\n\
         {delim}\n\
         mv -f \"$t\" \"$d/answer.json\"\n"
    ))
}

fn short_hash(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The `inputSchema` of both tools: `{"spec": <spec>}`, with the spec's
/// definitions at the root where `$ref`s resolve.
fn tool_input_schema() -> Value {
    let mut spec = match schema() {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let mut defs = match spec.remove("$defs") {
        Some(Value::Object(d)) => d,
        _ => Map::new(),
    };
    spec.remove("$schema");
    spec.remove("title");
    defs.insert("Spec".to_owned(), Value::Object(spec));
    json!({
        "type": "object",
        "properties": { "spec": { "$ref": "#/$defs/Spec" } },
        "required": ["spec"],
        "$defs": defs,
    })
}
