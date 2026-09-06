# Design Notes

Rationale, settled decisions, known limitations. The code is the source of
truth for *how* things work; this file is the *why*. Consult when it
matters; no need to keep it in context otherwise.

## Shared tier-3 machinery

* `llm.rs`: `LlmConfig` from env — `LLAMA_URL`, `LLAMA_MODEL`,
  `LLAMA_TEMPERATURE`, `LLAMA_REASONING_EFFORT` (chunk bytes come from
  `LLAMA_CHUNK_BYTES`, read by `chunk.rs`). One-shot `chat()` under a
  per-call `tokio::time::timeout` = base 60 s + 10 ms per `max_tokens`
  token (base covers prefill + connection; 10 ms/tok ≈ 100 tok/s, below
  measured decode). `reasoning_effort: None` → `chat_template_kwargs:
  {enable_thinking: false}`; `Some(e)` → `reasoning_effort: e` (mutually
  exclusive on the wire). Lenient `extract_json()` (strip code fences,
  first `{` .. last `}`). `#[cfg(test)] pub(crate)` `start_mock_llm` /
  `mock_config` live at module level here, shared by tool tests.
* `chunk.rs`: chunks of `chunk_bytes` (default 256 KB) snapped to line
  boundaries, constant 1/8 overlap. Chunk N's exclusive zone = all bytes
  after the inherited overlap; exclusive zones tile the scanned range
  exactly (every byte in exactly one chunk — no duplicates, no gaps).
  `Chunk<'a>` borrows the doc (no per-chunk clones): `start` / `end`
  (line-aligned byte offsets in the decoded doc), `exclusive_start`
  (== `start` for chunk 0, else previous chunk's end), `text`,
  `line_starts` (absolute doc byte offset of each line — a slice of one
  doc-level table), `context_lines` (lines before `exclusive_start`; 0
  for chunk 0). Design points not obvious from the code:
  * chunk 0's end also forward-snaps (a raw cut there would break the
    tail<overlap case and leave chunk 0 ending mid-line);
  * starts snap back, ends snap forward (snap-back on the end would
    shrink the overlap and could drop bytes out of every chunk's view);
  * `limit` = max bytes to scan from `from` (`None` = to doc end) —
    deliberately NOT a small default window: a small window on a *search*
    tool yields misleading "no hits" false negatives; the 64-chunk cap is
    the protection instead;
  * user offsets may land mid-character in a valid UTF-8 doc — never
    slice the `str` at a user offset (backward scan on `doc.as_bytes()`;
    floor `range_end` with `floor_char_boundary`, the safe side);
  * tail < overlap → final chunk starts deep in the previous one and
    carries just the tail; last line may lack a trailing `\n` (a line's
    span is `[line_start, next_line_start)`, `range_end` for the final
    line);
  * no tail overlap (deferred): the owner of a straddling region doesn't
    see its post-cut tail; mitigated by a "continues" flag in the note +
    reading with margin. Trigger to add: hits systematically missed at
    boundaries. Adding it later doesn't touch the exclusive-zone
    invariant.
* The doc is decoded once with `String::from_utf8_lossy`; for valid UTF-8
  (the norm for web docs) offsets are identical to the raw bytes that
  `read_doc` / `search_doc` take (v1 consistency decision).
* Hard cap 64 chunks on both LLM tools; over-cap → error pointing at
  offset/limit.
* Unified args `{id, query, offset?, limit?}` — query required for
  triage, optional for summarize; triage adds optional `context`,
  summarize adds optional `max_words`.

## triage_doc

Contract:

* Per-chunk model output: `{"regions": [{line_start, line_end, score,
  note}]}` — ≤`max_hits` regions, 1-based lines within the chunk.
  Exclusive-zone rule (prompt): report only regions that *start* in the
  exclusive zone; out-of-zone or invalid model output is dropped
  tool-side.
* `max_hits` optional (default 5, clamped 1..=1000): one knob governs both
  the per-chunk prompt cap (model reports ≤ N regions per chunk) and the
  global top-N. A low fixed per-chunk cap would silently truncate dense
  chunks before global ranking. `max_tokens = 128 + 64 * max_hits`.
  ~450 rendered chars per hit, so N is a caller context budget, not a
  capability cap; the 64-chunk scan cap and the 1000 knob are the only
  hard limits.
* Chunk text is line-numbered in the prompt (1-based, right-justified,
  tab-separated; ~5–6% token overhead) — without it the model cannot
  report line numbers reliably, and exclusive-zone validation and snippet
  placement both depend on them. Doc-absolute line numbers come from a
  running newline counter (`lines_before`) — triage's output carries doc
  line numbers (contrast summarize: byte spans only, so no counter).
* Hits are rendered tool-side: score + note + location (byte + line span)
  + verbatim ~256-char snippet starting at the region-start line,
  line-bounded. The LLM returns pointers + metadata only; the snippet is
  a mechanical slice (no text relay → no drift). Zero matches is a normal
  (non-error) result.
* Failure semantics: chat/parse failure → chunk reported untriaged (byte
  span + reason), scan continues; "0 hit(s)" is only honest when every
  chunk was triaged. Parseable-but-off-contract JSON (e.g. missing
  `regions` key) → no hits (lenient v1).
* Per-chunk calls are stateless — no relay between chunks
  (parallelizable; roadmap 6). Optional call-level `context` (e.g. a
  `summarize_doc` story, a glossary of self-defined terms) is prepended to
  every chunk's prompt as rough orientation; the chunk text is
  authoritative. No size cap (v1); the tool description conveys the
  intent.
* Determinism is a feature: temperature 0, thinking off (see Findings).
* Output format (pinned by tests): header `Triage for 'QUERY': N hit(s),
  scanned M chunk(s), bytes a..b` (QUERY verbatim); one
  `   untriaged: bytes a..b (reason)` line per untriaged chunk (scan
  order) when any; then hits as `1. [score] note` / `   line ls..le,
  bytes a..b` / `   |`-prefixed snippet — continuation lines share the
  3-space indent.
* The prompt template lives in `fn system_prompt(exclusive_line,
  max_hits)` — `format!` requires a literal at the call site, so a const
  template is not possible; the contract is pinned by a test.

Current prompt (v5d + guardrail): neutral opening ("Find the parts of
this document chunk that are relevant to the query"), a classify line
(mention query vs theme query), branch-scoped qualifying gates, a breadth
line (query asks for "possibly / tangentially" relevant → include
peripheral bearings), anti-padding rules (cap-not-target; an explicit
zero-region license; padding is wrong), the orientation-only context
clause (overlap lines are for orientation; never report them and never
use them to skip content), the score line (1–10 ordinal directness, not
calibrated confidence; 1 is still a real passing mention), note ≤15
words descriptive, and the guardrail line (a PLAIN line, no bullet — the
exact form the A/B battery validated): output must be exactly one
complete, syntactically valid JSON object, no reasoning/deliberation in
the output. The phrasing contract (mention vs theme phrasing; bare
abstract noun phrases can return 0) is documented in DESCRIPTION.

Score semantics (settled reading):

* A coarse directness ladder, not a graded-relevance spectrum. Healthy
  runs work in ~7–10 (7 = metaphor/implicit, 8 = explicit but background,
  9 = prominent, 10 = central). 1–6 are dormant except score 1, which
  the model uses as a self-flag for junk it included out of caution (a
  1 hit = the model saying "ignore me").
* The query controls the qualifying gate (whether it opens at all) and
  the tail's edge (peripheral mentions admitted at 7), not the score band.
* Scoring-stability decision rule (user-adopted): drift of the model's
  implicit scoring function w.r.t. seemingly-irrelevant prompt details is
  tracked PER REGION: ±1–2 tiers = margin of error; ~5 tiers (10→5) =
  merits closer investigation; ~7 tiers (10→3) = concerning. Use this to
  vet prompt / quant / engine changes (cheaply, via the probe loop).

Known limitations:

* Bare abstract phrasings of a pervasive subject can return a silent 0
  (indistinguishable from "not in document") — query phrasing is the
  user-facing lever.
* A single shared gate cannot be strict for binary queries and permissive
  for themes: a softened clause (v4 attempt) regressed all controls — the
  model reads added ambiguity as a reason to report *less* (anti-padding
  primes conservatism).
* Oversized output (N = 1000 → hundreds of KB) is not stored in v1; the
  DESCRIPTION carries the cost warning. Store-and-preview for oversized
  triage output is a deferred follow-up.

Prompt evolution (condensed — the code has the final form):

1. First KJV run: 36% of the top-1000 were self-admitted "no animals"
   padding → anti-padding relevance rules.
2. v2 KJV run: context-suppression bug — chunks deduped against their own
   overlap context and under-reported continuations (Deut 14 lists:
   0 hits at ctx≈695, twice, vs 74 at ctx=0) → orientation-only clause.
3. v4 attempt: softened relevance clause for theme queries regressed all
   three controls (e.g. descriptions 288→10) → reverted.
4. v5d (adopted): query-adaptive. The opening line is the key variable —
   "contain relevant mentions of the query" primes mention semantics and
   overrides the branch gates; the neutral "are relevant to the query"
   opens them. Mention control paid no cost (210→220); a breadth sweep
   0→120 verified; natural phrasing verified; zero junk.
5. Temperature 0.7 experiment: the gate opens as a lottery (224 hits,
   then 0 on the repeat) → temperature stays 0.
6. Guardrail (2026-09-05): the chunk-10 truncation incident (Findings
   below) → the one-line output-discipline guardrail; validated via the
   A/B battery and the full v6 run (0 untriaged).

## summarize_doc

Contract (settled):

* N == 1 (subset fits in one chunk — the common case): a single
  line-numbered `chat()` call returns `{"summary" (≤max_words), "map"
  (≤10: {line_start, line_end, label})}` directly. Map entries are
  chunk-relative line spans; the tool converts them to absolute doc bytes
  via `Chunk.line_starts` (one lookup per line; no running line counter —
  the output carries byte spans only).
* N > 1, map phase (sequential): call N's input = query + S1..S(N-1)
  verbatim + line-numbered chunk N + overlap note + metaknowledge (a
  later editor assembles the final story and prunes; the summarizer sees
  the past but not the future → include borderline material rather than
  guessing at global importance). S_N = `{"summary" (≤P words),
  "pointers" (≤5: {line_start, line_end, label ≤10 words})}` —
  query-aware, scope: this chunk only, written with earlier summaries in
  view (consistent terminology). A logical unit straddling the seam is
  summarized by this, the later part, flagged as a continuation; seam
  pointers may extend into the leading overlap — validation clamps to the
  whole chunk text, not the exclusive zone (contrast triage, which
  hard-drops out-of-zone starts). S_N is fixed once generated, never
  re-compressed.
* Reduce ("editor") phase: input = query + all S_N with tool-converted
  absolute byte spans + pointers + a degraded-chunk list; output =
  `{"summary" (≤max_words, coherent whole-document story from the query's
  perspective), "map" (≤10: {start, end, label}, selected/merged from the
  per-chunk candidates)}`. The reduce prompt states the seam mechanics
  (overlap ≈1/8; seam units owned by the later part). The reduce exists
  because it is the only role with a whole-document view: document-level
  judgments, list→narrative composition, fixed output budget, map
  selection. Asymmetry note: triage keeps the hard exclusive-zone rule
  because it has no editor pass to reconcile duplication.
* Budgets: `max_words` arg (default 400, clamped 1..=4000 — a
  context-window-relative ceiling is deferred) = the story budget.
  Per-chunk P = max(250, ceil(2*max_words/N)) (rule A: the reduce always
  has ≥2× the story budget of material; small N → bigger writer
  budgets). `max_tokens = 2×(phase word budget) + 150` (650 / 950 at
  defaults). Temperature 0.2 set on the `LlmConfig` at summarize's call
  sites (overrides `LLAMA_TEMPERATURE`; the env knob stays for manual
  experiments). Timeout stays base + 10 ms/token.
* Render (pinned by test): header `Summary of bytes a..b, N chunk(s)`;
  optional `Query: <verbatim, own line(s)>` (multi-sentence queries
  expected); a `SUMMARY` section (label deliberately not STORY); a `MAP`
  of `   1. [bytes a..b] label` entries (3-space indent).
* Validation (lenient): pointer/map counts enforced (extras dropped);
  word budgets left to prompt + max_tokens; parseable-but-off-contract
  JSON → empty contribution ("" summary, no pointers) with the chunk
  flagged to the editor; parse failure → whole call errors. Failure
  semantics: any chunk failure → the whole call errors, naming the
  chunk's byte span + reason.
* Documented large-doc workflow (belongs in the tool description): rough
  overview of the whole → pick spans from the map → re-summarize the
  subset (same budget, less to fit) → `read_doc` for verbatim detail. The
  overview (or a targeted re-summarize, e.g. a glossary) can be handed to
  `triage_doc` as `context`.

## Findings (model/engine behavior under our prompts)

* Determinism: at temperature 0 both engines (ninfer/NVFP4 and
  llama.cpp/Q4_K_M) are bit-reproducible per input (ninfer: v0 5/5, v2
  2/2, c0 2/2; Q4_K_M: 2/2 on both variants). The engine is
  deterministic; the prompt landscape is not flat — sensitive inputs sit
  on knife-edges where a few tokens (or a server restart) flip the
  outcome: a 5-token phantom line flipped a degenerate output to a
  healthy 105-region one; the v4↔v5 same-engine score wobble is best read
  as knife-edge chunks flipping across a restart.
* The triage chunk-10 truncation (2026-09-05, resolved): with thinking
  OFF, the model's selection deliberation leaked in-band into the output
  (the last region's `note` became a deliberation monologue; the model
  emitted EOS mid-string of its own accord at 2,624 tokens, far under the
  64k cap) — a deterministic degenerate attractor of the exact prompt on
  the NVFP4 engine (bit-identical across 5 runs / 3 server sessions) but
  NOT on the Q4_K_M engine (healthy on the identical bytes). Thinking ON
  routes the deliberation to `reasoning_content` but is ~7× slower,
  yields fewer regions, and with the guardrail emitted an EMPTY content —
  not a tool default (still available via `LLAMA_REASONING_EFFORT`). The
  37-token guardrail line fixes both observed failure classes —
  truncation AND markdown-fencing (fencing was a session-dependent
  attractor: unfenced in the v4/v5 server sessions, fenced in later
  sessions) — and its output is deterministic and healthy.
* Quantization sensitivity: identical prompt bytes → different per-engine
  outputs. The guardrail's side effects differ per quant (NVFP4: mild
  ladder widening — v6 {10:94, 9:300, 8:560, 7:46} vs v5 {10:15, 9:279,
  8:577, 7:129}; Q4_K_M: the whole ladder collapsed to all-10s, plus
  selection changes). Any prompt fix validated on one quant must be
  re-validated on another via the probe loop before trusting. If the old
  engine is used again, the guardrail line should be reconsidered for it.
* Region-set quality of the guardrail change: no junk added; mostly finer
  re-segmentation of multi-animal verses; production omissions few and
  minor (one real: Jer 2:20 moles/bats; two borderline: Prov 12:27
  "hunting", Ps 106:19 golden calf); net line coverage slightly up (233
  vs 230).
* Some behavior is stable across quants even when calibration isn't (the
  12 context-zone regions on KJV chunk 10 appear on both engines).

## Long-running behavior

* Calls are sequential (politeness to a single local server; parallelism
  is roadmap 6). The stdio loop blocks for the duration of a call
  (accepted for v1; revisit under roadmap 6).

## Test conventions

* Unit: chunker (exclusive zones tile, starts line-aligned, `context_lines`
  correct, tail<overlap, mid-line `from` snap, limit Some/None,
  max_chunks, C<8 exact tiling, empty range), `extract_json`, output
  formatting, span clamping.
* Integration: local mock LLM (TcpListener + canned JSON bodies); core
  functions take an explicit `LlmConfig` so tests can point at the mock.
  `chat()` builds a fresh `reqwest::Client` per call → one TCP connection
  per chunk → the mock must loop `listener.incoming()` and serve one
  canned body per connection. `start_mock_llm` / `mock_config` are
  `#[cfg(test)] pub(crate)` in `llm.rs`.
