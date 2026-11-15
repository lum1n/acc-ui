//! Prints the generated texts:
//! `cargo run -p acc-ui-agent --example dump -- WHAT`, where WHAT is
//! `conventions`, `conventions-skill`, `schema`, `skill`, `instructions`,
//! `server`, `setup` or `tools`.
fn main() {
    let what = std::env::args().nth(1).unwrap_or_default();
    let out = match what.as_str() {
        "conventions" => acc_ui::conventions(),
        "conventions-skill" => acc_ui::conventions_skill(),
        "schema" => serde_json::to_string_pretty(&acc_ui::schema()).unwrap_or_default(),
        "skill" => acc_ui_agent::skill_markdown(),
        "instructions" => acc_ui_agent::instructions(),
        "server" => acc_ui_agent::server_script(),
        "setup" => acc_ui_agent::setup_script(),
        "tools" => acc_ui_agent::host::tools_list().to_string(),
        _ => {
            eprintln!(
                "usage: dump conventions|conventions-skill|schema|skill|instructions|server|setup|tools"
            );
            std::process::exit(2);
        }
    };
    println!("{out}");
}
