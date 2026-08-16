# Behavior Specification: Man Page Tool

> **Status:** This is a living specification of the currently intended behavior.
> It may be revised when testing reveals incorrect assumptions, when the Rust
> implementation suggests a better design, or when the user changes the desired
> functionality.
>
> Features marked as deferred are not requirements for the current implementation.
> When this document conflicts with verified behavior of the target platform,
> update the specification rather than blindly reproducing the mistake.

This document describes the observable behavior of the existing TypeScript
man-page tool (`man-page-tool.ts`) and the intended Rust implementation.

Where the Rust implementation deliberately diverges from the TypeScript tool,
the reason is recorded as a design decision or rationale.

## Current Implementation Scope

The current Rust implementation should provide:

1. lookup of a named man page;
2. an optional man-page section;
3. configurable returned-output limits;
4. a visible truncation marker and `truncated` flag;
5. configurable subprocess timeout;
6. typed domain errors;
7. exit-status handling suitable for Linux `man-db`;
8. terminal-formatting cleanup and line-ending normalization;
9. a dedicated `ManPageResult` return type.

The following are currently deferred:

* search mode;
* client-initiated cancellation;
* fully bounded streaming output collection;
* complete portability across non-`man-db` implementations;
* MCP-specific presentation and error formatting;

---

## 1. Domain API

The lookup function returns a typed result:

```rust
Result<ManPageResult, ManError>
```

The result type is:

```rust
struct ManPageResult {
    content: String,
    truncated: bool,
}
```

`content` contains the cleaned man-page text.

`truncated` is `true` when content was omitted because of the configured output
limit.

### Rationale

A dedicated struct is clearer than returning `(String, bool)` and can be
extended later, for example to report the resolved section.

The domain layer does not return MCP response types or user-facing suggestion
strings. Conversion into MCP content blocks and presentation-oriented error
messages belongs in the future MCP adapter layer.

---

## 2. Inputs

### Topic

The lookup accepts a man-page topic such as:

```text
ls
printf
systemctl
```

The current domain signature is approximately:

```rust
async fn lookup_man_page(
    topic: &str,
    section: Option<&str>,
    config: &ManLookupConfig,
) -> Result<ManPageResult, ManError>
```

### Section

The section is optional.

Examples:

```text
printf
printf(1)
printf(3)
```

When a section is supplied, the lookup applies specifically to that section.

A `NotFound` result means that no page matched the requested combination of
topic and optional section. It does not imply that the implementation checked
whether the same topic exists in some other section.

### Validation

The existing TypeScript tool validates inputs as follows:

#### Topic

* Length: 1–64 characters.
* Pattern: starts with an ASCII letter.
* Remaining characters may contain ASCII letters, digits, `.`, `_`, or `-`.

Equivalent pattern:

```text
^[A-Za-z][A-Za-z0-9._-]*$
```

#### Section

* Optional.
* Exactly one character.
* Allowed values: `1`–`8`, `n`, `p`, or `l`, case-insensitively.

Equivalent pattern:

```text
^[1-8npl]$
```

### Current decision

Input validation may be implemented after the core subprocess behavior is
working.

The subprocess must still be invoked without a shell. Argument separation and
`--` must be used so that input cannot be interpreted as command-line options
to `man`.

---

## 3. Command Invocation

The intended command shape is:

```text
man -P cat [-s section] -- topic
```

The optional section is passed with the `-s` flag before `--`.

The subprocess environment may also set:

```text
MANPAGER=cat
```

### Required behavior

* Invoke `man` directly through `std::process::Command`.
* Do not invoke a shell.
* Pass every argument separately.
* Use `--` before the topic.
* Disable interactive paging.
* Capture stdout and stderr.
* Apply the configured timeout.

### Current assumption

`-P cat` and `MANPAGER=cat` disable interactive paging, but they may not by
themselves guarantee completely clean plain text. Actual captured output should
be inspected on the target system before finalizing terminal-format cleanup.

---

## 4. Configuration

The lookup uses configuration containing at least:

```rust
struct ManLookupConfig {
    timeout: Duration,
    max_output_bytes: usize,
}
```

Default values for normal lookup:

| Setting               |    Default |
| --------------------- | ---------: |
| Timeout               | 10 seconds |
| Returned output limit | 8192 bytes |

Search-mode limits are deferred until search mode is implemented.

---

## 5. Output Limiting

The returned man-page content is limited to the configured number of bytes.

When content exceeds the limit:

1. preserve as much valid UTF-8 content as fits;
2. omit the remaining content;
3. append a visible truncation marker;
4. return `truncated: true`.

The marker is currently:

```text
[
... truncated ...
]
```

Its exact surrounding newline behavior should be covered by tests.

If no content is omitted:

```rust
truncated == false
```

### Initial implementation

The initial implementation may collect the subprocess output in full and
truncate it afterward.

This limits the returned result but does not limit peak memory usage while the
subprocess is running.

### Draining output

The stdout and stderr pipes are drained concurrently with the wait for the
child to exit.

Waiting for the child to exit before reading is not safe: the OS pipe buffer
holds 65536 bytes on the current platform, and a child that writes more than
that blocks until the pipe is drained, so a wait-then-read design deadlocks.
Several installed pages exceed the buffer (verified on the current system:
`curl` ~260 KB, `git-config` ~311 KB, `cmake-modules` ~871 KB), so concurrent
draining is a requirement, not an optimization.

### Deferred hardening

A later implementation may collect output incrementally, retain only the
configured number of bytes, and continue draining or otherwise handling the
subprocess pipes safely.

Bounded streaming collection is not required for the initial implementation.

---

## 6. Timeout Handling

The timeout is enforced with the Tokio runtime: the subprocess is spawned
with `tokio::process::Command`, and the wait — together with the concurrent
pipe draining — is wrapped in `tokio::time::timeout`.

The following behavior applies:

The timeout begins after the subprocess has been spawned.

If the process has not completed within the configured duration:

1. terminate the child process;
2. ensure the child is waited on and reaped;
3. clean up owned pipe and process resources;
4. return:

```rust
Err(ManError::Timeout)
```

The implementation must not leave the child running after returning a timeout.
On the current implementation this holds because `tokio::process::Child::kill`
sends SIGKILL on Unix and then waits for the child (reaps it), unlike the
standard library's `Child::kill`, which does not wait.

Note on orphaned helper processes: `man` forks its own helpers (sub-`man`
processes and a groff formatting pipeline). On timeout only the direct child
is killed and reaped; the helpers are orphaned and exit shortly afterwards.
On a normal system, init reaps them. In containers where PID 1 does not reap
(this workspace, where the agent harness is PID 1), each orphan left over
from a timeout remains as a permanent zombie process. This only matters when
a caller shrinks the timeout, since `man` does not approach the 10 second
default in practice. Killing the whole process group (new session via
`pre_exec`, then `kill(-pgid, SIGKILL)`) is the standard remedy and is
deliberately deferred; see PROJECT.md.

If the process exits naturally before the deadline, the timeout path must not
run.

---

## 7. Exit-Status Handling

The initial implementation targets Linux `man-db`.

Documented `man-db` statuses include:

| Status | Meaning                                                   |
| -----: | --------------------------------------------------------- |
|    `0` | Success                                                   |
|    `1` | Usage, syntax, or configuration error                     |
|    `2` | Operational error                                         |
|    `3` | A child process returned a nonzero status                 |
|   `16` | A requested page, file, or keyword did not exist or match |

### Intended mapping

#### Success

Exit status `0`:

* clean stdout;
* apply truncation;
* return `ManPageResult`.

#### Not found

Exit status `16` with no usable stdout:

```rust
Err(ManError::NotFound)
```

#### Other subprocess failure

Any other nonzero status with no usable stdout:

```rust
Err(ManError::SubprocessError {
    exit_code,
    stderr,
})
```

#### Nonzero status with stdout

Do not automatically treat all nonzero exits with stdout as successful.

The correct behavior should be determined through tests and observed `man-db`
behavior. Partial stdout may accompany a real formatter, pager, or operational
failure.

Until verified otherwise, prefer preserving the failure status rather than
silently converting it into success.

### Portability note

Exit statuses may differ for other `man` implementations. If portability is
added later, isolate platform-specific status interpretation.

---

## 8. Output Post-Processing

Before returning successful stdout:

1. decode according to the chosen invalid-UTF-8 policy;
2. normalize Windows-style line endings:

   * replace `\r\n` with `\n`;
3. remove terminal formatting that interferes with plain-text use;
4. trim trailing whitespace from the end of the complete output;
5. apply or finalize truncation according to the selected processing order.

### Terminal-formatting cleanup

The TypeScript implementation removes ANSI CSI sequences approximately matching:

```text
\u001b\[[0-9;]*[A-Za-z]
```

The Rust implementation should not treat that pattern as a complete definition
of plain-text cleanup.

Captured man-page output may also contain backspace-based overstriking used for
bold or underlined text.

The final cleanup behavior should be based on actual observed subprocess output
on the target system.

**Verification result (current system, `man-db` with `man -P cat`):**
Output is clean — no ANSI escape sequences, no backspace overstriking,
no carriage returns, no trailing whitespace. Verified across multiple pages
(`ls`, `man`, `grep`, `colordiff`).

Post-processing (ANSI removal, `\r\n` normalization, trailing-whitespace trim)
is optional and can be added later for cross-platform robustness if needed.

Possible cleanup mechanisms include:

* ANSI CSI removal;
* backspace and overstrike cleanup;
* appropriate `man` or `groff` options;
* piping through a plain-text formatter such as `col -b`, if justified.

Avoid adding unnecessary subprocesses before verifying that they are needed.

---

## 9. Error Types

The domain error type currently contains:

```rust
enum ManError {
    NotFound,
    SpawnError {
        message: String,
    },
    SubprocessError {
        exit_code: Option<i32>,
        stderr: String,
    },
    Timeout,
}
```

### Variant meanings

| Variant           | Meaning                                                  |
| ----------------- | -------------------------------------------------------- |
| `NotFound`        | No page matched the requested topic and optional section |
| `SpawnError`      | The operating system could not start `man`               |
| `SubprocessError` | `man` started but failed                                 |
| `Timeout`         | The process exceeded the configured deadline             |

### Current design decision

`NotFound` carries no topic or section fields because the immediate caller
already knows what it requested.

Context may be added later if errors need to travel independently or format
their own messages away from the call site.

### Presentation-layer behavior

Helpful suggestions such as:

```text
Try `man -k ...`
```

do not belong in the domain function.

The future MCP adapter may convert `ManError::NotFound` into a more helpful
user-facing tool response.

---

## 10. Deterministic Tests

Unit tests should cover behavior that does not require the host’s installed man
pages where practical.

Likely deterministic test targets include:

* configuration defaults;
* argument construction;
* optional-section handling;
* input validation;
* output truncation;
* UTF-8 boundary handling;
* truncation-marker behavior;
* line-ending normalization;
* terminal-formatting cleanup;
* exit-status interpretation;
* error construction and mapping.

Subprocess abstraction should not be introduced prematurely solely to make every
error path mockable.

Pure processing and interpretation logic may be extracted into helper functions
and tested directly.

---

## 11. Integration and Smoke Tests

Tests invoking the real `man` command are environment-dependent and should be
treated as integration or smoke tests.

They may depend on:

* Linux;
* `man-db` being installed;
* particular pages being installed;
* locale and formatter configuration;
* host-specific output.

Useful smoke cases include:

* a commonly available page such as `man`;
* a deliberately nonexistent topic;
* an explicit section;
* captured output inspection for terminal formatting;
* exit-status verification;
* a forced timeout (a millisecond-scale deadline) returning `ManError::Timeout`;
* a page larger than the OS pipe buffer (e.g. `curl`) succeeding without
  deadlocking.

Tests should clearly distinguish assumptions about the host environment from
portable domain behavior.

---

## 12. Search Mode — Deferred

The TypeScript tool supports an optional query and context-line count.

Intended eventual behavior:

1. fetch a larger portion of the man page;
2. split it into lines;
3. locate case-insensitive matches;
4. include configurable context above and below each match;
5. mark matching lines;
6. include one-based line numbers;
7. limit accumulated excerpts;
8. indicate truncation;
9. return a useful no-match result.

Historical defaults:

| Setting                      |            Value |
| ---------------------------- | ---------------: |
| Full-page search input limit |     524288 bytes |
| Excerpt-output limit         |       4096 bytes |
| Context lines                |                3 |
| Allowed context range        |             0–50 |
| Query length                 | 1–256 characters |

Illustrative format:

```text
Search results for 'socket' in man page for 'ss':

42: >> matching line
43:     context line
```

Search mode is not part of the current implementation scope.

Its exact API and formatting should be reconsidered when implementation begins
rather than copied mechanically from the TypeScript version.

---

## 13. Cancellation — Deferred

The TypeScript implementation accepts an `AbortSignal`.

If cancellation is already active or becomes active during execution, it
terminates the subprocess and returns an error.

The current synchronous stdio MCP server does not yet provide client-initiated
cancellation.

Cancellation should be reconsidered when the server gains:

* async request handling;
* MCP cancellation support;
* multiple in-flight calls;
* web fetching or other long-running tools.

---

## 14. Deferred or Open Design Questions

The following decisions should be made when they become relevant:

* whether output should eventually be collected with a hard streaming bound;
* exact invalid-UTF-8 behavior;
* final terminal-formatting cleanup method;
* behavior for nonzero exit statuses that also produce stdout;
* whether input validation belongs in the domain API or MCP adapter;
* whether search mode should be part of the same function or a separate API;
* portability beyond Linux `man-db`;
* cancellation behavior;
* whether resolved section information should be included in `ManPageResult`.

These are open design questions, not implicit permission for the agent to choose
a complex solution without discussion.
