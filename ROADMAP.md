# Rust MCP Server – Learning Roadmap

## Current Status

| Field | Value |
|-------|-------|
| Phase reached | 1 (complete) |
| Last worked | 2026-07-27 |
| Open questions | — |
| Notes for next session | Start Phase 2: behavior spec, truncation, timeout |

> Update this table at the end of each working session so a fresh Cline chat
> can resume without guessing where you left off.

---

## Target Audience Profile

- Strong TypeScript and Python background
- Some Haskell and PureScript (algebraic data types, pattern matching)
- Older C/C++ experience (pointers, manual memory management)
- Very little Rust

This roadmap leverages your functional programming background while teaching
Rust's ownership model gradually through concrete code.

---

## Pair-Programming Protocol

The purpose of this project is to learn Rust, not merely to produce a working
MCP server. Unless explicitly asked to implement something directly, use the
following collaboration style:

1. **Do not implement an entire phase at once.** Break each phase into the
   smallest coherent tasks, normally one function, type, test, or design
   decision at a time.

2. **Before editing code, explain the next task.** Describe:

   * what we are trying to accomplish;
   * which Rust concepts it introduces;
   * any important design choices;
   * what files are likely to change.

3. **Give the learner the first opportunity to write meaningful Rust.**
   For small, educational pieces, ask the learner to propose or write the code
   before supplying a complete implementation.

4. **Use guidance rather than hidden completion.** When the learner is stuck,
   provide progressively stronger help:

   * conceptual hint;
   * relevant type signature or API;
   * partial skeleton;
   * complete implementation only when requested or clearly necessary.

5. **Do not silently repair compiler errors.** Show the relevant diagnostic,
   explain what Rust is enforcing, and let the learner suggest a correction
   before editing the code.

6. **No unexplained Rust mechanisms.** In particular, do not introduce
   `.clone()`, `Box`, `Arc`, `Mutex`, explicit lifetime annotations, `'static`
   bounds, trait objects, or async machinery without explaining why the design
   requires them and what simpler alternatives exist.

7. **After each coherent task, pause for review.** Explain:

   * what changed;
   * why the code compiles;
   * how ownership and borrowing behave;
   * what tests demonstrate;
   * whether the result is idiomatic or intentionally simplified.

8. **Do not commit automatically unless asked.** Suggest an appropriate commit
   boundary and message, but allow the learner to inspect the diff first.

9. **Update Current Status only at meaningful stopping points.** Do not mark a
   phase complete until its code and learning goals have both been reviewed.

Phase 0 scaffolding is exempt from these rules because it contains little
meaningful Rust. Later phases should follow this protocol by default.

---

## Guiding Principles

1. **Concrete before abstract** – Real working code before interfaces or generics.
2. **Sync before async** – Core logic has no inherent concurrency needs.
3. **Simple borrowing early** – `&str` as a function parameter is not the same
   as designing multi-input lifetime signatures. Introduce references naturally;
   defer only nontrivial lifetime elision and explicit annotations.
4. **Domain before protocol** – Man-lookup logic stays independent of MCP types.
5. **One hard concept per phase** – Each phase introduces at most one difficult
   Rust idea.
6. **Tests that don't need the network or man pages** – Deterministic unit tests
   are primary; calling the real `man` command is an integration or smoke test.

---

## Phase 0 – Project Scaffolding

**Deliverable:** A Cargo project (edition 2024) that compiles and runs a trivial
`main()` printing "hello", with one passing unit test.

**Rust Concepts Introduced:**
- `cargo init` and the generated layout
- `Cargo.toml` structure: name, version, **edition = "2024"**
- `src/main.rs` as the binary entry point
- `cargo run`, `cargo build`, `cargo test`
- The `#[test]` attribute

**Completion Criteria:**
- [ ] `cargo run` prints output and exits cleanly
- [ ] `cargo test` runs at least one test that passes
- [ ] Edition 2024 is set in `Cargo.toml`
- [ ] Initial git commit made

**Can Defer:** Workspaces, `[lib]` + `[bin]` crates, custom profiles, feature
flags, build scripts, `.cargo/config.toml`.

---

## Phase 1 – Rust Fundamentals Through a Man-Page Pager

**Deliverable:** A pure-Rust function with signature roughly

```
fn lookup_man_page(topic: &str, section: Option<&str>) -> Result<String, ManError>
```

that returns plain text from a man page. The function takes borrowed string
slices (`&str`) and an optional section, returning an owned `String` on success
or a typed error on failure.

**Rust Concepts Introduced:**
- Functions with borrowed parameters (`&str`) vs. owned return values (`String`)
  — this is the most common Rust pattern and requires no explicit lifetimes
- `enum` as algebraic data types for errors
- Pattern matching with `match` and `if let`
- `Result<T, E>` and the `?` operator
- `std::process::Command` for spawning subprocesses
- Basic module structure (`mod`, `use`, `pub`)

**Completion Criteria:**
- [ ] Function compiles, takes `&str` params, returns `Result<String, ManError>`
- [ ] Error enum has at least `NotFound` and `SubprocessError` variants
- [ ] Deterministic unit tests cover error paths without calling real `man`
- [ ] One smoke test calls the function with a known topic (e.g., `"ls"`)

**Can Defer:** Reading `.gz` files directly, custom `std::error::Error` impls,
scanning man paths manually. Using `Command` to shell out is fine for now.

---

## Phase 2 – Behaving Like the Existing TypeScript Tool

**Deliverable:** A documented behavior spec (in code as comments or a separate
`.md` file) capturing the observable behavior of the existing TypeScript man-page
tool, plus Rust code that reproduces it.

**What to Document and Reproduce:**
- Output limiting (truncation at N characters, elision marker)
- Timeout handling (kill subprocess after T seconds, return distinct error)
- Process exit codes (distinguish "page not found" from "man crashed")
- Error message format returned to the caller
- Section resolution behavior (default section, explicit section, ambiguous)

**Rust Concepts Introduced:**
- Struct design for configuration (timeout, max output bytes)
- Method impl blocks (`impl ManLookupConfig { ... }`)
- Borrowing struct fields in method calls
- Introducing `std::time::Duration` and timeout patterns with Command

**Completion Criteria:**
- [ ] Behavior spec written (can be a comment block or `BEHAVIOR.md`)
- [ ] Output is truncated to a configurable limit with an elision marker
- [ ] Subprocess timeout is enforced and produces a distinct error variant
- [ ] Exit codes are distinguished in error variants
- [ ] Unit tests verify truncation, config defaults, and error construction
- [ ] Smoke tests confirm real `man` calls match documented behavior

**Can Defer:** Signal-based timeout (using a thread + channel or just letting
`Command` output grow is acceptable initially). The key learning goal is
capturing *what* the TypeScript tool does before worrying about *how fast*.

---

## Phase 3 – JSON Serialization with Serde

**Deliverable:** The domain result from Phase 2 serializes to JSON. No MCP types
yet — just verifying that the man-lookup result and error types round-trip
cleanly through `serde_json`.

**Rust Concepts Introduced:**
- Adding external crates (`serde`, `serde_json`) to `Cargo.toml`
- Derive macros: `#[derive(Serialize, Deserialize, Debug, Clone)]`
- Serde attributes: `#[serde(rename_all = "snake_case")]`, skip fields
- The difference between serializing a struct vs. an enum
- Test modules with `#[cfg(test)]`

**Completion Criteria:**
- [ ] Domain result type serializes to a known JSON shape
- [ ] Error type serializes (or a derived error message does)
- [ ] Unit tests assert exact JSON output for success and error cases
- [ ] Deserialization round-trip tested for the result type

**Can Defer:** `thiserror`, `anyhow`, custom `Serializer`/`Deserializer` impls,
validated input schemas. Standard library + serde derives are enough.

---

## Phase 4 – JSON-RPC 2.0 Framing Layer

**Deliverable:** A module that reads JSON-RPC 2.0 messages from stdin and writes
JSON-RPC 2.0 responses to stdout, one message per line. This phase handles only
the *framing* — parsing the envelope, correlating IDs, and formatting responses.
No MCP semantics yet.

**Rust Concepts Introduced:**
- `std::io::{stdin, stdout, BufRead, Write, BufReader}`
- Reading lines with `BufReader::lines()`
- Writing with `stdout.write_all()` and flushing
- `serde_json::Value` for unstructured JSON routing
- Distinguishing JSON-RPC **requests** (have `id`), **responses** (have `id`),
  and **notifications** (no `id`) at the envelope level

**JSON-RPC 2.0 Envelope Types:**
```
Request:  { jsonrpc: "2.0", method: string, params?: object, id: number|null }
Response: { jsonrpc: "2.0", result?: object, error?: object, id: number|null }
Notification: Request object with no id member
```

**Completion Criteria:**
- [ ] Can parse a JSON-RPC request line from stdin
- [ ] Can format a JSON-RPC success response and write to stdout
- [ ] Can format a JSON-RPC error response with proper error codes
- [ ] Notifications (no id) are recognized and not answered
- [ ] Unit tests use string inputs/outputs, no real I/O needed

**Can Defer:** Batch requests, `$rpc` params, progress streaming, precise
JSON-RPC error code mapping. A generic error wrapper is fine initially.

---

## Phase 5 – MCP Lifecycle and Capabilities

**Deliverable**: The server correctly handles the MCP handshake and lifecycle: the initialize request, the notifications/initialized notification, and clean process termination when stdin reaches EOF.

**Rust Concepts Introduced:**
- State tracking with an enum (`Uninitialized`, `Initialized`, `Ready`)
- Matching on method strings to route requests
- Introducing the `Tool` concept as a plain struct (not yet a trait) with
  name, description, and input schema fields

**MCP Lifecycle:**
1. Client sends `initialize` request with protocol version and capabilities
2. Server responds with its own version and capabilities (declaring `tools`)
3. Client sends `initialized` notification (no response expected)
4. Server is now ready to accept tool calls

**Completion Criteria:**
- [ ] `initialize` request returns server info + tool capabilities
- [ ] `initialized` notification is logged/acknowledged but not answered
- [ ] Requests before initialization are rejected with an error
- [ ] State transitions are tested (unit tests mock the I/O)

**Can Defer:** Sampling, logging, resource subscriptions, pagination. Focus on
the minimal lifecycle needed for tool support.

---

## Phase 6 – Wiring Domain Logic into MCP Tool Dispatch

**Deliverable:** The man-lookup domain function from Phase 2 is adapted into an
MCP tool. The MCP layer receives a `tools/call` request, extracts parameters,
calls the domain function, and wraps the result (or error) in an MCP-compatible
JSON-RPC response.

**Key Architecture Point:** The domain layer (`man_lookup`) knows nothing about
MCP. The MCP adapter layer imports the domain result and maps it to MCP content
blocks. This separation keeps both layers independently testable.

**Rust Concepts Introduced:**
- Adapter pattern: a function that converts `ManResult` → `Vec<ContentBlock>`
- Extracting typed parameters from `serde_json::Value` using `get()` and
  `as_str()`
- Error mapping: domain errors become MCP tool errors with appropriate messages
- Traits in practice: implementing `std::fmt::Display` for error types

**Completion Criteria:**
- [ ] `tools/list` returns the man-lookup tool with name, description, schema
- [ ] `tools/call` with valid params returns man-page text as a content block
- [ ] `tools/call` with invalid params returns a proper error
- [ ] Domain errors map to human-readable MCP error content
- [ ] Unit tests cover adapter mapping without calling real `man`
- [ ] End-to-end smoke test: pipe JSON-RPC messages via stdin, verify stdout

**Can Defer:** Parallel tool calls, per-tool timeouts beyond the domain layer,
structured error codes in MCP responses.

---

## Phase 7 – Review and Refactoring Checkpoint

**Deliverable:** Cleaned-up code with idiomatic error handling, documentation,
and clippy compliance. No new features — only quality improvements.

**Rust Concepts Introduced:**
- `thiserror` crate for derive-based error types
- Doc comments (`///`) and `cargo doc`
- `cargo clippy` and addressing warnings
- Refactoring with test coverage as a safety net

**Completion Criteria:**
- [ ] `cargo clippy` produces no warnings
- [ ] Error types use `thiserror` derive
- [ ] Public functions and types have doc comments
- [ ] `cargo doc --open` renders cleanly
- [ ] Module boundaries reviewed: domain vs. adapter vs. transport
- [ ] All tests still pass

---

## Phase 8 – Comparing Dispatch Strategies

**Deliverable:** A concrete comparison of four ways to dispatch tool calls in
Rust, using the existing man-lookup tool as the test case. No "right answer" is
presumed — the goal is understanding trade-offs.

**Strategies to Compare:**
1. **Direct `match` on tool name** – Simple string matching, no indirection.
2. **Tool enum** – An `enum Tool { ManLookup }` that grows with each new tool,
   dispatched via match on the enum variant.
3. **Generics with `impl Trait`** – Static dispatch using generic functions
   where tools implement a common trait.
4. **`Box<dyn Trait>`** – Dynamic dispatch with a vector of trait objects,
   enabling registration without modifying dispatch code.

**Rust Concepts Introduced (Consolidated):**
- Traits as Rust's interface mechanism
- Static vs. dynamic dispatch and when each matters
- `Sized` bound and why `T: Sized` is default
- `Box<dyn Trait>` and vtables
- Generic monomorphization vs. virtual dispatch

**Completion Criteria:**
- [ ] All four strategies implemented (can coexist in test code)
- [ ] Write-up (in `ROADMAP.md` or a separate note) documenting trade-offs:
  compile time, runtime overhead, extensibility, code clarity
- [ ] No strategy is declared "winner" — the choice depends on project needs

**Can Defer:** Plugin systems, dynamic loading, reflection-like patterns. This
phase is about understanding, not committing.

---

## Phase 9 – Introducing Async (If Concurrency Is Needed)

**Deliverable:** An async version of the stdio server using Tokio, IF nonblocking
behavior is actually needed.

**When Async Makes Sense:**
- The server must handle multiple in-flight requests concurrently
- Tool calls need cancellation support (e.g., client aborts mid-execution)
- Future integration with an HTTP-based agent that already uses async I/O
- Multiple tools run in parallel and results are aggregated

**Important Clarification:** Async does NOT make individual subprocesses faster.
A `man` call takes as long as it takes. Async enables *nonblocking concurrency*
— while one tool runs, the server can accept and begin processing another
request. It also enables clean cancellation when a client gives up waiting.

**Rust Concepts Introduced:**
- `tokio` runtime and `#[tokio::main]`
- `async fn` and `.await` propagation
- `tokio::process::Command` for async subprocess execution
- `tokio::io::{AsyncBufReadExt, AsyncWriteExt}` for async stdin/stdout
- The `Send` trait (auto-derived, but important for understanding task boundaries)

**Completion Criteria:**
- [ ] Server handles stdin/stdout asynchronously
- [ ] Subprocess calls use `tokio::process::Command`
- [ ] Existing tests still pass (or are ported to async)
- [ ] No `Arc<Mutex<...>>` introduced unless genuinely needed

**Can Defer:** `tokio::spawn`, task channels (`mpsc`), custom executors,
`Future` impls by hand, `Arc`/`Mutex`/`RwLock`. The stdio server is
single-client and doesn't need shared mutable state.

---

## Concept Introduction Summary

| Concept | Phase | Why Then? |
|---------|-------|-----------|
| Cargo, edition 2024 | 0 | Bootstrapping |
| Functions, `&str` borrowing, `String` returns | 1 | Natural param/return pattern |
| Enums, match, Result, `?` | 1 | Error handling from day one |
| `std::process::Command` | 1 | Subprocess spawning |
| Structs, impl blocks, Duration | 2 | Config and timeout behavior |
| Serde, derive macros | 3 | JSON serialization |
| Stdin/stdout I/O traits | 4 | JSON-RPC framing |
| `serde_json::Value` routing | 4 | Semi-structured envelope parsing |
| State machine enum | 5 | MCP lifecycle tracking |
| Tool description struct (trait-free) | 5 | Declaring capabilities |
| Adapter pattern, error mapping | 6 | Domain-to-MCP bridge |
| `thiserror`, clippy, cargo doc | 7 | Quality checkpoint |
| Traits: match / enum / generics / dyn | 8 | Understanding dispatch trade-offs |
| Async, tokio (conditional) | 9 | Concurrency and cancellation |

---

## Concepts Deliberately Deferred

These are NOT needed for a simple MCP tool server:

- **`Arc`, `Mutex`, `RwLock`** – Single-client stdio has no shared state needs.
- **Channels (`mpsc`, `oneshot`)** – No internal parallelism yet.
- **Nontrivial lifetime design** – Simple `&str` params use elision rules;
  multi-input/output lifetime annotations come only if the design demands them.
- **`unsafe` Rust** – Never needed for this project.
- **Build scripts (`build.rs`)** – Overkill.
- **Procedural macros** – Derive macros suffice.
- **FFI with C libraries** – The `man` binary is available directly.
- **No-std or embedded patterns** – Irrelevant.

---

## The Rust Agent: A Separate Project

The agent (a loop that calls tools based on queries, possibly via an LLM API)
is out of scope for this roadmap. It will require its own roadmap covering:

- HTTP client patterns in Rust (`reqwest`)
- Prompt templating and response parsing
- Tool-calling loops with termination conditions
- Possibly a different transport (SSE/HTTP instead of stdio)

When ready, start a new repository or branch and reference the completed MCP
server as a dependency or example.

---

## How to Resume This Roadmap in a Fresh Chat

1. Read the **Current Status** table at the top.
2. Check `git log` or file structure to confirm the phase.
3. Ask "I'm on Phase N — what's the next step?"
4. Each phase is self-contained with deliverables and completion criteria.
5. Deterministic unit tests should pass before advancing.

---

## Reasoning Behind the Ordering

1. **Cargo before code** – Understanding the build system prevents confusion
   when adding dependencies in Phase 3.

2. **Domain logic before protocol** – The man-lookup function is the hardest
   domain logic. Getting it right in isolation (Phases 1–2) teaches Rust without
   protocol complexity.

3. **Behavior spec before implementation details** – Phase 2 explicitly
   documents what the TypeScript tool does before replicating it. This prevents
   cargo-culting and ensures test coverage matches real requirements.

4. **JSON serialization before I/O** – Serde (Phase 3) teaches type system and
   macros with offline unit tests. No streaming pressure.

5. **JSON-RPC framing before MCP semantics** – Phase 4 handles the envelope
   (requests vs. responses vs. notifications) independently of MCP lifecycle.
   This separates wire-format concerns from protocol-state concerns.

6. **MCP lifecycle before tool dispatch** – Phase 5 establishes the state machine
   (initialize → initialized → ready). Phase 6 then wires tools into an already-
   correct protocol shell.

7. **Domain and MCP layers stay separate** – The man-lookup function never
   imports MCP types. An adapter converts between them. This keeps both
   independently testable and prevents type leakage.

8. **Review checkpoint before comparing strategies** – Phase 7 cleans up
   accumulated technical debt. Phase 8 then explores alternatives on a solid
   foundation rather than compounding early mistakes.

9. **Dispatch comparison is exploratory, not prescriptive** – Phase 8 shows all
   four approaches without declaring a winner. The choice depends on whether you
   add more tools, need plugins, or care about compile time.

10. **Async is conditional and correctly motivated** – Phase 9 triggers on
    concurrency needs (cancellation, parallel tools, HTTP agent integration),
    NOT on making subprocesses faster.