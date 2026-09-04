# Rust MCP Project

## Goal

Learn Rust by building:

1. a man-page tool;
2. a minimal MCP server exposing it;
3. a web-search tool with async HTTP and page fetching;
4. later, a separate Rust agent.

## Working Style

Assume the user is an experienced programmer who is new to Rust.

* Work collaboratively in small coherent steps. Inspect the current code before planning a change.
* If you encounter an unexpected complication that would require substantial additional reasoning, experimentation, or scope expansion beyond the current task, pause and explain what you found before pursuing it further. Let me decide whether to investigate it now, defer it, or continue with the original task. Small checks needed to understand or complete the current task are fine without asking.
* By default, let me implement changes myself. Do not edit Rust source or tests unless I explicitly ask you to implement the change.
* Prefer focused explanations and snippets over complete solutions, since the goal is for me to write and understand the Rust myself.
* Prefer the simplest design or fix that satisfies the current requirement.
* Explain Rust-specific design choices, unfamiliar language features, and compiler errors.
* Avoid unnecessary complexity such as explicit lifetimes, trait objects, `Arc`, `Mutex`, or complex generics; introduce them when the problem genuinely benefits from them and explain why.
* Answer questions and tangents directly before returning to implementation.
* Use the existing source code as evidence of Rust concepts already encountered.
* You may edit project documentation when I explicitly ask for documentation updates.

## Roadmap

1. ~~Complete synchronous man-page tool.~~
2. ~~Expose through minimal MCP server.~~
3. ~~Introduce async and webpage fetching.~~
4. ~~Add SearXNG web search.~~
5. Large-document support, tiered (see Design direction):
   5a. ~~Tier 1: `fetch_url` stores every fetch (raw file + JSON sidecar,
       sha256-of-URL id); inline below a size threshold, id + preview above.~~
   5b. ~~Tier 2: `read_doc` (offset/limit) and in-document grep, so stored
       documents can be narrowed without loading them.~~
   5c. Tier 3: LLM document analysis — `triage_doc` (ranked hits: pointers
       + relevance + verbatim snippets) and `summarize_doc` (coherent story
       + structural map) via one-shot calls to the local llama.cpp endpoint.
       Design settled; `triage_doc` done, `summarize_doc` next.
6. Concurrency/resource control as needed (batch fetching, politeness,
   long-running jobs).
7. Plan the Rust agent separately, informed by the tier-3 agent loop.

`BEHAVIOR.md` records the current intended behavior of the man-page tool.
Treat it as revisable rather than immutable: verify questionable assumptions,
and discuss proposed changes with the user before implementing them.

## Design direction (web search / large documents)

Goal: a free, powerful web-search tool for an LLM caller, where depth is
decided by the user or the agent as needed.

Key constraint: the caller's context window is the real limit, not disk.
Storage is unbounded in principle (configurable resource limits, never hard
capability ceilings); *narrowing* is the tool's job, and the agent only
consumes small targeted slices.

Tiers, climbed as needed:

* Tier 0: `search_web` snippets (engine-generated, ~100-350 chars).
* Tier 1: fetch & store the raw document (source of truth on disk).
* Tier 2: grep / offset-limit reads over stored documents.
* Tier 3: LLM chunk triage (context-isolated; returns locations + relevance,
  not text).

Tier 1 decisions (v1, done):

* `fetch_url` always stores; 32 KB threshold (inline below, preview above).
* Storage: `./data` (env `TOOLBOX_DATA`), `<sha256(url)>` raw file plus a
  `<sha256(url)>.json` sidecar (url, fetched_at, content_type, bytes).
* Re-fetch overwrites; no caching yet.

Tier 2 decisions (v1, done):

* `read_doc`: default window 4096 bytes; id must be the full sha256 hex
  (no prefix matching).
* `search_doc`: case-insensitive plain substring, line-oriented, max 20
  matches, 200-char snippets; zero matches is a normal (non-error) result.
* Known limitation (roadmap 6): both read the whole file with blocking
  `fs::read` — fine for web pages, needs streaming for multi-GB docs.

Tier 3 decisions (design settled; `triage_doc` in progress):

Two new tools on shared machinery. Purpose split: `triage_doc` is semantic
narrowing (search-engine role: ranked hits, for when `search_doc`'s lexical
matching fails on vocabulary mismatch); `summarize_doc` is survey (coherent
story + structural map, query optional). Both operate on stored documents
(`fetch_url` ids) via one-shot calls to the local llama.cpp endpoint.

Endpoint facts (measured):

* llama.cpp OpenAI-compatible at `http://172.17.0.1:8081/v1`, model
  `qwen3.8-27b-q4xl` (Q4_K, n_ctx 200192). The server is dedicated to this
  project, so the whole context window is available to a single call.
* Prefill ~2,850 tok/s (batch), generation ~148 tok/s.
* The model thinks by default; disable with
  `"chat_template_kwargs": {"enable_thinking": false}`. All calls use
  temperature 0 and small max_tokens.
* 256KB of text ≈ 40–55k tokens; one 256KB-chunk call ≈ 15–25s.

Shared machinery:

* `llm.rs`: `LlmConfig` from env — `LLAMA_URL`, `LLAMA_MODEL`; one-shot
  `chat()` under a per-call `tokio::time::timeout` = base 60 s + 10 ms per
  `max_tokens` token (see `triage_doc` addenda; man_page.rs pattern);
  lenient `extract_json()` (strip code fences, first `{` .. last `}`).
* `chunk.rs`: `default_chunk_bytes()` from env `LLAMA_CHUNK_BYTES`
  (default 256KB, read by `chunk.rs`, not `llm.rs`); chunks of
  `chunk_bytes` snapped to line boundaries, constant
  1/8 overlap. Chunk N's exclusive zone = all bytes after the inherited
  overlap; exclusive zones are exactly contiguous, so every byte belongs to
  exactly one chunk (no duplicates, no gaps). Line table (line starts in
  bytes) for span conversion. The document is decoded once with
  `String::from_utf8_lossy`; for valid UTF-8 (the norm for web docs)
  offsets are identical to the raw bytes that `read_doc`/`search_doc`
  take (v1 consistency decision).
* Hard cap of 64 chunks on both tools; over-cap → error pointing at
  offset/limit.
* Unified args: `{id, query, offset?, limit?}` — query required for
  triage, optional for summarize; triage alone adds an optional
  `context` (see triage contract).

chunk.rs design (implemented; 11 unit tests pass):

* Note: chunk 0's end also goes through `snap_forward` (a raw cut there
  would break the "tail < O" case and leave chunk 0 ending mid-line).


* `Chunk<'a>` borrows the doc (no per-chunk clones): `start`, `end` (both
  line-aligned byte offsets in the decoded doc), `exclusive_start` (==
  `start` for chunk 0, else the previous chunk's end), `text: &'a str`,
  `line_starts: Vec<usize>` (absolute byte offset of each line of this
  chunk — a slice of one doc-level table built for the scanned range),
  `context_lines` (lines before `exclusive_start`; 0 for chunk 0).
* `split<'a>(doc, from, limit: Option<usize>, chunk_bytes, max_chunks)
  -> Vec<Chunk<'a>>`. `limit` = max bytes to scan from `from` (input
  window, mirroring read_doc's byte limit); `None` = to end of doc —
  deliberately NOT a small default window like read_doc's 4096, because a
  small window on a *search* tool yields misleading "no hits" false
  negatives; the 64-chunk cap is the protection instead.
* Layout: raw step C − O (O = C/8); start snaps *back* to a line start,
  end snaps *forward* to a line end; invariant `exclusive_start_N ==
  end_{N−1}` by construction, so exclusive zones tile the range exactly.
* Boundary conditions: (1) `range_end = min(from + limit, total)` with
  `saturating_add`; empty range → zero chunks (caller errors "nothing to
  scan"); (2) `from` snaps back to its line start (a scan may begin up to
  one line early); (3) end forward-snap includes the rest of the cut line;
  (4) emit a next chunk only while `prev_end < range_end` — progress is at
  least C − O, so no infinite loop; works down to C < 8 where O = 0 (exact
  tiling, no overlap); (5) tail < O → final chunk starts deep in the
  previous one and carries just the tail; (6) last line may lack a `\n`;
  a line's span is `[line_start, next_line_start)` or `range_end` for the
  final line (needed to convert a region's `line_end` to a byte end);
  (7) `max_chunks` stops emission; the tool reports "scanned X of Y";
  (8) snap-back never snap-forward (forward would shrink the overlap and
  could drop bytes out of every chunk's view); (9) user offsets may land
  mid-character in a valid UTF-8 doc — never slice the str at a user offset
  (backward scan on `doc.as_bytes()`); floor `range_end` with
  `floor_char_boundary` (safe side: scan never passes `from + limit`).
  After that, every str slice is at a line start (inherently a boundary) or
  the floored `range_end`.
* No tail overlap (deferred): the owner of a straddling region doesn't see
  its post-cut tail; mitigated by a "continues" flag in the region's
  `note` + reading with margin.
  Trigger to add: hits systematically missed at boundaries. Adding it later
  doesn't touch the exclusive-zone invariant.
* Tests (deterministic, ~20 lines of 40-byte lines, C = 160): exclusive
  zones tile (`exclusive_start[i+1] == end[i]`, first start at snapped
  `from`, last `end == range_end`); starts line-aligned; `context_lines`
  correct; tail < O case; mid-line `from` snap; `limit` Some/None;
  `max_chunks` respected; C = 4 (O = 0) exact tiling; `from ≥ total` →
  empty vec.

`triage_doc` contract:

* Per-chunk model output: `{"regions": [{line_start, line_end, score,
  note}]}` — ≤`max_hits` regions, 1-based lines within the chunk.
* Exclusive-zone rule (prompt): report only regions that *start* in the
  exclusive zone. Out-of-zone or invalid model output is dropped tool-side.
* `score` is ordinal priority (relative within this document), not
  calibrated confidence. The prompt states the floor as 1–10, where 1 is
  still a real, passing mention of the queried subject (tool-side
  validation stays lenient at 0–10). `note` ≤15 words, descriptive (not
  a paraphrase of the snippet).
* Per-chunk calls are stateless — no relay between chunks (parallelizable;
  contrast summarize's running story: triage is a map, summarize a
  reduce). Optional call-level `context` string (e.g. the `summarize_doc`
  story, or a targeted re-summarize such as a glossary of self-defined
  terms) is prepended to every chunk's prompt as rough orientation —
  useful where a mid-doc chunk is hard to interpret without the beginning;
  the chunk text is authoritative. No size cap (v1); the tool description
  conveys the intent.
* Hits are rendered tool-side: score + note + location (byte + line span)
  + verbatim snippet starting at the line containing the region start,
  ~256 chars, line-bounded (grep-like context). The LLM returns pointers +
  metadata only; the snippet is a mechanical slice (no text relay → no
  drift). Top `max_hits` (default 5). Zero matches is a normal (non-error)
  result.

`triage_doc` addenda (settled during implementation):

* `max_hits` argument: optional, default 5, hard cap 1000 (clamped,
  documented in the schema). One knob governs both the per-chunk prompt cap
  (model reports ≤ N regions per chunk — a low fixed per-chunk cap would
  silently truncate dense chunks before global ranking) and the global
  top-N across all chunks. ~450 rendered chars per hit, so N is a caller
  context budget, not a capability cap; the 64-chunk scan cap and the 1000
  knob are the only hard limits.
* `max_tokens = 128 + 64 * max_hits` (kept in `u32`); the per-call timeout
  in `llm::chat` is `cfg.timeout + max_tokens * 10 ms` (assumes 100 tok/s,
  below the measured 148) — `cfg.timeout` (60 s) becomes a *base* covering
  prefill + connection.
* Chunk text is line-numbered in the prompt (1-based chunk line + `\t`,
  ~5–6% token overhead) — without it the model cannot report line numbers
  reliably, and both exclusive-zone validation and snippet placement depend
  on them.
* The prompt states the exclusive line chunk-relative, 1-based:
  `chunk.context_lines + 1`. The system prompt template lives in
  `system_prompt()` — `format!` requires a literal at the call site, so
  a const template is not possible; the contract is pinned by a test.
* Relevance rules (prompt, added after the first live KJV triage): a
  region qualifies only if it directly mentions the queried subject
  (thematic adjacency alone does not); "at most N" is a cap, not a
  target — padding is explicitly forbidden and a zero-region report is
  explicitly licensed. Pre-fix, 36% of the KJV top-1000 were
  self-admitted "no animals" padding.
* Context-suppression fix (prompt, after the v2 KJV re-run): with the
  old context clause, chunks whose overlap lines already contained
  query-relevant content under-reported continuations in their exclusive
  zone — the model deduped against the context (Deut 14's clean/unclean
  list: 0 hits at ctx≈695, twice, vs 74 with ctx=0; Lev 11 and Balaam's
  donkey also lost). The clause now says the overlap is orientation only
  and must not be used to skip content. Verified on the exact failing
  chunk: 66 hits incl. the Deut 14 lists, 0 junk.
* Observed score usage (healthy runs): the model works in ~7–10
  (7 = metaphor/implicit, e.g. "brutish men", "mighty hunter"; 8 =
  explicit but background; 9 = prominent; 10 = central). 1–6 were only
  ever produced by v2's context-suppressed chunks (self-admitted
  non-mentions), so the prompt's score-1 floor is effectively dormant.
  Refined with a phrasing experiment (same window, 3 queries):
  "mentions of animals" → 210, "the character of God" → 0,
  "descriptions of God" → 288, all scoring 7–10. The query controls
  the qualifying *gate* (does the strict "directly mentions" clause
  open at all — abstract phrasings close it) and the tail's edge
  (peripheral mentions admitted at 7), not the score band. The score
  is a coarse directness ladder, not a graded-relevance spectrum.
  Known consequence: abstract phrasings of a pervasive subject can
  return a silent 0 (indistinguishable from "not in document").
  Tested fix (v4, reverted): softening the clause to "genuinely
  addresses the query / substantive treatment (describing, discussing,
  clearly exemplifying)" regressed all three controls (animals 210→131,
  character of God 0→0, descriptions 288→10) — the model reads added
  ambiguity as a reason to report *less* (anti-padding primes
  conservatism), so a single shared gate cannot be strict for binary
  queries and permissive for themes.
* Query-adaptive prompt (adopted after the v4 failure; one tool, the
  query phrasing controls the scan): classify line ("first interpret
  the query: mentions of X → mentions; theme/property/question →
  treatments") + branch-scoped gates (mention: v3 wording; theme:
  "bears on ... even when the passage's main topic is something else")
  + breadth line (query asks for "possibly / tangentially" relevant →
  include peripheral bearings). Critical finding from the variant
  matrix: the OPENING line is the key — "contain relevant mentions of
  the query" primes mention semantics and overrides the branch gates
  (every variant keeping v3's opening returned 0 for the theme query);
  the neutral "are relevant to the query" opens them. Final matrix
  (same window, temp 0): animals 210→220 (no cost), "anything that
  could possibly bear on the character of God, even tangentially"
  0→120 (quality verified: top = creation/image/covenant, tail =
  sovereignty/justice bearings), bare "the character of God" still 0 —
  the classify step does not flip bare abstract noun phrases; now a
  documented phrasing requirement in DESCRIPTION. Natural phrasing
  verified: "anything that tells us something about what god is like"
  → 24 hits (default non-incidental tier); the same ask with
  "possibly ... even tangentially" → 120 — the strictness knob is
  observable end-to-end, zero junk in both. Temperature check
  (0.7): the gate opens as a lottery (224 hits, then 0 on the repeat)
  and even good queries wobble (descriptions 288→23), so temperature
  stays 0 for triage.
* Parseable-but-off-contract JSON (e.g. missing `regions` key) → treated
  as no hits (lenient v1); chat/parse failure → chunk reported untriaged
  (byte span + reason), scan continues. "0 hit(s)" is only honest when
  every chunk was triaged.
* Doc-absolute line numbers via a running newline counter
  (`lines_before`): first chunk counts `doc[..chunk.start]`, each later
  chunk adds the count over `[prev.start..chunk.start]`.
* Output format (pinned by tests): header
  `Triage for 'QUERY': N hit(s), scanned M chunk(s), bytes a..b`
  (QUERY = the query verbatim — no angle brackets in the output);
  one `   untriaged: bytes a..b (reason)` line per untriaged chunk (scan
  order) when any; then hits as `1. [score] note` / `   line ls..le,
  bytes a..b` / `   |`-prefixed snippet — continuation lines (untriaged
  and hit details) share a 3-space indent.
* Oversized output (N = 1000 → hundreds of KB) is *not* stored in v1: the
  tool description carries the cost warning (sequential scan; per-chunk
  calls grow with N). Store-and-preview for oversized triage output
  (fetch_url pattern, derived-artifact id scheme) is a deferred follow-up
  step.
* Manual end-to-end test (run): KJV 4.4 MB, 20 chunks, `max_hits: 1000`
  — ~42 min live, 1000 hits, 0 untriaged, output saved to
  `kjv_animals_triage.txt`; surfaced the padding bug fixed by the
  relevance-rules bullet above.

`summarize_doc` contract:

* N == 1 (subset fits in one chunk — the common case): a single call
  returns `{"story", "map"}` directly.
* N > 1, map phase: call N's input = query + S1..S(N-1) verbatim + chunk
  N raw + overlap note. S_N = `{"summary" (≤250 words), "pointers" (≤5:
  {line_start, line_end, label ≤10 words})}` — query-aware, scope: this
  chunk only, written with the earlier summaries in view (consistent
  terminology; the leading overlap is context, not new content — but a
  logical unit straddling the seam is summarized by this, the later part,
  flagged as a continuation; may note relations to earlier parts). Seam-unit
  pointers may extend into the leading overlap — valid, not an error;
  summarize's validation clamps to the whole chunk text, not the exclusive
  zone (contrast triage, which hard-drops out-of-zone starts). S_N is fixed
  once generated and never re-compressed. Prompt
  metaknowledge: a later editor assembles the final story and prunes, and
  the summarizer sees the past but not the future (book-reading
  constraint) → include borderline material rather than guessing at global
  importance.
* Reduce ("editor") phase: input = query + all S_N with tool-converted
  absolute byte spans + pointers; output = `{"story" (≤400 words, coherent
  whole-document story from the query's perspective), "map" (≤10:
  {start, end, label}, selected/merged from the per-chunk candidates)}`.
  The tool converts lines → absolute bytes before the reduce so the model
  does no arithmetic. The reduce prompt states the seam mechanics
  (overlap ≈1/8; seam units are summarized by the later part) so the editor
  merges them without double-counting. Note the asymmetry: triage keeps the
  hard exclusive-zone rule because it has no editor pass to reconcile
  duplication.
* Why the reduce exists (settled rationale): it is the only role with a
  whole-document view. Unique value: document-level net assessment
  (judgments that belong to no single chunk) + list→narrative composition
  + fixed output budget + map selection. Per-chunk writers see the past
  but not the future, so they cannot make final relevance calls.
* Open constant: story length fixed (≤400 words) vs proportional to chunk
  count (constant density, capped) — decide after seeing real outputs.
* Documented large-doc workflow (belongs in the tool description): rough
  overview of the whole → pick spans from the map → re-summarize the
  subset (same summary budget, less to fit) → `read_doc` for verbatim
  detail. The overview (or a targeted re-summarize, e.g. a glossary) can
  be handed to `triage_doc` as `context`.

Long-running behavior:

* Calls are sequential (politeness to a single local server; parallelism is
  roadmap 6). The stdio loop blocks for the duration of the call (accepted
  for v1; revisit under roadmap 6).
* Triage: a chunk that times out or yields unparseable JSON is reported as
  untriaged and the scan continues. Summarize: the chain is sequential and
  dependent, so a failed chunk fails the whole call (stateless; a retry
  re-runs from chunk 1).

Tests:

* Unit: chunker (overlap, exclusive zones, line table), `extract_json`,
  output formatting, span clamping.
* Integration: local mock LLM server (TcpListener + canned JSON,
  fetch_url.rs pattern); core functions take an explicit `LlmConfig` so
  tests can point at the mock. Note: `chat()` builds a fresh
  `reqwest::Client` per call → one TCP connection per chunk → the mock
  must loop `listener.incoming()` and serve one canned body per
  connection.

## MCP protocol target

This project initially targets MCP protocol version `2025-11-25`.

Before implementing or changing MCP wire behavior, consult:

* `docs/mcp/2025-11-25/SUMMARY.md` for a project-focused overview;
* the relevant vendored specification page in `docs/mcp/2025-11-25/`;
* `docs/mcp/2025-11-25/schema.ts` when exact field shapes are unclear.

`SUMMARY.md` is generated guidance and may be incomplete or mistaken. The
vendored specification and schema are authoritative.

## Local Rust references

The container includes the `rust-docs` and `rust-src` rustup components.

When exact API behavior or signatures matter, verify them against the local
documentation or source for the installed version rather than relying on model
memory.

* Rust documentation: `rustup doc --path`
* Rust sysroot: `rustc --print sysroot`
* Standard-library source:
  `$(rustc --print sysroot)/lib/rustlib/src/rust/library`
* Dependency source: locate the exact installed version under
  `$CARGO_HOME/registry/src/`
* Generated crate documentation, when available: `target/doc/`; generate focused
  docs with `cargo doc -p <crate> --no-deps` when useful.

Use `rg`, local rustdoc HTML, or source as appropriate. Inspect only the
relevant material rather than loading large documentation trees into context.

## Current State

Architecture:

* `main.rs` — async stdio loop
* `server.rs` — MCP request routing
* `mcp.rs` — JSON-RPC/MCP protocol types and shared helpers
* `man_page.rs` — man-page tool (async subprocess, timeout via Tokio)
* `fetch_url.rs` — async HTTP fetching with reqwest (stores every fetch)
* `search_web.rs` — SearXNG web search with reqwest
* `store.rs` — on-disk document store (sha256 id, raw file + JSON sidecar)
* `read_doc.rs` — offset/limit reads of stored documents
* `search_doc.rs` — substring search within stored documents
* `llm.rs` — llama.cpp client (env config, one-shot `chat`, `extract_json`)
* `chunk.rs` — line-aligned overlapping chunks with exclusive zones
* `triage_doc.rs` — tier-3 LLM triage of stored documents

Implemented:

* MCP lifecycle and tool dispatch
* `man_page`
* `fetch_url` (stores every fetch in `data/`; inline below 32 KB, id + preview above)
* `search_web` (local SearXNG instance; `SEARXNG_URL` env var, optional `num_results` arg)
* `read_doc` / `search_doc` (tier-2 narrowing over stored documents)
* subprocess timeout for `man_page` (concurrent pipe drain; kill + reap on deadline)
* async Tokio runtime
* unit and end-to-end integration tests
* `llm.rs` (complete: `LlmConfig` with `LLAMA_URL`/`LLAMA_MODEL` env
  fallbacks, `chat()`, `extract_json` + 11 tests; mock-server test for
  `chat` deferred until first consumer)
* `chunk.rs` (line-aligned overlapping chunks, exclusive zones, 11 unit
  tests; `LLAMA_CHUNK_BYTES` read by `default_chunk_bytes()`; `Chunk`
  derives `Debug`)
* `triage_doc` (complete): args `{id, query, offset?, limit?, context?,
  max_hits?}` (default 5, clamped to 1000 inside `triage()`); `prepare`;
  validated helpers `snippet_for` + `region_hits`/`region_from`; the
  per-chunk LLM loop (line-numbered prompt with the exclusive line,
  `max_tokens = 128 + 64 * max_hits`, running `lines_before`,
  untriaged-then-continue, stable `total_cmp` ranking, top-N, pinned
  render format). `llm::chat` timeout is base + 10 ms per token,
  `LlmConfig` fields are pub for tests. Tests: 2 mock-LLM unit tests
  (pinned format; max_hits cap + tie order) plus a structural live
  stdio integration test (endpoint dependency commented).
* multi-line tool descriptions (`DESCRIPTION` const per tool; `fetch_url`
  documents the inline/stored split and the large-doc workflow)

Current goal:

* Implement tier 3 (roadmap 5c): `triage_doc` and `summarize_doc` on the
  shared `llm.rs` (one-shot llama.cpp calls) and `chunk.rs` (overlapping
  chunks, exclusive zones). `triage_doc` complete; suite 77 unit + 13
  integration green.
* Earlier step (done): the triage LLM loop — `triage()` with `max_hits`,
  mock-LLM unit tests, wiring (arg/schema/DESCRIPTION, real
  `handle_call`), per-call timeout in `llm::chat`, structural live
  integration test. One format clarification: the pinned header's
  `<query>` is placeholder notation — the query renders verbatim, no
  angle brackets.
* Earlier step (done): relevance-rules prompt fix after the KJV triage
  showed 36% padding in the top-1000 — qualifies = direct mention of the
  queried subject, score-1 floor (still a real mention), explicit
  zero-region license, cap-not-target wording; template moved to
  `system_prompt()`, pinned by test. Verified on the calibration chunk:
  219 hits (was 201), zero self-admitted junk, tail is real, latency
  unchanged.
* Earlier step (done): `LlmConfig` knobs — `temperature: f32` (default
  0.0; triage keeps 0, summarize planned ~0.2) and
  `reasoning_effort: Option<String>` (None = today's explicit
  thinking-off via `chat_template_kwargs`; `Some(e)` sends the knob and
  drops `chat_template_kwargs`). Body construction factored into pure
  `chat_request()`, pinned by 2 tests. `Default` reads
  `LLAMA_TEMPERATURE` / `LLAMA_REASONING_EFFORT` env vars, so live A/B
  runs are an env change, not a rebuild. The server thinks by default
  ("extra high"); the endpoint accepts `reasoning_effort` none/low.
* This step (done): context-suppression prompt fix (addendum above)
  plus full KJV re-run (v3).
* This step (done): query-adaptive triage prompt (addendum above) —
  mention and theme queries in one tool, steered by query phrasing;
  zero cost to the mention control (220 vs 210), breadth sweep
  validated (120 hits on the KJV window), DESCRIPTION documents the
  phrasing contract.
* Next small step: `summarize_doc.rs` per the settled contract (N == 1
  direct `{"story","map"}` call; N > 1 map phase with the running story,
  then the editor/reduce phase), wired the same way. The mock LLM
  helper currently lives in `triage_doc`'s test module — move it to a
  shared spot (e.g. `llm.rs`) when `summarize_doc` needs it.
