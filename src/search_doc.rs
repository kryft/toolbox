use crate::mcp;
use serde_json::Value;

const MAX_MATCHES: usize = 20;
const SNIPPET_CHARS: usize = 200;

#[derive(serde::Deserialize)]
struct SearchArgs {
    id: String,
    pattern: String,
}

fn find_lines(body: &str, pattern: &str) -> Vec<(usize, usize, String)> {
    let needle = pattern.to_lowercase();
    let mut out = Vec::new();
    let mut line_start = 0usize;
    for (i, line) in body.lines().enumerate() {
        if line.to_lowercase().contains(&needle) {
            out.push((
                line_start,
                i + 1,
                line.chars().take(SNIPPET_CHARS).collect(),
            ));
        }
        line_start += line.len() + 1; // +1 for the \n
    }
    out
}

pub fn tool_definition() -> Value {
    serde_json::json!({
        "name": "search_doc",
        "description": "Search for a string in a doc that was fetched and stored by the fetch_url tool.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "id of the doc to be searched"},
                "pattern": { "type": "string", "description": "string to be searched. Plain substrings only; regexes are not supported."}
            },
            "required": ["id", "pattern"]
        }
    })
}

pub fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let parsed_args: SearchArgs =
        serde_json::from_value(args).map_err(|_| mcp::invalid_params("bad params"))?;

    if parsed_args.pattern.is_empty() {
        return Err(mcp::invalid_params("empty pattern"));
    }

    let raw = match crate::store::load(&parsed_args.id) {
        Ok(b) => b,
        Err(err) => return Ok(mcp::error_message_json(&err)),
    };

    let text = String::from_utf8_lossy(&raw);
    let matches = find_lines(&text, &parsed_args.pattern);

    let mut out = format!(
        "Search for '{}' in {}: {} match(es)\n",
        parsed_args.pattern,
        parsed_args.id,
        matches.len()
    );

    for (line_start, line_num, snippet) in matches.iter().take(MAX_MATCHES) {
        out.push_str(&format!("line {line_num}, byte {line_start}: {snippet}\n"));
    }
    if matches.len() > MAX_MATCHES {
        out.push_str(&format!(
            "... and {} more matches",
            matches.len() - MAX_MATCHES
        ));
    }

    Ok(serde_json::json!({
        "content": [
            { "type": "text", "text": out },
        ],
        "isError": false
    }))
}
