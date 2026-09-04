use crate::{chunk, llm, mcp};
use serde_json::Value;

#[derive(serde::Deserialize)]
pub struct TriageArgs {
    id: String,
    context: Option<String>,
    max_hits: Option<usize>,
    query: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

const MAX_CHUNKS: usize = 64;
const SNIPPET_BYTES: usize = 256;

const DESCRIPTION: &str = r#"Find the parts of a doc (stored by fetch_url) most relevant to a
query. An LLM analyzes the document chunk by chunk, so this is slower
than search_doc; use it for semantic matching, e.g. when search_doc's
exact substring search misses on vocabulary mismatch.
Query phrasing controls the scan: "mentions of X" / "where does X
appear" look for mentions of specific things; "views on X" / "anything
that bears on X" look for treatments of a theme (add "possibly" or
"even tangentially" for a broader sweep). Bare abstract phrases ("the
character of X") tend to return zero — phrase the theme as what to
look for.
Returns up to max_hits (default 5, max 1000) hits: relevance score,
short note, byte/line location, and a verbatim snippet. Zero hits is a
normal result, not an error.
Cost warning: the scan is sequential (one LLM call per chunk, at most
64 chunks) and each call's output budget grows with max_hits, so a
large max_hits on a large doc is slow and yields a large result."#;

// System prompt for one chunk analysis. The JSON contract and the
// relevance floor are mirrored in PROJECT.md; keep the two in sync.
// (format! needs a literal at the call site, so the template lives in a
// function, not a const.)
fn system_prompt(exclusive_line: usize, max_hits: usize) -> String {
    format!(
        concat!(
            "Find the parts of this document chunk that are relevant to the query.\n",
            "Respond with a JSON object only: {{\"regions\": [{{\"line_start\": n, \"line_end\": n, \"score\": s, \"note\": t}}]}}\n",
            "- line numbers are 1-based within the chunk, line_end >= line_start\n",
            "- lines before line {} are repeated context from the previous chunk, for orientation only: never report them, and never use them to skip content; report relevant regions in the remaining lines even if they continue a topic visible in the context\n",
            "- first interpret the query: a query about specific things (e.g. \"mentions of X\", \"where does X appear\") asks for mentions; a query about a theme, property, question, or subject (e.g. \"the character of X\", \"views on X\") asks for treatments of it\n",
            "- a mention query: a region qualifies only if it directly mentions the queried things; never report a region that merely touches their general theme\n",
            "- a theme query: a region qualifies if it bears on the queried subject, by describing, discussing, or clearly exemplifying it, even when the passage's main topic is something else; never report a region that merely grazes the subject\n",
            "- breadth: if the query explicitly asks for anything even possibly or tangentially relevant, include peripheral bearings; otherwise include only clearly non-incidental bearings\n",
            "- report at most {} regions, most relevant first, and never more than the number of qualifying regions you found; padding with weak regions is wrong\n",
            "- if the chunk contains no qualifying region, respond with {{\"regions\": []}}\n",
            "- score: 1-10, ordinal priority within this document, not calibrated confidence; 1 is still a real, passing mention of the queried subject\n",
            "- note: at most 15 words describing why the region matters; do not paraphrase the text\n",
        ),
        exclusive_line,
        max_hits
    )
}

pub fn tool_definition() -> Value {
    serde_json::json!({
        "name": "triage_doc",
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "document id"},
                "query": { "type": "string", "description": "free-form relevance query"},
                "context": { "type": "string", "description": "optional document context given to each chunk's analysis, such as a summary of the whole document (e.g. from summarize_doc) or a glossary of self-defined terms"},
                "offset": { "type": "integer", "description": "start scanning here; offset in bytes"},
                "limit": { "type": "integer", "description": format!("maximum number of bytes to scan (by default scans the entire doc, up to the maximum number of chunks)")},
                "max_hits": { "type": "integer", "description": "maximum number of hits to return (default 5, capped at 1000)"}
            },
            "required": ["id", "query"]
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
            "nothing to scan (offset/limit leave an empty range)",
        ));
    }

    if chunks.len() == MAX_CHUNKS && chunks.last().unwrap().end < range_end {
        return Err(format!(
            "document too large for one triage (64 chunks, ended at byte {} of {}); \
        narrow with offset/limit",
            chunks.last().unwrap().end,
            total
        ));
    }
    Ok(chunks)
}

#[derive(Debug)]
struct Hit {
    score: f64,
    note: String,
    line_start: usize, // 1-based, doc absolute
    line_end: usize,
    byte_start: usize,
    byte_end: usize, // end of the region's last line
    snippet: String,
}

fn snippet_for(doc: &str, byte_start: usize) -> String {
    let start = doc.floor_char_boundary(byte_start.min(doc.len()));

    let end = doc.floor_char_boundary(start.saturating_add(SNIPPET_BYTES).min(doc.len()));

    let end = doc[start..end].rfind('\n').map_or(end, |i| start + i + 1);
    doc[start..end].to_string()
}

fn region_hits(
    chunk: &chunk::Chunk,
    lines_before: usize,
    doc: &str,
    value: &Value,
    max_regions: usize,
) -> Vec<Hit> {
    let Some(regions) = value.get("regions").and_then(Value::as_array) else {
        return Vec::new();
    };

    regions
        .iter()
        .filter_map(|r| region_from(chunk, lines_before, doc, r, chunk.line_starts.len()))
        .take(max_regions)
        .collect()
}

fn region_from(
    chunk: &chunk::Chunk,
    lines_before: usize,
    doc: &str,
    region: &Value,
    n_lines: usize,
) -> Option<Hit> {
    let ls = region.get("line_start").and_then(Value::as_u64)?;
    let le = region.get("line_end").and_then(Value::as_u64)?;
    let score = region.get("score").and_then(Value::as_f64)?;
    let note = region.get("note").and_then(Value::as_str)?;

    if ls < 1 || le > n_lines as u64 || ls > le {
        return None;
    }
    if !score.is_finite() || !(0.0..=10.0).contains(&score) {
        return None;
    }
    if note.is_empty() {
        return None;
    }

    let byte_start = chunk.line_starts[(ls - 1) as usize];
    if byte_start < chunk.exclusive_start {
        return None;
    };
    let byte_end = chunk
        .line_starts
        .get(le as usize)
        .copied()
        .unwrap_or(chunk.end);

    Some(Hit {
        score: score,
        note: note.to_string(),
        line_start: lines_before + ls as usize,
        line_end: lines_before + le as usize,
        byte_start,
        byte_end,
        snippet: snippet_for(doc, byte_start),
    })
}

fn chunk_prompt(
    query: &str,
    context: Option<&str>,
    c: &chunk::Chunk,
    max_hits: usize,
) -> (String, String) {
    let system = system_prompt(c.context_lines + 1, max_hits);
    let n = c.line_starts.len();
    let width = n.to_string().len();
    let mut body = String::new();
    // add line numbers
    for (i, line) in c.text.split_inclusive('\n').enumerate() {
        body.push_str(&format!("{:width$}\t{line}", i + 1));
    }
    let mut user = format!("Query: {query}\n");
    if let Some(ctx) = context {
        user.push_str(&format!(
            "Document context (orientation only; the chunk text is authoritative):\n{ctx}\n"
        ));
    }
    user.push_str("Chunk:\n");
    user.push_str(&body);
    (system, user)
}

pub async fn triage(
    doc: &str,
    query: &str,
    context: Option<&str>,
    offset: usize,
    limit: Option<usize>,
    chunk_bytes: usize,
    max_hits: usize,
    cfg: &llm::LlmConfig,
) -> Result<String, String> {
    let chunks = prepare(doc, offset, limit, chunk_bytes)?;

    let max_hits = max_hits.clamp(1, 1000);

    let mut hits: Vec<Hit> = Vec::new();
    let mut untriaged: Vec<(usize, usize, String)> = Vec::new(); // byte_start, byte_end, reason
    let mut lines_before = 0;

    for (i, c) in chunks.iter().enumerate() {
        if i == 0 {
            lines_before = doc[..c.start].matches('\n').count();
        } else {
            lines_before += doc[chunks[i - 1].start..c.start].matches('\n').count();
        }

        let (system, user) = chunk_prompt(query, context, c, max_hits);
        let max_tokens = (128 + 64 * max_hits) as u32;

        match llm::chat(cfg, &system, &user, max_tokens).await {
            Err(reason) => untriaged.push((c.start, c.end, reason)),
            Ok(text) => match llm::extract_json(&text) {
                Err(reason) => untriaged.push((c.start, c.end, reason)),
                Ok(value) => hits.extend(region_hits(c, lines_before, doc, &value, max_hits)),
            },
        }
    }

    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    let top: Vec<&Hit> = hits.iter().take(max_hits).collect();

    let mut out = String::new();

    out.push_str(&format!(
        "Triage for '{}': {} hit(s), scanned {} chunk(s), bytes {}..{}\n",
        { query },
        { top.len() },
        { chunks.len() },
        { chunks[0].start },
        { chunks.last().unwrap().end }
    ));

    for (start, end, reason) in untriaged.iter() {
        out.push_str(&format!("   untriaged: bytes {start}..{end} ({reason})\n"));
    }

    for (i, hit) in top.iter().enumerate() {
        out.push_str(&format!("{}. [{}] {}\n", i + 1, hit.score, hit.note));
        out.push_str(&format!(
            "   line {}..{}, bytes {}..{}\n",
            hit.line_start, hit.line_end, hit.byte_start, hit.byte_end
        ));

        for line in hit.snippet.lines() {
            out.push_str(&format!("   |{line}\n"));
        }
    }

    Ok(out)
}

pub async fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let parsed_args: TriageArgs =
        serde_json::from_value(args).map_err(|_| mcp::invalid_params("bad params"))?;

    if parsed_args.query.is_empty() {
        return Err(mcp::invalid_params("empty query"));
    }

    let raw = match crate::store::load(&parsed_args.id) {
        Ok(b) => b,
        Err(err) => return Ok(mcp::error_message_json(&err)),
    };

    let text = String::from_utf8_lossy(&raw);

    match triage(
        &text,
        &parsed_args.query,
        parsed_args.context.as_deref(),
        parsed_args.offset.unwrap_or(0),
        parsed_args.limit,
        chunk::default_chunk_bytes(),
        parsed_args.max_hits.unwrap_or(5),
        &llm::LlmConfig::default(),
    )
    .await
    {
        Ok(out) => Ok(serde_json::json!({
            "content": [{ "type": "text", "text": out }],
            "isError": false
        })),
        Err(err) => Ok(mcp::error_message_json(&err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `total` bytes of one-char lines (line starts at every even offset).
    fn uniform(total: usize) -> String {
        "a\n".repeat(total / 2)
    }

    #[test]
    fn small_doc_yields_one_chunk_covering_the_range() {
        let doc = "hello\nworld\n";
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks[0].end, doc.len());
        assert_eq!(chunks[0].text, doc);
    }

    #[test]
    fn offset_at_or_past_end_is_nothing_to_scan() {
        let doc = "hello\nworld\n";
        for offset in [doc.len(), doc.len() + 100] {
            let err = prepare(&doc, offset, None, 64).unwrap_err();
            assert!(err.contains("nothing to scan"), "unexpected: {err}");
        }
    }

    #[test]
    fn zero_limit_is_nothing_to_scan() {
        let doc = "hello\nworld\n";
        let err = prepare(&doc, 0, Some(0), 64).unwrap_err();
        assert!(err.contains("nothing to scan"), "unexpected: {err}");
    }

    #[test]
    fn over_cap_is_an_error() {
        // c = 32: 64 chunks cover ~1.8 KB, and the doc is bigger.
        let doc = uniform(64 * 32 + 256);
        let err = prepare(&doc, 0, None, 32).unwrap_err();
        assert!(err.contains("too large"), "unexpected: {err}");
    }

    #[test]
    fn small_limited_window_is_not_over_cap() {
        // A fully scanned window that ends before the doc end is a
        // normal result, not an error.
        let doc = uniform(64 * 32 + 256);
        let chunks = prepare(&doc, 0, Some(32), 32).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(
            chunks.last().unwrap().end < doc.len(),
            "window ends before doc end"
        );
    }

    #[test]
    fn window_filling_exactly_64_chunks_is_not_over_cap() {
        // The spec-fix regression: with 64 chunks emitted, over-cap must
        // mean "window not fully scanned" (last.end < range_end), not
        // "last.end < total" — which this case would falsely trigger.
        // c = 32: 64 chunks advance 63 * (32 - 4) = 1764 bytes past the
        // first chunk's end (32), so a 1796-byte window fits exactly.
        let doc = uniform(2 * 1796);
        let chunks = prepare(&doc, 0, Some(1796), 32).unwrap();
        assert_eq!(chunks.len(), MAX_CHUNKS);
        assert!(chunks.last().unwrap().end < doc.len());
    }

    /// `n` lines of "line NN filler\n" (15 bytes each).
    fn numbered(n: usize) -> String {
        (0..n).map(|i| format!("line {i:02} filler\n")).collect()
    }

    // --- snippet_for ---

    #[test]
    fn snippet_snaps_back_to_last_complete_line() {
        let doc = "aaaaaaaa\n".repeat(21) + "b"; // 190 bytes, trailing partial line
        let s = snippet_for(&doc, 0);
        assert_eq!(s.len(), 189, "21 complete lines, no trailing b");
        assert!(s.ends_with('\n'));
    }

    #[test]
    fn snippet_long_line_is_cut_at_window_end() {
        let doc = "x".repeat(300); // no newline anywhere
        let s = snippet_for(&doc, 0);
        assert_eq!(s.len(), SNIPPET_BYTES);
    }

    #[test]
    fn snippet_at_or_past_end_is_empty() {
        let doc = "hello\nworld\n";
        assert_eq!(snippet_for(&doc, doc.len()), "");
        // Regression for the misplaced guard: the floor must apply to the
        // *argument* — floor_char_boundary panics on idx > len.
        assert_eq!(snippet_for(&doc, doc.len() + 1000), "");
    }

    #[test]
    fn snippet_floors_multi_byte_window_end() {
        let doc = "中".repeat(100); // 300 bytes, 3-byte chars, no newline
        let s = snippet_for(&doc, 0);
        assert_eq!(s.len(), 255, "256 lands mid-character");
        assert_eq!(s.chars().count(), 85);
    }

    #[test]
    fn snippet_floors_mid_char_start() {
        let doc = "ab中c\ntail\n"; // 中 spans bytes 2..5
        let s = snippet_for(&doc, 3);
        assert!(s.starts_with('中'));
    }

    // --- region_hits / region_from ---

    #[test]
    fn valid_region_maps_to_doc_coordinates() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        // chunk 0: bytes 0..75, lines 1..5
        let value = serde_json::json!({
            "regions": [{
                "line_start": 2, "line_end": 3, "score": 9.0, "note": "two filler lines"
            }]
        });
        let hits = region_hits(&chunks[0], 0, &doc, &value, 10);
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert_eq!(h.line_start, 2);
        assert_eq!(h.line_end, 3);
        assert_eq!(h.byte_start, 15);
        assert_eq!(h.byte_end, 45); // end of line 3 = start of line 4
        assert_eq!(h.score, 9.0);
        assert_eq!(h.note, "two filler lines");
        assert!(h.snippet.starts_with("line 01 filler\n"));
        assert!(h.snippet.ends_with("line 17 filler\n"));
        assert!(!h.snippet.contains("line 18"));
    }

    #[test]
    fn second_chunk_lines_are_rebased_with_lines_before() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let c = &chunks[1];
        assert_eq!(c.start, 60); // 5th line (1-based) -> 4 lines before
        let value = serde_json::json!({
            "regions": [{"line_start": 2, "line_end": 2, "score": 4.0, "note": "n"}]
        });
        let hits = region_hits(c, 4, &doc, &value, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line_start, 6); // 4 + 2
        assert_eq!(hits[0].line_end, 6);
        assert_eq!(hits[0].byte_start, 75);
    }

    #[test]
    fn exclusive_zone_boundary() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let c = &chunks[1]; // exclusive_start 75; chunk line 1 (byte 60) is overlap
        assert_eq!(c.exclusive_start, 75);
        let value = serde_json::json!({
            "regions": [
                {"line_start": 1, "line_end": 1, "score": 8.0, "note": "in the overlap"},
                {"line_start": 2, "line_end": 2, "score": 7.0, "note": "at the zone start"}
            ]
        });
        let hits = region_hits(c, 4, &doc, &value, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note, "at the zone start");
    }

    #[test]
    fn region_on_last_chunk_line_ends_at_chunk_end() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let c = &chunks[0]; // 5 lines
        let value = serde_json::json!({
            "regions": [{"line_start": 5, "line_end": 5, "score": 1.0, "note": "last"}]
        });
        let hits = region_hits(c, 0, &doc, &value, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].byte_start, 60);
        assert_eq!(hits[0].byte_end, c.end); // no next line start -> chunk.end
    }

    #[test]
    fn max_regions_truncates_in_doc_order() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let value = serde_json::json!({
            "regions": [
                {"line_start": 1, "line_end": 1, "score": 2.0, "note": "first"},
                {"line_start": 2, "line_end": 2, "score": 9.0, "note": "second"},
                {"line_start": 3, "line_end": 3, "score": 5.0, "note": "third"}
            ]
        });
        let hits = region_hits(&chunks[0], 0, &doc, &value, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].note, "first");
        assert_eq!(hits[1].note, "second");
    }

    #[test]
    fn invalid_regions_are_dropped() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let cases: &[(&str, Value)] = &[
            (
                "zero-based line_start",
                serde_json::json!({"regions": [{"line_start": 0, "line_end": 1, "score": 5, "note": "n"}]}),
            ),
            (
                "line_end past chunk",
                serde_json::json!({"regions": [{"line_start": 4, "line_end": 6, "score": 5, "note": "n"}]}),
            ),
            (
                "line_start after line_end",
                serde_json::json!({"regions": [{"line_start": 3, "line_end": 2, "score": 5, "note": "n"}]}),
            ),
            (
                "score above 10",
                serde_json::json!({"regions": [{"line_start": 1, "line_end": 1, "score": 11, "note": "n"}]}),
            ),
            (
                "negative score",
                serde_json::json!({"regions": [{"line_start": 1, "line_end": 1, "score": -1, "note": "n"}]}),
            ),
            (
                "score as string",
                serde_json::json!({"regions": [{"line_start": 1, "line_end": 1, "score": "high", "note": "n"}]}),
            ),
            (
                "missing note",
                serde_json::json!({"regions": [{"line_start": 1, "line_end": 1, "score": 5}]}),
            ),
            (
                "empty note",
                serde_json::json!({"regions": [{"line_start": 1, "line_end": 1, "score": 5, "note": ""}]}),
            ),
            (
                "regions not an array",
                serde_json::json!({"regions": "none"}),
            ),
            ("missing regions key", serde_json::json!({})),
            ("region not an object", serde_json::json!({"regions": [42]})),
        ];
        for (name, value) in cases {
            let hits = region_hits(&chunks[0], 0, &doc, value, 10);
            assert!(hits.is_empty(), "{name}: expected no hits, got {hits:?}");
        }
    }

    #[test]
    fn valid_region_survives_among_invalid_ones() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let value = serde_json::json!({
            "regions": [
                {"line_start": 99, "line_end": 99, "score": 9.0, "note": "out of range"},
                {"line_start": 2, "line_end": 2, "score": 3.0, "note": "ok"}
            ]
        });
        let hits = region_hits(&chunks[0], 0, &doc, &value, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note, "ok");
    }

    // --- triage (mock LLM) ---

    /// Serves canned chat-completion responses, one per connection.
    /// `chat()` builds a fresh reqwest::Client per call, so each chunk's
    /// call is one TCP connection; the mock loops until the bodies run
    /// out. (Move to a shared test helper when summarize_doc needs it.)
    fn start_mock_llm(bodies: Vec<String>) -> String {
        use std::io::{BufRead, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("failed to bind mock llm");
        let url = format!("http://{}", listener.local_addr().unwrap());

        std::thread::spawn(move || {
            for body in bodies {
                let Ok((mut socket, _)) = listener.accept() else { break };
                let mut reader = std::io::BufReader::new(&socket);
                // Read request headers until the blank line; the small
                // test bodies ride along in the same buffer fills.
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let content = serde_json::json!({
                    "choices": [{ "message": { "content": body } }]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    content.len(),
                    content
                );
                let _ = socket.write_all(response.as_bytes());
            }
        });

        url
    }

    fn mock_config(url: &str) -> llm::LlmConfig {
        llm::LlmConfig {
            base_url: url.to_string(),
            model: "mock".to_string(),
            timeout: std::time::Duration::from_secs(5),
            temperature: 0.0,
            reasoning_effort: None,
        }
    }

    /// numbered(10) with chunk_bytes 64 yields exactly 3 chunks:
    /// [0,75) / [60,135) / [120,150) with lines_before 0 / 4 / 8.
    #[tokio::test]
    async fn triage_loop_renders_pinned_format() {
        let doc = numbered(10); // 150 bytes, 15-byte lines
        let url = start_mock_llm(vec![
            // chunk 0: high + mid regions, one past the chunk end (dropped)
            serde_json::json!({
                "regions": [
                    {"line_start": 4, "line_end": 4, "score": 9, "note": "high score here"},
                    {"line_start": 2, "line_end": 3, "score": 7, "note": "first chunk region"},
                    {"line_start": 9, "line_end": 9, "score": 10, "note": "past the chunk end"}
                ]
            })
            .to_string(),
            // chunk 1: prose -> untriaged, the scan continues
            "I cannot analyze this text.".to_string(),
            // chunk 2: one region in the overlap (dropped), one valid
            serde_json::json!({
                "regions": [
                    {"line_start": 1, "line_end": 1, "score": 10, "note": "in the overlap"},
                    {"line_start": 2, "line_end": 2, "score": 9, "note": "last line region"}
                ]
            })
            .to_string(),
        ]);

        let out = triage(&doc, "what filler is here", None, 0, None, 64, 5, &mock_config(&url))
            .await
            .unwrap();

        let expected = r#"Triage for 'what filler is here': 3 hit(s), scanned 3 chunk(s), bytes 0..150
   untriaged: bytes 60..135 (no JSON object found)
1. [9] high score here
   line 4..4, bytes 45..60
   |line 03 filler
   |line 04 filler
   |line 05 filler
   |line 06 filler
   |line 07 filler
   |line 08 filler
   |line 09 filler
2. [9] last line region
   line 10..10, bytes 135..150
   |line 09 filler
3. [7] first chunk region
   line 2..3, bytes 15..45
   |line 01 filler
   |line 02 filler
   |line 03 filler
   |line 04 filler
   |line 05 filler
   |line 06 filler
   |line 07 filler
   |line 08 filler
   |line 09 filler
"#;
        assert_eq!(out, expected);
    }

    #[tokio::test]
    async fn triage_loop_caps_output_at_max_hits() {
        let doc = numbered(10);
        let url = start_mock_llm(vec![
            serde_json::json!({
                "regions": [
                    {"line_start": 4, "line_end": 4, "score": 9, "note": "high score here"},
                    {"line_start": 2, "line_end": 3, "score": 7, "note": "first chunk region"}
                ]
            })
            .to_string(),
            "I cannot analyze this text.".to_string(),
            serde_json::json!({
                "regions": [
                    {"line_start": 2, "line_end": 2, "score": 9, "note": "last line region"}
                ]
            })
            .to_string(),
        ]);

        let out = triage(&doc, "filler", None, 0, None, 64, 2, &mock_config(&url))
            .await
            .unwrap();

        assert!(out.contains("2 hit(s)"), "unexpected: {out}");
        // Ties keep doc order: chunk 0's 9 ranks before chunk 2's 9.
        assert!(out.contains("1. [9] high score here"), "unexpected: {out}");
        assert!(out.contains("2. [9] last line region"), "unexpected: {out}");
        assert!(
            !out.contains("first chunk region"),
            "7-score hit must be cut at max_hits 2: {out}"
        );
        assert!(out.contains("untriaged: bytes 60..135"), "unexpected: {out}");
    }

    #[test]
    fn system_prompt_pins_relevance_rules() {
        let doc = numbered(3);
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        let (system, _user) = chunk_prompt("what is rust", None, &chunks[0], 5);
        let expected = concat!(
            "Find the parts of this document chunk that are relevant to the query.\n",
            "Respond with a JSON object only: {\"regions\": [{\"line_start\": n, \"line_end\": n, \"score\": s, \"note\": t}]}",
            "\n- line numbers are 1-based within the chunk, line_end >= line_start\n",
            "- lines before line 1 are repeated context from the previous chunk, for orientation only: never report them, and never use them to skip content; report relevant regions in the remaining lines even if they continue a topic visible in the context\n",
            "- first interpret the query: a query about specific things (e.g. \"mentions of X\", \"where does X appear\") asks for mentions; a query about a theme, property, question, or subject (e.g. \"the character of X\", \"views on X\") asks for treatments of it\n",
            "- a mention query: a region qualifies only if it directly mentions the queried things; never report a region that merely touches their general theme\n",
            "- a theme query: a region qualifies if it bears on the queried subject, by describing, discussing, or clearly exemplifying it, even when the passage's main topic is something else; never report a region that merely grazes the subject\n",
            "- breadth: if the query explicitly asks for anything even possibly or tangentially relevant, include peripheral bearings; otherwise include only clearly non-incidental bearings\n",
            "- report at most 5 regions, most relevant first, and never more than the number of qualifying regions you found; padding with weak regions is wrong\n",
            "- if the chunk contains no qualifying region, respond with {\"regions\": []}\n",
            "- score: 1-10, ordinal priority within this document, not calibrated confidence; 1 is still a real, passing mention of the queried subject\n",
            "- note: at most 15 words describing why the region matters; do not paraphrase the text\n",
        );
        assert_eq!(system, expected);
    }
}
