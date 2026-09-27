# kgrep

The `k` is for kcode, the one harness this is built for. Checked before adopting:
free on crates.io, and no meaningful incumbent (the GitHub hits top out at seven
stars, and are a log searcher, a k-mer grep, and a Kubernetes grep).

One shaper for how an agent finds its way around a codebase, built to spend
tokens and latency well and to keep memory inside a declared ceiling.

This file is the plan of record: what is settled, what we have measured, where the
code actually is, and what comes next. `docs/` holds the longer reasoning.

- `docs/DESIGN.md` — the shaper, the cache, and the levels of synergy.
- `docs/SOTA.md` — how every other harness retrieves code, with sources.
- `docs/AGENTGREP.md` — the tool we absorb, and the weaknesses we inherit.
- `bench/README.md` — the benchmark, the baseline, and one retracted claim.

## What this is

A single tool for the questions agents ask constantly: where is X defined, who
calls X, what is inside this file, which files are about this topic. It answers
with a **packet**: ranked, structured, budgeted, and carrying its reasons, so the
agent can act on one call instead of ten.

Behind that one interface sit several **sources** of evidence. Each is
independently useful, each is optional, and each must be streaming and bounded to
be allowed in.

| Source | Answers | Cost | State |
|---|---|---|---|
| Lexical scan | exact text and regex | none, always available | working |
| Structure sketch | symbols, ranges | bounded per file | working, line-based |
| Tag index (ctags) | where is X defined, and what kind of thing it is | one mmapped file, no binary growth | not started, Stage 4 |
| mmapped n-gram index | fast regex on huge repos | mmapped, evictable | not started |
| Harness state | what the agent read, what changed | nearly free | partial, in agentgrep |
| Resolution (ours, approximate) | who might use this | bounded index | not started |
| Graph | what is central, what clusters | optional artifact | not started |

The shaper is ours and is the product. The sources are interchangeable.

## The objective, and the memory guardrail

**Maximise capability per unit of tokens and latency.** Those are the two costs
the agent actually feels: tokens are money and context, latency is what makes a
tool feel bad. CPU matters mostly *as* latency, plus heat and battery on a laptop.

**Memory is a guardrail, not an objective.** It matters in classes, not in bytes:

- **Forbidden:** anything that scales unboundedly with repository size, or that is
  orders of magnitude larger than the harness itself. Language servers (hundreds
  of megabytes to gigabytes) and embedding models are the real enemy, and they are
  excluded.
- **Allowed, and often the right trade:** memory capped by a declared number, or
  memory-mapped so the OS can evict it, when it buys tokens or latency.

An earlier draft of this file demanded flat memory. That was too tight: it refused
cheap wins to protect a number nobody can perceive. The positioning is already won
by being Rust and by not shipping a language server, which is an order of
magnitude, not a factor of two. Spending a bounded amount of memory to cut tokens
or latency is usually correct.

**The guardrail is a number, not a vibe.** "We'll keep what we need" is how you
end up with a vector store by accident. So: a declared ceiling, measured, where
exceeding it is a failure rather than a discussion.

Rules that still hold:

1. **Work is per file and released per file**, unless held in a bounded cache.
2. **Nothing scales unboundedly with the repository.** Bounded by a declared
   number, or mmapped, or it does not ship.
3. **Bounded results.** Ranked output keeps a bounded top-K. Nothing collects
   every match before sorting.
4. **No language servers, no embedding models, in-process.** LSP is consumed over
   MCP, only if the user already runs one.
5. **Every tool call has a budget.** Exceeding it truncates, spills, or defers. It
   never grows unbounded.

Rule 3 resolves a tension worth naming: **you must score everything, but you do
not need to keep everything.** Scoring is a streaming fold; keeping is what gets
bounded. That is how ranking and a bounded footprint coexist, and neither
agentgrep nor `rg` does it today, `rg` because it never collects and agentgrep
because it collects everything.

### Measured per change

Three numbers, not one:

| Number | Why it matters | How |
|---|---|---|
| tokens to answer | money and context, the primary currency | `scripts/bench.py` |
| latency | what the agent and the user feel | same harness, timed |
| peak RSS | the guardrail, against a declared ceiling | `scripts/memcheck.sh` |

A change that improves tokens or latency while staying under the ceiling is a win.
A change that exceeds the ceiling needs a reason, not a shrug.

## The product: the packet

`Query` in, `Packet` out. `grep`, `find`, `outline`, and `trace` are thin
constructors over `Query`, not four parallel implementations.

The budget rules, learned from the benchmark and from reading agentgrep:

- **Rank before spending.** Truncating an unranked list selects an arbitrary N.
  agentgrep sorts by path, so its cap keeps the alphabetically earliest matches.
- **Budget in tokens, not matches.** A match line can be 240 characters, so "200
  matches" ranges over roughly a factor of five in output size.
- **Cut detail before coverage.** Name every matching file; expand the top few;
  let the tail be a count. Existence is the cheapest information to keep and the
  most expensive to drop.
- **Never lose information silently.** Always report the true total and what was
  dropped, so the caller can raise the budget deliberately.
- **The bounded packet is the default**, with an explicit unbounded opt-out. A
  guard that lives in the caller gets forgotten, which is exactly what happened
  to the CLI.
- **Bound during the scan, not at render.** A cap applied after collection guards
  the context window and does nothing for memory.

## Where we are: code

1,727 lines of Rust, 8 tests passing, clippy clean at `-D warnings`.

| Module | State |
|---|---|
| `model.rs` | `Query`, `Hit`, `Packet`, `Budget`, `Group`, `LineMatch` |
| `cli.rs` | four verbs, flags matching agentgrep's contract |
| `scan.rs` | absorbed from agentgrep: walk, ignore rules, non-UTF-8 handling |
| `outline.rs` | absorbed structure extraction plus the `outline` verb |
| `lexical.rs` | absorbed grep, reshaped, bounded, `rg` path dropped |
| `packet.rs` | text and JSON rendering, matching agentgrep's JSON shape |
| `peak.rs` | peak RSS reporting behind `KGREP_PEAK_RSS` |
| `main.rs` | wires `grep` and `outline`; `find` and `trace` still exit 2 |

Tooling that exists and works:

- `scripts/bench.py` + `bench/tasks.json` — 17 verified navigation tasks, scored on
  recall and tokens-to-answer. Baseline in `bench/baseline.json`.
- `scripts/memcheck.sh` — peak RSS against repo size, small by default.

Known gaps in the code, honestly:

- `find` and `trace` are unimplemented, so the four-verb surface is half done.
- No ranking for grep hits; `score` is always 0, which blocks "rank before
  spending".
- The output cap is a match count, not a token budget, and is not yet the default.
- Non-UTF-8 display paths use a simpler `#raw=hex` suffix than agentgrep's `#b=`
  scheme, and `outline` cannot yet resolve a path carrying it, so agentgrep's
  round-trip contract does not hold in kgrep.
- The benchmark drives the CLI, which is not the path kcode uses. It needs a
  harness-faithful oracle.

## Where we are: theory

Settled:

- The product is the packet, not the engine. Sources are pluggable; the shaper is
  ours.
- Flat memory with respect to repository size is the binding constraint and the
  differentiator.
- No language servers or embedding models in-process.
- agentgrep contributes code; graphify contributes ideas only.
- The scan is being commoditized upstream (they intend to rebuild `rg` in-process
  from the same libraries). Competing there is competing where the incumbent is
  already moving. Differentiation lives in the shaping, the memory property, and
  the harness integration.

Open, and genuinely unknown:

- **What is a grep hit's relevance score?** "Rank before spending" needs one, and
  grep currently has none. Candidates: does the match land in a symbol's *label*,
  how many matches the file has (specificity), file role, structural centrality
  later. This is the first real ranking question and it is unsolved.
- **What is the right default token budget?** Unknown until measured.
- **How should the tail be ordered?** If we name every matching file, in what
  order, and does that order matter to a model?
- **Does structural ranking actually improve answers?** Testable once the
  declaration source exists. Currently an assumption, and it is the assumption
  Stage 4 rests on.
- **How the name ages.** `kgrep` is anchored to kcode, which is meaningful only
  while kcode is the sole consumer. That is a deliberate trade, not an oversight.
- **Graph freshness policy.** How stale is too stale, if the artifact is built.

## What we have measured

17 tasks over kcode, verified ground truth, `rg` absent so agentgrep uses its
native fallback.

| tool | recall | median tok→ans | p90 | max output tok |
|---|---|---|---|---|
| agentgrep 0.1.6 | 17/17 | 383 | 1,762 | 154,351 |
| kgrep | 17/17 | 383 | 1,762 | 154,356 |

What holds:

- **Recall is not the problem.** Both find the answer every time. "We also find
  it" is not a case for anything.
- **Specific queries are already good**, at a median of 383 tokens to the answer.
  That is the bar to beat.
- **kgrep tracks agentgrep to within one token per task**, which is what
  faithful absorption looks like.

What was retracted: the 154k-token maximum is a **CLI-only** artifact. jcode calls
`run_grep` then renders with `Some(200)`, so the model never sees it. The mistake
was measuring the CLI and calling it the harness. The finding that survives is
better: **the guard lives in the caller, not the library**, so the CLI forgot it
and a new caller would reintroduce the bug jcode already fixed.

## The roadmap

The goal in one line: **beat agentgrep on tokens and latency, at a comparable or
better footprint, or conclude that we cannot and stop.** Every stage ends in
evidence, and every stage must be independently useful.

A stage is two to four sessions of work. Order matters where noted, and the
reasons are stated so a later reader can disagree with them on the merits.

### Stage 1 — bound the packet, in parallel (done)

All three parts landed, and the exit criteria are met. See `bench/README.md`.

**1a. The numbers.** Done. kcode costs 5.5 MB light and 12.6 MB heavy; process
baseline about 4.8 MB. Memory was **not flat**: the walk materialized the file
list at roughly 268 bytes per file. Ceiling declared at 32 MB for repositories up
to 100,000 files.

**1b. The ranking signal.** Done. Four cheap signals together are **3.8x better on
median tokens-to-answer** (383 to 101) and 2.5x on p90, at no latency cost. Two
findings: specificity is *worse than nothing* on its own, and the three per-task
regressions all trace to the structure sketch not parsing struct fields, so they
should close when Stage 4 lands.

**1c. Stream, parallelize, bound.** Done, as one change.

- **Streamed**, so the file list is gone: the per-file term fell from 268 bytes to
  about 40.
- **Parallel**, so the latency regression reversed: 18.1 ms against agentgrep's
  21.5 ms.
- **Bounded**, in two places, because there were two unbounded terms: detail by
  an estimated token budget, and coverage by a cap on listed files. Both report
  the truth rather than hiding it. Worst-case output fell **9.6x**, from 154,351
  tokens to 16,061, with recall unchanged and median cost untouched.

| criterion | required | measured |
|---|---|---|
| recall | stays 17/17 | 17/17 |
| generic-query tokens | down at least 5x | 6.6x and 7.2x |
| latency | at or below agentgrep | 18.1 ms versus 21.5 ms |
| peak RSS | under ceiling, flat 1k to 5k files | 5,720 to 5,880 KB |

So **kgrep now beats agentgrep on both objectives**: 3.8x better on median tokens
to the answer, 2.5x better on p90, 9.6x better on the worst case, and faster.

### Stage 2 — four verbs on one core (done)

All four verbs are on the shaper, sharing one ranking weight table rather than
carrying a copy each, which was the specific duplication agentgrep had between
`find` and `trace`. The walk streams and is parallel for all of them, and the
packet bound applies to all of them.

**Parity against agentgrep 0.1.6 on kcode:** `find` produced an identical top
result on all ten queries sampled, and `trace` on 16 of 16.

**`trace` found three real bugs**, all the same shape, a literal check
short-circuiting a normalised one:

1. Regions were only seeded from lines containing the subject, so a declaration
   whose body never repeats its own name was invisible, and a test that merely
   mentioned the name outranked the definition it was testing.
2. Symbols whose *name* matches the subject are now candidates in their own
   right.
3. The subject gate and an empty-mention early return both rejected such a file
   before the label-driven regions could run.

The fix that mattered: names are compared with punctuation stripped, because a
subject is written `mcp_call` and the type that defines it is written
`McpCallInput`. Without that, `relation:defined` could never find a definition.

**One deliberate gap**, recorded in `src/trace.rs`: agentgrep's `--context-json`
familiarity is accepted but not applied. That is harness state, which is Stage 5,
and half-porting it would put the seam in the wrong place.

### Stage 3 — make the measurement absolute (done)

**The honest oracle exists.** `bench/oracle` links agentgrep `v0.1.6` at the
pinned revision `b01b8040` and renders through the same call path kcode uses,
including the `Some(200)` grep cap. It sits outside kgrep, because kgrep must not
depend on what it replaces, and it disappears after the swap. The earlier CLI
numbers are superseded by it.

**Differential parity passes.** `scripts/parity.py` builds a corpus that is
nasty on purpose and compares the set of files found: 13 cases covering
non-UTF-8 names, file and directory symlinks, globs, glob+type, hidden,
no-ignore, binary refusal, empty files and extension-less files. All agree.

**It found a real behavioural difference, and fixed it.** agentgrep v0.1.6 never
calls `follow_links`, so it uses the `ignore` default of false, and it has no
symlink guard either, so a symlink to a *file* is searched while a symlink to a
*directory* is not descended. kgrep had inherited v0.1.7's guard, which excluded
both. That is fixed.

**Which also settled a flag.** Following symlinks is a v0.1.7 behaviour, kcode
runs v0.1.6, and ripgrep does not follow by default either. The default is now
not to follow, with `--follow` as the opt-in in place of `--no-follow`.

**And `outline` accepts what `grep` prints** for a name that is not valid UTF-8,
so search-then-read round-trips.

- **Exit met:** parity on the edge corpus, deliberate differences written down,
  and the benchmark measuring the harness path.

### Stage 4 — declarations, from ctags (next)

Consume `universal-ctags` as a **tag index**: one sorted file, name to file, line,
kind and scope, read mmapped and never resident. ctags runs at index time only.
Nothing new is compiled in, so the binary does not grow, and the line-based
scanner stays as the fallback for when the index is absent or stale.

Chosen over an in-process parse because it gets most of the benefit at none of
the cost. Verified kinds include `m field`, `P method` and `e enumerator`
alongside structs and functions, and it covers roughly a hundred languages
against our four, which is the difference between finding nothing and finding
declarations. The one thing it does not give is end lines: `rust.c`, `go.c`,
`python.c` and `typescript.c` never call `setTagEndLine`, so there are no real
extents. An in-process parse for ranges is **cut**, not deferred.

What changes for the caller:

- **A declaration index.** "Where is X defined" becomes a lookup rather than a
  scan, and it can say *what* X is, because the tag carries a kind and a scope.
- **The ranking signal gets real labels.** The three measured regressions trace
  to struct fields being invisible to the line scanner, and ctags emits them.
- **Structure where we had none.** Any repository outside our four languages
  currently gets no structure at all, only match lines.

This is still a **signal, not a separate map**: which symbol a hit lives in, what
kind it is, what encloses it. Those change the answers to ordinary questions, and
no cross-file resolution is needed.

Three things to get right:

- **The index must stay optional.** Ranking is index-independent today. Without
  the index we are exactly where we are now, and that is the floor; the index is
  the ceiling. Degrading has to be clean and silent-free.
- **Freshness is a real failure mode.** A stale tag is a wrong answer, silently.
  The index needs a cheap validity check, and staleness has to be visible rather
  than assumed.
- **The kind table is ours.** ctags kinds are per-language and inconsistent, so
  mapping tag kinds onto our symbol model is code we own and test.

- **Exit:** the three regressions close and recall holds; a definition query is
  answered from the index without a scan; peak RSS stays under the ceiling with no
  new resident structure proportional to repository size.

#### Footnote: edits are not ours to improve

Symbol-scoped edits were going to arrive with real ranges. Ranges are not coming,
because ctags does not emit end lines, so that plan is dead rather than deferred.

What survives is the cheaper version, which needs no ranges at all: let the
harness's `edit` take a line range as an alternative anchor to `old_string`. The
caller already has line numbers from `read` and `outline`, so this removes the
need to echo the text being replaced. That is worth having, because `edit` must
echo it while `patch` costs *more*, since a unified diff contains the removed
lines. But it is an optimization of a working path rather than a missing
capability, it is unmeasured, and it belongs to the edit tool rather than to us.
What symbol addressing added beyond a line range is surviving drift and
expressing intent. Both are real, and neither is worth a parse on this evidence.

### Stage 5 — harness state as a source

What the agent has already read, what changed since the last call, where the user
is working. Nearly free, and the one advantage an in-harness tool has that no
external tool can match. agentgrep touches it with `context.json`.

- **Exit:** measurable improvement on tasks where prior reading is relevant.

### Stage 6 — resolution, approximate and ours

"What uses this", answered by name-based resolution over our own index: which
symbols share a name, which imports bring that name into scope, which files could
plausibly refer to it. Good for reading, and useful as a ranking signal.

**This is deliberately approximate, so it is not safe for rewriting.** The exact
tier, meaning cross-file rename, find-implementations across inheritance, and type
hierarchy, needs compiler-accurate resolution, and this project does not drive a
language server. So that tier is out of scope, and the gate on cross-file edits
is expected to stay closed.

The shape of the product follows: a retrieval tool with safe local edits, not a
refactoring engine.

- **Exit:** a references verb that is useful to read, with its imprecision stated
  in the output rather than hidden.

### Stage 7 — the graph artifact, last and optional

Only the report-shaped questions, as a separate mmapped artifact: what is central,
what clusters, what is surprising. Note that the *valuable* part of the graph idea
is Stage 4, as a ranking and zoom signal over the shaper. This stage is what
remains once that is done, and it may not earn its place.

- **Exit:** it answers something the earlier stages cannot, at no resident cost.

### Cheap win, available now

The output cap belongs in the library, not the caller. That is a small,
clearly-correct fix to agentgrep that upstream would plausibly accept, and if it
lands, kcode improves immediately without this project shipping anything. See
`docs/AGENTGREP.md`, weakness 5. Worth attempting regardless of how this project
goes, and good faith while superseding someone's work.

### Gate — the swap

Point kcode at this. Blocked on: parity (Stage 3), attribution written, and a
clean consumer build. The name should be settled by here, because kcode's tool
names are what the model sees.

### Stop condition

If after Stage 1 the tokens do not improve measurably, or the improvement does not
survive Stage 3's honest oracle, the honest move is to keep agentgrep, contribute
the fixes upstream if they will take them, and spend the effort on the harness
instead. Deciding that early is a win, not a failure.

### Not doing

Language servers in-process, a language-server integration of our own, depending
on or recommending a third-party wrapper such as Serena, embedding models, vector
stores, installers, hooks managers, editors, MCP/HTTP servers of our own, watch
mode, HTML/canvas/SVG/wiki exporters, media or document ingest, cross-file
rewriting, an in-process parse for real ranges, and any resident structure
proportional to repository size.

The parse is cut rather than deferred: it buys end lines we have no measured use
for, and everything else it offers, ctags offers cheaper. If a real range need
appears with a number attached, that is the moment to revisit it.

Note on MCP: kcode already supports arbitrary MCP servers as a client, so a user
who wants a language server can configure one without anything from us. That is a
kcode feature, not ours, and it is why nothing is lost by dropping it here.

## Consumers and the kcode seam

Today kcode depends on agentgrep as a library and calls:

```
::agentgrep::cli::{FindArgs, FullRegionMode, GrepArgs, OutlineArgs, SmartArgs}
::agentgrep::find::{FindResult, run_find}
::agentgrep::outline::run_outline
::agentgrep::search::{GrepResult, run_grep}
::agentgrep::smart_dsl::{SmartQuery, parse_smart_query}
::agentgrep::smart_engine::{SmartResult, run_smart}
::agentgrep::render::{render_find_output, render_grep_output,
                      render_outline_output, render_smart_output}
```

Its wrapper lives in `kcode/crates/jcode-app-core/src/tool/agentgrep.rs` (+
`args.rs`, `context.rs`), and two of its behaviours are load-bearing and easy to
miss:

- It renders with `Some(200)`, so the model sees a bounded packet already.
- It feeds `context.json` for familiarity, which is the seam stage 5 grows from.

**kcode is in scope to refactor**, bounded to that integration site. **No
dependency swap until kgrep is done and tested.**

## Layout

```
src/
  model.rs      // the core: Query, Hit, Packet, Budget
  cli.rs        // clap surface: grep | find | outline | trace
  scan.rs       // file walking, ignore rules, candidate collection
  lexical.rs    // matching, grouping, the grep verb
  outline.rs    // file structure, and the outline verb
  packet.rs     // ranking, budgets, text and JSON rendering
  peak.rs       // peak RSS, for the memory claim
bench/          // tasks, baseline, findings
scripts/        // bench.py, memcheck.sh
```

Let structure emerge from the core. Do not pre-create empty modules.

## Commands

```bash
cargo build            # debug build
cargo build --release  # release build
cargo test             # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check

scripts/bench.py --bin target/release/kgrep --corpus ~/kcode
scripts/memcheck.sh    # small by default; pass sizes to make it bigger
```

## Working rules for agents

- **Memory is a guardrail; tokens and latency are the product.** Nothing may
  scale unboundedly with repo size. Spending a bounded amount to cut tokens or
  latency is usually correct, and needs no apology.
- **Measure before claiming.** The benchmark exists because a confident claim in
  this repository was already wrong once. If a number is stated, it came from a
  run, and the run's shape is described.
- **Read before you claim.** A structural claim is not a fact until you have read
  the code behind it. Never dress a guess as a finding.
- **Finish what you start.** Half-migrated is the most expensive state there is.
- **One home per concept.** Guards belong in the core, not in each caller.
- **No capability regressions.** Never delete what the system needs to do to make
  a file smaller.
- **No artifacts, no state.** You read the code fresh; the structure is the
  memory. Keep docs in sync, not aspirational.
- **Commit as you go.** Small, whole, working commits.

## Decisions

Settled: Rust, one crate. One shaper with pluggable sources. Tokens and latency
as the objective, with a declared memory ceiling as the guardrail, rather than
flatness for its own sake. No language servers or embedding models in-process.
graphify contributes ideas, agentgrep contributes code. No dependency swap until
done. Structure comes from ctags declarations, so language coverage is whatever
ctags covers rather than a list we maintain.

Settled by decision, not by evidence:

- **One consumer: kcode only.** No MCP server, no third-party CLI contract, and
  kcode's integration site and tool names are ours to change. This is why
  harness state is a real advantage rather than a theoretical one.
- **No Serena, and no language-server wrapper of our own.** We rewrite the cheap
  tier ourselves: outline and symbols, a name index, symbol-scoped edits, and
  approximate references. We accept losing the exact tier, meaning cross-file
  rename, find-implementations, and type hierarchy, because that needs
  compiler-accurate resolution that only a language server provides.
- **The graph is a signal, not a product.** Its value lands in Stage 4 as ranking
  and zoom over the shaper. The separate artifact is Stage 7 and may not earn its
  place.
- **Editing is staged.** Symbol-scoped edits within a file need only real ranges,
  which Stage 4 would provide, and are a bonus of it rather than a stage of their
  own. Cross-file operations need exact resolution and are gated on Stage 6
  producing evidence of it. Approximate resolution is fine for ranking and fatal
  for rewriting, so the gate is real.
- **Supersede agentgrep, and send the obviously-correct fixes upstream first.**

Open: the default token budget. Graph freshness. Whether cross-file edits follow
resolution. The grep relevance constants, which are borrowed rather than tuned.

## Attribution

Neither upstream is ours. agentgrep is MIT (`1jehuang`); graphify is Apache-2.0
(`Safi Shamsi and the Graphify contributors`). Absorbing agentgrep's source makes
kgrep a derivative work of it, so its notice is owed. Deferred by choice, but
it must land before release or before kcode points at this.
