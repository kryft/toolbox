use crate::{chunk, llm, mcp};
use serde_json::Value;

#[derive(serde::Deserialize)]
pub struct SummarizeArgs {
    id: String,
    query: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    max_words: Option<usize>,
    max_map_entries: Option<usize>,
}

const MAX_CHUNKS: usize = 64;

/// Attempts per LLM call (map, story, and map editor). The engine is
/// non-deterministic (DESIGN.md findings): a knife-edge chunk can emit a
/// prose-only response
/// (no JSON at all — observed live on sparse-query runs) or a malformed
/// one on one attempt and a healthy object on the next. Unparseable
/// responses are retried; a chat() transport error is not (the server is
/// down, not the model flaky).
const ATTEMPTS: usize = 3;

/// Minimum words in the prose before the first `{` of an unparseable map
/// response to salvage it as the chunk's summary. Below this it is an
/// apology or garbage (e.g. "I cannot summarize this text."), and the
/// chunk fails instead.
const SALVAGE_MIN_WORDS: usize = 25;

/// Thinking knob for the map editor call only. Sent per call
/// (`llm::chat_reasoning`), so the global `LLAMA_REASONING_EFFORT` env
/// knob neither sets it nor leaks into the decision (and stays out of
/// triage). ON (low) after the noquery4 A/B (replayed fail-dump input,
/// 2026-09-26): thinking OFF, the model either transcribed all 99
/// candidates (truncated at the cap) or — with a quota — filled it by
/// walking document order and stopping at 48% (it cannot keep a
/// per-chunk count over a long generation without thinking); thinking
/// LOW + the per-chunk quota below produced exactly 2 per chunk, 100%
/// coverage, 0 overlaps, 31 s (4361 thinking tokens). If thinking
/// wins, the cap carries an explicit headroom for it (see `summarize`).
const MAP_EDIT_REASONING: Option<&str> = Some("low");

/// Thinking-token headroom added to the map editor's max_tokens when
/// MAP_EDIT_REASONING is on (thinking tokens count against the cap;
/// measured 4361/6237 on the 20-chunk replay arms, content ≈1400–1900;
/// a LIVE noquery5 attempt consumed ~10.8k thinking tokens before its
/// content truncated at 26/40 entries — the 8192 headroom under-sized
/// it ~2×, so 16384 ≈ 1.5× the live worst case; Qwen 3.8 27B "can
/// think quite a lot").
const MAP_EDIT_THINK_HEADROOM: usize = 16384;

const DESCRIPTION: &str = r#"Summarize a doc (stored by fetch_url), optionally focused on a
query. An LLM reads the doc chunk by chunk, so this is slow; use it to
compress a large doc before or instead of scanning it — summarize, then
run triage_doc with the summary as its `context`, or aim a scan window
with the map's byte spans.
A thematic focus works best (e.g. "the life of Jesus"); for
enumerative "find all X" queries use triage_doc instead — the map is a
short importance-ranked shortlist, not an inventory.
Returns a summary (default at most 400 words; max_words, capped at
4000) plus a map of the most important regions with byte spans, in
document order (default: two entries per chunk, at least 10;
max_map_entries).
Cost warning: the scan is sequential (one LLM call per chunk, at most
64 chunks, plus two final editor calls — story, then map); cost grows
with the doc size and max_words. A failed chunk or story call fails the
whole call; a failed map call falls back to a mechanical whole-document
map."#;

// System prompts for the three phases: map (per-chunk writer), story
// (whole-document prose editor), and map editor (whole-document map
// selection). The JSON contracts are mirrored in DESIGN.md
// (summarize_doc); keep the two in sync. (format! needs a literal at the
// call site, so the templates live in functions, not consts.)
fn map_system_prompt(exclusive_line: Option<usize>, max_words: usize) -> String {
    // The overlap note is a per-chunk fact: absent when there are no
    // leading context lines, so the prompt never refers to a previous
    // chunk that does not exist.
    let overlap_note = match exclusive_line {
        Some(line) => format!(
            "- lines before line {line} are repeated context from the previous chunk, for orientation only: never report them as new content, and never use them to skip content; if a logical unit begins in that context and continues in the remaining lines, it is yours to summarize (you are the later part of the seam) — flag it as a continuation, and its pointer may start in the context (line 1); some overlap with the previous chunk's summary is expected, an editor will reconcile it later\n"
        ),
        None => String::new(),
    };
    format!(
        concat!(
            "Summarize this document chunk, focused on the query if one is given.\n",
            "Respond with a JSON object only: {{\"summary\": s, \"pointers\": [{{\"line_start\": n, \"line_end\": n, \"label\": t}}]}}\n",
            "- summary: at most {} words, about this chunk only; with a query, what the chunk says that is relevant to it (a short orienting note on the chunk's general subject is fine); without one, what the chunk is about\n",
            "- pointers: the regions of this chunk worth flagging, for the query if one is given; at most 5, in line order (line_start ascending), and never more than you found; this is a filter, not a ranking — judge each region on whether it is worth flagging, and list them as they appear in the chunk; if you found more than 5, keep only the 5 worth flagging; padding with weak pointers is wrong\n",
            "- label: at most 10 words naming the region; do not paraphrase the text\n",
            "- line numbers are 1-based within the chunk, line_end >= line_start\n",
            // Second positional arg: implicit capture is disabled for
            // format strings expanded from concat!, so the note is a
            // regular argument (as exclusive_line was).
            "{}",
            "- a logical unit cut off at the end of the chunk continues in the next chunk; note in the summary that it continues\n",
            "- an editor assembles the final story and prunes; you see the past (the earlier summaries) but not the future, so include borderline material rather than guessing at global importance; keep terminology consistent with the earlier summaries\n",
            "- if the chunk contains no substantive content, respond with {{\"summary\": \"\", \"pointers\": []}}\n",
            // Shape anchor at token 0 (2026-09-27 A/B, experiments/
            // guardrail_ab.py): the post-hoc "no text before or after"
            // line did not stop the essay preamble / pure-prose mode on
            // the dense NT chunk (1.2–1.4 KB preamble 3/4 runs, prose-
            // only 1/4); this one did (0 bytes 7/8; DESIGN.md Findings).
            "The first character of your response must be {{ and the last character must be }}. The response is the JSON object itself: no text before it, no text after it, no reasoning or commentary anywhere.\n",
        ),
        max_words, overlap_note
    )
}

/// The story editor (reduce phase, first call): the coherent prose.
/// The seam note is story-specific (merging straddling units in the
/// narrative); the map editor gets its own merge line.
fn story_system_prompt(max_words: usize, n_chunks: usize) -> String {
    let mut s = format!(
        concat!(
            "Assemble the final summary of the document from the per-chunk summaries below.\n",
            "Respond with a JSON object only: {{\"summary\": s}}\n",
            "- summary: at most {} words; a coherent story of the whole document from the query's perspective (a neutral overview if no query is given); compose it, do not list the chunks; merge duplicates, keep terminology consistent, prune what does not matter\n",
            "- if a byte range is listed as degraded below, it has no usable summary: write the story from what is given and do not claim content from that range\n",
        ),
        max_words,
    );
    if n_chunks > 1 {
        s.push_str(
            "- adjacent chunk summaries overlap by about one eighth of a chunk, and a unit straddling a seam is summarized by the later chunk (flagged as a continuation): merge such units into one and do not double-count them\n",
        );
    }
    s.push_str(
        "Your output must be exactly one complete, syntactically valid JSON object with no text before or after it. Never write reasoning, deliberation, or commentary anywhere in the output.\n",
    );
    s
}

/// The map editor (reduce phase, second call): the final map's
/// selection, a focused task (selection and merging only, no prose).
/// The split itself is the point: in the combined reduce prompt the map
/// was the secondary job, and on the no-query run the model walked the
/// candidates in document order and stopped at the budget (noquery2:
/// 43% of the doc; noquery3: 60%) while the story covered the whole
/// canon — selection is a comparison workload that needs to be the
/// whole task to be done at all. The per-chunk quota (noquery4 A/B,
/// 2026-09-26) is the spread mechanism: a global quota ("up to 40" or
/// "exactly 40, spread across the document") is still filled by a
/// document-order walk when thinking is off; "about 2 per chunk, 20
/// chunks" gives the model a checkable plan, and MAP_EDIT_REASONING's
/// thinking budget is what lets it keep the per-chunk count.
fn map_edit_system_prompt(map_budget: usize, n_chunks: usize) -> String {
    // The quota clause is the only budget-/chunk-dependent fragment:
    // a per-chunk plan when the budget reaches one entry per chunk
    // (the default budget is 2·N), a whole-document spread clause
    // below that (explicit small max_map_entries).
    let quota = if map_budget >= n_chunks {
        format!(
            "aim for exactly {map_budget} entries — about {} per chunk (the document has {n_chunks} chunks below): from each chunk's candidates keep only the ones that matter most, so every part of the document is represented",
            map_budget / n_chunks
        )
    } else {
        format!(
            "aim for exactly {map_budget} entries (fewer only if there are fewer candidates), spread across the entire document (the document has {n_chunks} chunks below — with fewer entries than chunks, skip the least important chunks; the first and last chunks are as eligible as any other)"
        )
    };
    format!(
        concat!(
            "Build the final map of the document from the per-chunk summaries and pointer candidates below.\n",
            "Respond with a JSON object only: {{\"map\": [{{\"start\": a, \"end\": b, \"label\": t}}]}}\n",
            "- map: the document's most important regions, for the query if one is given (all of them if no query is given), in byte order (start ascending); {}\n",
            "- this is a selection, not a transcription — copying the candidate list is wrong, and the same passage offered by adjacent chunks is one entry, not two\n",
            "- select or merge from the pointer candidates given below only, merging candidates that overlap or describe the same passage — keep their byte spans exactly as given, never invent or adjust a span\n",
            "- if a byte range is listed as degraded below, it has no usable summary and no candidates: never take a span from it\n",
            "Your output must be exactly one complete, syntactically valid JSON object with no text before or after it. Never write reasoning, deliberation, or commentary anywhere in the output.\n",
        ),
        quota
    )
}

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
                "max_words": { "type": "integer", "description": "maximum number of words in the summary (default 400, capped at 4000)"},
                "max_map_entries": { "type": "integer", "description": "maximum number of entries in the map (default: two per chunk, at least 10; capped at 100)"}
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

fn map_call(
    query: Option<&str>,
    chunk: &chunk::Chunk,
    max_words: usize,
    earlier: Option<&str>,
) -> (String, String) {
    let mut user_prompt = String::new();
    if let Some(q) = query {
        user_prompt.push_str(&format!("Query: {q}\n"));
    }
    // S1..S(N-1) verbatim between the query and the chunk: absent for
    // the first chunk, threaded by the map loop in `summarize`.
    if let Some(e) = earlier {
        user_prompt.push_str(&format!("Earlier chunk summaries (verbatim):\n{e}\n"));
    }
    user_prompt.push_str("Chunk:\n");
    let n = chunk.line_starts.len();
    let width = n.to_string().len();
    // add line numbers
    for (i, line) in chunk.text.split_inclusive('\n').enumerate() {
        user_prompt.push_str(&format!("{:width$}\t{line}", i + 1));
    }

    // The overlap note is a per-chunk fact: present only when this
    // chunk has leading context lines (never for the first chunk).
    // It names the first non-overlap line (context_lines + 1).
    let system_prompt = map_system_prompt(
        (chunk.context_lines > 0).then_some(chunk.context_lines + 1),
        max_words,
    );

    (system_prompt, user_prompt)
}

#[derive(Debug, Clone)]
struct Pointer {
    label: String,
    byte_start: usize,
    byte_end: usize, // end of the region's last line
}

/// One chunk's map contribution: the writer summary plus pointers with
/// tool-converted absolute byte spans. `degraded` = parseable but empty
/// ("" summary and no pointers).
#[derive(Debug, Clone)]
struct Contribution {
    span: (usize, usize), // chunk byte span, doc absolute
    summary: String,
    pointers: Vec<Pointer>,
    degraded: bool,
}

#[derive(Debug)]
struct MapEntry {
    start: usize,
    end: usize,
    label: String,
}

/// Line span -> byte span via `line_starts` (one lookup per line; no
/// running line counter — the output carries byte spans only). Unlike
/// triage's `region_from`, a pointer may start in the leading overlap:
/// validation clamps to the whole chunk text, not the exclusive zone
/// (the editor reconciles the expected overlap).
fn pointer_from(chunk: &chunk::Chunk, entry: &Value) -> Option<Pointer> {
    let ls = entry.get("line_start").and_then(Value::as_u64)?;
    let le = entry.get("line_end").and_then(Value::as_u64)?;
    let label = entry.get("label").and_then(Value::as_str)?;

    if ls < 1 || le > chunk.line_starts.len() as u64 || ls > le {
        return None;
    }
    if label.is_empty() {
        return None;
    }

    let byte_start = chunk.line_starts[(ls - 1) as usize];
    let byte_end = chunk
        .line_starts
        .get(le as usize)
        .copied()
        .unwrap_or(chunk.end);

    Some(Pointer {
        label: label.to_string(),
        byte_start,
        byte_end,
    })
}

/// Lenient parse of one map response: off-contract fields become empty,
/// invalid pointers are dropped, at most 5 kept (doc order).
fn contribution_from(chunk: &chunk::Chunk, value: &Value) -> Contribution {
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let pointers: Vec<Pointer> = value
        .get("pointers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|e| pointer_from(chunk, e))
                .take(5)
                .collect()
        })
        .unwrap_or_default();

    let degraded = summary.is_empty() && pointers.is_empty();

    Contribution {
        span: (chunk.start, chunk.end),
        summary,
        pointers,
        degraded,
    }
}

/// One entry for the S-block of a later map call: the chunk's span for
/// orientation, then its summary verbatim (fixed once generated, never
/// re-compressed). Pointers are left out — the writer needs earlier
/// terminology and orientation, not earlier spans (contrast the reduce
/// input, which carries them).
fn earlier_entry(c: &Contribution) -> String {
    format!("[bytes {}..{}] {}\n", c.span.0, c.span.1, c.summary)
}

/// Lenient parse of one reduce map entry. Spans are validated in range
/// only: the editor may merge candidates, so there is no exact-match
/// check against the given pointers (contrast the map phase).
fn map_entry_from(value: &Value, total: usize) -> Option<MapEntry> {
    let start = value.get("start").and_then(Value::as_u64)?;
    let end = value.get("end").and_then(Value::as_u64)?;
    let label = value.get("label").and_then(Value::as_str)?;

    if start >= end || end as usize > total || label.is_empty() {
        return None;
    }

    Some(MapEntry {
        start: start as usize,
        end: end as usize,
        label: label.to_string(),
    })
}

/// The shared editor input: the per-chunk blocks (span, summary,
/// pointers) in document order plus the degraded list. The story and
/// map editors see the same candidates; the map editor is NOT shown the
/// story (no anchoring — the map maps the document, not the prose).
fn contributions_block(contributions: &[Contribution]) -> String {
    let mut user =
        String::from("Chunk summaries (one block per chunk, in document order):\n");
    for c in contributions {
        user.push_str(&format!("[bytes {}..{}]\n", c.span.0, c.span.1));
        if c.degraded {
            continue;
        }
        user.push_str(&format!("Summary: {}\n", c.summary));
        user.push_str("Pointers:\n");
        if c.pointers.is_empty() {
            user.push_str("   (none)\n");
        }
        for (i, p) in c.pointers.iter().enumerate() {
            user.push_str(&format!(
                "   {}. [bytes {}..{}] {}\n",
                i + 1,
                p.byte_start,
                p.byte_end,
                p.label
            ));
        }
    }
    let degraded: Vec<&Contribution> = contributions.iter().filter(|c| c.degraded).collect();
    if !degraded.is_empty() {
        let spans: Vec<String> = degraded
            .iter()
            .map(|c| format!("bytes {}..{}", c.span.0, c.span.1))
            .collect();
        user.push_str(&format!(
            "Degraded chunks (no usable summary): {}\n",
            spans.join(", ")
        ));
    }
    user
}

fn story_call(
    contributions: &[Contribution],
    query: Option<&str>,
    max_words: usize,
) -> (String, String) {
    let system = story_system_prompt(max_words, contributions.len());
    let mut user = String::new();
    if let Some(q) = query {
        user.push_str(&format!("Query: {q}\n"));
    }
    user.push_str(&contributions_block(contributions));
    (system, user)
}

fn map_edit_call(
    contributions: &[Contribution],
    query: Option<&str>,
    map_budget: usize,
) -> (String, String) {
    let system = map_edit_system_prompt(map_budget, contributions.len());
    let mut user = String::new();
    if let Some(q) = query {
        user.push_str(&format!("Query: {q}\n"));
    }
    user.push_str(&contributions_block(contributions));
    (system, user)
}

/// Pinned render: header, optional `Query:` label + verbatim query
/// line(s), `SUMMARY`, `MAP` (3-space indent); no blank lines.
fn render(query: Option<&str>, chunks: &[chunk::Chunk], summary: &str, map: &[MapEntry]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Summary of bytes {}..{}, {} chunk(s)\n",
        chunks[0].start,
        chunks.last().unwrap().end,
        chunks.len()
    ));
    if let Some(q) = query {
        out.push_str("Query:\n");
        out.push_str(q);
        out.push('\n');
    }
    out.push_str("SUMMARY\n");
    out.push_str(summary);
    out.push('\n');
    out.push_str("MAP\n");
    for (i, e) in map.iter().enumerate() {
        out.push_str(&format!(
            "   {}. [bytes {}..{}] {}\n",
            i + 1,
            e.start,
            e.end,
            e.label
        ));
    }
    out
}

/// The prose before the first `{` of an unparseable LLM response (the
/// whole text when there is no `{`), trimmed. `None` when it has fewer
/// than `SALVAGE_MIN_WORDS` words: an apology or garbage, not a summary
/// — the chunk fails instead of salvaging it.
fn prose_salvage_candidate(raw: &str) -> Option<String> {
    let prose = raw.split('{').next()?.trim();
    (prose.split_whitespace().count() >= SALVAGE_MIN_WORDS).then(|| prose.to_string())
}

/// Default final-map size: two entries per chunk (a document map at
/// per-chunk granularity), floored at 10 (the old fixed cap — small
/// runs keep the familiar 10). The per-chunk writer still offers at
/// most 5 pointers each (that is part of its token budget); a larger
/// budget just gives the editor more candidates to select or merge
/// from. `summarize` clamps the final value to 1..=100 (N = 64 gives
/// 128 → 100).
fn default_map_budget(n: usize) -> usize {
    (n * 2).max(10)
}

/// The map editor's failure path: a mechanical whole-document map — the
/// first pointer of each non-degraded chunk (a chunk without pointers
/// contributes nothing), in document order, budget-clamped. Deterministic
/// and byte-ordered by construction (each pointer's span lies inside
/// its own chunk, and the chunks are in order); the sort at the call
/// site keeps one code path. The LLM map is a quality layer over this
/// floor, not the only way to get a map.
fn fallback_map(contributions: &[Contribution], map_budget: usize) -> Vec<MapEntry> {
    let mut entries: Vec<MapEntry> = contributions
        .iter()
        .filter_map(|c| c.pointers.first())
        .take(map_budget)
        .map(|p| MapEntry {
            start: p.byte_start,
            end: p.byte_end,
            label: p.label.clone(),
        })
        .collect();
    entries.sort_by_key(|e| e.start);
    entries
}

/// Dev aid for the LLM calls: dump the full exchange (system prompt,
/// user prompt — with the S-block for map calls, the contributions
/// block for the editors — and the raw model response) to /tmp.
/// Failure dumps (phase `fail_...`) make a failure diagnosable without
/// re-running the chunk; success dumps (phase `dump_...`) capture the
/// reduce input for prompt A/B replay without a live run. Best-effort:
/// a failed write never masks the real error. Returns the path (used in
/// error messages). `phase` names the file
/// (`/tmp/summarize_{phase}.txt`); `span_desc` is the human-readable
/// line.
fn dump_call(phase: &str, span_desc: &str, system: &str, user: &str, raw: &str) -> String {
    let path = format!("/tmp/summarize_{phase}.txt");
    let _ = std::fs::write(
        &path,
        format!(
            "=== {phase} — {span_desc} ===\n=== system prompt ===\n{system}\n=== user prompt ({} bytes) ===\n{user}\n=== raw response ({} bytes) ===\n{raw}\n",
            user.len(),
            raw.len()
        ),
    );
    path
}

pub async fn summarize(
    doc: &str,
    query: Option<&str>,
    offset: usize,
    limit: Option<usize>,
    chunk_bytes: usize,
    max_words: usize,
    max_map_entries: Option<usize>,
    cfg: &llm::LlmConfig,
) -> Result<String, String> {
    let chunks = prepare(doc, offset, limit, chunk_bytes)?;
    let max_words = max_words.clamp(1, 4000);
    let n = chunks.len();
    // Final-map budget: the arg wins (clamped 1..=100), else the default
    // (two entries per chunk, floor 10).
    let map_budget = max_map_entries.unwrap_or_else(|| default_map_budget(n)).clamp(1, 100);
    // Per-chunk writer budget (rule A): the reduce always gets >=2x the
    // story budget of material. Distinct from the story-budget
    // `max_words`; everything map-phase reads from `p`.
    let p = (max_words * 2).div_ceil(n).max(250);

    // Map phase: one line-numbered chat() per chunk. A chat() error fails
    // the whole call immediately (the server, not the model, is the
    // problem); an unparseable response is retried (ATTEMPTS total) and
    // then fails the whole call (naming the chunk span); a
    // parseable-but-off-contract response becomes a degraded
    // contribution for the reduce.
    let mut contributions: Vec<Contribution> = Vec::new();
    // The S-block threaded into each map call: the earlier chunks'
    // entries in document order, empty (-> None) for the first.
    let mut earlier = String::new();
    for c in &chunks {
        let earlier_arg = (!earlier.is_empty()).then_some(earlier.as_str());
        let (system, user) = map_call(query, c, p, earlier_arg);
        // 4× the word budget + 150: the model pretty-prints its JSON
        // (newlines/indents are extra tokens) and over-produces pointers
        // despite the "at most 5" contract — the first KJV run truncated
        // two verbose chunks at 2× (EOF mid-pointer-array); the measured
        // worst chunk (197-word summary + 11 pointers = 716 tokens at
        // p = 250) fits 4× (1150) with 61% headroom. The cap costs
        // ~nothing: it only bounds decode (paid only when written), the
        // timeout already scales with it, and the DESIGN.md context fit
        // (prompt + max_tokens) keeps its margin.
        let max_tokens = (4 * p + 150) as u32;
        // Unparseable is retried (ATTEMPTS total): the engine is
        // non-deterministic, so a knife-edge chunk may emit a prose-only
        // response (no JSON at all) once and a healthy object next time.
        // Across the attempts, keep the best prose salvage for the case
        // where every attempt is unparseable.
        let mut value: Option<Value> = None;
        let mut last_reason = String::new();
        let mut salvage: Option<String> = None;
        for attempt in 1..=ATTEMPTS {
            let text = llm::chat(cfg, &system, &user, max_tokens)
                .await
                .map_err(|reason| {
                    format!("summarize failed (bytes {}..{}): {reason}", c.start, c.end)
                })?;
            match llm::extract_json(&text) {
                Ok(v) => {
                    value = Some(v);
                    break;
                }
                Err(reason) => {
                    last_reason = reason;
                    let cand = prose_salvage_candidate(&text);
                    if cand.as_ref().map_or(false, |s| {
                        s.split_whitespace().count()
                            > salvage
                                .as_ref()
                                .map_or(0, |s| s.split_whitespace().count())
                    }) {
                        salvage = cand;
                    }
                    let _ = dump_call(
                        &format!("fail_map_{}..{}_attempt{attempt}", c.start, c.end),
                        &format!(
                            "chunk bytes {}..{} (context_lines {}, attempt {attempt}/{ATTEMPTS})",
                            c.start, c.end, c.context_lines
                        ),
                        &system,
                        &user,
                        &text,
                    );
                }
            }
        }
        // A parsed object wins; else the best prose salvage; else the
        // chunk fails (naming the span + the file of the last attempt).
        let contrib = match (value, salvage) {
            (Some(v), _) => contribution_from(c, &v),
            // Every attempt unparseable but substantive prose survived:
            // the prose-only mode (dense thematic chunk + essay-framed
            // query) is a STABLE attractor that retry cannot break, and
            // the prose is a good summary missing only the JSON — use it
            // as the chunk's summary with no pointers. Not flagged
            // degraded: the editor composes from it like any summary;
            // only the map loses this chunk's candidates.
            (None, Some(prose)) => Contribution {
                span: (c.start, c.end),
                summary: prose,
                pointers: Vec::new(),
                degraded: false,
            },
            (None, None) => {
                let path =
                    format!("/tmp/summarize_fail_map_{}..{}_attempt{ATTEMPTS}.txt", c.start, c.end);
                return Err(format!(
                    "summarize failed (bytes {}..{}): {last_reason} \
                     after {ATTEMPTS} attempts (raw exchange saved to {path})",
                    c.start, c.end
                ));
            }
        };
        if !contrib.degraded {
            earlier.push_str(&earlier_entry(&contrib));
        }
        contributions.push(contrib);
    }

    // All contributions empty -> no editor call on nothing.
    if contributions.iter().all(|c| c.degraded) {
        let spans: Vec<String> = contributions
            .iter()
            .map(|c| format!("bytes {}..{}", c.span.0, c.span.1))
            .collect();
        return Err(format!(
            "summarize failed: every chunk returned an empty or off-contract summary ({})",
            spans.join(", ")
        ));
    }

    // Reduce phase: two focused calls — the story editor (the coherent
    // prose) and the map editor (the final map selection). Story first:
    // a failed story never burns a map call, and the story's prose-only
    // responses are its intended output, so the salvage path applies to
    // it (a prose-only essay IS the story — in the map phase salvage
    // patches that mode; here it is the point). The map editor has no
    // salvage (its prose is garbage): on failure the tool falls back to
    // the mechanical whole-document map (first pointer per chunk), so a
    // map-editor attractor costs nothing — the run still returns a story
    // + a navigable map. Same retry semantics as the map phase.
    let (system, user) = story_call(&contributions, query, max_words);
    // 4× the story budget + 150, the map phase's rationale (the model
    // pretty-prints its JSON): 1750 at the 400-word default.
    let max_tokens = (4 * max_words + 150) as u32;
    let mut value: Option<Value> = None;
    let mut last_reason = String::new();
    let mut salvage: Option<String> = None;
    for attempt in 1..=ATTEMPTS {
        let text = llm::chat(cfg, &system, &user, max_tokens)
            .await
            .map_err(|reason| format!("summarize failed (story): {reason}"))?;
        match llm::extract_json(&text) {
            Ok(v) => {
                value = Some(v);
                // Success dump: the reduce input, replayable through
                // prompt A/B without a live run.
                let _ =
                    dump_call("dump_story", "story (whole document)", &system, &user, &text);
                break;
            }
            Err(reason) => {
                last_reason = reason;
                // Keep the best prose salvage for the case where every
                // attempt is unparseable (the prose-only attractor).
                let cand = prose_salvage_candidate(&text);
                if cand.as_ref().map_or(false, |s| {
                    s.split_whitespace().count()
                        > salvage
                            .as_ref()
                            .map_or(0, |s| s.split_whitespace().count())
                }) {
                    salvage = cand;
                }
                let _ = dump_call(
                    &format!("fail_story_attempt{attempt}"),
                    &format!("story (whole document, attempt {attempt}/{ATTEMPTS})"),
                    &system,
                    &user,
                    &text,
                );
            }
        }
    }
    // A parsed object wins; else the best prose salvage; else the call
    // fails (the story is the call's reason to exist — unlike the map,
    // there is no fallback for it).
    let summary = match (value, salvage) {
        (Some(v), _) => v
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        (None, Some(prose)) => prose,
        (None, None) => {
            let path = format!("/tmp/summarize_fail_story_attempt{ATTEMPTS}.txt");
            return Err(format!(
                "summarize failed (story): {last_reason} \
                 after {ATTEMPTS} attempts (raw exchange saved to {path})"
            ));
        }
    };
    if summary.is_empty() {
        return Err(String::from(
            "summarize failed (story): model returned no summary",
        ));
    }

    // The map editor: selection only. Its input is the same
    // contributions block the story saw — not the story itself (no
    // anchoring).
    let (system, user) = map_edit_call(&contributions, query, map_budget);
    // 150 + 80 per entry + the thinking headroom (when the map editor
    // thinks): measured ~48 tok/entry pretty-printed on the noquery4
    // replay — the 20/entry compact coefficient under-sized the live
    // run (all three map-editor attempts truncated at 950, and the
    // mechanical fallback carried the run, as designed). 80/entry is
    // the 4× house convention over 20 and leaves ~20% headroom over the
    // measured 40-entry output (~2000 tok). With MAP_EDIT_REASONING on
    // the thinking tokens count against the cap too — the headroom
    // covers them (16,384 ≈ 1.5× the live worst case, ~10.8k on a
    // noquery5 attempt; the replay-measured 4.4/6.2k under-sized it).
    // The cap costs ~nothing: it only bounds decode (paid only when
    // written); the timeout scales +10 ms/token with it.
    let max_tokens =
        (150 + 80 * map_budget + MAP_EDIT_REASONING.map_or(0, |_| MAP_EDIT_THINK_HEADROOM)) as u32;
    let mut entries: Option<Vec<MapEntry>> = None;
    for attempt in 1..=ATTEMPTS {
        let text = llm::chat_reasoning(cfg, MAP_EDIT_REASONING, &system, &user, max_tokens)
            .await
            .map_err(|reason| format!("summarize failed (map): {reason}"))?;
        // Off-contract (parseable but no map array) is a failed attempt
        // too: the response did not do the call's one job.
        let parsed: Result<Vec<MapEntry>, String> =
            llm::extract_json(&text).and_then(|v| {
                v.get("map")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|e| map_entry_from(e, doc.len()))
                            .take(map_budget)
                            .collect()
                    })
                    .ok_or_else(|| String::from("parseable response has no map array"))
            });
        match parsed {
            Ok(es) => {
                entries = Some(es);
                let _ = dump_call(
                    "dump_mapedit",
                    "map editor (whole document)",
                    &system,
                    &user,
                    &text,
                );
                break;
            }
            Err(_reason) => {
                // The reason is in the dump (the raw response); the
                // fallback below makes it moot for the call itself.
                let _ = dump_call(
                    &format!("fail_mapedit_attempt{attempt}"),
                    &format!("map editor (whole document, attempt {attempt}/{ATTEMPTS})"),
                    &system,
                    &user,
                    &text,
                );
            }
        }
    }
    // A parseable, on-contract response wins; else — unparseable or
    // off-contract on all attempts — the mechanical map. The failure
    // dumps of the attempts (/tmp/summarize_fail_mapedit_attempt*.txt)
    // are the record; the run still returns a story + a navigable map.
    let mut map: Vec<MapEntry> = match entries {
        Some(es) => es,
        None => fallback_map(&contributions, map_budget),
    };
    // Byte order is a tool guarantee, not a model promise: stable-sort by
    // start (the noquery2 run had local inversions despite "in byte
    // order" — a sort is a comparison workload the model can't be
    // trusted with thinking off). Duplicates end up adjacent too. The
    // fallback is ordered by construction; the sort keeps one code path.
    map.sort_by_key(|e| e.start);

    Ok(render(query, &chunks, &summary, &map))
}

pub async fn handle_call(args: Value) -> Result<Value, mcp::JsonRpcErrorResponse> {
    let parsed_args: SummarizeArgs =
        serde_json::from_value(args).map_err(|_| mcp::invalid_params("bad params"))?;

    let query = parsed_args.query.as_deref().filter(|q| !q.is_empty());

    let raw = match crate::store::load(&parsed_args.id) {
        Ok(b) => b,
        Err(err) => return Ok(mcp::error_message_json(&err)),
    };

    let text = String::from_utf8_lossy(&raw);

    let mut cfg = llm::LlmConfig::default();
    cfg.temperature = 0.2;

    match summarize(
        &text,
        query,
        parsed_args.offset.unwrap_or(0),
        parsed_args.limit,
        chunk::default_chunk_bytes(),
        parsed_args.max_words.unwrap_or(400),
        parsed_args.max_map_entries,
        &cfg,
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

    /// `n` lines of "line NN filler\n" (15 bytes each).
    fn numbered(n: usize) -> String {
        (0..n).map(|i| format!("line {i:02} filler\n")).collect()
    }

    // --- prompt contracts (pinned by test, as in triage) ---

    #[test]
    fn map_prompt_pins_contract() {
        // The overlap note is the only line that differs between the
        // first chunk (no leading context lines) and later ones, so
        // both variants are pinned from shared head/tail pieces.
        // They are spliced with named capture, so the constants are
        // data: their literal braces need no escaping here (contrast
        // the format! literal in map_system_prompt itself).
        let head = concat!(
            "Summarize this document chunk, focused on the query if one is given.\n",
            "Respond with a JSON object only: {\"summary\": s, \"pointers\": [{\"line_start\": n, \"line_end\": n, \"label\": t}]}\n",
            "- summary: at most 250 words, about this chunk only; with a query, what the chunk says that is relevant to it (a short orienting note on the chunk's general subject is fine); without one, what the chunk is about\n",
            "- pointers: the regions of this chunk worth flagging, for the query if one is given; at most 5, in line order (line_start ascending), and never more than you found; this is a filter, not a ranking — judge each region on whether it is worth flagging, and list them as they appear in the chunk; if you found more than 5, keep only the 5 worth flagging; padding with weak pointers is wrong\n",
            "- label: at most 10 words naming the region; do not paraphrase the text\n",
            "- line numbers are 1-based within the chunk, line_end >= line_start\n",
        );
        let note = "- lines before line 1 are repeated context from the previous chunk, for orientation only: never report them as new content, and never use them to skip content; if a logical unit begins in that context and continues in the remaining lines, it is yours to summarize (you are the later part of the seam) — flag it as a continuation, and its pointer may start in the context (line 1); some overlap with the previous chunk's summary is expected, an editor will reconcile it later\n";
        let tail = concat!(
            "- a logical unit cut off at the end of the chunk continues in the next chunk; note in the summary that it continues\n",
            "- an editor assembles the final story and prunes; you see the past (the earlier summaries) but not the future, so include borderline material rather than guessing at global importance; keep terminology consistent with the earlier summaries\n",
            "- if the chunk contains no substantive content, respond with {\"summary\": \"\", \"pointers\": []}\n",
            // The shape anchor (2026-09-27 A/B): token-0 constraint that
            // stopped the essay preamble / fence where the old
            // "exactly one JSON object" line did not (DESIGN.md Findings).
            "The first character of your response must be { and the last character must be }. The response is the JSON object itself: no text before it, no text after it, no reasoning or commentary anywhere.\n",
        );
        assert_eq!(
            map_system_prompt(Some(1), 250),
            format!("{head}{note}{tail}")
        );
        assert_eq!(map_system_prompt(None, 250), format!("{head}{tail}"));
    }

    #[test]
    fn story_prompt_pins_contract() {
        let system = story_system_prompt(400, 1);
        let expected = concat!(
            "Assemble the final summary of the document from the per-chunk summaries below.\n",
            "Respond with a JSON object only: {\"summary\": s}\n",
            "- summary: at most 400 words; a coherent story of the whole document from the query's perspective (a neutral overview if no query is given); compose it, do not list the chunks; merge duplicates, keep terminology consistent, prune what does not matter\n",
            "- if a byte range is listed as degraded below, it has no usable summary: write the story from what is given and do not claim content from that range\n",
            "Your output must be exactly one complete, syntactically valid JSON object with no text before or after it. Never write reasoning, deliberation, or commentary anywhere in the output.\n",
        );
        assert_eq!(system, expected);
        // The seam note is the only N-dependent fragment: absent at N == 1.
        assert!(!system.contains("one eighth"));
        assert!(story_system_prompt(400, 2).contains("overlap by about one eighth"));
    }

    #[test]
    fn map_edit_prompt_pins_contract() {
        // The quota clause is the only budget-/chunk-dependent fragment:
        // the per-chunk plan (budget >= n) and the whole-document spread
        // clause (budget < n) are pinned from shared head/tail pieces,
        // as the map prompt's overlap note is.
        let head = concat!(
            "Build the final map of the document from the per-chunk summaries and pointer candidates below.\n",
            "Respond with a JSON object only: {\"map\": [{\"start\": a, \"end\": b, \"label\": t}]}\n",
            "- map: the document's most important regions, for the query if one is given (all of them if no query is given), in byte order (start ascending); ",
        );
        let tail = concat!(
            "\n- this is a selection, not a transcription — copying the candidate list is wrong, and the same passage offered by adjacent chunks is one entry, not two\n",
            "- select or merge from the pointer candidates given below only, merging candidates that overlap or describe the same passage — keep their byte spans exactly as given, never invent or adjust a span\n",
            "- if a byte range is listed as degraded below, it has no usable summary and no candidates: never take a span from it\n",
            "Your output must be exactly one complete, syntactically valid JSON object with no text before or after it. Never write reasoning, deliberation, or commentary anywhere in the output.\n",
        );
        assert_eq!(
            map_edit_system_prompt(40, 20),
            format!(
                "{head}aim for exactly 40 entries — about 2 per chunk (the document has 20 chunks below): from each chunk's candidates keep only the ones that matter most, so every part of the document is represented{tail}"
            )
        );
        assert_eq!(
            map_edit_system_prompt(10, 20),
            format!(
                "{head}aim for exactly 10 entries (fewer only if there are fewer candidates), spread across the entire document (the document has 20 chunks below — with fewer entries than chunks, skip the least important chunks; the first and last chunks are as eligible as any other){tail}"
            )
        );
    }

    // --- budgets / salvage ---

    #[test]
    fn default_map_budget_is_twice_the_chunks_floored_at_ten() {
        assert_eq!(default_map_budget(1), 10);
        assert_eq!(default_map_budget(5), 10);
        assert_eq!(default_map_budget(10), 20);
        assert_eq!(default_map_budget(20), 40);
        // 128 > the 100 cap; summarize() clamps the final value.
        assert_eq!(default_map_budget(64), 128);
    }

    #[test]
    fn prose_salvage_candidate_takes_prose_before_the_first_brace() {
        // Pure prose response (no braces at all): the whole text.
        let prose = (0..30).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        assert_eq!(prose_salvage_candidate(&prose).as_deref(), Some(prose.as_str()));
        // Preamble + broken object: only the preamble.
        let broken = format!("{prose} {{\"summary\": \"truncated\"");
        assert_eq!(prose_salvage_candidate(&broken).as_deref(), Some(prose.as_str()));
        // No prose before the brace: nothing to salvage.
        assert_eq!(
            prose_salvage_candidate("{\"summary\": \"truncated\""),
            None
        );
        // Too short to be a summary (an apology): nothing to salvage.
        assert_eq!(
            prose_salvage_candidate("I cannot summarize this text."),
            None
        );
        // The boundary: exactly SALVAGE_MIN_WORDS is salvaged.
        let at = (0..SALVAGE_MIN_WORDS).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        assert_eq!(prose_salvage_candidate(&at).as_deref(), Some(at.as_str()));
    }

    // --- map_call ---

    #[test]
    fn map_call_pins_user_message() {
        let doc = numbered(4); // 60 bytes -> 1 chunk [0, 60)
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        let (system, user) = map_call(Some("what filler is here"), &chunks[0], 800, None);
        assert!(system.contains("at most 800 words"));
        // Single-chunk doc -> context_lines == 0: no overlap note.
        assert!(!system.contains("repeated context"));
        assert_eq!(
            user,
            "Query: what filler is here\n\
Chunk:\n\
1\tline 00 filler\n\
2\tline 01 filler\n\
3\tline 02 filler\n\
4\tline 03 filler\n"
        );
        let (_, user_noq) = map_call(None, &chunks[0], 800, None);
        assert_eq!(
            user_noq,
            "Chunk:\n\
1\tline 00 filler\n\
2\tline 01 filler\n\
3\tline 02 filler\n\
4\tline 03 filler\n"
        );
    }

    #[test]
    fn map_call_places_earlier_summaries_between_query_and_chunk() {
        let doc = numbered(4);
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        let (_, user) = map_call(
            Some("filler"),
            &chunks[0],
            250,
            Some("S1: prior chunk text"),
        );
        let q = user.find("Query: filler\n").unwrap();
        let e = user
            .find("Earlier chunk summaries (verbatim):\nS1: prior chunk text\n")
            .unwrap();
        let c = user.find("Chunk:\n").unwrap();
        assert!(
            q < e && e < c,
            "order: query, earlier summaries, chunk:\n{user}"
        );
    }

    #[test]
    fn map_call_omits_the_overlap_note_only_for_the_first_chunk() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        assert_eq!(chunks[0].context_lines, 0);
        assert_eq!(chunks[1].context_lines, 1);
        let (first, _) = map_call(None, &chunks[0], 250, None);
        let (later, _) = map_call(None, &chunks[1], 250, None);
        assert!(!first.contains("repeated context"));
        // The note names the first non-overlap line (context_lines + 1).
        assert!(later.contains("lines before line 2 are repeated context"));
    }

    // --- pointer_from / contribution_from ---

    #[test]
    fn pointer_converts_line_spans_to_byte_spans() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let c = &chunks[0]; // bytes 0..75, 5 lines, line_starts [0, 15, 30, 45, 60]
        let value = serde_json::json!({"pointers": [
            {"line_start": 1, "line_end": 1, "label": "first"},
            {"line_start": 2, "line_end": 3, "label": "mid"},
            {"line_start": 5, "line_end": 5, "label": "last"}
        ]});
        let contrib = contribution_from(c, &value);
        assert!(!contrib.degraded);
        assert_eq!(
            (contrib.pointers[0].byte_start, contrib.pointers[0].byte_end),
            (0, 15)
        );
        assert_eq!(
            (contrib.pointers[1].byte_start, contrib.pointers[1].byte_end),
            (15, 45)
        );
        assert_eq!(
            (contrib.pointers[2].byte_start, contrib.pointers[2].byte_end),
            (60, 75)
        );
        // last line has no next line start -> ends at chunk.end
    }

    #[test]
    fn pointer_may_start_in_the_overlap_contrast_triage() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let c = &chunks[1]; // start 60, exclusive_start 75; line 1 (byte 60) is overlap
        assert_eq!(c.exclusive_start, 75);
        let value = serde_json::json!({"pointers": [
            {"line_start": 1, "line_end": 1, "label": "in the overlap"},
            {"line_start": 2, "line_end": 2, "label": "at the zone start"}
        ]});
        let contrib = contribution_from(c, &value);
        // Triage hard-drops the overlap start; summarize keeps it (the
        // editor reconciles the expected overlap).
        assert_eq!(contrib.pointers.len(), 2, "overlap pointer must survive");
        assert_eq!(contrib.pointers[0].byte_start, 60);
        assert_eq!(contrib.pointers[0].byte_end, 75);
    }

    #[test]
    fn invalid_pointers_are_dropped_and_extras_cut_at_five() {
        let doc = numbered(22);
        let chunks = chunk::split(&doc, 0, None, 64, 64);
        let c = &chunks[0]; // 5 lines
        let cases: &[(&str, Value)] = &[
            (
                "zero-based line_start",
                serde_json::json!({"line_start": 0, "line_end": 1, "label": "n"}),
            ),
            (
                "line_end past chunk",
                serde_json::json!({"line_start": 4, "line_end": 6, "label": "n"}),
            ),
            (
                "line_start after line_end",
                serde_json::json!({"line_start": 3, "line_end": 2, "label": "n"}),
            ),
            (
                "missing label",
                serde_json::json!({"line_start": 1, "line_end": 1}),
            ),
            (
                "empty label",
                serde_json::json!({"line_start": 1, "line_end": 1, "label": ""}),
            ),
            (
                "label as number",
                serde_json::json!({"line_start": 1, "line_end": 1, "label": 5}),
            ),
            ("pointer not an object", serde_json::json!(42)),
        ];
        for (name, entry) in cases {
            let value = serde_json::json!({"summary": "s", "pointers": [entry.clone()]});
            let contrib = contribution_from(c, &value);
            assert!(contrib.pointers.is_empty(), "{name}");
            assert!(
                !contrib.degraded,
                "{name}: a summary still counts as a contribution"
            );
        }

        for (name, value) in &[
            (
                "pointers not an array",
                serde_json::json!({"summary": "s", "pointers": "none"}),
            ),
            ("missing pointers key", serde_json::json!({"summary": "s"})),
        ] {
            let contrib = contribution_from(c, value);
            assert!(contrib.pointers.is_empty(), "{name}");
        }

        // Six valid pointers -> cut at 5, in doc order.
        let ptr = |ls: u64| serde_json::json!({"line_start": ls, "line_end": ls, "label": format!("line {ls}")});
        let value = serde_json::json!({"summary": "s", "pointers": [ptr(1), ptr(2), ptr(3), ptr(4), ptr(5), ptr(2)]});
        let contrib = contribution_from(c, &value);
        assert_eq!(contrib.pointers.len(), 5);
        assert_eq!(
            contrib
                .pointers
                .iter()
                .map(|p| p.byte_start)
                .collect::<Vec<_>>(),
            vec![0, 15, 30, 45, 60]
        );
    }

    #[test]
    fn off_contract_json_is_a_degraded_contribution() {
        let doc = numbered(4);
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        let value = serde_json::json!({"regions": []}); // parseable, off-contract
        let contrib = contribution_from(&chunks[0], &value);
        assert!(contrib.degraded);
        assert!(contrib.summary.is_empty());
        assert!(contrib.pointers.is_empty());
        assert_eq!(contrib.span, (0, 60));
    }

    #[test]
    fn summary_without_pointers_is_not_degraded() {
        let doc = numbered(4);
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        let value = serde_json::json!({"summary": "just words"});
        let contrib = contribution_from(&chunks[0], &value);
        assert!(!contrib.degraded);
        assert!(contrib.pointers.is_empty());
    }

    #[test]
    fn earlier_entry_pins_the_s_block_line() {
        let doc = numbered(4);
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        // A multi-line summary stays verbatim (no re-compression, no
        // newline mangling); the entry ends with its own newline.
        let value = serde_json::json!({"summary": "line one\nline two", "pointers": []});
        let contrib = contribution_from(&chunks[0], &value);
        assert_eq!(
            earlier_entry(&contrib),
            "[bytes 0..60] line one\nline two\n"
        );
    }

    // --- map_entry_from ---

    #[test]
    fn invalid_map_entries_are_dropped_and_extras_cut_at_ten() {
        let total = 60;
        let cases: &[(&str, Value)] = &[
            (
                "start equals end",
                serde_json::json!({"start": 30, "end": 30, "label": "n"}),
            ),
            (
                "start after end",
                serde_json::json!({"start": 40, "end": 30, "label": "n"}),
            ),
            (
                "end past doc",
                serde_json::json!({"start": 0, "end": 61, "label": "n"}),
            ),
            ("missing label", serde_json::json!({"start": 0, "end": 30})),
            (
                "empty label",
                serde_json::json!({"start": 0, "end": 30, "label": ""}),
            ),
            (
                "start as string",
                serde_json::json!({"start": "0", "end": 30, "label": "n"}),
            ),
        ];
        for (name, entry) in cases {
            assert!(map_entry_from(entry, total).is_none(), "{name}");
        }
        let ok = map_entry_from(
            &serde_json::json!({"start": 0, "end": 30, "label": "n"}),
            total,
        )
        .unwrap();
        assert_eq!((ok.start, ok.end), (0, 30));

        // Eleven valid entries -> cut at 10.
        let entry = |i: u64| serde_json::json!({"start": i * 4, "end": i * 4 + 3, "label": format!("e{i}")});
        let mut entries = Vec::new();
        for i in 0..11u64 {
            if let Some(e) = map_entry_from(&entry(i), 60) {
                entries.push(e);
            }
            if entries.len() == 10 {
                break; // mirrors the take(10) at the call site
            }
        }
        assert_eq!(entries.len(), 10);
        assert_eq!(entries.last().unwrap().start, 36);
    }

    // --- render ---

    #[test]
    fn render_pins_sections_and_indent() {
        let doc = numbered(4);
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        let out = render(
            Some("q"),
            &chunks,
            "the story",
            &[MapEntry {
                start: 0,
                end: 30,
                label: String::from("l"),
            }],
        );
        assert_eq!(
            out,
            r#"Summary of bytes 0..60, 1 chunk(s)
Query:
q
SUMMARY
the story
MAP
   1. [bytes 0..30] l
"#
        );
        let out_noq = render(None, &chunks, "the story", &[]);
        assert_eq!(
            out_noq,
            r#"Summary of bytes 0..60, 1 chunk(s)
SUMMARY
the story
MAP
"#
        );
    }

    // --- summarize (mock LLM) ---

    #[test]
    fn failed_call_dump_writes_the_exchange() {
        let path = dump_call(
            "fail_map_0..60",
            "chunk bytes 0..60 (context_lines 0)",
            "system here",
            "user here",
            "raw response here",
        );
        assert_eq!(path, "/tmp/summarize_fail_map_0..60.txt");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("=== fail_map_0..60 — chunk bytes 0..60 (context_lines 0) ==="));
        assert!(written.contains("=== system prompt ===\nsystem here"));
        assert!(written.contains("=== user prompt (9 bytes) ===\nuser here"));
        assert!(written.contains("=== raw response (17 bytes) ===\nraw response here"));
    }

    #[test]
    fn success_dump_writes_the_exchange() {
        // The success-path twin: the reduce input, for prompt A/B replay
        // without a live run.
        let path = dump_call("dump_story", "story (whole document)", "s", "u", "r");
        assert_eq!(path, "/tmp/summarize_dump_story.txt");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("=== dump_story — story (whole document) ==="));
    }

    /// The mock LLM bodies for a healthy run, in call order: the map
    /// response (per chunk), the story response, the map-editor response.
    fn summarize_bodies() -> (String, String, String) {
        (
            serde_json::json!({
                "summary": "Four filler lines about nothing in particular.",
                "pointers": [
                    {"line_start": 1, "line_end": 2, "label": "first two filler lines"},
                    {"line_start": 4, "line_end": 4, "label": "last filler line"}
                ]
            })
            .to_string(),
            serde_json::json!({"summary": "The document is four filler lines."})
                .to_string(),
            serde_json::json!({
                "map": [
                    {"start": 0, "end": 30, "label": "first two filler lines"},
                    {"start": 45, "end": 60, "label": "last filler line"}
                ]
            })
            .to_string(),
        )
    }

    #[tokio::test]
    async fn summarize_renders_pinned_format() {
        let doc = numbered(4); // 60 bytes -> 1 chunk [0, 60)
        let (map_body, story_body, mapedit_body) = summarize_bodies();
        let url = llm::start_mock_llm(vec![map_body, story_body, mapedit_body]);

        let out = summarize(
            &doc,
            Some("what filler is here"),
            0,
            None,
            64,
            400,
            None,
            &llm::mock_config(&url),
        )
        .await
        .unwrap();

        let expected = r#"Summary of bytes 0..60, 1 chunk(s)
Query:
what filler is here
SUMMARY
The document is four filler lines.
MAP
   1. [bytes 0..30] first two filler lines
   2. [bytes 45..60] last filler line
"#;
        assert_eq!(out, expected);
    }

    #[tokio::test]
    async fn summarize_without_query_omits_the_query_section() {
        let doc = numbered(4);
        let (map_body, story_body, mapedit_body) = summarize_bodies();
        let url = llm::start_mock_llm(vec![map_body, story_body, mapedit_body]);

        let out = summarize(&doc, None, 0, None, 64, 400, None, &llm::mock_config(&url))
            .await
            .unwrap();

        assert_eq!(
            out,
            r#"Summary of bytes 0..60, 1 chunk(s)
SUMMARY
The document is four filler lines.
MAP
   1. [bytes 0..30] first two filler lines
   2. [bytes 45..60] last filler line
"#
        );
    }

    #[tokio::test]
    async fn map_edit_is_sorted_to_byte_order_by_the_tool() {
        let doc = numbered(4);
        // The map editor emits the map out of order; the tool stable-
        // sorts by start, so the render is byte order regardless of
        // model order.
        let (map_body, story_body, _) = summarize_bodies();
        let mapedit = serde_json::json!({
            "map": [
                {"start": 45, "end": 60, "label": "later"},
                {"start": 0, "end": 30, "label": "earlier"}
            ]
        })
        .to_string();
        let url = llm::start_mock_llm(vec![map_body, story_body, mapedit]);

        let out = summarize(&doc, Some("filler"), 0, None, 64, 400, None, &llm::mock_config(&url))
            .await
            .unwrap();
        let map = out.split("MAP\n").nth(1).unwrap();
        let earlier = map.find("[bytes 0..30] earlier").unwrap();
        let later = map.find("[bytes 45..60] later").unwrap();
        assert!(earlier < later, "tool must sort to byte order:\n{out}");
    }

    #[tokio::test]
    async fn unparseable_first_attempt_is_retried_and_recovers() {
        let doc = numbered(4);
        // Attempt 1 gets a prose-only response; attempt 2 gets the good
        // map body. The mock serves exactly these bodies (then the story
        // + map-editor bodies), which also pins that the retry consumed
        // one call.
        let (map_body, story_body, mapedit_body) = summarize_bodies();
        let url = llm::start_mock_llm(vec![
            String::from("Prose only, no JSON here."),
            map_body,
            story_body,
            mapedit_body,
        ]);

        let out = summarize(&doc, Some("filler"), 0, None, 64, 400, None, &llm::mock_config(&url))
            .await
            .unwrap();
        assert!(
            out.starts_with("Summary of bytes 0..60, 1 chunk(s)"),
            "unexpected: {out}"
        );
    }

    #[tokio::test]
    async fn prose_only_response_is_salvaged_as_a_summary() {
        let doc = numbered(4);
        // All three map attempts are prose-only (the live essay mode: a
        // good summary, no JSON), so the contribution must come from
        // salvage, not from any parsed object. The mock serves three
        // map bodies, then the story + map-editor bodies: attempt 1 a
        // 30-word essay, attempt 2 a longer 40-word essay (the one the
        // salvage keeps — longest wins), attempt 3 a short apology
        // (rejected, under SALVAGE_MIN_WORDS).
        let essay = |n: usize| {
            (0..n).map(|i| format!("word{i}")).collect::<Vec<_>>().join(" ")
        };
        let story = serde_json::json!({"summary": "One salvaged filler chunk."})
            .to_string();
        let mapedit = serde_json::json!({"map": []}).to_string();
        let url = llm::start_mock_llm(vec![
            essay(30),
            essay(40),
            String::from("I cannot summarize this text."),
            story,
            mapedit,
        ]);

        let out = summarize(&doc, Some("filler"), 0, None, 64, 400, None, &llm::mock_config(&url))
            .await
            .unwrap();

        assert!(
            out.starts_with("Summary of bytes 0..60, 1 chunk(s)"),
            "unexpected: {out}"
        );
        assert!(
            out.contains("SUMMARY\nOne salvaged filler chunk."),
            "unexpected: {out}"
        );
    }

    #[tokio::test]
    async fn map_parse_failure_fails_after_attempts_naming_the_chunk() {
        let doc = numbered(4);
        // The mock serves one body per call: three identical bad bodies
        // pin the retry contract (ATTEMPTS = 3), then the call fails.
        let bad = String::from("I cannot summarize this text.");
        let url = llm::start_mock_llm(vec![bad.clone(), bad.clone(), bad]);

        let err = summarize(
            &doc,
            Some("filler"),
            0,
            None,
            64,
            400,
            None,
            &llm::mock_config(&url),
        )
        .await
        .unwrap_err();

        assert!(err.contains("bytes 0..60"), "unexpected: {err}");
        assert!(err.contains("no JSON object found"), "unexpected: {err}");
        assert!(err.contains("after 3 attempts"), "unexpected: {err}");
    }

    #[tokio::test]
    async fn all_degraded_chunks_fail_before_the_reduce() {
        let doc = numbered(4);
        // Parseable but off-contract: no summary, no pointers -> degraded.
        let url = llm::start_mock_llm(vec![serde_json::json!({"regions": []}).to_string()]);

        let err = summarize(
            &doc,
            Some("filler"),
            0,
            None,
            64,
            400,
            None,
            &llm::mock_config(&url),
        )
        .await
        .unwrap_err();

        assert!(err.contains("empty or off-contract"), "unexpected: {err}");
        assert!(err.contains("bytes 0..60"), "unexpected: {err}");
    }

    #[tokio::test]
    async fn story_without_summary_fails_the_call() {
        let doc = numbered(4);
        let url = llm::start_mock_llm(vec![
            serde_json::json!({"summary": "ok", "pointers": []}).to_string(),
            serde_json::json!({"map": []}).to_string(),
        ]);

        let err = summarize(
            &doc,
            Some("filler"),
            0,
            None,
            64,
            400,
            None,
            &llm::mock_config(&url),
        )
        .await
        .unwrap_err();

        assert!(err.contains("story"), "unexpected: {err}");
        assert!(err.contains("no summary"), "unexpected: {err}");
    }

    #[tokio::test]
    async fn map_edit_failure_falls_back_to_the_mechanical_map() {
        let doc = numbered(4); // 1 chunk, two pointers (0..30, 45..60)
        let (map_body, story_body, _) = summarize_bodies();
        // The story parses; all three map-editor attempts are prose-only
        // (the essay attractor). The run must still succeed, on the
        // fallback map: the first pointer of the only chunk (the second
        // pointer is not in the fallback).
        let bad = String::from("Prose only, no JSON here.");
        let url = llm::start_mock_llm(vec![map_body, story_body, bad.clone(), bad.clone(), bad]);

        let out = summarize(&doc, Some("filler"), 0, None, 64, 400, None, &llm::mock_config(&url))
            .await
            .unwrap();
        assert!(
            out.contains("SUMMARY\nThe document is four filler lines."),
            "unexpected: {out}"
        );
        let map = out.split("MAP\n").nth(1).unwrap();
        assert!(
            map.contains("[bytes 0..30] first two filler lines"),
            "fallback must take the first pointer per chunk:\n{out}"
        );
        assert!(
            !map.contains("[bytes 45..60]"),
            "fallback is first-pointer-per-chunk only:\n{out}"
        );
    }

    #[test]
    fn fallback_map_is_the_first_pointer_per_chunk_budget_clamped() {
        // Hand-crafted contributions (the structs, not the parse):
        // chunks in document order, the writer's pointer order within.
        fn c(start: usize, end: usize, pointers: Vec<(usize, usize, &str)>) -> Contribution {
            Contribution {
                span: (start, end),
                summary: String::from("s"),
                pointers: pointers
                    .into_iter()
                    .map(|(b, e, l)| Pointer {
                        label: l.to_string(),
                        byte_start: b,
                        byte_end: e,
                    })
                    .collect(),
                degraded: false,
            }
        }
        let contribs = vec![
            c(0, 100, vec![(10, 20, "a"), (30, 40, "b")]),
            c(100, 200, vec![(110, 120, "c")]),
            c(200, 300, vec![]), // no pointers -> contributes nothing
            c(300, 400, vec![(310, 320, "d")]),
        ];
        let all = fallback_map(&contribs, 10);
        assert_eq!(
            all.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(),
            vec!["a", "c", "d"]
        );
        assert_eq!(
            all.iter().map(|e| (e.start, e.end)).collect::<Vec<_>>(),
            vec![(10, 20), (110, 120), (310, 320)]
        );
        // Budget clamp: two entries = the two earliest chunks' first
        // pointers.
        let two = fallback_map(&contribs, 2);
        assert_eq!(
            two.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
        // No pointers at all -> empty map, not an error.
        assert!(fallback_map(&[c(0, 100, vec![])], 10).is_empty());
        // The sort: a hand-crafted contribution whose first pointer
        // starts before a previous chunk's (out of band for real chunks
        // — pointer spans lie inside their chunk) still ends up
        // byte-ordered.
        let weird = vec![
            c(0, 100, vec![(90, 99, "second")]),
            c(100, 200, vec![(80, 89, "first")]),
        ];
        let sorted = fallback_map(&weird, 10);
        assert_eq!(sorted[0].label, "first");
        assert_eq!(sorted[1].label, "second");
    }

    #[test]
    fn map_edit_call_pins_user_message() {
        let doc = numbered(4);
        let chunks = prepare(&doc, 0, None, 64).unwrap();
        let value = serde_json::json!({
            "summary": "s",
            "pointers": [{"line_start": 1, "line_end": 2, "label": "first two"}]
        });
        let contrib = contribution_from(&chunks[0], &value);
        let (system, user) = map_edit_call(&[contrib.clone()], Some("filler"), 10);
        // n = 1, budget 10 -> the per-chunk quota clause (budget >= n).
        assert!(system.contains("about 10 per chunk"));
        assert_eq!(
            user,
            "Query: filler\n\
Chunk summaries (one block per chunk, in document order):\n\
[bytes 0..60]\n\
Summary: s\n\
Pointers:\n   1. [bytes 0..30] first two\n"
        );
        // The story editor shares the user prompt shape (and the query
        // line).
        let contrib2 = contribution_from(&chunks[0], &value);
        let (_, story_user) = story_call(&[contrib2], Some("filler"), 400);
        assert_eq!(story_user, user);
        let (_, noq_user) = map_edit_call(&[contrib], None, 10);
        assert!(!noq_user.starts_with("Query:"));
    }

    // --- multi-chunk (N > 1) ---

    /// 330-byte doc; chunk_bytes 200 (overlap 25) -> exactly two
    /// chunks: [0, 210) and [180, 330) (chunk 1's line 1 = byte 180 is
    /// in the overlap).
    #[tokio::test]
    async fn multi_chunk_run_threads_earlier_summaries() {
        let doc = numbered(22);
        let url = llm::start_mock_llm(vec![
            serde_json::json!({
                "summary": "The first part.",
                "pointers": [{"line_start": 1, "line_end": 2, "label": "the start"}]
            })
            .to_string(),
            serde_json::json!({"summary": "The second part continues it.", "pointers": []})
                .to_string(),
            serde_json::json!({"summary": "Two parts, one story."}).to_string(),
            serde_json::json!({
                "map": [{"start": 0, "end": 30, "label": "the start"}]
            })
            .to_string(),
        ]);

        let out = summarize(
            &doc,
            Some("filler"),
            0,
            None,
            200,
            400,
            None,
            &llm::mock_config(&url),
        )
        .await
        .unwrap();

        // Success also pins the call sequence: map, map, story, mapedit
        // — the mock serves exactly those four bodies, in order.
        let expected = r#"Summary of bytes 0..330, 2 chunk(s)
Query:
filler
SUMMARY
Two parts, one story.
MAP
   1. [bytes 0..30] the start
"#;
        assert_eq!(out, expected);
    }

    #[tokio::test]
    async fn mid_run_degraded_chunk_fails_nothing_and_reaches_the_reduce() {
        let doc = numbered(22);
        let url = llm::start_mock_llm(vec![
            // Chunk 0: parseable but off-contract -> degraded, skipped
            // from the S-block; the run must still succeed.
            serde_json::json!({"regions": []}).to_string(),
            serde_json::json!({"summary": "The real part.", "pointers": []}).to_string(),
            serde_json::json!({"summary": "One usable part."}).to_string(),
            serde_json::json!({"map": []}).to_string(),
        ]);

        let out = summarize(
            &doc,
            Some("filler"),
            0,
            None,
            200,
            400,
            None,
            &llm::mock_config(&url),
        )
        .await
        .unwrap();

        assert!(
            out.starts_with("Summary of bytes 0..330, 2 chunk(s)"),
            "unexpected: {out}"
        );
        assert!(
            out.contains("SUMMARY\nOne usable part."),
            "unexpected: {out}"
        );
    }
}
