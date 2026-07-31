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
- Let the user write meaningful new Rust code first.
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

## Current Work

Current goal:
- reproduce the existing TypeScript man-page behavior in Rust.

Completed:
- basic lookup;
- typed errors and result type;
- configurable output truncation.

Next:
- fix command-line argument construction (`-P cat`, `--`, section placement).

Deferred:
- subprocess timeout handling (deferred until async/Tokio is introduced for web search).
