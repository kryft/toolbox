use crate::{chunk, mcp};
use serde_json::Value;

#[derive(serde::Deserialize)]
pub struct SummarizeArgs {
    id: String,
    query: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    max_words: Option<usize>,
}

const MAX_CHUNKS: usize = 64;

const DESCRIPTION: &str = r#"Summarize a doc (stored by fetch_url), optionally focused on a
query. An LLM reads the doc chunk by chunk, so this is slow; use it to
compress a large doc before or instead of scanning it — summarize, then
run triage_doc with the summary as its `context`, or aim a scan window
with the map's byte spans.
Returns a summary (default at most 400 words; max_words, capped at
4000) plus a map of the most important regions with byte spans.
Cost warning: the scan is sequential (one LLM call per chunk, at most
64 chunks); cost grows with the doc size and max_words. A failed chunk
fails the whole call."#;

pub fn tool_definition() -> Value {
    serde_json::json!({
        "name": "summarize_doc",
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "document id"},
                "query": { "type": "string", "description": "optional free-form focus for the summary"},
                "offset": { "type": "integer", "description": "start here; offset in bytes"},
                "limit": { "type": "integer", "description": format!("maximum number of bytes to summarize (by default summarizes the entire doc, up to the maximum number of chunks)")},
                "max_words": { "type": "integer", "description": "maximum number of words in the summary (default 400, capped at 4000)"}
            },
            "required": ["id"]
        }
    })
}

fn prepare<'a>(
    doc: &'a str,
    offset: usize,
    limit: Option<usize>,
    chunk_bytes: usize,
) -> Result<Vec<chunk::Chunk<'a>>, String> {
    let total = doc.len();
    let range_end = doc.floor_char_boundary(
        offset
            .saturating_add(limit.unwrap_or(usize::MAX))
            .min(total),
    );

    let chunks = chunk::split(doc, offset, limit, chunk_bytes, MAX_CHUNKS);

    if chunks.is_empty() {
        return Err(String::from(
            "nothing to summarize (offset/limit leave an empty range)",
        ));
    }

    if chunks.len() == MAX_CHUNKS && chunks.last().unwrap().end < range_end {
        return Err(format!(
            "document too large for one summary (64 chunks, ended at byte {} of {}); \
        narrow with offset/limit",
            chunks.last().unwrap().end,
            total
        ));
    }
    Ok(chunks)
}

pub async fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let _parsed_args: SummarizeArgs =
        serde_json::from_value(args).map_err(|_| mcp::invalid_params("bad params"))?;

    Ok(mcp::error_message_json("summarize_doc is not implemented yet"))
}
