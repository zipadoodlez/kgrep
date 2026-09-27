# graphgrep (working name)

One shaper for how an agent finds its way around a codebase, built so that memory
stays flat no matter how large the repository is.

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
| tree-sitter / ast-grep | real nesting and patterns | bounded per file | not started |
| Tag index | where is X defined | one mmapped file | not started |
| mmapped n-gram index | fast regex on huge repos | mmapped, evictable | not started |
| Harness state | what the agent read, what changed | nearly free | partial, in agentgrep |
| Resolution (LSP via MCP) | who uses this | never ours to run | not started |
| Graph | what is central, what clusters | optional artifact | not started |

The shaper is ours and is the product. The sources are interchangeable.

## The constraint: memory

The differentiator, not a detail. Other harnesses and other indexes hold the
codebase in RAM. We do not. Every decision is judged by capability per byte.

1. **Nothing resident.** Work is per file, released per file.
2. **Peak memory may scale with the largest file, never with the repository.**
   A hundred-file repo and a half-million-file repo must peak in the same place.
3. **Any persistent artifact is mmapped, never deserialized.** Touch the lookup
   table, read payloads by offset, let the OS page and evict.
4. **Bounded results.** Ranked output keeps a bounded top-K. Nothing collects
   every match before sorting.
5. **No language servers, no embedding models, in-process.** A language server is
   hundreds of megabytes to gigabytes. LSP is consumed over MCP, only if the user
   already runs one.
6. **Every tool call has a budget.** Exceeding it truncates, spills, or defers.
   It never grows unbounded.

Rule 4 is the interesting one, because it resolves a tension: **you must score
everything, but you do not need to keep everything.** Scoring is a streaming fold;
keeping is what gets bounded. That is how ranking and flat memory coexist, and
neither agentgrep nor `rg` does it today, `rg` because it never collects and
agentgrep because it collects everything.

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
| `peak.rs` | peak RSS reporting behind `GRAPHGREP_PEAK_RSS` |
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
  round-trip contract does not hold in graphgrep.
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
- **Does structural ranking actually improve answers?** Testable once a graph or
  AST source exists. Currently an assumption.
- **Is the graph ever worth building**, given flat memory and that its questions
  are the rarest ones?
- **Where does the scope stop?** Retrieval only, or also symbol-level rewriting?
  Serena does both; we have assumed retrieval only.
- **The name.** `graphgrep` names a source we demoted. Every dictionary word
  checked is taken on crates.io, and `treegrep`/`agrep` collide in the same
  namespace. Unresolved.
- **Graph freshness policy.** How stale is too stale, if we ever build it.

## What we have measured

17 tasks over kcode, verified ground truth, `rg` absent so agentgrep uses its
native fallback.

| tool | recall | median tok→ans | p90 | max output tok |
|---|---|---|---|---|
| agentgrep 0.1.6 | 17/17 | 383 | 1,762 | 154,351 |
| graphgrep | 17/17 | 383 | 1,762 | 154,356 |

What holds:

- **Recall is not the problem.** Both find the answer every time. "We also find
  it" is not a case for anything.
- **Specific queries are already good**, at a median of 383 tokens to the answer.
  That is the bar to beat.
- **graphgrep tracks agentgrep to within one token per task**, which is what
  faithful absorption looks like.

What was retracted: the 154k-token maximum is a **CLI-only** artifact. jcode calls
`run_grep` then renders with `Some(200)`, so the model never sees it. The mistake
was measuring the CLI and calling it the harness. The finding that survives is
better: **the guard lives in the caller, not the library**, so the CLI forgot it
and a new caller would reintroduce the bug jcode already fixed.

## The roadmap

Each stage is independently useful and must not raise peak memory. Exit criteria
are stated so a stage ends in evidence rather than opinion.

### Stage 1 — the bounded packet (next)

- Move the output cap into the library default: bounded unless asked otherwise.
- Budget in tokens; rank before spending; cut detail before coverage; keep the
  true totals; add an unbounded opt-out.
- Needs the grep relevance score, which is open question number one. A first cut
  can reuse `find`'s signal set, which also unifies the two verbs' ranking.
- **Exit:** recall does not regress on the bench, maximum output falls by an order
  of magnitude on generic queries, and peak RSS stays flat in `memcheck`.

### Stage 2 — four verbs on one core

- Port `find` and `trace` onto the same shaper. Ranking lands here properly.
- **Exit:** all four verbs implemented and covered by the benchmark.

### Stage 3 — honest measurement

- A harness-faithful bench binary that links agentgrep `v0.1.6` and calls
  `run_grep` + `render_grep_output(Some(200))`, so the oracle mirrors what a model
  actually sees.
- Differential parity on the edges: non-UTF-8 names, symlinks, globs, glob+type,
  extension-less files. Document deliberate differences rather than hiding them.
- Fix the non-UTF-8 round-trip so `outline` accepts what `grep` emits.
- **Exit:** parity on UTF-8 corpora, deliberate differences written down, and the
  benchmark measuring the harness path.

### Stage 4 — real structure

- Adopt `ast-grep-core` and `ast-grep-language` (skip `ast-grep-config`), with a
  small query file per language. Keep the line-based scanner as the fallback for
  languages with no grammar.
- This is where nesting and zoom arrive, and it costs no resident memory.
- **Exit:** nested structure available; outline more accurate; bench unchanged or
  better.

### Stage 5 — harness state as a source

- What the agent has already read, what changed since the last call, where the
  user is working. Nearly free, and it is the one advantage an in-harness tool has
  that no external tool can match. agentgrep touches it with `context.json`.
- **Exit:** answers visibly improve on tasks where prior reading is relevant.

### Stage 6 — resolution, the largest capability gap

- "What uses this" and "what implements this", the questions an agent needs before
  editing. Consume via MCP if a language server is already running; otherwise
  name-based resolution over our own structure.
- **Exit:** a references verb whose precision is good enough to act on.

### Stage 7 — the graph, last and optional

- An mmapped artifact answering only the architectural questions: what is central,
  what clusters, how things link. Only if it earns its place after 1-6.
- **Exit:** it answers something the earlier stages cannot, at no resident cost.

### Then, and only then, the swap

Point kcode at graphgrep. Requires parity, attribution, and a clean consumer
build.

### Not doing

Language servers in-process, embedding models, vector stores, installers, hooks
managers, editors, MCP/HTTP servers, watch mode, HTML/canvas/SVG/wiki exporters,
media or document ingest, refactoring and rewriting, and any resident structure
proportional to repository size.

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
dependency swap until graphgrep is done and tested.**

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

scripts/bench.py --bin target/release/graphgrep --corpus ~/kcode
scripts/memcheck.sh    # small by default; pass sizes to make it bigger
```

## Working rules for agents

- **Memory is the product.** Any change that makes peak memory scale with repo
  size is a regression, however fast or neat it is.
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

Settled: Rust, one crate. One shaper with pluggable sources. Flat memory as the
binding constraint. No language servers or embedding models in-process. graphify
contributes ideas, agentgrep contributes code. No dependency swap until done.
Initial extraction languages: Rust, Python, TypeScript/JavaScript, Go.

Open: the name. The grep relevance score. The default token budget. Whether the
graph is ever worth building. Graph freshness. Whether scope includes rewriting.

## Attribution

Neither upstream is ours. agentgrep is MIT (`1jehuang`); graphify is Apache-2.0
(`Safi Shamsi and the Graphify contributors`). Absorbing agentgrep's source makes
graphgrep a derivative work of it, so its notice is owed. Deferred by choice, but
it must land before release or before kcode points at this.
