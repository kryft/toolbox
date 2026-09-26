# Current Task State

Read before starting work; update at step boundaries. As the task moves
on, rotate this file; settled design moves to DESIGN.md.

## Task: summarize_doc — step 4 (docs + 20-chunk KJV live run) [IN PROGRESS]

Status: step 3 DONE (2026-09-24, assistant-implemented under the
default-implement style); the code is complete after step 3, so step 4
is docs + live validation, not more code. Landed: the map loop threads
the S-block — `earlier_entry` (one line per earlier chunk: `[bytes
a..b] <summary>`, span for orientation, summary verbatim, no pointers;
degraded chunks skipped) accumulated in `summarize()`'s map loop and
passed to `map_call` (None for the first chunk), so the S-block header
("Earlier chunk summaries (verbatim):") is pinned for real. New tests:
`earlier_entry_pins_the_s_block_line` (incl. a multi-line summary
staying verbatim), `multi_chunk_run_threads_earlier_summaries` (N=2
end-to-end render pin on a 330-byte / 2-chunk doc; success also pins
the map, map, reduce call sequence),
`mid_run_degraded_chunk_fails_nothing_and_reaches_the_reduce`. Judgment
call recorded: the S-block entry format (span + summary line, no
pointers — the writer needs earlier terminology, not earlier spans;
contrast the reduce input, which carries them); DESIGN.md's map bullet
records it. Suite: 98 unit + 13 integration, green. Not yet committed
(HEAD 58953aa = step 1; step 2 + the 2026-09-24 review fix + step 3 +
the whole step-4 robustness batch + the 2026-09-26 editor split are
all uncommitted — see open question 1).

Step 4 (next): docs + the 20-chunk KJV live run — the first multi-chunk
live validation (step 2's live smoke was the 559-byte, N=1 example.com
doc). Run: one blocking call or the user's terminal — never `nohup &`
from inside the session; ~14 min on the current engine (full per-chunk
prefill; APC gives no multi-chunk speedup). Then close out roadmap 5c
and commit.

Step 4 progress (2026-09-26, post-reboot): the first 20-chunk KJV live
run FAILED as designed (fail-fast): the map phase truncated at the
per-chunk token cap. Root cause, now quantified by probe: the model
over-produces pointers — on the chunk at bytes 917582..1179805
(Joshua–1 Samuel, query "all mentions of animals") it wrote a 197-word
summary + 11 pointers (the prompt says at most 5) = 716 completion
tokens, over the old cap max_tokens = 2p+150 = 650 (p = 250 at n = 20,
max_words 400) → EOF mid-pointer-array → extract_json failed → the call
failed naming the chunk; the code comment records two verbose chunks
truncated at the 2× cap across the first-run attempts (the second
chunk's span was not recorded). Fix (uncommitted; 3× interim raised to
4× on 2026-09-26, user-approved after cost analysis): map AND reduce
max_tokens raised 2× → 4× (4p+150 = 1150 / 4*max_words+150 = 1750 at
defaults; the cap costs ~nothing — it only bounds decode, paid only
when written, the timeout already scales +10 ms/token, and the
prompt+max_tokens context fit keeps its margin — DESIGN.md budget +
context-fit bullets updated). Probe measured prompt_tokens 99,643 for
this chunk (worst map call ≈101k ≈ 39% of the 262k window). Probe
validation (temporary #[ignore] probe_raw_map_response — now a raw
reqwest call mirroring llm::chat exactly so the usage block is visible;
temp 0 / thinking off, the run itself is temp 0.2): the same chunk at
the 900 cap returned a complete parseable object —
completion_tokens 716, 20% headroom (61% at the 1150 cap). User's
cost model verified 2026-09-26: the summary-size consumers are the
reduce input (all summaries+pointers ≤ ~26k tokens at n = 64, P = 250)
and the S-block threaded into later map calls (≤ 20–30k tokens) —
trivial vs the 262k window at v1's n ≤ 64 / P ≤ 2·max_words range.
Second probe run also confirmed
APC alive post-reboot (cached_tokens 99,636 on the identical repeat),
and the two temp-0 repeats differ by ~1 byte / 2058 — consistent with
the not-bit-reproducible finding (APC state confound), no new
information. DEFERRED FINDING (prompt-calibration call, after the live
run): the 11 pointers come back in DOCUMENT order, not "most important
first" as contracted, so the lenient take(5) kept the 5 earliest
(Joshua/Judges) and dropped the Samson/lion + David/lion-and-bear
pointers — the chunk's most substantial animal passages. The step-2
lenient-parse leniency showing its cost; and it is "most important
first" (array order = importance) that makes the take(5) truncation
safe. Options on the table: tighten the pointer line in the map prompt
(pinned by test; A/B via the probe), raise the per-chunk pointer cap to
match the model's natural behavior, or accept the leniency.

Word fix A/B done (2026-09-26, probe, animal chunk, same 900 cap):
reworked the pointer line ("a shortlist ... this is a ranking of what
matters most, not an inventory in document order — if you found more
than 5, keep only the 5 that matter most"), kept — it caps the over-
production (11→5 pointers, 716→615 tokens, no truncation). But on this
ENUMERATIVE query the ordering is still document order (ascending lines,
ptrs 1-2 near-duplicate) and still drops the Samson/lion + David
lion-and-bear headlines; the summary PROSE (228 words) covers them all —
so the story is fine, only the map is weak here. Consistent with the
query-misuse theory: "all mentions" has no importance axis, so the
ranking instruction has nothing to grip. Verdict: keep the wording
(crash class gone, harmless), it is NOT a ranking fix for enumerative
queries (that's triage_doc). Open: (a) probe a FOCUS query (Jesus/God)
on a dense chunk to confirm the map ranks where intended; (b) add one
line to DESCRIPTION steering enumerative "find all X" to triage_doc.
A/B artifacts: /tmp/kjv_map_probe_oldwording.txt (A), /tmp/kjv_map_probe.txt
(B).

Focus-query probe (2026-09-26): "the life of Jesus" on dense NT chunk
bytes 3,441,088..3,703,302 (Matthew 8-28 + all Mark + Luke 1-11), cap
1150. Selection is RIGHT: 5 pointers = the chunk's most important Jesus
regions (Last Supper/Gethsemane/arrest/trial, Crucifixion+Burial,
Resurrection+Great Commission [Matthew], Mark ministry-to-resurrection,
Luke birth/childhood), no padding; 164-word summary comprehensive. BUT
the model wrote a ~950-char PROSE PREAMBLE before the JSON (violates the
"no text before or after" line) — and it is SYSTEMATIC: two identical
temp-0 runs are byte-for-byte identical (same preamble, same pointers).
Impact: NON-FATAL (extract_json jumps to the first `{`; the preamble has
no braces) and NON-CONTAMINATING (the preamble is discarded; the S-block
+ reduce only see the parsed summary field). Cost: ~150-200 wasted
tokens per affected chunk (bounded by the 1150 cap; low truncation
risk). Same output-discipline class as the triage guardrail incident,
but the map prompt already carries the strong "exactly one JSON object"
line and the model still preambles on this dense narrative chunk — so a
duplicate guardrail line may not suppress it (the triage guardrail was
session-dependent). The animals chunk (B arm) had NO preamble, so it is
chunk/query-specific. DECISION PENDING: proceed to the live run as-is
(non-fatal, non-contaminating, bounded cost; the run reveals the
cross-chunk frequency; a preamble-induced truncation is the trigger for
a guardrail/cap fix) vs. investigate the preamble first (re-probe more
chunks/queries to estimate frequency, and/or A/B a stronger guardrail).
Artifacts: /tmp/kjv_map_probe_jesus_chunk15.txt (+ _r2, byte-identical).

Live run 1 (no query, 2026-09-26): FAILED at chunk bytes
1147022..1409188 with serde "trailing characters at line 1 column
1458" — a NEW failure mode: the model wrote a complete JSON object
followed by MORE text containing braces (a duplicated object is the
suspected shape; the temp-0 probe of that chunk at cap 1150 returned a
healthy single object with a 902-char preamble and 8 pointers, so the
exact live response at temp 0.2 was not reproduced — non-bit-
reproducible config). Root cause: extract_json's "first { .. last }"
slice spans object + trailing garbage → serde rejects. FIX (committed-
pending, same style as the lenient parse): extract_json now locates
the FIRST COMPLETE object via a string/escape-aware brace-depth scan
(`first_complete_object` in llm.rs); handles preamble, trailing prose
with braces, and duplicated objects; 6 new llm tests (104 unit total,
green); DESIGN.md shared-machinery line + a Findings entry updated
(both live incidents: the systematic ~900–950-char preamble — observed
on the Jesus-query and no-query variants, absent on animals; and the
post-object extra content). The run's own failure semantics worked as
designed: the error named the chunk span, and the probe quantified the
shape. (All uncommitted; suite 106 unit + 13 integration green.)

Live run BATTERY — COMPLETE (2026-09-26). Three runs on the full 20-
chunk KJV doc, each one blocking call, saved to workspace root:
  * kjv_summary_noquery.txt  — no query (312-word story, 10 map entries)
  * kjv_summary_jesus.txt    — query "the life of Jesus" (367 words, 10)
  * kjv_summary_god.txt      — query "the character of God" (325 words, 10)
All three produced coherent, query-appropriate stories and correct,
verifiable byte-span maps (spans cross-check the book map: e.g. the
Ten-Commandments entry is bytes 296046 in BOTH the no-query and god
runs; the god run's I AM / John 3:16 / 1 John 4:8 / Rev 19:16 entries
all land on the right books). Story word counts 312/367/325, all under
the 400 budget; map at the 10-entry cap each time. Runtimes 610s / 594s
/ 582s (faster than the 14-min estimate — APC warm between runs + the
no-query/short-query chunks decode shorter).

Query wording (user delegation): the god query had to be reworded ONCE.
The first draft "what this document reveals about the character and
attributes of God" FAILED the run at chunk bytes 3441088..3703302
(Matthew 8–Luke 11, the densest God-material) with "no JSON object
found after 3 attempts": all three attempts were PURE PROSE (1073–
1533 bytes, zero braces) — a STABLE attractor, not knife-edge (the model
writes a long "Regarding God's character and attributes, the text
reveals..." essay and never emits the object). The SAME chunk gave a
healthy 451-token JSON for the Jesus query and no-query, so it's a
query×chunk interaction triggered by the essay-inducing "what this
document reveals about..." phrasing. Reworded to "the character of
God" (shorter, direct) and it succeeded. LESSON for the user: keep
summarize_doc queries short and thematic, not "what does this doc say
about X" essay frames; enumerative "find all X" belongs to triage_doc.

Two more robustness changes landed to get the runs done (both
uncommitted, both tested, DESIGN.md updated):
  1. Failure capture: every failed map/reduce attempt dumps the full
     exchange (system prompt, user prompt incl. the S-block, raw
     response) to /tmp/summarize_fail_*.txt and the error message names
     the file — a failure is now diagnosable without re-running the
     chunk (the probe can't reproduce it: it omits the S-block the live
     run threads). This is what exposed the prose-only mode.
  2. Per-call retry (ATTEMPTS = 3): an unparseable map/reduce response
     is retried up to 3× before the call fails (the engine is
     non-deterministic, so a knife-edge chunk may emit prose once and a
     healthy object next); a chat() transport error still fails
     immediately. The god run PROVED this was load-bearing: chunk
     1147022..1409188 was prose-only on attempts 1–2 and recovered on
     attempt 3; chunk 688210..950416 was prose-only on attempt 1 and
     recovered on attempt 2. Without retry the god run (and probably the
     jesus run) would have failed. The systematic 3441088 attractor is
     the one case retry can't fix (3/3 prose) — that needed the query
     reword. Contract note: "unparseable → whole call errors" now means
     "unparseable on all 3 attempts AND no salvageable prose → whole call
     errors" (see the salvage item in the open questions below — done);
     the parseable-but-off-contract → degraded contribution path is
     unchanged.

Step 4 open questions / next steps (for the user, on return):
  1. COMMIT. Steps 2+3+4 are all uncommitted (HEAD is still step 1,
     58953aa). Everything from the 2026-09-24 review fix forward is in
     the working tree: the S-block threading (step 3), the 4× max_tokens
     bump, the reworked pointer-line wording, the extract_json first-
     complete-object fix, the failure-capture dump, the per-call retry,
     the prose-only salvage, and the proportional map size
     (max_map_entries). Suggest a few logical commits (step 3, the
     prompt/wording + 4× cap, the robustness + salvage + map-size
     batch) or one summarize_doc step-4 commit — user's call.
  2. Remove the temporary probe_raw_map_response test (it is #[ignore],
     now env-knobbed: PROBE_TARGET / PROBE_QUERY / PROBE_CAP). Keep it a
     bit longer if the user wants the probe loop for follow-up A/B; it
     was the fastest way to vet prompt changes.
  3. DONE (2026-09-26, after the battery; user discussion settled the
     shape): prose-only salvage + proportional map size. (a) SALVAGE: if
     all 3 attempts of a map call are unparseable but any attempt's
     prose before the first `{` has ≥ SALVAGE_MIN_WORDS (25) words, the
     longest such prose is used as the chunk's summary — pointers =
     none, NOT flagged degraded (the editor composes from it like a
     normal summary; only the map loses the chunk's candidates; the
     reduce prompt needed no change — a summary with "Pointers: (none)"
     is an existing shape). Rationale settled in discussion: pure prose
     (dropping JSON + pointers) would not have prevented the essay mode
     (the essay IS the summary) — it would only have made the essay a
     valid output, which salvage does while keeping the map (the map's
     unique value = read_doc(offset, limit) navigation; grep-ability
     comes free from the story). The <25-word case (apologies/garbage)
     still fails the call. 3 new tests (prose_salvage_candidate unit
     incl. the boundary + longest-wins across attempts via the e2e
     mock). (b) MAP SIZE: new optional `max_map_entries` arg (clamped
     1..=100), default max(10, 2·N) — two entries per chunk (a document
     map at per-chunk granularity; the first pass was max(10, N/2),
     which kept the 20-chunk KJV at 10 — the status quo — so the user
     bumped it to N*2: N=20 → 40, N=64 → 128 → clamped 100); the
     per-chunk writer stays at ≤5 pointers (a writer-token-budget
     matter); reduce `take(map_budget)`, prompt "at most {}", and reduce
     `max_tokens = 4·max_words + 150 + 20·map_budget` (2550 at defaults
     with the N=20 budget of 40). Tests: default_map_budget unit
     (renamed twice_chunks_floored_at_ten), reduce-prompt pin extended
     (budget variant), 8 summarize() call sites + handle_call +
     SummarizeArgs take the new arg. DESIGN.md: args line, reduce
     bullet (≤map_budget), budget bullet (formula + 2550/18,150 — the
     earlier 3750 figure was a miscalculation, now corrected), context
     fit (2,550), validation bullet (salvage semantics). (c) WRITER
     POINTER ORDER (2026-09-26, user musing, assistant implemented,
     user approved; the same reasoning was extended to the reduce map —
     see (d) below): the writer pointer line drops "most important
     first" — pointers are now a FILTER in LINE ORDER (line_start
     ascending): a binary worth-flagging judgment, no local importance
     comparison (the model's natural emission order is line order —
     the 11-pointer animals emission was doc-order emission violating
     the old ranking contract, not a failed sort; and local ranking
     contradicts the writer's own include-borderline-material
     instruction). (d) REDUCE MAP ORDER (2026-09-26, user decision after
     the 40-entry run): the reduce map is now ALWAYS DOCUMENT-ORDERED
     too — the prompt line reads "in byte order (start ascending)". The
     user's reasoning: with thinking off the model can't be trusted to
     rank (a correct ordering is a comparison/sort workload), and a
     consistently document-ordered map beats an inconsistently
     prioritized one — a reader can scan it like a phonebook. The
     reduce's candidate pool is already document-ordered (chunks in
     order, each chunk's pointers in line order), so byte-order output
     is a selection from an ordered list, not a re-sort.
     take(map_budget) keeps the leading entries on over-emission (the
     front of the document; observed runs never over-emit the map —
     10/10 and 40/40 hit the cap exactly). Landed: reduce prompt line
     + pin (budget assertion) + DESCRIPTION ("in document order") +
     DESIGN map bullet (rationale rewritten) + reduce bullet (in byte
     order).
     Suite 109 unit
     + 13 integration green.
     Validation pending: re-run the god query with the ESSAY-FRAMED
     wording ("what this document reveals about the character and
     attributes of God") — the one that failed 3/3 prose at chunk
     3441088. VALIDATION DONE (run "god_essay", 624s, saved to
     kjv_summary_god_essay.txt): the essay-framed god query that failed
     3/3 before now COMPLETES. Story 318 words (coherent
     Genesis-to-Revelation portrait). Map = 40 entries (exactly the new
     2·N default budget), STRICTLY ASCENDING = pure document order, a
     clean table-of-contents spanning the whole canon (creation →
     covenants → kings → prophets → Gospels → epistles → Revelation);
     the previously-dropped Revelation tail (King eternal, God is love,
     New Jerusalem) now lands. Salvage fired on chunk 1376368..1638594
     (1 Kings 5–1 Chronicles): prose-only on all 3 attempts, the longest
     (~230-word god-query essay, incl. markdown bold) became its summary,
     and the story visibly incorporates it — salvage works end-to-end
     live. The formerly-attractor chunk 3441088 (Matthew 8–Luke 11)
     emitted valid JSON this run (non-deterministic; the attractor did
     not fire), so that specific case was not re-exercised. Ordering
     finding (validates the user's prediction): with a 40-entry map the
     reduce emits pure document order despite "most important first";
     with a 10-entry map it did light importance reordering (Ten
     Commandments ahead of Red Sea) — the instruction degrades
     gracefully to document order on long maps. FOLLOW-UP (user
     decision): the reduce is now explicitly document-ordered too (see
     (d) above) — consistent order over unreliable ranking; the 40-
     entry run's pure document order was the observed behavior the new
     contract now pins.
     (e) NOQUERY2 RUN + MAP DEFECTS + FIX (2026-09-26, user approved
     all three): ran no-query again under the new contract (40-entry
     default + byte-order reduce) — kjv_summary_noquery2.txt, 622 s,
     story 275 words (fine). The map exposed three defects: (1) it
     STOPPED at Nehemiah 1 — byte 1,923,875, 43% of the doc; Job/
     Psalms/prophets/the whole NT missing (god_essay's identical
     contract had spanned the full canon — inconsistent global
     selection); (2) two local order inversions despite "in byte
     order" (Deut 1 after Numbers 31; 1 Sam 17 before 1 Sam 15) — the
     model can't sort 40 items with thinking off, confirmed; (3) a
     duplicated region (Goliath challenge + defeat, both at 1168380).
     Diagnosis: "most important first" had been doing the global-
     selection work; "in byte order, at most 40" reads as "walk the
     candidates in document order, stop at 40." FIX (implemented):
     (1) TOOL-SIDE SORT — entries stable-sorted by start after parsing,
     before render: byte order guaranteed by construction (test:
     reduce_map_is_sorted_to_byte_order_by_the_tool, mock reduce emits
     out of order); (2) reduce prompt gains the whole-document
     selection line ("choose the best candidates from across the
     entire document, not just the earliest chunks"); (3) "merging
     candidates that overlap or describe the same passage" (the
     ignored merge instruction). Prompt pin updated. Suite 110 unit +
     13 integration green; binary rebuilt.
     NEXT STEP (user to trigger after context compact): noquery3 A/B
     re-run (experiments/run_summarize.sh noquery3) — same no-query
     workload,
     new reduce contract + tool-side sort; check the map spans the
     whole canon and is strictly ascending (it must be, by
     construction) with fewer duplicates.
     (f) NOQUERY3 A/B + REDUCE SPLIT (2026-09-26): noquery3 (644 s,
     story 312 words, all spans verify, one label nit: Othniel/"Midian"
     — span is right, Midian is Gideon's fight): the mechanical fixes
     held (strictly ascending by construction; noquery2's doubled
     Goliath region now one entry; 0 dup starts) but coverage did NOT
     improve — the map ends at the Suffering Servant Song (Isa 52–53),
     byte 2,670,013 = 60% of the doc; the whole NT missing from the
     map. The split is clean on QUERY PRESENCE, not prompt wording:
     query-driven runs span the full canon (god 99%, god_essay 99% —
     even under the old importance-first contract); no-query runs walk
     and stop at the budget (noquery 77%, noquery2 43%, noquery3 60%)
     — no query gives "best" no axis. The story is fine either way
     (312 words, full canon in prose). USER'S IDEA (adopted,
     assistant-implemented): split the reduce into two focused calls —
     a story editor (the prose) and a map editor (the pointer
     selection/merging), the map editor allowed to run with thinking
     on. Settled shape: story FIRST (a failed story never burns a map
     call; the story's prose-only responses are its INTENDED output, so
     the salvage path applies to it — a prose-only essay IS the story;
     the map editor has no salvage, its prose is garbage); map-failure
     semantics = option (iii) (user-approved): the tool-side
     mechanical fallback (`fallback_map`: first pointer of each
     non-degraded chunk, budget-clamped, byte-ordered by construction)
     — the run still returns story + navigable map; the map editor gets
     the SAME contributions block the story saw (NOT the story — no
     anchoring, the map maps the document, not the prose). Plumbing:
     `llm::chat_reasoning` per-call thinking override (Some(e) sends
     `reasoning_effort: e` for the call only, ignoring the config/env
     knob — triage's corpus stays thinking-off; probe-verified live
     that ninfer honors `reasoning_effort`: reasoning_content +
     usage.reasoning_tokens, content still clean JSON); the
     `MAP_EDIT_REASONING` const (the single knob); success dumps of
     both reduce calls (/tmp/summarize_dump_story.txt,
     /tmp/summarize_dump_mapedit.txt) — the reduce input is now
     replayable through prompt A/B without a live run (~30 s/arm;
     replaces most of the probe test's use for the reduce phase).
     NOQUERY4 (643 s, first live run of the split): the map editor
     FAILED as designed — the 950 cap (150 + 20·40, the compact
     20/entry coefficient) was under-sized: pretty-printed entries
     MEASURE ~48 tok/entry, all three attempts truncated byte-
     identically (2196 bytes, mid-label), and the fallback carried the
     run: the 20-entry map (one per chunk, 98% span, ascending, 0 dups)
     WAS the mechanical fallback — option (iii) proved live, end to
     end. Replay A/B on the captured input (the fast loop): (1) at a
     3350 cap the model transcribed all 99 candidates (71 out,
     truncated at 74%) — "up to 40" is not a selection constraint
     without a query to give importance an axis; (2) quota wording
     ("exactly 40, about 2 per chunk" / "exactly 40, spread across the
     document"), thinking OFF: quota obeyed (41 entries, finish=stop,
     ascending) but 48% coverage — the per-chunk count cannot be kept
     over a long generation without thinking (the same bookkeeping
     limit as the noquery2 inversions); (3) per-chunk quota + thinking
     LOW: exactly 2 per chunk, every chunk, 100% coverage, 0 overlaps,
     31 s (4361 thinking tokens) — ADOPTED. Landed: per-chunk quota
     prompt ("aim for exactly {budget} entries — about {budget/n} per
     chunk (the document has {n} chunks below)" when budget ≥ n, else
     a spread clause; + "a selection, not a transcription" + the merge
     line), MAP_EDIT_REASONING = Some("low"), cap = 150 + 80·budget +
     16,384 thinking headroom (headroom present only when thinking on;
     19,734 at defaults — the headroom was 8,192, sized from the
     replay-measured 4.4/6.2k thinking tokens, and raised 2026-09-26
     after noquery5 attempt 1 consumed ~10.8k of them before its
     content truncated at 26/40 entries; 16,384 ≈ 1.5× the live worst
     case), off-contract (parseable, no map array) =
     failed attempt, story salvage, success dumps, `fallback_map`.
     DESIGN.md updated (the reduce split, 80/entry + headroom,
     context fit, validation, and a Findings entry with the full A/B
     chain). NOQUERY5 (703 s, thinking on — the validation run): 40
     entries, strictly ascending (the tool sort a no-op), 0 dups/
     overlaps, 100% span (Genesis 1 → Revelation), story 258 words
     coherent full-canon. Two notes: attempt 1 truncated (thinking
     consumed headroom; compact JSON cut at 1870 bytes) → retry
     recovered on attempt 2 (the residual risk, open 7c); and the
     selection went GLOBAL, not uniform — chunk 0 took 4 entries while
     chunks 8–9 (2 Chronicles 36/Ezra–Nehemiah; Job/early Psalms)
     took 0 ("about 2 per chunk" read loosely; defensible — chunk 7
     already maps the fall of Judah, chunk 8's top candidate the same
     event in a later book — but the map's contents gap at Job).
     Quota strictness = user's call (open 7a). Suite 116 unit + 13
     integration green; binary rebuilt.
     (g) JESUS2 RUN (2026-09-26, query-run validation, open 7b —
     DONE): same query as the battery ("the life of Jesus"), out name
     jesus2 so the old battery artifact stays for the A/B record
     (kjv_summary_jesus2.txt, 614 s, story 368 words). The flexible
     quota (kept per open 7a's user decision) did exactly what it
     should for a query: the map CONCENTRATES where the query matters —
     per-chunk 0,0,0,0,0,0,0,0,0,2,2,3,2,0,7,5,5,5,4,5: zero entries
     for Genesis–Ezra (Jesus not there), messianic-foreshadowing
     entries in the prophetic chunks (Ps 2/22/110, Isa 7/9/53, Jer
     Branch/New Covenant, Dan 7/9, Micah 5, Zech), dense across the NT
     (genealogy/virgin birth, transfiguration, Last Supper →
     resurrection, Acts testimony, Christ hymn, Hebrews high priest,
     1 John, Revelation). 40 entries, strictly ascending, 0 dups/
     overlaps, 98% span; all spot-checks land (Matt 1:1, Luke 13:1
     pericope, Phil 2:5, Micah 5:2, Rev 1:1). Story is query-framed
     (OT = foreshadowing). Map editor succeeded attempt 1 (no
     thinking-truncation retry at the 19,734 cap). A/B vs the old
     combined-reduce jesus run (10 entries, importance-first): the new
     map is 4× denser and spans the foreshadowing → Revelation arc the
     old 10-entry map could only sketch.
  4. Prompt-guard the prose-only mode: the map prompt already says
     "exactly one JSON object, no text before or after" and the model
     still preambles/essayizes on dense thematic chunks. A/B a stronger
     guardrail line via the probe (the triage guardrail was only partly
     effective — session-dependent) before assuming a fix works. Salvage
     has made this lower-stakes (the essay mode now costs one chunk's
     map candidates, not the run), so this is a quality tweak, not a
     reliability fix.
  5. The enumerative-vs-focus query distinction ("find all X" →
     triage_doc; thematic → summarize_doc) should get a one-line note in
     the summarize_doc DESCRIPTION so users aim queries at the right
     tool. Not yet added.
  6. Roadmap 5c close-out: with the battery done and (after the above)
     committed, mark 5c complete in PROJECT.md and rotate this task out
     of CURRENT.md.
  7. EDITOR SPLIT + QUOTA STRICTNESS (2026-09-26, the map-coverage
     workstream after noquery3; see the progress block below):
     (a) DONE (2026-09-26, user decision): KEEP the flexible quota —
     the global selection is right for query runs: a query relevant to
     only part of the doc should concentrate entries there, and a hard
     "exactly 2 per chunk, no chunk fewer" would waste budget on
     irrelevant chunks. The no-query gap (0 entries at Job, open at the
     time) is accepted; re-evaluate if it doesn't work.
     (b) DONE (2026-09-26, jesus2 run — see (g)): the new map editor
     (per-chunk quota + thinking low) live-validated WITH a query: the
     flexible quota concentrated the 40 entries on the messianic
     foreshadowing + the NT (0 entries in the Jesus-less Genesis–Ezra
     chunks) — the query-aware behavior a hard uniform quota would not
     have allowed.
     (c) DONE (2026-09-26, user-approved): the thinking-headroom risk
     was exercised live (noquery5 attempt 1: ~10.8k thinking tokens,
     content truncated at 26/40 entries at the 11,542 cap; retry
     recovered) — `MAP_EDIT_THINK_HEADROOM` raised 8,192 → 16,384
     (≈1.5× the live worst case; "Qwen 3.8 27B can think quite a lot").
     Cap now 19,734 at defaults; the timeout scales +10 ms/token, so
     the worst-case map-editor call is ~4 min — fine.

Step 2 (done; 2a user; 2b+2c + the 2026-09-24 review fix assistant, on
delegation; reviewed and approved by the user). Landed: uniform map phase →
reduce phase (no N == 1 direct call — the writer/editor split holds at
every N; DESIGN.md), `summarize(...)` mirroring triage (empty query
absent, not an error; `max_words` default 400 at the call site,
`clamp(1, 4000)` in `summarize()`; temp 0.2), `map_call` (`Query:`
line + provisional S-block slot + line-numbered body), per-chunk
writer budget `p = (2*max_words).div_ceil(n).max(250)` with
`max_tokens = 2p+150`, lenient map parse (two-lookup byte conversion;
overlap starts kept — contrast triage's hard zone drop; `take(5)`;
parseable-but-off-contract → degraded contribution; parse failure →
whole call errors naming the chunk span), all-degraded guard before
the reduce, reduce phase (`max_tokens = 2*max_words+150`; entries
range-validated, `take(10)`, no exact-match check — the editor may
merge), pinned `render`, 17 new unit tests (95 total) incl. both
prompt-contract pins; the server test renamed
`tools_call_summarize_doc_unknown_id`. The 2026-09-24 review fix: the
map prompt pin had drifted from the user's wording tweaks (code wins;
`map_prompt_pins_contract` now pins both variants from shared
head/note/tail pieces) and the overlap note is now omitted at
`context_lines == 0` (chunk 1) — the code now matches the DESIGN
contract ("absent at chunk 1"); wiring test
`map_call_omits_the_overlap_note_only_for_the_first_chunk`. Also
learned: `format_args!` disables implicit named capture for format
strings expanded from `concat!` (the note is a positional arg, as
`exclusive_line` was). Live smoke on the 559-byte example.com doc:
correct pinned render + sound story/map (this pulls the small-doc
end-to-end forward out of step 4). Judgment calls approved by the user:
(1) the reduce input format (per-chunk `[bytes a..b]` blocks +
Summary:/Pointers: + a Degraded line — a degraded chunk appears twice:
the bare span block and the trailing list); (2) a parseable reduce
response with empty/missing summary → whole-call error (fail-fast);
(3) the S-block header "Earlier chunk summaries (verbatim):" is
provisional — always None in step 2, pinned for real in step 3.

Working style (settled 2026-09-24): the assistant implements agreed
changes by default; the user still reviews all the code as if written
by them and may write some pieces themselves (PROJECT.md updated).

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

## Test state

* 116 unit + 13 integration, all green (new model default, no env
  override; 1 ignored = the temporary probe_raw_map_response test,
  removed after the KJV run). Binary `target/debug/toolbox` current
  (7 tools). Suite grew 98→110 during the step-4 robustness batch,
  110→111 with the noquery3 tool-side map-sort test, 111→116 with the
  reduce split (story/map-edit prompt pins replacing the reduce pin,
  `success_dump_writes_the_exchange`,
  `map_edit_failure_falls_back_to_the_mechanical_map`,
  `fallback_map_is_the_first_pointer_per_chunk_budget_clamped`,
  `map_edit_call_pins_user_message`, and the llm per-call-override pin;
  `reduce_map_is_sorted_to_byte_order_by_the_tool` renamed
  `map_edit_is_sorted_to_byte_order_by_the_tool`,
  `reduce_without_summary_fails_the_call` renamed
  `story_without_summary_fails_the_call`).
* Warnings: 1 pre-existing (`constant INTERNAL_ERROR is never used`,
  `src/mcp.rs`) — all summarize dead-code warnings are gone.
* Known flake: `man_page::tests::lookup_times_out` (1 ms-timeout race
  under full-suite parallel load; passes standalone and on re-run;
  pre-existing, unrelated).

## Engine & endpoint (live facts)

* `http://172.17.0.1:8081/v1` — currently **ninfer**, model
  `qwen3.8-27b` (NVFP4 weights, **NVFP4 KV cache** as of 2026-09-10;
  previously FP8 KV), max_model_len 262,000. The user swaps
  quantizations in place: the served model id stays `qwen3.8-27b`, so a
  quant swap needs no `LLAMA_MODEL` change (check `/v1/models` only to
  know which quant is live); a changed id 404s the live integration test
  `tools_call_triage_doc_success`. Old engine (available): llama.cpp,
  `qwen3.8-27b-q4xl` (GGUF Q4_K_M, ctx 200,192), same URL.
  Post-reboot 2026-09-26: back on the same config (262k window
  confirmed via /v1/models).
* Measured rates (thinking off, temp 0; the chunk-10 v2 payload =
  108,174 prompt tokens; artifacts in `experiments/probe_nvfp4/`,
  `experiments/probe_groupwise-int/`, `experiments/probe_nvfp4_kvfp4/`):
  * NVFP4 + FP8 KV (2026-09-07): decode ~175 tok/s (short-prompt probe;
    consistent with the old-era ~171 — MTP speculative decoding, ~91%
    acceptance); cold prefill ~3,850 tok/s (two cold runs, 27.9/28.2 s,
    `cached_tokens: 0` verified by mutating the first system-prompt
    token); warm prefill (identical repeat) 0.1 s.
  * NVFP4 + NVFP4 KV (2026-09-10, live): decode ~173 tok/s; cold
    prefill ~3,640 tok/s (two cold runs, 29.8/29.7 s — 0.3% reproducible;
    ~6% slower than the FP8-KV pair, not distinguishable from session
    variance without interleaved A/B); warm prefill 0.1 s (APC on).
    Chunk-10 v2 quality: healthy (90 regions, parse OK, finish=stop, 12
    context leaks) — no sign of KV-quantization oddness. Vs the FP8-KV
    v2 (93 regions): 80 spans identical; the drops/new are mostly
    re-segmentations of the same underlying regions; 25 of the 80 common
    regions at −1 score tier (none at −2 or worse) — inside the ±1–2
    tier margin-of-error band; calibration acceptance is the user's call
    (stability rule). Definitive check run 2026-09-10: the v6 triage
    workload re-run twice on this config (v7 / v7r — see triage_doc
    current state); the re-run was full per-chunk prefill (~15 min), not
    decode-only (APC does not speed up multi-chunk re-runs — see the APC
    bullet).
  * groupwise-int (2026-09-07): decode ~163 tok/s; cold prefill ~2,090
    tok/s (51.7 s, `cached_tokens: 0`).
  * KJV 20-chunk runtime estimates (triage ~2.6k completion tokens/
    chunk; summarize map ~2k): NVFP4+FP8KV ~15 min / ~13 min (matches
    the observed v6 15 m 03 s — validates the method); NVFP4+NVFP4KV
    ~16 min / ~14 min (v7 re-run observed 14 m 47 s); groupwise-int
    ~23 min / ~21 min. Prefill dominates the call and is per-chunk: APC
    gives no meaningful speedup on multi-chunk re-runs (each chunk body
    is a unique ~108k prefix; only the shared system+query prefix is
    cached — see the APC bullet).
* APC (prefix caching): ON for both NVFP4 sessions — identical payload
  repeats read `prompt_tokens_details.cached_tokens: 108167` (0.1 s
  warm prefill) on 2026-09-07 (FP8 KV) and 2026-09-10 (NVFP4 KV).
  NOT observed for the groupwise-int session: the
  identical v2 payload ~90 s after the full v2 run read `cached_tokens:
  0` — either APC was disabled on that restart or the blocks were
  evicted (~229k fresh tokens processed on that server in the
  meantime); open; user finding: the two restarts used identical startup flags, so
  "APC disabled on that restart" is largely ruled out — leading
  hypothesis is eviction (or some engine-state difference); verify with
  an immediate identical-payload repeat if groupwise-int is ever run
  again (low priority — see the engine-choice note below).
  Scope (corrected 2026-09-10 after the v7/v7r re-runs): APC caches
  shared prefixes; across a multi-chunk triage/summarize run the only
  shared prefix is the system+query (~200–400 tokens) — each ~108k chunk
  body is unique, so every chunk pays full prefill and APC gives no
  meaningful speedup on multi-chunk re-runs. Observed: the identical
  v7→v7r re-run took 14 m 47 s (cold-equivalent), not decode-only. The
  0.1 s warm hit applies only to re-running the exact same single
  payload (identical chunk body). Prompt A/B runs always pay full
  prefill (a system-prompt mutation invalidates the whole prefix).
* Engine choice (2026-09-10, settled): NVFP4 + NVFP4 KV (live) — it has
  the NVFP4 speed AND a 262k window, so it dominates groupwise-int,
  which was the path to a longer context (its 190k window + ~1.85×
  slower prefill; the APC miss now looks like a groupwise-int
  engine-state quirk). Margins at 262k: worst map call ≈140k → ~47%;
  triage worst ≈172k = 108k prompt + 64k max_tokens at max_hits 1000 →
  ~35%; dense-text ≈177k → ~32%; reduce trivial. Accepted costs: the
  ~6% prefill delta (unverified vs session variance) and the chunk-10
  score drift (25 common regions at −1 tier — user's calibration call;
  the v6 workload re-run is done — v7/v7r, see triage_doc current
  state). The step-4 KJV summarize run goes on this config (~14 min;
  full per-chunk prefill, APC gives no multi-chunk speedup).
* Token counts are quant-invariant: the identical chunk-10 payload
  tokenizes to the identical count on all three (108,137/108,174 on
  NVFP4, groupwise-int, and llama.cpp Q4_K_M) — ~2.9 bytes/token for KJV
  text is the model's real rate, not an engine artifact; the old-era
  "256KB ≈ 40–55k tokens" was a rough estimate, never a measurement.
  Prompt-side budget math: DESIGN.md → summarize_doc (context fit).
* Engine behavior: serial (one request running at a time); queue timeout
  30 s → clean HTTP 503 for any waiting request.
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
* Guardrail re-probed 2026-09-07 on the quant-swap probe pair:
  groupwise-int healthy on both variants (v0 no longer reproduces the
  NVFP4 degenerate truncator; ladder intact, shifted up one tier at the
  7→8 boundary — within margin of error); current NVFP4 restart: v0
  picked up the fenced attractor this session (session-dependent,
  `extract_json` strips fences; the old 2,624-token truncation did not
  recur) — v2 guardrail output healthy and unfenced.
* v6 workload re-run on NVFP4+NVFP4KV (2026-09-10, the definitive
  config check): v7 (cold) and v7r (identical re-run, 14 m 47 s) both
  1000 hits (capped), 0 untriaged, 20 chunks. Ladder v7 {10:76, 9:301,
  8:524, 7:99} vs v6 {10:94, 9:300, 8:560, 7:46} — a config-level
  shift (10-band −20%, 7-band +53), within the stability rule's 1–2-tier
  distribution-shift band; the 10-band compression is the user's
  calibration call (2026-09-10 demotion analysis: the demoted/dropped
  10s skew to background narrative mentions — saddling an ass, dogs
  licking blood — while all 12 promotions went to genuinely central
  passages (four horsemen, the Lamb, the seven-headed beast); the
  10-band got cleaner. One soft spot: Behemoth/Leviathan (Job 40–41,
  the two most substantial animal treatments) dropped 10→9 — one tier,
  within band. User: analysis closed for now.) Run-to-run (v7 vs v7r, both temp 0 / thinking off):
  956/1000 identical spans, score drift only ±1 (−1: 24, +1: 1), 11
  note-prose-only flips; the 88 differing spans concentrate in chunk 0
  (the densest competing-candidate region). So this config is NOT
  bit-reproducible across identical temp-0 runs — the prior
  bit-reproducibility finding (DESIGN.md → Findings) held for llama.cpp
  Q4_K_M and the FP8-KV ninfer sessions; cause here is confounded
  between KV-quant numerics and differing APC cache state between the
  two runs (isolate with a third back-to-back run, offered). Artifacts:
  `kjv_animals_triage_v7.txt` / `_v7r.txt` (workspace root), raw JSONs
  /tmp/kjv_triage_v7{,r}.json. Bench script restored to
  /tmp/run_triage_bench.sh.

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
  `tools/call` → `./target/debug/toolbox`; reusable bench scripts:
  `/tmp/run_triage_bench.sh` (identical v3/v4/v5/v6 triage workload: full
  doc, query "all mentions of animals", max_hits 1000; /tmp is reboot-
  wipeable) and `experiments/run_summarize.sh` (in-repo copy; also
  /tmp/run_summarize.sh until reboot): `run_summarize.sh <out_name>
  [query]` pipes the 3-line JSON-RPC framing (initialize /
  notifications/initialized / tools/call summarize_doc with
  `{id, query?}`) into `./target/debug/toolbox` under `timeout 2400`,
  writes the raw response to /tmp/kjv_summary_<out>.json and extracts
  the tool text to workspace-root `kjv_summary_<out>.txt`; prints
  exit/elapsed/isError. The KJV summary artifacts live at workspace
  root: kjv_summary_{noquery,jesus,god,god_essay,noquery2,noquery3,
  noquery4,noquery5,jesus2}.txt (noquery4 = the fallback-carried run;
  noquery5 = the first thinking-on map editor, 100%-span map;
  jesus2 = the first query run under the split — the map concentrates
  on the query's arc; the pre-split battery runs noquery/jesus/god/
  god_essay are kept for the A/B record). The reduce-split
  A/B loop: `/tmp/summarize_dump_*.txt` (success) and
  `/tmp/summarize_fail_mapedit_attempt*.txt` (failure) carry the full
  system + user prompt of the map-editor call — replay either through
  a bare curl (~10–40 s/arm) to iterate on that prompt without a live
  run. Replay artifacts: /tmp/kjv_mapedit_replay_cap{950,3350}.txt,
  /tmp/kjv_mapedit_ab_{B_perchunk,C_global}{,_low}.txt.

## Deferred / open

* Store-and-preview for oversized triage output (fetch_url pattern).
* Context-window-relative `max_words` ceiling (4000 clamp kept for v1).
* No tail overlap in the chunker (trigger: hits systematically missed at
  boundaries).
* Streaming reads for multi-GB docs (roadmap 6).
* If the old engine (Q4_K_M) is used again: reconsider the guardrail line
  for it (it collapses that quant's score ladder to all-10s).
