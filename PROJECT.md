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
   5c. Tier 3: LLM chunk triage — tool-side one-shot calls to the local
       llama.cpp endpoint, context-isolated, returning pointers to relevant
       regions rather than text.
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

Tier 3 endpoint (when we get there): llama.cpp at
`http://172.17.0.1:8081/v1`, model `qwen3.8-27b-q4xl` (OpenAI-compatible).

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

* Design and implement tier 3 (roadmap 5c): LLM chunk triage via the local
  llama.cpp endpoint. Settle the output contract (region pointers +
  relevance, no body text) and the long-running-call behavior before coding.
  See Design direction.
