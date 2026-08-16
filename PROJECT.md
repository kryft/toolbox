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
5. Add large-document fetching/storage and summarization as useful.
6. Add concurrency/resource control as needed.
7. Plan the Rust agent separately.

`BEHAVIOR.md` records the current intended behavior of the man-page tool.
Treat it as revisable rather than immutable: verify questionable assumptions,
and discuss proposed changes with the user before implementing them.

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
* `fetch_url.rs` — async HTTP fetching with reqwest
* `search_web.rs` — SearXNG web search with reqwest

Implemented:

* MCP lifecycle and tool dispatch
* `man_page`
* `fetch_url`
* `search_web` (local SearXNG instance)
* subprocess timeout for `man_page` (concurrent pipe drain; kill + reap on deadline)
* async Tokio runtime
* unit and end-to-end integration tests

Deferred:

* Process-group kill for `man_page` timeouts (new session via `pre_exec` plus
  `kill(-pgid, SIGKILL)`). `man` forks helper processes that are orphaned on
  timeout; a normal init reaps them, but this container's PID 1 does not, so
  they remain as zombies. Only worth adding if timeouts are expected to fire
  in real use; it would require `unsafe` `pre_exec` and a `libc` dependency.

Current goal:

* Plan the next step together. Candidates:
  - end-to-end integration test for `search_web` (depends on the local SearXNG instance);
  - large-document fetching/storage and summarization (roadmap item 5);
  - polish: `search_web` tool description and config handling.
