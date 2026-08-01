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
- Do not complete an entire feature unless explicitly asked.
- Use the existing source code as evidence of Rust concepts already encountered.

## Roadmap

1. Complete the synchronous man-page tool.
2. Expose it through a minimal MCP server.
3. Introduce async when HTTP work makes it useful.
4. Add web search and webpage fetching.
5. Add concurrency and resource control as needed.
6. Plan the Rust agent separately.

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

## Current Work

Current goal:
- build a minimal stdio MCP server exposing the man-page tool.

Completed (man-page tool):
- basic lookup;
- typed errors and result type;
- configurable output truncation;
- command-line argument construction (`-P cat`, `-s`, `--`);
- input validation (topic and section);
- `Display` impl for `ManError`;
- split into `man_page` module.

Next:
- add `serde`/`serde_json` dependencies;
- define JSON-RPC request/response types;
- implement the stdio dispatch loop;
- handle `initialize`, `tools/list`, `tools/call`.

Deferred:
- subprocess timeout handling (deferred until async/Tokio is introduced for web search).
- output post-processing (ANSI removal, \r\n normalization, trailing-whitespace trim) — tested
  on current system and output from `man -P cat` is clean; can be added later for cross-platform
  robustness.
