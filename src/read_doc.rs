use serde_json::Value;

use crate::mcp;

const DEFAULT_LIMIT: usize = 4096;

#[derive(serde::Deserialize)]
struct ReadArgs {
    id: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

pub fn read_chunk(id: &str, offset: usize, limit: usize) -> Result<String, String> {
    let body = crate::store::load(id)?;
    let total = body.len();
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);
    let chunk = String::from_utf8_lossy(&body[start..end]);
    let tail = if end < total {
        format!("\n[more: bytes {end}..{total} available]")
    } else {
        String::new()
    };
    Ok(format!(
        "Document {id} ({total} bytes total)\nbytes {start}..{end}:\n{chunk}{tail}"
    ))
}

pub fn tool_definition() -> Value {
    serde_json::json!({
        "name": "read_doc",
        "description": "Read a segment of a doc that was fetched and stored by the fetch_url tool.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "document id"},
                "offset": { "type": "integer", "description": "start reading here; offset in bytes"},
                "limit": { "type": "integer", "description": format!("maximum number of bytes to read (default {DEFAULT_LIMIT})")}
            },
            "required": ["id"]
        }
    })
}

pub fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let parsed_args: ReadArgs =
        serde_json::from_value(args).map_err(|_| mcp::invalid_params("bad params"))?;

    let offset = parsed_args.offset.unwrap_or(0);
    let limit = parsed_args.limit.unwrap_or(DEFAULT_LIMIT);

    let chunk_res = read_chunk(parsed_args.id.as_str(), offset, limit);

    match chunk_res {
        Ok(chunk) => Ok(serde_json::json!({
            "content": [
                { "type": "text", "text": chunk },
            ],
            "isError": false
        })),
        Err(err) => Ok(mcp::error_message_json(&err)),
    }
}
