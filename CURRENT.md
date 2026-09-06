# Current Task State

Read before starting work; update at step boundaries. As the task moves
on, rotate this file; settled design moves to DESIGN.md.

## Task: summarize_doc — step 2 (N == 1) [IN PROGRESS]

Step 1 (done; assistant-implemented on explicit delegation):
`src/summarize_doc.rs` skeleton — `SummarizeArgs {id, query?, offset?,
limit?, max_words?}`, DESCRIPTION (large-doc workflow — summarize, then
`triage_doc` with the summary as `context` — + cost warning + a failed
chunk fails the whole call), `tool_definition` (required `["id"]`;
max_words documents default 400 / cap 4000), `MAX_CHUNKS = 64` +
`prepare` copied from triage (over-cap message "one summary"), stub
`handle_call` (parse args — bad shape → `invalid_params`; then
`Ok(error_message_json("summarize_doc is not implemented yet"))`).
Server wired (7th tool; dispatch arm; unit `tools_list_returns_tools`
6→7 + name assert; integration `tools_list` count 6→7; stub test). Mock
helpers moved to `llm.rs` as `#[cfg(test)] pub(crate)`. Three temporary
dead-code warnings (SummarizeArgs fields never read, `MAX_CHUNKS`,
`prepare` unused) drop when step 2 reads the args and chunks the doc.

Step 2 spec (N == 1 — the doc/subset fits in one chunk):

* One line-numbered `chat()` call (body construction as in triage:
  1-based right-justified numbers + tab + `split_inclusive` lines), with
  a summarize system prompt (not triage's), returning `{"summary"
  (≤max_words), "map" (≤10: {line_start, line_end, label})}`.
* Convert map line spans → absolute doc bytes via `Chunk.line_starts`
  (one lookup per line; no running line counter — output is byte spans
  only).
* Pinned render: header `Summary of bytes a..b, N chunk(s)`; optional
  `Query: <verbatim, own line(s)>` (omit when no query); `SUMMARY`
  section; `MAP` of `   1. [bytes a..b] label` (3-space indent).
* Call-site `LlmConfig`: temperature 0.2 (overrides `LLAMA_TEMPERATURE`;
  env knob stays for manual experiments); `max_tokens = 2*max_words + 150`
  (650 at the default 400); timeout stays base + 10 ms/token.
* `max_words` default 400, clamped 1..=4000; empty-string query
  normalization (`Option::filter`) lands here with the rest of the arg
  handling.
* Mock test (canned body; pinned render).
* Full contract (N>1 map + reduce phases, budget rule A, validation
  split, failure semantics): DESIGN.md → summarize_doc.
* User implements by default (step 1 was delegated; each round is the
  user's call).

## Test state

* 78 unit + 13 integration, all green (new model default, no env
  override). Binary `target/debug/toolbox` current (7 tools).
* Warnings: 1 pre-existing (`constant INTERNAL_ERROR is never used`,
  `src/mcp.rs`) + the 3 temporary summarize dead-code ones.
* Known flake: `man_page::tests::lookup_times_out` (1 ms-timeout race
  under full-suite parallel load; passes standalone and on re-run;
  pre-existing, unrelated).

## Engine & endpoint (live facts)

* `http://172.17.0.1:8081/v1` — currently **ninfer**, model
  `qwen3.8-27b` (NVFP4), max_model_len ~190,000. The user swaps
  quantizations/engines: check `/v1/models`; set `LLAMA_MODEL` when the
  default is stale (a stale id 404s the live integration test
  `tools_call_triage_doc_success`). Old engine (available): llama.cpp,
  `qwen3.8-27b-q4xl` (GGUF Q4_K_M, ctx 200,192), same URL.
* ninfer: decode ~171 tok/s with MTP speculative decoding (~91%
  acceptance); serial (one request running at a time); queue timeout 30 s
  → clean HTTP 503 for any waiting request. Measured: a 262 KB KJV chunk
  = 102,113 prompt tokens (chunk 10, with overlap: 108,137). Both engines
  tokenize the identical payload to the identical count (108,137 on
  ninfer AND on llama.cpp Q4_K_M) — so this is the model's real token
  rate (~2.9 bytes/token for this text), not a ninfer-specific thing; the
  old-era "256KB ≈ 40–55k tokens" was a rough estimate, not a
  measurement. Budget math: 102k prompt + 64k max_tokens = 166k < 190k
  context — thinner margin than the old math suggested; keep in mind for
  summarize budgets.
* One-server topology (user-confirmed): the agent harness and the
  toolbox's triage/summarize calls all hit this one model. Consequences:
  (1) from inside the agent session, launch long LLM jobs as ONE
  BLOCKING bash call — never `nohup &` (while blocked, the harness sends
  no chat requests, so the engine is exclusive to the job; `nohup &`
  makes the harness a competing client → 30-s-timeout 503s in both
  directions). The user's own terminal is cleanest of all. (2) The
  assistant's quality judgments on tool output are ASYMMETRIC
  self-evaluation: the harness runs the same weights at temp 1.0 with
  thinking (far larger reasoning budget → good at catching local errors:
  junk, missing landmarks, corruption), but shared weights = shared
  priors → systematic family biases (score-calibration style) stay
  invisible and are the user's judgment; the judge is nondeterministic
  while the examined output is deterministic. (3) The tools' temp 0 /
  thinking-off config is right for tools: reproducibility + predictable
  latency/context (xhigh thinking on a 102k-token chunk would break both
  and would invalidate the prompt-experiment corpus).

## triage_doc current state

* Shipped: prompt v5d + guardrail (one-line output-discipline line;
  incident + evidence: DESIGN.md → Findings). v6 full KJV run: 15 m 03 s,
  1000 hits (capped), 0 untriaged (20 chunks); the formerly degenerate
  span 2293947..2556131 yielded 86 hits. Score calibration accepted under
  the stability rule (v6 {10:94, 9:300, 8:560, 7:46} vs v5 {10:15, 9:279,
  8:577, 7:129} — ~100 regions up 2–3 tiers; nothing in the
  investigate/concerning band). Artifacts: `kjv_animals_triage{,_v2,_v3,
  _v4,_v5,_v6}.txt` (v6 = current baseline); raw run JSONs in /tmp
  (`kjv_triage_vN.json`).

## Experimental tooling (fast prompt A/B loop)

* `experiments/`: `battery.py` (chunk-10 variant battery), `controls.py`
  (chunk-0 pair + repeats), `probe.py <model> [--url] [--out]` (re-runs
  the exact chunk-10 v0/v2 payloads against any engine — used for the
  Q4_K_M probe); artifacts: `chunk10_exact.json` (byte-exact
  full-run prompt payload), `chunk10_degenerate.json` (the 2,624-token
  signature), `chunk10_healthy_105.json`, `kjv_id.txt`, `battery/`
  (results.json, results_controls.json, per-run payloads/responses),
  `probe_q4xl_r{1,2}/`.
* A ~55–120 s curl of an exact chunk prompt ≈ 20× the speed of a full
  run; use it to vet prompt/quant changes before a 15-minute run.
  Rebuild recipe: mirror `chunk.rs` in BYTE space (the KJV file is CRLF
  and multi-byte UTF-8 — 4,455,950 bytes vs 4,451,854 chars; a char-space
  line table drifts); line table = [0] + the byte after every 0x0a (+
  total); `snap_forward(x)` = first entry ≥ x, `snap_back(x)` = last
  entry ≤ x; CH=262144, overlap=CH/8; NO trailing empty numbered line
  (`split_inclusive` emits none — a Python `re.split` phantom line was
  the 5-token perturbation that flipped the degenerate output to a
  healthy one). Chunk 10 params: s=2293947, e=2556131,
  exclusive=2326744, context_lines=1012, n_lines=7001, width=4.
* KJV doc: `data/<id>` (id in `experiments/kjv_id.txt`; 4,455,950 bytes).
  Invocation pattern: printf `initialize` / `notifications/initialized` /
  `tools/call` → `./target/debug/toolbox`; reusable bench script
  `/tmp/run_triage_bench.sh` (identical v3/v4/v5/v6 workload: full doc,
  query "all mentions of animals", max_hits 1000).

## Deferred / open

* summarize_doc steps 3–5: N>1 map phase → reduce phase → docs +
  small-doc end-to-end + 20-chunk KJV run (blocking call or user's
  terminal — never `nohup &` from inside the session).
* Store-and-preview for oversized triage output (fetch_url pattern).
* Context-window-relative `max_words` ceiling (4000 clamp kept for v1).
* No tail overlap in the chunker (trigger: hits systematically missed at
  boundaries).
* Streaming reads for multi-GB docs (roadmap 6).
* If the old engine (Q4_K_M) is used again: reconsider the guardrail line
  for it (it collapses that quant's score ladder to all-10s).
