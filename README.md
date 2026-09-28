# kgrep

Code search and retrieval for agents: one shaper, several sources, a bounded
packet.

A library first, not a tool. `Query` in, `Packet` out, with relevance ranking, a
**token-denominated** budget, and structure in about a hundred languages. It answers
the questions an agent asks constantly, where is X defined, who calls X, what is in
this file, which files are about this topic, and answers with one ranked, budgeted
result instead of ten noisy ones.

Its consumer is kcode's search tool. The numbers below are measured through the same
call path the harness uses, not through a CLI the harness never touches.

## Measured

25 verified navigation tasks over a real repository, both tools driven the same way:

| | recall | median tokens to the answer | p90 | worst case |
|---|---|---|---|---|
| agentgrep (what kcode runs today) | 24/25 | 285.4 | 828.0 | 6,752 |
| **kgrep** | **25/25** | **28.8** | **464.8** | 16,061 |

Nearly 10x fewer tokens to reach an answer at the median, and it finds one more
question. The worst case is deliberately larger: agentgrep caps output and
occasionally drops the file holding the answer, and we chose to spend more tokens
rather than lose the answer.

The measurement record is `bench/README.md`, including the claims that were made
and later retracted, kept in place so the next reader can see how they died.

## Build, test, run

```bash
cargo build --release
cargo test                                   # 26 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Four verbs, one core:

```bash
kgrep grep   "parse_config"             # ranked matches, bounded by tokens
kgrep find   session_search             # ranked file discovery
kgrep outline crates/.../display.rs     # what a file declares
kgrep trace  subject:auth_status relation:rendered   # the relation DSL
```

Useful flags: `--json`, `--paths-only`, `--max-tokens`, `--unbounded` (the
deliberate opt-out), `--type`, `--glob`, and `--hidden`.

`outline` uses `ctags` when it is installed and the scanner cannot parse the
language, which is what gives it reach beyond Rust, TypeScript, JavaScript, Python
and Markdown. Nothing requires ctags; its absence costs coverage, not correctness.
`KGREP_CTAGS=off` forces the scanner, which is the control for measuring what ctags
buys.

## Measuring

```bash
scripts/bench.py --bin target/release/kgrep --corpus ~/kcode   # tokens and latency
scripts/memcheck.sh                                            # peak RSS against repo size
scripts/gate2.py --bin target/release/kgrep                    # bounds a mechanism's benefit
```

Three numbers matter, and they are tracked separately: **tokens** to answer, because
that is money and context; **latency**, because that is what makes a tool feel bad;
and **peak RSS**, because that is the guardrail.

Memory is a guardrail with a declared ceiling, not an objective. Nothing may scale
unboundedly with repository size. Bounded, or memory-mapped, or it does not ship.

## Where things live

Four documents, one job each. A fact that fits two lives in the more specific one and
the other links to it.

| file | answers |
|---|---|
| `AGENTS.md` | what is settled, what was measured, the roadmap, and the rules for working here |
| `bench/README.md` | what we measured, including the retractions |
| `docs/PRIOR-ART.md` | how other harnesses retrieve code, what we absorb, and why we are shaped this way |
| `README.md` | what this is, and how to use it |

## Code

```text
src/
  model.rs      // the core: Query, Where, Verb, Budget, Hit, Packet
  lexical.rs    // grep: matching, grouping, grouping under symbols
  find.rs       // ranked file discovery
  outline.rs    // file structure, and the ctags fallback for other languages
  trace.rs      // the relation DSL, and region expansion
  rank.rs       // four cheap ranking signals, one shared weight table
  packet.rs     // budgets and rendering, text and JSON
  scan.rs       // file walking, ignore rules, non-UTF-8 paths
  cli.rs        // the command surface, and the flags-to-Query adapter
  peak.rs       // peak RSS, behind KGREP_PEAK_RSS
bench/          // tasks, the harness-faithful oracle, findings
scripts/        // bench.py, memcheck.sh, gate2.py, parity.py
```

The one input type is `Query`: a shared narrowing, plus exactly one verb. The one
answer type is `Packet`, except for `outline`, whose shape genuinely differs.

## Using it as a dependency

```toml
kgrep = { git = "https://github.com/zipadoodlez/kgrep.git", rev = "<revision>" }
```

Pin a revision resolved from a tag (`git ls-remote <url> refs/tags/v0.1.1`) rather
than a tag or a branch, because a tag can move and a revision cannot. The interface
is frozen: additive changes only, so a pinned consumer takes additions by bumping the
revision and never has to absorb a rename.

## Status

Not shipped. kcode has the integration on a branch; its dependency is pinned and
verified by building an outside crate against it. The next capability is a
repository-wide tag index, so that `grep`, `find` and `trace` get structure in any
language rather than only `outline`.

## Licence

MIT. `kgrep` is a derivative work of [agentgrep](https://github.com/1jehuang/agentgrep),
whose notice is retained; see `LICENSE` and `NOTICE`.
