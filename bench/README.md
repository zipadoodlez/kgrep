# Benchmark: does retrieval surface the answer, and how much must be read?

Stage one of the project. We measure the tool, not the model, because the
question "is this better than agentgrep" is about what the tool hands the model,
and that is measurable without spending a single API token.

## How it works

`bench/tasks.json` holds navigation tasks about a real corpus (kcode by
default). Each has a verified ground truth: `expect_path` names a file that must
appear in the output, `expect_symbol` names a real definition in that file. Every
one was checked to exist before being written down.

`scripts/bench.py` runs a binary over the tasks and reports, per task:

- **found** — did the ground truth appear in the output at all (recall).
- **tok→ans** — estimated tokens of output a reader must get through before the
  ground truth appears. This is the cost that actually matters: tokens spent
  looking.
- **out tok** — estimated tokens of the whole output.

Tokens are estimated at 4 characters per token, which is comparable across tools
and honest about being an approximation. Character counts are exact.

```bash
scripts/bench.py \
  --bin <agentgrep-binary> \
  --bin target/release/kgrep \
  --corpus ~/kcode \
  --json bench/baseline.json
```

To build the agentgrep oracle, use the version kcode actually pins (`v0.1.6`,
`rev b01b804`), which Cargo already has checked out under
`~/.cargo/git/checkouts/`. Build a copy of it; do not clone `main`, which is a
later release and would lock in the wrong behaviour.

## Memory baseline, 2026-09-27

Measured with `scripts/memcheck.sh`, reading the tool's own `/proc/self/status`
high-water mark.

**Method warning.** An earlier attempt measured with an external wrapper and
reported a flat 13.7 MB for every query and both binaries. That was the wrapper:
a forked parent shares its pages with the child, so the wrapper's own footprint
lands in the child's peak. `posix_spawn` plus `wait4` is better but still
inflated (11,676 KB where the tool reports 5,460 KB). Only the tool's own
high-water mark is trustworthy, so only kgrep can currently be measured this
way.

**kcode, 1,234 tracked files:**

| query | peak RSS |
|---|---|
| `grep` (matches nothing) | 5,536 KB |
| `grep --paths-only fn` | 5,660 KB |
| `grep fn` (very heavy result) | 12,644 KB |
| `grep --type rs fn` | 12,628 KB |

**Scaling, same query over synthetic repos:**

| files | before 1c-i | after 1c-i |
|---|---|---|
| 100 | 4,824 KB | 5,540 KB |
| 1,000 | 5,120 KB | 5,636 KB |
| 5,000 | 6,108 KB | 5,932 KB |

The trade is visible and it is the one we chose: **the baseline rose by about
700 KB and the per-file term fell by roughly 3.6x**, from 268 bytes per file to
about 75. That is threads and their buffers bought with memory to remove the
only structural term that grew with the repository. At 100,000 files the old
shape cost 26.8 MB of file list; the new one costs under a tenth of that.

The residual 75 bytes per file is not a held list, since nothing is collected any
more. It is most likely allocator retention across the walk. Worth confirming at
10x the size before treating it as noise, and worth folding into `memcheck` as
an assertion once Stage 1c is complete.

## The finding: memory is not flat, and the cause is the file list

4,824 KB at 100 files and 6,108 KB at 5,000 files is **1,284 KB over 4,900
files, or about 268 bytes per file**. Extrapolated, a million-file repository
costs roughly 268 MB before any search work happens.

The cause is specific: `collect_file_entries` **materializes the whole file list**
into a `Vec<FileEntry>` before scanning, because the scan wants a sorted list and
wants to chunk it across threads. That is the only term in the tool that scales
with the repository, and it is exactly the class of growth the memory rule
forbids.

The floor of roughly 4.8 MB is process baseline, binary and libraries, which is
fine. The 12,644 KB on a very heavy result is the match budget working: `fn`
matches far more lines than the 20,000-match cap, so storage is bounded, at
roughly 0.4 KB per stored match.

**The fix is already the next planned change.** `ignore`'s parallel walker
(`build_parallel`) streams entries instead of collecting them, which removes the
linear term *and* supplies the parallelism that the latency regression needs. So
the file-list materialization and the missing threading are one fix, not two, and
both land in Stage 1c.

## Provisional ceiling

Until the streaming walker lands, nothing can honestly be declared. Provisionally:
**32 MB peak on any repository up to 100,000 files** — comfortable against every
number above except the extrapolated file list, which this ceiling is intended to
force out. Re-measure after Stage 1c and tighten or confirm.

## Baseline, 2026-09-27

17 tasks, kcode as corpus, `rg` absent so agentgrep uses its native fallback
(native versus native, an honest comparison). Measured on a quiet machine after
an earlier run was contended and had to be discarded.

**At absorption, before ranking existed.** This is the parity evidence: the port
reproduced agentgrep's output to within a token per task.

| tool | recall | median tok→ans | p90 | max out tok |
|---|---|---|---|---|
| agentgrep 0.1.6 | 17/17 | 383 | 1,762 | 154,351 |
| kgrep (path order) | 17/17 | 383 | 1,762 | 154,356 |

**Harness-faithful, all four verbs, 25 tasks.** The reference is now
`bench/oracle`, which links agentgrep `v0.1.6` at the pinned revision
`b01b8040` and renders through the same call path kcode uses, including the
`Some(200)` grep cap. This is the honest oracle, and it replaces the raw CLI
numbers above.

| tool | recall | median tok→ans | p90 | max out tok | median latency |
|---|---|---|---|---|---|
| agentgrep 0.1.6, harness path | 24/25 | 285.4 | 828.0 | 6,752 | 28.1 ms |
| kgrep | **25/25** | **28.8** | **464.8** | 16,061 | 26.7 ms |

Four things, and two of them are corrections to earlier claims.

**1. The harness path was already bounded, and now that is measured rather than
inferred.** agentgrep's worst case through the oracle is 6,752 tokens, not
154,351. The earlier retraction was right, and the oracle confirms it.

**2. agentgrep loses a task.** With the 200-match cap applied at render time over
a list sorted by path, the file holding the answer for `grep swarm` is never
rendered at all. Recall 24/25. This is the failure mode predicted in
`docs/AGENTGREP.md`, weakness 1, now demonstrated: truncating an unranked list
drops the answer, not merely the detail.

**3. kgrep holds 25/25** because coverage is not what it cuts. The bound drops
detail first, and names every file up to a separate cap of 500, so the answer is
present even when its detail is not.

**4. On the worst case kgrep is *larger*: 16,061 against 6,752.** That is the
trade, made deliberately and now visible: agentgrep is cheaper because it stops
early, and it stops early enough to lose the answer; kgrep spends more to keep it.
The 500-file coverage cap is the knob if that trade should move, and a middle
setting would be worth measuring.

So the honest summary against the harness path is: **10x better on median tokens,
1.8x on p90, and one more task found, at a larger worst case.** Not the 10x on
every axis the CLI comparison suggested.

Latency is per call, including process start-up, which a harness calling
in-process does not pay. So these overstate what kcode sees and are best read as
an upper bound and as a relative comparison between the two tools.

## Stage 1c-ii: the packet bound

The bound is two things, because there were two unbounded terms.

**Detail** is bounded by an estimated token budget, `--max-tokens`, spent in
ranked order after ranking. A hit that cannot afford detail keeps its name, role,
score and true match count, and **releases the memory its detail held**. The top
hit always gets full detail even if it alone exceeds the budget, so a single
dense file cannot leave the caller with nothing.

**Coverage** is also capped, at `--max-hits`, because the file list is itself a
term that grows with the repository: a generic term in a large repo matches tens
of thousands of files and their names alone would blow any budget. This was the
second unbounded term and it was easy to miss.

Both caps report the truth rather than hiding it:

```
... 7338 more matches counted but not stored (budget reached)
... 287 of 306 listed files have no detail; raise --max-tokens or narrow the query
... 41 more matching files not listed; raise --max-hits or narrow the query
```

Effect on the two generic queries, which is what the bound was for:

| task | agentgrep tok→ans | kgrep before | kgrep now |
|---|---|---|---|
| `grep swarm` | 93,943 | 93,943 | **14,295** |
| `grep todo` | 6,628 | 6,628 | **922** |

Worst-case output fell from 154,351 to 16,061 tokens, **9.6x**. Recall is
unchanged at 17/17, and median tokens to the answer is unchanged at 100.8, so the
bound costs nothing on ordinary queries.

It also reduced peak memory on heavy queries, from 14,024 KB to 10,204 KB,
because a summarized hit frees the detail it was holding rather than merely not
printing it.

## Stage 1 exit, all four met

| criterion | required | measured |
|---|---|---|
| recall | stays 17/17 | 17/17 |
| generic-query tokens | down at least 5x | 6.6x and 7.2x |
| latency | at or below agentgrep | 18.1 ms versus 21.5 ms |
| peak RSS | under ceiling, flat 1k→5k files | 5,720 → 5,880 KB, about 40 bytes per file |

The ceiling was 32 MB for repositories up to 100,000 files. At 40 bytes per file
that projects to roughly 4 MB of file-count growth at 100,000 files, leaving the
rest of the ceiling for an index.

## The latency finding, and it is my regression

Tokens and recall are identical, and kgrep is **2.2x slower** on grep while
matching exactly on outline. The cause was guessed and then verified rather than
asserted. Best of five runs, `grep webfetch` over kcode, 8 cores available:

| | all cores | pinned to one core (`taskset -c 0`) |
|---|---|---|
| agentgrep 0.1.6 | 21 ms | 89 ms |
| kgrep | 71 ms | 84 ms |

Two things fall out:

1. **agentgrep gets a 4.2x speedup from parallelism**; kgrep does not move,
   because it is serial. So parallelism is the entire cause.
2. **Single-threaded, kgrep is slightly faster** (84 ms versus 89 ms), which
   is roughly the few milliseconds agentgrep spends trying to spawn `rg` before
   falling back. The absorbed serial work is therefore on par or better, and the
   regression is exactly one missing thing: the threaded scan.

This is the first evidence for measuring three axes rather than one. On tokens
and recall the two tools are indistinguishable, and the entire difference is
visible only in latency.

The fix is not simply "add threads", because it interacts with the bounded
packet: parallel workers need either per-worker bounded heaps merged at the end,
or a cheap two-pass over a ranked shortlist. Bounding and parallelism have to be
designed together.

## Differential parity, 2026-09-27

`scripts/parity.py` builds a corpus that is nasty on purpose and compares the
**set of files found** by kgrep and by the oracle. Not the rendering: whether
they look at the same files, which is where two implementations of a walker
drift apart and which no normal repository exercises.

Thirteen cases, all agreeing: literal and regex grep, `--type`, `--glob`,
`glob + type` together, `--hidden`, `--no-ignore`, an extension-less file, a
symlink to a file, a symlink to a directory, a name that is not valid UTF-8, a
binary file, and two `find` queries.

Comparison strips the display suffix, because the two tools deliberately spell
non-UTF-8 names differently (agentgrep `#b=`, kgrep `#raw=`, both injective).
Whether they find the same files is the question; how they render an
unrepresentable name is a documented difference.

### What the harness found

**A real behavioural difference, and a version difference underneath it.**
agentgrep v0.1.6 has no `follow_links` call at all, so it uses the `ignore`
crate's default of false. But it also has no symlink guard, so a symlink **to a
file** is searched, because `is_file()` follows the link, while a symlink **to a
directory** is not descended. kgrep had inherited v0.1.7's guard, which excluded
both. The guard is gone and kgrep now matches 0.1.6 exactly, with `--follow`
adding directory symlinks on top.

**That settled a flag.** Following symlinks landed in v0.1.7; kcode runs 0.1.6,
and ripgrep does not follow by default either. So the default is now *not* to
follow, with `--follow` as the opt-in, replacing `--no-follow`. The duplicate
paths that following produced, one under the real path and one under the link,
are gone with it.

**And `outline` now closes the round trip.** A path printed by `grep` for a name
that is not valid UTF-8 carries a `#raw=<hex>` suffix; `outline` decodes it, so
search-then-read works for exactly the files whose names cannot be retyped.

Note that parity is against the **pinned** version. `--no-follow` does not exist
in 0.1.6, so it cannot be compared there at all, and anything else that landed in
v0.1.7 needs the same treatment: check which version kcode runs before calling a
difference a bug.

## The ranking experiment, 2026-09-27

"Rank before spending" needs a score, and grep had none. Six variants were
measured against the same 17 tasks, with recall unchanged at 17/17 throughout:

| signals | median tok→ans | p90 |
|---|---|---|
| none (control, path order) | 383.2 | 1,762.5 |
| path | 286.5 | 974.5 |
| symbol | 234.2 | 1,893.0 |
| specificity | 454.8 | 2,495.8 |
| role | 322.0 | 1,647.5 |
| **all** | **100.8** | **703.8** |

`all` is **3.8x better on the median and 2.5x better on p90**, at no measurable
latency cost (52.5 ms versus 52.7 ms). Adopted as the default; `KGREP_RANK=none`
restores path order as the control.

**A signal that sounds sensible hurts on its own.** Specificity, meaning "prefer
files with fewer matches", is *worse than doing nothing*: 454.8 against 383.2, and
p90 almost 42% worse. It only helps in combination. This is why the variants were
measured separately instead of the whole set being adopted on intuition.

**Per task, 9 improved sharply, 4 unchanged, 3 got worse.** The unchanged four are
the outline tasks, where ranking does not apply. The improvements are large
(974.5 to 20.8; 1,762.5 to 25.8; 828.0 to 27.5), so the median moves for real.

**A retracted claim: the three regressions were blamed on missing struct fields,
and that was wrong.** The earlier version of this section said they share one root
cause, that the line scanner cannot see struct fields, and that Stage 4 would close
them. Measured with ctags in hand, that is not what is happening. They are three
separate problems, and no structural source reaches any of them.

The instrument: `ctags` 6.2.1 over kcode, `--fields=+nKSZ --extras=+q`, which emits
**41,249 tags in 7.5 MB in 6.5 s**, and it does emit fields exactly as hoped:

```
show_agentgrep_output  crates/jcode-config-types/src/display.rs  field  line:64  scope:struct:DisplayConfig
```

So the mechanism the claim rested on is real. It just does not decide these tasks.

- **`grep-config-flag` is a name ambiguity, not an invisible field.**
  `show_agentgrep_output` is declared as a field in **three** files, and
  `crates/jcode-tui-messages/src/cache.rs` is the one that wins, with 3 matches to
  `display.rs`'s 2. Both are inside the maximally-specific band, so they tie on
  specificity, and both score zero on symbols today. ctags gives *both* the exact
  tag, and gives cache.rs two of them against display.rs's one, so it would widen
  the gap rather than close it. Telling them apart means knowing which definition
  the query *means*, which is resolution and centrality, not structure.
- **`grep-swarm-stress` is path dominance.** The query is one generic word, and
  `PATH_FULL` is worth 120, so every file literally named `swarm.rs` outranks the
  answer in `communicate.rs`, whose path says nothing. ctags cannot touch this, and
  it would add exact `swarm` tags in five more files. This is a weighting question
  and it is measurable without ctags at all.
- **`grep-mcp-tool` has no declaration to find.** No ctags tag named `mcp_call`
  exists anywhere in kcode. It is a string-keyed tool name in a table, and the file
  that ranks first is a consumer. There is no structure for a structural source to
  return.

So "the ranking is doing its job on incomplete structure data" was generous to the
ranking and unkind to the structure. Two of the three are questions structure
cannot answer, and the third is a weight we chose. The honest candidates are a
ranking change for path dominance, and reporting ambiguity rather than pretending
to resolve it.

**Re-run on the current 25-task bench** (`bench/rank-all.json` and
`bench/rank-none.json`, written by the run above): median tokens-to-answer **28.8**
with ranking against **286.5** without, p90 464.8 against 974.5, recall 25/25 both
ways. Per task, 8 of the 12 grep tasks improve, the same 3 regress, and one is
unchanged, so the picture holds on the wider set rather than resting on the
original 17.

**The path hypothesis, tested and rejected.** `grep-swarm-stress` looked like path
dominance: the query is one generic word, `PATH_FULL` is 120, and every file named
`swarm.rs` outranks the answer. Sweeping the weight refutes it.

| `PATH_FULL` | median tok->ans | p90 | swarm-stress |
|---|---|---|---|
| 120 (current) | 28.8 | 464.8 | 14,271 |
| 60 | 28.8 | 464.8 | 14,296 |
| 30 | 32.2 | 1,037.2 | 14,264 |
| 0 | 100.8 | 1,083.0 | 14,188 |

Lowering the path weight never helps the task and eventually costs 3.5x the median,
so the weight is earning its place. Removing the signal entirely moves the target by
83 tokens out of 14,271, which is 0.6% and not a fix.

While there: a single-token query that matches a path was scored twice, once as the
whole query and once as the token, because for one token those are the same substring
test. That double count is removed, and it is measurably neutral on this bench, so it
is a correctness fix rather than a win.

**What the task actually needs.** The answer is not a path or a declaration at all.
`crates/jcode-app-core/src/tool/communicate.rs:874` holds the string literal that
registers the tool, since `CommunicateTool::new().name() == "swarm"`. That is **the
same shape as `grep-mcp-tool`**: two of the three regressions are one problem, a name
registered as a string, which no path, symbol table or tag index can see. The third,
`grep-config-flag`, is the ambiguity above. So the count is one ambiguity and two
string-keyed names, and structure reaches neither.

Every cheap signal was tried against this task and none of them picks that file:

- path: it is 94th in path order, and the control already places it 10,812 tokens deep.
- declared symbols: `server/swarm.rs` leads, not `communicate.rs`.
- match count: `server/swarm.rs` has 745 against `communicate.rs`'s 123.
- whole-word count: `server/swarm_persistence_tests.rs` has 102, `communicate.rs` 61.
- an exactly-quoted `"swarm"` literal alone: test files lead, 9 each.

`communicate.rs` is the right answer because it *registers* the tool, which is a fact
about what the name refers to rather than about where the text sits. That is the
resolution problem, and it is Stage 6, not a weight. **Recommendation: record this
task as beyond the cheap tier rather than tune toward it**, and note that
`server/swarm.rs` with 745 mentions is a defensible answer for a human too, so the
label itself deserves a second look.

**Caveat.** These weights were chosen on 17 tasks and borrowed from agentgrep's
own finder. The signal *choice* is well supported by this set; the constants are
not tuned, and the task set is small. A second, larger set would harden it.

## Gate 1: does harness context do anything? 2026-09-28

Before porting agentgrep's harness-context mechanism to kgrep, or writing a
spec for it, the cheap question comes first: does it change any output at all?
It has to be asked against agentgrep, because kgrep accepts `--context-json` and
ignores it, so kgrep was never an implementation of this.

Method: `bench/oracle/target/release/agentgrep-harness trace subject:lsp
relation:implementation kind:code path:src/tool --path <fixture>`, run four
times, once per context file. The fixture is one file with eleven top-level
items, so the structure tiers have room to show, and it is in `target/`, which is
ignored.

| arm | lines | chars | what changed |
|---|---|---|---|
| no context | 97 | 2669 | — |
| `known_files` | 94 | 2557 | structure 10 to **6** items, plus "reduced repeated structure from harness context" |
| plus `focus_files` | 94 | 2556 | structure 10 to **4** items, plus "compressed file structure from harness context" |
| plus `known_regions` | 88 | 2530 | 3 regions `full region` to **`snippet`**, plus 3 "compressed repeated region" notes |

So it is live, and the three thresholds are hard-coded and conjunctive in
`smart_engine.rs`:

```
structure_budget_for_file   10 items, or 6 if structure>=0.8 and version>=0.6
                            and prune>=0.7, or 4 if the file is also focused
should_prune_region         prune>=0.7 and body>=0.7 and version>=0.6
```

### What it turned out to be, and it is not one mechanism

**The two halves carry different risk.** Region "pruning" is *compression*: the
region keeps its signature line, `full region` becomes `snippet`, and a note says
why. The agent still knows the region exists and where. Structure "pruning" is
**deletion**: the items are gone, collapsed into `... N more symbols`. This
benchmark has already lost a task to a cap that deleted the answering thing, so
the half that deletes is the half to be careful with.

**It is inert without a calibrated producer.** Every gate needs confidences above
0.6 to 0.8, and `Familiarity`'s fields default to zero, so a `context.json`
listing paths with no confidences does exactly nothing. Whatever value this
feature has lives in the producer's calibration, not in the consumer's logic.

**The magnitude is small, and this fixture is its best case.** Here 100% of the
hits are in the marked file, and the whole output still moves only 4 to 5% in
characters. Structure is capped at 10 by default, so the tiers only bite on files
with more than 6 items, and only on files the harness has already marked known.

### Why this matters for the port decision

A 10x was already won by *ordering*, which puts the answer first. This mechanism
trims a few percent off the block for a file already known, so it is a
second-order effect on top of a first-order fix, and the opportunity it addresses
is mostly gone. Combined with the confidences nothing currently emits, that is
most of the reason dropping it during the `trace` port cost nothing measurable.

**Verdict: gate 1 passes, and it does not decide the port.** It establishes that
the mechanism is real, that it is conservative where it compresses and destructive
where it deletes, and that its best case is a few percent. Whether that is worth
having on top of kgrep's ranking is gate 2, which measures it against the oracle's
ranking replaced by kgrep's.

## What this tells us

> **Correction, same day. The max-output claim below is wrong for the harness
> path and is retained only to show the mistake.** This benchmark drives the
> **CLI**, which renders with no cap. jcode does not use the CLI: it calls
> `run_grep` and renders with `render_grep_output(..., Some(200))`, capping
> displayed matches at 200 and reporting the true total in the header. The model
> never sees 154k tokens of grep output. The 154k number describes a path the
> harness never takes. See "Where the guard actually lives" below.

**1. Recall is not the problem.** Both tools find the answer on every task. Any
project that only claims "we find it too" has no case.

**2. Specific queries are already good.** Median 383 tokens to the answer is a
reasonable price, and both tools pay it. The cap does not bind on these, so this
holds on the harness path too.

**3. (Retracted) Generic queries are catastrophic.** The CLI dumps 154,351 tokens
for `swarm`, but that is the uncapped CLI path. Re-measurement through a
harness-faithful path is required before any claim about what the model sees.

**4. The mean is still a lie.** Even where outliers are real, a mean dominated by
two tasks misrepresents the distribution. Median and p90 belong in the report.

**5. kgrep matches agentgrep to within one token per task**, which is what
absorption should look like and is the first evidence the port is faithful.

## Where the guard actually lives

jcode already hit this problem and fixed it in the harness:

```rust
// ... a search for a common key across 2,027 benchmark transcripts produced
// 923k chars in a single call. The header still reports the true total ...
let max_regions = params.max_regions.or(Some(DEFAULT_GREP_MAX_REGIONS));  // 200
```

Two consequences, and the second is the real finding:

1. **The benchmark must mirror this path.** A bench binary that links agentgrep
   and calls `run_grep` then `render_grep_output(Some(200))` is the only honest
   oracle for what a model sees. The CLI is not.
2. **The guard is in the wrong home.** It lives in the caller, so every consumer
   must remember to pass it. That is exactly why the CLI omits it, and why a new
   caller would reintroduce the 923k-character bug. A safety default belongs in
   the library, where it cannot be forgotten. That is a real improvement, and a
   different one from "add a budget that does not exist".

## The first thing to build (revised)

**Move the output cap into the library default**, so the bounded packet is what
you get without asking, and `--unbounded` (or an explicit `Budget`) is the
deliberate opt-out. Then re-measure through a harness-faithful bench binary and
require that recall does not regress.

