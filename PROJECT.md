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

## Local Rust references

The container includes the `rust-docs` and `rust-src` rustup components.

When exact Rust API behavior or signatures matter, prefer checking the local
official documentation rather than relying on model memory:

- Documentation root: `rustup doc --path`
- Toolchain root: `rustc --print sysroot`
- Standard-library source:
  `$(rustc --print sysroot)/lib/rustlib/src/rust/library`

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

## Current Work

Current goal:
- tests and refactor complete; next phase is async / web search.

Completed (man-page tool):
- basic lookup;
- typed errors and result type;
- configurable output truncation;
- command-line argument construction (`-P cat`, `-s`, `--`);
- input validation (topic and section);
- `Display` impl for `ManError`;
- split into `man_page` module.

Completed (MCP server):
- `serde`/`serde_json` dependencies;
- JSON-RPC request/response/error types in `mcp` module;
- message parsing (`parse_message`) with request/notification dispatch;
- stdio dispatch loop (read lines, parse, handle, write responses);
- `initialize` handler (protocol version, capabilities, server info);
- `tools/list` handler (man_page tool definition);
- `tools/call` handler (argument extraction, man page lookup, error mapping);
- `truncated` flag included in tool call response;
- full lifecycle tested end-to-end (initialize → initialized → tools/list → tools/call);
- integration tests (`tests/integration.rs`) — spawn binary, pipe JSON-RPC messages.

Completed (refactor):
- `mcp.rs` — protocol types and `parse_message` only;
- `man_page.rs` — lookup logic + MCP adapter (`tool_definition`, `handle_call`);
- `server.rs` — request routing (`initialize`, `tools/list`, `tools/call` dispatch);
- `main.rs` — stdio loop + `write_response` helper only.

Completed (tests):
- unit tests for `parse_message` (valid requests, notifications, invalid JSON, missing fields, edge cases);
- unit tests for `handle_request` (initialize, tools/list, tools/call, unknown method);
- unit tests for man_page (validation, lookup, error display);
- integration tests (`tests/integration.rs`) — spawn binary, pipe JSON-RPC messages.

Next:
- (tests and refactor complete; ready for async / web search)

Deferred:
- subprocess timeout handling (deferred until async/Tokio is introduced for web search).
- output post-processing (ANSI removal, \r\n normalization, trailing-whitespace trim) — tested
  on current system and output from `man -P cat` is clean; can be added later for cross-platform
  robustness.
