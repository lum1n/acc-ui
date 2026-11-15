//! Markdown fences that carry components.
//!
//! | Info string   | Body                         |
//! |---------------|------------------------------|
//! | `acc-ui`      | an `acc-ui/1` spec           |
//! | `chart`       | CSV (see below), or a `chart` node as JSON |
//! | `mermaid`     | Mermaid source               |
//! | `html render` | HTML to show in the sandbox  |
//!
//! A plain ` ```html ` fence stays code: agents show HTML source far more
//! often than they mean it to run, so rendering takes the `render` flag.
//!
//! A CSV chart reads fine as plain text, so it's what the conventions
//! ([`crate::conventions`]) teach. Optional `key: value` lines (`kind`,
//! `title`, `x`, `y`, `stacked`) come first; then a header row whose first
//! column names the x axis and whose other columns are the series:
//!
//! ```text
//! kind: line
//! title: p95 latency (ms)
//! day,before,after
//! Mon,120,80
//! Tue,135,78
//! ```
//!
//! Without `kind`, text x values make a bar chart and numbers a line chart.
//!
//! While a fence is still streaming, show it as code; parse it once closed.

use serde_json::{Map, Value, json};

use crate::normalize::{Normalized, normalize, normalize_value};

/// The fence's language and flags, split from its info string.
fn info_words(info: &str) -> (String, Vec<String>) {
    let mut words = info.split_whitespace().map(str::to_ascii_lowercase);
    let lang = words.next().unwrap_or_default();
    (lang, words.collect())
}

/// Whether a fence with this info string is one of ours, before its body is
/// complete. Lets a renderer pick a placeholder while it streams.
pub fn is_component_fence(info: &str) -> bool {
    let (lang, flags) = info_words(info);
    match lang.as_str() {
        "acc-ui" | "chart" | "mermaid" => true,
        "html" => flags.iter().any(|f| f == "render"),
        _ => false,
    }
}

/// Parses a closed fence. `None` when the fence is not a component fence;
/// otherwise the normalised spec, which is `None` with errors when the body
/// is unusable (show the fence as code then).
pub fn parse_fence(info: &str, body: &str) -> Option<Normalized> {
    if info_words(info).0 == "acc-ui" {
        return Some(normalize(body));
    }
    Some(match fence_node(info, body)? {
        Ok(node) => normalize_value(json!({ "root": node })),
        Err(e) => Normalized {
            spec: None,
            errors: vec![e],
        },
    })
}

/// The root node a closed component fence holds, before normalising, for
/// embedding in a larger tree. `None` when the fence is not a component
/// fence.
pub(crate) fn fence_node(info: &str, body: &str) -> Option<Result<Value, String>> {
    if !is_component_fence(info) {
        return None;
    }
    let (lang, _) = info_words(info);
    Some(match lang.as_str() {
        "acc-ui" => match serde_json::from_str::<Value>(body) {
            Ok(Value::Object(mut spec)) => Ok(spec.remove("root").unwrap_or(Value::Object(spec))),
            Ok(_) => Err("an acc-ui fence must hold a JSON object".to_owned()),
            Err(e) => Err(format!("not valid JSON: {e}")),
        },
        "chart" if !body.trim_start().starts_with('{') => csv_chart(body),
        "chart" => match serde_json::from_str::<Value>(body) {
            Ok(Value::Object(mut chart)) => {
                chart.entry("type").or_insert_with(|| "chart".into());
                Ok(Value::Object(chart))
            }
            Ok(_) => Err("a chart fence must hold CSV or a JSON object".to_owned()),
            Err(e) => Err(format!("not valid JSON: {e}")),
        },
        "mermaid" => Ok(json!({ "type": "mermaid", "code": body })),
        _ => Ok(json!({ "type": "html", "html": body })),
    })
}

/// A chart node from a CSV chart fence.
fn csv_chart(body: &str) -> Result<Value, String> {
    let mut chart = Map::new();
    chart.insert("type".to_owned(), "chart".into());
    let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty()).peekable();
    while let Some(line) = lines.peek() {
        let Some((key, value)) = line.split_once(':') else {
            break;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        let field = match key.as_str() {
            "kind" | "type" => "kind",
            "title" => "title",
            "x" | "x_label" => "x_label",
            "y" | "y_label" => "y_label",
            "stacked" => "stacked",
            _ => break,
        };
        let v = if field == "stacked" {
            Value::Bool(matches!(value, "true" | "yes" | "1"))
        } else if field == "kind" {
            Value::String(value.to_ascii_lowercase())
        } else {
            Value::String(value.to_owned())
        };
        chart.insert(field.to_owned(), v);
        lines.next();
    }
    let header = csv_row(lines.next().ok_or("a chart needs a header row and data")?);
    if header.len() < 2 {
        return Err("a chart's header needs an x column and at least one series".to_owned());
    }
    chart
        .entry("x_label")
        .or_insert_with(|| Value::String(header[0].clone()));
    let mut series: Vec<Vec<Value>> = vec![Vec::new(); header.len() - 1];
    let mut text_x = false;
    for line in lines {
        let cells = csv_row(line);
        let Some((x, ys)) = cells.split_first() else {
            continue;
        };
        let x = match x.parse::<f64>() {
            Ok(n) if n.is_finite() => Value::from(n),
            _ => {
                text_x = true;
                Value::String(x.clone())
            }
        };
        for (s, y) in series.iter_mut().zip(ys) {
            if let Ok(y) = y.replace([' ', '_'], "").parse::<f64>() {
                s.push(json!({ "x": x, "y": y }));
            }
        }
    }
    chart
        .entry("kind")
        .or_insert_with(|| if text_x { "bar" } else { "line" }.into());
    let series: Vec<Value> = header[1..]
        .iter()
        .zip(series)
        .map(|(name, points)| json!({ "name": name, "points": points }))
        .collect();
    chart.insert("series".to_owned(), Value::Array(series));
    Ok(Value::Object(chart))
}

/// One CSV row: comma-separated (tab-separated when it has tabs), with
/// `"quoted, fields"` and `""` for a quote inside them.
fn csv_row(line: &str) -> Vec<String> {
    let sep = if line.contains('\t') { '\t' } else { ',' };
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => cells.push(std::mem::take(&mut cell).trim().to_owned()),
            c => cell.push(c),
        }
    }
    cells.push(cell.trim().to_owned());
    cells
}
