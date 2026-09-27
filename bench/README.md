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

## Baseline, 2026-09-27

17 tasks, kcode as corpus, `rg` absent so agentgrep uses its native fallback
(native versus native, an honest comparison).

| tool | recall | mean tok→ans | median | p90 | max output tok |
|---|---|---|---|---|---|
| agentgrep 0.1.6 | 17/17 | 6,340 | 383 | 1,762 | 154,351 |
| graphgrep | 17/17 | 6,339 | 383 | 1,762 | 154,356 |

Excluding the two deliberately generic queries (`grep-swarm-stress`,
`grep-todo-stress`):

| tool | mean tok→ans | median | max |
|---|---|---|---|
| agentgrep 0.1.6 | 481 | 379 | 1,762 |
| graphgrep | 480 | 378 | 1,762 |

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

