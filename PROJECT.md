# Rust MCP Project

Read this file first. Companion docs:

* `CURRENT.md` — current-task state: the in-progress work, live endpoint
  facts, test state, next step. Read before starting work; update at
  step boundaries.
* `DESIGN.md` — design rationale, settled decisions, findings, known
  limitations. Consult when the *why* matters; the code is the source of
  truth for *how*.
* `BEHAVIOR.md` — intended behavior of the man-page tool.

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
* Implement agreed changes by default (2026-09-24 shift — the user
  delegates more of the writing and still reviews all the code as if
  written by them; some pieces the user may still want to write
  themselves).
* Prefer focused explanations and snippets over complete solutions when
  discussing a change, since the goal is for the user to understand the
  Rust as if written by them.
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
5. Large-document support, tiered (tier design and contracts in
   DESIGN.md):
   5a. ~~Tier 1: `fetch_url` stores every fetch (raw file + JSON sidecar,
       sha256-of-URL id); inline below a size threshold, id + preview above.~~
   5b. ~~Tier 2: `read_doc` (offset/limit) and in-document grep, so stored
       documents can be narrowed without loading them.~~
   5c. Tier 3: LLM document analysis — `triage_doc` (ranked hits:
       pointers + relevance + verbatim snippets) and `summarize_doc`
       (coherent story + structural map) via one-shot calls to the local
       LLM endpoint. **`triage_doc` done; `summarize_doc` code-complete
       after step 3; step 4 (docs + 20-chunk KJV run) next — see
       CURRENT.md.**
6. Concurrency/resource control as needed (batch fetching, politeness,
   long-running jobs).
7. Plan the Rust agent separately, informed by the tier-3 agent loop.

`BEHAVIOR.md` records the current intended behavior of the man-page tool.
Treat it as revisable rather than immutable: verify questionable
assumptions, and discuss proposed changes with the user before
implementing them.

## Architecture (glanceable map — the code is truth)

* `main.rs` — async stdio loop
* `server.rs` — MCP request routing (7 tools)
* `mcp.rs` — JSON-RPC/MCP protocol types and shared helpers
* `man_page.rs` — man-page tool (async subprocess, timeout via Tokio)
* `fetch_url.rs` — async HTTP fetching with reqwest (stores every fetch)
* `search_web.rs` — SearXNG web search with reqwest
* `store.rs` — on-disk document store (sha256 id, raw file + JSON sidecar)
* `read_doc.rs` — offset/limit reads of stored documents
* `search_doc.rs` — substring search within stored documents
* `llm.rs` — local LLM client (env config, one-shot `chat`,
  `extract_json`, `#[cfg(test)]` mock helpers)
* `chunk.rs` — line-aligned overlapping chunks with exclusive zones
* `triage_doc.rs` — tier-3 LLM triage (done; prompt v5d + guardrail)
* `summarize_doc.rs` — tier-3 LLM summary (steps 1–3 done; step 4: docs + KJV run)

## Current State (one paragraph — details in CURRENT.md)

Tiers 0–2 and `triage_doc` are complete and validated on a 4.4 MB KJV
document (v6 run: 1000 hits, 0 untriaged, ~15 min on the current
engine). `summarize_doc` steps 1–3 (skeleton, map + reduce, N>1
S-block threading) are done — the code is complete after step 3; step 4
is docs + the 20-chunk KJV live run. Suite: 98
unit + 13 integration, green.

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

When exact API behavior or signatures matter, verify them against the
local documentation or source for the installed version rather than
relying on model memory.

* Rust documentation: `rustup doc --path`
* Rust sysroot: `rustc --print sysroot`
* Standard-library source:
  `$(rustc --print sysroot)/lib/rustlib/src/rust/library`
* Dependency source: locate the exact installed version under
  `$CARGO_HOME/registry/src/`
* Generated crate documentation, when available: `target/doc/`; generate
  focused docs with `cargo doc -p <crate> --no-deps` when useful.

Use `rg`, local rustdoc HTML, or source as appropriate. Inspect only the
relevant material rather than loading large documentation trees into
context.
