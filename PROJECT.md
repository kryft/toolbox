# Rust MCP Project

## Goal

Learn Rust by building:

1. a man-page tool;
2. a minimal MCP server exposing it;
3. a web-search tool with async HTTP and page fetching;
4. later, a separate Rust agent.

## Working Style

Assume the user is an experienced programmer who is new to Rust.

- Work on one small coherent task at a time.
- Plan the next task together after reviewing the current code.
- You should let me do the task unless I tell you to do it.
- Don't give me the whole source code for a task unless I asked for it or it's very short.
- Do not edit Rust source or tests unless I explicitly ask you to implement the change.
- After diagnosing a problem, explain the cause and propose the smallest fix first.
- Even for trivial or mechanical changes, stop and let me decide whether to implement them.
- You may edit project documentation when I explicitly ask for documentation updates.
- Explain Rust-specific design choices and compiler errors.
- Answer questions and tangents directly before returning to implementation.
- Prefer the simplest design that fits the current requirement.
- Do not introduce advanced mechanisms such as explicit lifetimes, trait
  objects, async, `Arc`, `Mutex`, or complex generics unless the current
  problem genuinely benefits from them; explain the need first.
- Use the existing source code as evidence of Rust concepts already encountered.

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

- `docs/mcp/2025-11-25/SUMMARY.md` for a project-focused overview
- the relevant vendored specification page in `docs/mcp/2025-11-25/`
- `docs/mcp/2025-11-25/schema.ts` when exact field shapes are unclear

`SUMMARY.md` is generated guidance and may be incomplete or mistaken. The
vendored specification and schema are authoritative.

## Local Rust references

The container includes the `rust-docs` and `rust-src` rustup components.

When exact Rust API behavior or signatures matter, prefer checking the local
official documentation rather than relying on model memory:

- Documentation root: `rustup doc --path`
- Toolchain root: `rustc --print sysroot`
- Standard-library source:
  `$(rustc --print sysroot)/lib/rustlib/src/rust/library`

For Rust crate API questions, prefer local documentation when available. Run cargo doc and inspect target/doc/ for the versions actually used by the project. Use rustup doc for standard-library/toolchain documentation.

Search or render only the relevant files; do not load the full documentation
into context unnecessarily.

You can locate a specific standard library item with something like:

```bash
docs_root="$(dirname "$(rustup doc --path)")"
find "$docs_root/std" -iname '*child*'
rg -n 'try_wait|wait_with_output' "$docs_root/std"
```

You can inspect source with e.g.

```bash
rg -n 'pub fn try_wait' \
  "$(rustc --print sysroot)/lib/rustlib/src/rust/library"
```

You can turn a specific HTML page into readeble text with

```bash
w3m -dump \
  "$(dirname "$(rustup doc --path)")/std/process/struct.Child.html"
```

## Current State

Architecture:

- `main.rs` — async stdio loop
- `server.rs` — MCP request routing
- `mcp.rs` — JSON-RPC/MCP protocol types and shared helpers
- `man_page.rs` — man-page tool
- `fetch_url.rs` — async HTTP fetching with reqwest

Implemented:

- MCP lifecycle and tool dispatch
- `man_page`
- `fetch_url`
- async Tokio runtime
- unit and end-to-end integration tests

Current goal:

- Add `search_web` using the local SearXNG instance.

Deferred:

- subprocess timeout handling