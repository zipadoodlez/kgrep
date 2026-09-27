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
  --bin target/release/graphgrep \
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
high-water mark is trustworthy, so only graphgrep can currently be measured this
way.

**kcode, 1,234 tracked files:**

| query | peak RSS |
|---|---|
| `grep` (matches nothing) | 5,536 KB |
| `grep --paths-only fn` | 5,660 KB |
| `grep fn` (very heavy result) | 12,644 KB |
| `grep --type rs fn` | 12,628 KB |

**Scaling, same query over synthetic repos:**

| files | peak RSS |
|---|---|
| 100 | 4,824 KB |
| 1,000 | 5,120 KB |
| 5,000 | 6,108 KB |

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

| tool | recall | median tok→ans | p90 | max out tok | median latency | p95 |
|---|---|---|---|---|---|---|
| agentgrep 0.1.6 | 17/17 | 383 | 1,762 | 154,351 | 30.3 ms | 44.2 ms |
| graphgrep | 17/17 | 383 | 1,762 | 154,356 | 64.3 ms | 100.7 ms |

Latency is per call, including process start-up, which a harness calling
in-process does not pay. So these overstate what kcode sees and are best read as
an upper bound and as a relative comparison between the two tools.

Excluding the two deliberately generic queries (`grep-swarm-stress`,
`grep-todo-stress`), tokens to answer are 379 median for both tools, while the
latency gap is unchanged, so it is not caused by output size.

## The latency finding, and it is my regression

Tokens and recall are identical, and graphgrep is **2.2x slower** on grep while
matching exactly on outline. The cause was guessed and then verified rather than
asserted. Best of five runs, `grep webfetch` over kcode, 8 cores available:

| | all cores | pinned to one core (`taskset -c 0`) |
|---|---|---|
| agentgrep 0.1.6 | 21 ms | 89 ms |
| graphgrep | 71 ms | 84 ms |

Two things fall out:

1. **agentgrep gets a 4.2x speedup from parallelism**; graphgrep does not move,
   because it is serial. So parallelism is the entire cause.
2. **Single-threaded, graphgrep is slightly faster** (84 ms versus 89 ms), which
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

**5. graphgrep matches agentgrep to within one token per task**, which is what
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

