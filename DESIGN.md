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
  exclusive on the wire). Lenient `extract_json()` (strip code fences, then
  the first brace-balanced top-level object via a string/escape-aware
  brace-depth scan — a `first { .. last }` slice breaks when the model
  writes prose before the object or extra text after it, e.g. a duplicated
  object: serde rejects the trailing characters; both observed live).
  `#[cfg(test)] pub(crate)` `start_mock_llm` /
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
  summarize adds optional `max_words` and `max_map_entries`.

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

* Phases are uniform in N — no N == 1 special case (settled step 2):
  every run is map phase → reduce phase (two calls: story editor, then
  map editor, split 2026-09-26 — see the reduce bullet), even when the
  subset fits one chunk. Rationale: the writer/editor prompt split holds
  at every N — the map prompt tells the writer to include borderline
  material because "a later editor assembles the final story and prunes";
  an N == 1 direct output would have to make those global-importance
  calls in the same pass. The map prompt is one template: the
  earlier-summaries block and the overlap note are per-chunk facts
  (absent at chunk 1: no past, `context_lines == 0`), not N-conditional.
* Map phase (sequential; one line-numbered `chat()` per chunk): call N's
  input = query + S1..S(N-1) verbatim (N > 1 only) + line-numbered chunk
  N + overlap note (N > 1 only) + metaknowledge (a
  later editor assembles the final story and prunes; the summarizer sees
  the past but not the future → include borderline material rather than
  guessing at global importance). S_N = `{"summary" (≤P words),
  "pointers" (≤5: {line_start, line_end, label ≤10 words}, in line
  order)}` — query-aware, scope: this chunk only, written with earlier
  summaries in view (consistent terminology). The pointers are a
  FILTER, not a ranking (2026-09-26, user decision): the writer judges
  each region on whether it is worth flagging and lists them in scan
  order — no local importance comparison (contradicts the include-
  borderline-material instruction, and the model's natural emission
  order is line order anyway; the "most important first" wording
  produced doc-order emissions that violated the contract on
  enumerative queries — A/B, Findings). Over-emission (>5) is cut by
  `take(5)` to the earliest five in the array — contract-conformant
  under line order. The REDUCE map is in byte order too (2026-09-26,
  user decision, replacing an earlier "most important first" call): a
  consistently document-ordered map beats an inconsistently
  prioritized one — a reader can scan it like a phonebook — and with
  thinking off the model can't be trusted to rank (a correct ordering
  is a comparison/sort workload); the reduce's candidate pool is
  already document-ordered (chunks in order, each chunk's pointers in
  line order), so byte-order output is a selection from an ordered
  list, not a re-sort. `take(map_budget)` keeps the leading entries on
  over-emission (the front of the document; observed runs never
  over-emit the map — the 10-entry and 40-entry runs hit the cap
  exactly). The noquery2 run (no query, 40 entries) showed the prompt
  alone can't deliver this: the map ended at Nehemiah 1 — 43% of the
  doc — with two local order inversions and a duplicated region
  ("in byte order" read as "walk the candidates in order, stop at 40").
  So: (1) the tool stable-sorts the map entries by start before render
  — byte order is guaranteed by construction, not by the model;
  (2) the prompt carries an explicit whole-document selection line
  ("choose the best candidates from across the entire document, not
  just the earliest chunks"); (3) "merging candidates that overlap or
  describe the same passage" (the duplicated Goliath region was the
  ignored merge instruction). (2) and (3) alone proved insufficient —
  noquery3 still ended at 60% — which is what motivated the reduce split
  (below): the map is now a focused call whose whole task is selection,
  with a per-chunk quota and a thinking budget. The S-block is one line
  per earlier
  chunk, `[bytes a..b] <summary>` (span for orientation; the summary
  verbatim, no pointers — the writer needs earlier terminology, not
  earlier spans); degraded chunks are skipped. A logical unit straddling
  the seam is
  summarized by this, the later part, flagged as a continuation; seam
  pointers may extend into the leading overlap — validation clamps to the
  whole chunk text, not the exclusive zone (contrast triage, which
  hard-drops out-of-zone starts). S_N is fixed once generated, never
  re-compressed. Pointers are chunk-relative line spans; the tool
  converts them to absolute doc bytes via `Chunk.line_starts` (one
  lookup per line; no running line counter — the output carries byte
  spans only). At N == 1 the reduce's map is at most the single chunk's
  ≤5 pointers (the map-budget ceiling only bites when merging from
  multiple chunks).
* Reduce phase (2026-09-26 split; two focused calls instead of one
  combined editor): the combined reduce made the map the secondary job —
  on the no-query runs the model covered the whole canon in the story
  prose while the map walked the candidates in document order and
  stopped at the budget (noquery2: 43%, noquery3: 60%, after the
  whole-document line and the tool-side sort).
  * Story editor (first): input = query + the contributions block
    (per-chunk `[bytes a..b]` + Summary + Pointers, degraded list — the
    same block the old reduce saw); output = `{"summary"}` (≤max_words,
    coherent whole-document story from the query's perspective). The
    seam note (overlap ≈1/8; seam units owned by the later part) is
    story-specific and present only when N > 1. Salvage applies here
    (the map phase's `SALVAGE_MIN_WORDS` prose rule): the prose-only
    "essay mode" is the story's INTENDED output, so a prose-only essay
    IS the summary — in the map phase salvage patches that mode; here it
    is the point. A parsed object with an empty summary, or no salvage
    after `ATTEMPTS`, fails the whole call (the story is the call's
    reason to exist — unlike the map, there is no fallback).
  * Map editor (second): input = the SAME contributions block (deliberately
    NOT the story — the map maps the document, not the prose; no
    anchoring); output = `{"map"}` (≤map_budget: {start, end, label},
    select/merge from the candidates only, spans kept exactly as given).
    The prompt is a focused selection task with a PER-CHUNK QUOTA (the
    spread mechanism; A/B evidence in Findings): "aim for exactly
    {budget} entries — about {budget/n} per chunk (the document has n
    chunks below)" when budget ≥ n (the default budget is 2·N), else a
    whole-document spread clause; plus "this is a selection, not a
    transcription" and the merge line. `MAP_EDIT_REASONING` =
    `Some("low")` — thinking is ON for this call only, per call via
    `llm::chat_reasoning` (the config's / env's global knob is ignored
    for the call, so triage's calibration corpus and every other call
    stay thinking-off). The finding that made this load-bearing: with
    thinking OFF the model cannot keep a per-chunk count over a long
    generation (a 40-entry quota got filled by a document-order walk at
    48% coverage); with thinking LOW + the per-chunk quota it produced
    exactly 2 per chunk, 100% coverage, 0 overlaps. Off-contract
    (parseable but no `map` array) counts as a failed attempt. After
    `ATTEMPTS` without an on-contract response, the tool falls back to
    the MECHANICAL map (`fallback_map`: first pointer of each
    non-degraded chunk, budget-clamped, byte-ordered by construction) —
    the run still returns a story + a navigable map (the noquery4 run
    proved this live: all three map-editor attempts truncated at the
    old cap and the fallback carried the run). The tool stable-sorts by
    start after parsing regardless (byte order is a tool guarantee; the
    fallback is sorted for one code path). Both exchanges are dumped on
    success (`/tmp/summarize_dump_story.txt`,
    `/tmp/summarize_dump_mapedit.txt`) so the reduce input can be
    replayed through prompt A/B without a live run (the fast loop that
    found the quota + thinking config).
  * The reduce exists because it is the only role with a whole-document
    view: document-level judgments, list→narrative composition, fixed
    output budget, map selection. Asymmetry note: triage keeps the hard
    exclusive-zone rule because it has no editor pass to reconcile
    duplication.
* Budgets: `max_words` arg (default 400, clamped 1..=4000 — a
  context-window-relative ceiling is deferred) = the story budget.
  Per-chunk P = max(250, ceil(2*max_words/N)) (rule A: the reduce always
  has ≥2× the story budget of material; small N → bigger writer
  budgets). Final map size: `max_map_entries` arg (optional, clamped
  1..=100), default max(10, 2·N) — two entries per chunk, a document map
  at per-chunk granularity (N = 20 → 40, N = 64 → 128 → 100; the floor
  keeps small runs at the familiar 10); the per-chunk writer stays at
  ≤5 pointers (a writer-budget matter), so a larger budget gives the
  editor more candidates to select or merge from. `max_tokens =
  4×(phase word budget) + 150` for the map (1150 at defaults with N ≥ 2,
  where P floors at 250; N == 1 gives P = 2·max_words, so the single map
  call is 8·max_words + 150 = 3350 at the default 400),
  `4·max_words + 150` for the story editor (1750 at defaults), and
  `150 + 80·map_budget + MAP_EDIT_THINK_HEADROOM` for the map editor
  (19,734 at the N = 20 budget of 40 with thinking on; the headroom is
  16,384 and present only when `MAP_EDIT_REASONING` is on). 80/entry is
  the 4× house convention over the old 20/entry compact coefficient —
  which under-sized the live run: pretty-printed entries MEASURE at
  ~48 tok/entry (noquery4 replay), and at 950 (150 + 20·40) all three
  map-editor attempts truncated byte-identically. Thinking tokens count
  against the map editor's cap (measured 4361/6237 on the 20-chunk A/B
  arms, ≈10.8k on the live noquery5 attempt 1 — its content truncated
  at 26/40 entries at the old 11,542 cap; the headroom was raised
  8,192 → 16,384 = ~1.5× the live worst case; a thinking truncation
  then costs one failed attempt, absorbed by the retry). Temperature 0.2 set on the `LlmConfig` at
  summarize's
  call sites (overrides `LLAMA_TEMPERATURE`; the env knob stays for
  manual experiments). Timeout stays base + 10 ms/token. Output budgets
  are independent of the doc's char→token rate: 4× covers ~1.3
  tokens/word of generated text + JSON scaffolding + pretty-printing
  (the model indents its JSON) plus the observed pointer over-production
  — the first KJV run truncated two map calls at 2× (`EOF while parsing
  a list`, the pointer array left unclosed), and the measured worst
  chunk (197-word summary + 11 pointers against the "at most 5"
  contract) is 716 tokens at P = 250 (probe, 2026-09-26). The cap costs
  ~nothing: it only bounds decode (paid only when written), and the
  context fit below keeps its margin.
* Prompt-side context fit (recomputed step 2 from the measured token
  rate — the original 256 KB ≈ 40–55k-token estimate was ~2× optimistic;
  KJV-class text incl. line numbers measures ~108k prompt tokens per
  262 KB chunk, identical on both engines, ≈2.9 bytes/token): worst map
  call ≈ 108k chunk + S-block (≤ (N−1)·P words = 15,750 at N = 64,
  P = 250 ≈ 20–30k tokens) + scaffolding + 3,350 max_tokens ≈ 136–141k
  < 262k live max_model_len (2026-09-10, NVFP4 + NVFP4 KV; the
  2026-09-07 analysis was against 190k — NVFP4 + FP8 KV — margin ~30%);
  the reduce calls (same contributions block ≈10k tokens + 1,750 story /
  19,734 map-editor max_tokens) fit with wide margin. Residual risk, reduced: token-dense
  text (~2 bytes/token:
  minified JSON/code) erodes the worst map call to ~180k — now fits with
  ~31% margin; only text denser than ~1.4 bytes/token (base64-class)
  would cross 262k. The clean failure mode (engine context error → chunk
  failure naming span + reason) remains the backstop; the fixes are
  `LLAMA_CHUNK_BYTES` and/or capping the S-block (roadmap 6).
* Render (pinned by test): header `Summary of bytes a..b, N chunk(s)`;
  optional `Query: <verbatim, own line(s)>` (multi-sentence queries
  expected); a `SUMMARY` section (label deliberately not STORY); a `MAP`
  of `   1. [bytes a..b] label` entries (3-space indent).
* Validation (lenient): pointer/map counts enforced (extras dropped);
  word budgets left to prompt + max_tokens; parseable-but-off-contract
  JSON → empty contribution ("" summary, no pointers) with the chunk
  flagged to the editor. Unparseable (or prose-only) response → retried
  up to `ATTEMPTS` (3) times, then, if any attempt's prose before the
  first `{` has ≥ `SALVAGE_MIN_WORDS` (25) words, that prose (longest
  across attempts) is salvaged as the chunk's summary — no pointers, not
  flagged degraded: the editor composes from it like a normal summary,
  only the map loses the chunk's candidates (the prose-only "essay
  mode" on dense thematic chunks is a stable attractor retry cannot
  break, and the prose is a good summary missing only the JSON); below
  the threshold the whole call errors, naming the chunk's byte span +
  reason. The story editor: salvage applies (the prose-only essay is its
  intended output); a parsed object with an empty summary, or no salvage
  after `ATTEMPTS`, fails the whole call. The map editor: NO salvage (its
  prose is garbage) and off-contract (parseable but no `map` array) is a
  failed attempt; after `ATTEMPTS` the tool falls back to the mechanical
  whole-document map (first pointer per non-degraded chunk) and the run
  still succeeds. A `chat()` transport error fails immediately (no
  retry). Every failed map/story/map-editor attempt dumps the full
  exchange (system prompt, user prompt incl. the S-block / contributions
  block, raw response) to `/tmp/summarize_fail_*.txt` so a failure is
  diagnosable without re-running the chunk (the probe can't: it omits the
  S-block), and the story + map-editor calls dump on SUCCESS too
  (`/tmp/summarize_dump_*.txt`) for prompt A/B replay. Failure semantics:
  any chunk or story failure → the whole call errors, naming the chunk's
  byte span + reason; a map-editor failure degrades to the fallback map.
  All contributions empty (every S_N parseable-but-off-contract; at N ==
  1: S₁ empty with no pointers) → the call errors before the reduce,
  naming each chunk's span + reason — no editor call on nothing.
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
* Output discipline in summarize map calls (2026-09-26, KJV live run +
  probes): the model writes a ~900–950-char PROSE PREAMBLE before the
  JSON object on some chunks — systematic (byte-identical across temp-0
  repeats on the dense NT chunk; observed on the Jesus-query and the
  no-query variants, absent on the animals-query chunk) — and in one
  live no-query map call wrote a SECOND OBJECT after the first (serde:
  "trailing characters"; the old first-{..last-} slice spanned both).
  Both are non-fatal: the preamble carries no braces, `extract_json`
  now takes the first complete object (string/escape-aware depth scan),
  and the S-block/reduce only ever see the parsed `summary` field — no
  downstream contamination. The cost is wasted decode tokens, bounded
  by the 4× cap. Same family as the triage guardrail finding, but the
  map prompt already carries the strong "exactly one JSON object" line
  and the model does it anyway — a duplicate guardrail line is not
  assumed to suppress it (uninvestigated).
* Map-editor selection under the no-query runs (2026-09-26, noquery4 +
  noquery5 + replay A/B on the captured reduce input — the
  success/failure dumps made this a ~30-s loop instead of 11-min runs):
  (1) The 950 cap (150 + 20·40, the compact 20/entry coefficient) was
  under-sized: pretty-printed entries measure ~48 tok/entry, so all
  three map-editor attempts truncated byte-identically at 950 tokens
  (2196 bytes, mid-label) and the mechanical fallback carried the run
  (20 entries = first pointer per chunk, 98% span; the run succeeded —
  the fallback path proved live). (2) At a 3350 cap the model did NOT
  select: it transcribed the candidates (99 in, 71 out, truncated at
  74%) — "up to 40" is not a selection constraint without a query to
  give importance an axis. (3) Quota wording ("exactly 40, about 2 per
  chunk" or "exactly 40, spread across the document"), thinking OFF:
  the quota was obeyed (41 entries, finish=stop, ascending) but coverage
  was 48% — the per-chunk count cannot be kept over a long generation
  without thinking; the walk fills the quota in document order (the same
  bookkeeping limit as the noquery2 order inversions). (4) Per-chunk
  quota + `reasoning_effort: low`: exactly 2 per chunk, every chunk, 100%
  coverage, 0 overlaps, 31 s (4361 thinking tokens) — ADOPTED
  (`MAP_EDIT_REASONING` on for this call only; the per-call override in
  `llm::chat_reasoning` keeps every other caller thinking-off).
  Live noquery5 (thinking on): 40 entries, strictly ascending (the tool
  sort was a no-op), 0 duplicates/overlaps, 100% span (Genesis 1 →
  Revelation), 703 s — but the selection went global, not uniform: chunk
  0 took 4 entries while chunks 8–9 (2 Chronicles 36/Ezra–Nehemiah and
  Job/early Psalms) took 0 — "about 2 per chunk" read loosely; defensible
  (chunk 7 already maps the fall of Judah, chunk 8's top candidate the
  same event in a later book) but the map's contents gap at Job. Open:
  whether the quota wording should be stricter ("exactly 2, no chunk
  more, no chunk fewer") — a quality call (uniform coverage vs density
  on dense chunks), the user's. Residual risk: a thinking run can
  consume the headroom and truncate the content (observed live: noquery5
  attempt 1 thought ~10.8k tokens and its content cut at 26/40 entries
  at the old 11,542 cap; retry recovered). The headroom was raised to
  16,384 (2026-09-26, ~1.5× that observation; Qwen 3.8 27B "can think
  quite a lot") — a truncation now costs one failed attempt only beyond
  that.

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
