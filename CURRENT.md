# Current Task State

Read before starting work; update at step boundaries. As the task moves
on, rotate this file; settled design moves to DESIGN.md.

## Task: none in progress (rotated 2026-09-27)

The summarize_doc task (roadmap 5c) completed 2026-09-27 — tiers 0–3
are all done and A/B-validated on the 4.4 MB KJV doc. This file held
its full progress; on rotation the durable parts moved to DESIGN.md:

* Engine & endpoint live facts (endpoint/quant, measured rates, APC
  scope, one-server topology rules) → DESIGN.md → "Engine & endpoint
  (live facts)".
* triage_doc shipped state (v6 baseline, v7/v7r config check,
  not-bit-reproducible finding) → DESIGN.md → "triage_doc — shipped
  state".
* Experimental tooling (A/B loop, bench scripts, KJV doc, chunker
  rebuild recipe, artifact inventory) → DESIGN.md → "Experimental
  tooling (prompt A/B loop)".
* Deferred/open items → DESIGN.md → "Deferred / open".
* summarize_doc contracts + the A/B trail (guardrail shape anchor,
  reduce split, quota/thinking A/B) → DESIGN.md → "summarize_doc" +
  "Findings".

Next candidates (roadmap in PROJECT.md): 6 — concurrency/resource
control as needed (batch fetching, politeness, long-running jobs),
informed by the tier-3 agent-loop findings; 7 — plan the Rust agent
separately.

## Test state

* 116 unit + 13 integration, all green (2026-09-27, new model default,
  no env override); binary `target/debug/toolbox` current (7 tools).
* 1 pre-existing warning (`INTERNAL_ERROR` never used, `src/mcp.rs`).
* Known flake: `man_page::tests::lookup_times_out` (1 ms-timeout race
  under full-suite parallel load; passes standalone and on re-run).
