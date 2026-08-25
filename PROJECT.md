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
       Design settled; implementation pending.
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

Tier 3 decisions (design settled, implementation pending):

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

* `llm.rs`: `LlmConfig` from env — `LLAMA_URL`, `LLAMA_MODEL`,
  `LLAMA_CHUNK_BYTES` (default 256KB); one-shot `chat()` under a 60s
  `tokio::time::timeout` (man_page.rs pattern); lenient `extract_json()`
  (strip code fences, first `{` .. last `}`).
* `chunk.rs`: chunks of `chunk_bytes` snapped to line boundaries, constant
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
  triage, optional for summarize.

`triage_doc` contract:

* Per-chunk model output: `{"regions": [{line_start, line_end, score,
  note}], "continuation_note": "..."}` — ≤5 regions, 1-based lines within
  the chunk.
* Exclusive-zone rule (prompt): report only regions that *start* in the
  exclusive zone. Out-of-zone or invalid model output is dropped tool-side.
* `score` 0–10 is ordinal priority (relative within this document), not
  calibrated confidence. `note` ≤15 words, descriptive (not a paraphrase
  of the snippet). `continuation_note` ≤2 sentences — an "editor's note"
  handoff, passed verbatim to the next chunk's call as prior context and
  not shown in the final output.
* Hits are rendered tool-side: score + note + location (byte + line span)
  + verbatim snippet starting at the line containing the region start,
  ~256 chars, line-bounded (grep-like context). The LLM returns pointers +
  metadata only; the snippet is a mechanical slice (no text relay → no
  drift). Top 5 hits. Zero matches is a normal (non-error) result.

`summarize_doc` contract:

* N == 1 (subset fits in one chunk — the common case): a single call
  returns `{"story", "map"}` directly.
* N > 1, map phase: call N's input = query + S1..S(N-1) verbatim + chunk
  N raw + overlap note. S_N = `{"summary" (≤250 words), "pointers" (≤5:
  {line_start, line_end, label ≤10 words})}` — query-aware, scope: this
  chunk only, written with the earlier summaries in view (consistent
  terminology, no re-covering the overlap, may note relations to earlier
  parts). S_N is fixed once generated and never re-compressed. Prompt
  metaknowledge: a later editor assembles the final story and prunes, and
  the summarizer sees the past but not the future (book-reading
  constraint) → include borderline material rather than guessing at global
  importance.
* Reduce ("editor") phase: input = query + all S_N with tool-converted
  absolute byte spans + pointers; output = `{"story" (≤400 words, coherent
  whole-document story from the query's perspective), "map" (≤10:
  {start, end, label}, selected/merged from the per-chunk candidates)}`.
  The tool converts lines → absolute bytes before the reduce so the model
  does no arithmetic.
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
  detail.

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
  tests can point at the mock.

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

Implemented:

* MCP lifecycle and tool dispatch
* `man_page`
* `fetch_url` (stores every fetch in `data/`; inline below 32 KB, id + preview above)
* `search_web` (local SearXNG instance; `SEARXNG_URL` env var, optional `num_results` arg)
* `read_doc` / `search_doc` (tier-2 narrowing over stored documents)
* subprocess timeout for `man_page` (concurrent pipe drain; kill + reap on deadline)
* async Tokio runtime
* unit and end-to-end integration tests

Current goal:

* Implement tier 3 (roadmap 5c): `triage_doc` and `summarize_doc` on the
  shared `llm.rs` (one-shot llama.cpp calls) and `chunk.rs` (overlapping
  chunks, exclusive zones). The design is settled — see Design direction,
  Tier 3 decisions. Suggested order: `llm.rs` → `chunk.rs` → `triage_doc`
  → `summarize_doc`.
