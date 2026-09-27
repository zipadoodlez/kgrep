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

**1. Recall is not the problem.** Both tools find the answer on every task. Any
project that only claims "we find it too" has no case.

**2. Specific queries are already good.** Median 383 tokens to the answer is a
reasonable price, and both tools pay it.

**3. Generic queries are catastrophic, and it is the same in both tools.**
Searching for `swarm` produces **154,351 tokens of output** and takes **93,943
tokens** to reach the answer. That is larger than a full context window, for one
query, and it is the single worst thing either tool does. `todo` costs 79,038.

This is the measured version of a gap we had only reasoned about: **there is no
budget on the output, only on matches.** Both tools answer a generic query by
dumping everything they found.

**4. It also means the mean is a lie.** 6,340 mean tokens is two outliers
dragging fourteen good results. Median and p90 belong in the report, not the
mean.

**5. graphgrep matches agentgrep to within one token per task**, which is what
absorption should look like and is the first evidence the port is faithful.

## The first thing to build

**A token budget on the packet.** Not a match cap, a token cap: the shaper
should stop, report what it dropped, and say so, rather than returning 154k
tokens. This is measurable with the harness that now exists, so it can be proven
better rather than asserted.
