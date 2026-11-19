//! Prints the generated texts:
//! `cargo run -p acc-ui-agent --example dump -- WHAT`, where WHAT is
//! `conventions`, `conventions-skill`, `schema`, `skill`, `instructions`,
//! `server`, `setup` or `tools`; `answer SESSION ID VALUES_JSON` prints the
//! script that answers a waiting `acc_ui_ask` (scripts/record-fixture.py).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let what = args.first().cloned().unwrap_or_default();
    let out = match what.as_str() {
        "conventions" => acc_ui::conventions(),
        "conventions-skill" => acc_ui::conventions_skill(),
        "schema" => serde_json::to_string_pretty(&acc_ui::schema()).unwrap_or_default(),
        "skill" => acc_ui_agent::skill_markdown(),
        "instructions" => acc_ui_agent::instructions(),
        "server" => acc_ui_agent::server_script(),
        "setup" => acc_ui_agent::setup_script(),
        "tools" => acc_ui_agent::host::tools_list().to_string(),
        "answer" if args.len() == 4 => {
            let values = match serde_json::from_str(&args[3]) {
                Ok(serde_json::Value::Object(m)) => m,
                _ => {
                    eprintln!("dump answer: VALUES_JSON must be a JSON object");
                    std::process::exit(2);
                }
            };
            let answer = acc_ui_agent::Answer::Values(values);
            match acc_ui_agent::answer_script(&args[1], &args[2], &answer) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("dump answer: {e}");
                    std::process::exit(2);
                }
            }
        }
        _ => {
            eprintln!(
                "usage: dump conventions|conventions-skill|schema|skill|instructions|server|setup|tools\n       dump answer SESSION ID VALUES_JSON"
            );
            std::process::exit(2);
        }
    };
    println!("{out}");
}
