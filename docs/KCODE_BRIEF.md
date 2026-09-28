# Brief: kgrep behind kcode's search tool

For whoever works the kcode side. Self-contained: the kcode facts are read out
of the tree with file references, and the kgrep interface is frozen and real.

**Status.** The refactor landed as kcode `cd9e9147`. What follows is now four
things: a record of the seam as it actually is, the frozen kgrep interface for
reference, a short review checklist for that commit, and the next piece of work.

## What shipped, and the names involved

`cd9e9147` replaced the four `build_*_args` functions with one mapping, and the
four `render_*_output` calls with kgrep's renderers. The functions in the tool
after it:

| function | what it does |
|---|---|
| `query_from_params` (`agentgrep/args.rs`) | the one mapping, flags to `Query` |
| `narrowed_where` (`agentgrep/args.rs`) | the narrowing the sweeping verbs share, as kgrep's `Where` |
| `grep_budget` (`agentgrep.rs`) | the caller's cap, as a `Budget` |
| `filter_packet_to_exact_file` (`agentgrep.rs`) | the three single-file filters, collapsed |
| `outline_target`, `resolved_search_scope` | resolving a root and an outline target |

**All five load-bearing behaviours survived**, which was the main risk: the 5 s
foreground budget with background adoption (`AGENTGREP_FOREGROUND_BUDGET`,
`adopt_with_options`), single-file filtering, the >2 s slow-call warning,
`maybe_write_context_json`, and the four modes with the `grep` alias.

## Landability

Both blockers are cleared, and the dependency has a concrete answer.

| blocker | state |
|---|---|
| kgrep must carry agentgrep's MIT notice | **cleared**, `LICENSE` and `NOTICE` at `3ea9b3f` |
| `Cargo.toml` points at `../../../kgrep` | **answered below**: a private remote, pinned by revision |

### The dependency, answered

kgrep now has a remote. It is **private**, under `zipadoodlez/kgrep`.

Pin a **revision**, resolved from a tag, rather than a tag or a branch:

```bash
git ls-remote https://github.com/zipadoodlez/kgrep.git refs/tags/v0.1.1
```

```toml
# crates/jcode-app-core/Cargo.toml — replace the path dependency
kgrep = { git = "https://github.com/zipadoodlez/kgrep.git", rev = "<that revision>" }
```

Written as a command on purpose. agentgrep is pinned by tag, but a tag can be
moved and a revision cannot, and a revision written into this file is a second
copy of a value that already has a source of truth. Resolve it once, pin it, and
this file never goes stale.

As of `v0.1.1` that revision is `bc205e8362831ca234976bea44cf92b614950b5b`. The
tag exists for findability; the revision is what belongs in `Cargo.toml`.

**Verified, not assumed.** A throwaway crate outside kgrep, depending on nothing
but that line, fetched the repository and ran the frozen API:

```
Compiling kgrep v0.1.0 (https://github.com/zipadoodlez/kgrep.git?rev=3bef4ff8...#3bef4ff8)
pin works: 1 files, 2 matches
```

Two things to know about a private dependency:

- **Fetching needs credentials.** It worked here because `gh` is authenticated on
  this machine. Any other machine, or CI, needs `gh auth setup-git` or an
  equivalent token, or the build fails at fetch rather than at compile.
- **Making it public later is one command, and it discloses more than it looks
  like.** Nine files in the repository name kcode's internals, including crate
  and tool names, and the integration brief itself. That is a separate decision
  from landing this.

## The frozen interface

These signatures are frozen. Additive changes only from here, so building against
them is safe.

```rust
use kgrep::model::{Budget, FullRegionMode, Query, RenderOptions, StructuralQuery, Verb, Where};

// One input: the shared narrowing, plus exactly one verb.
pub struct Query { pub where_: Where, pub paths_only: bool, pub verb: Verb }

pub struct Where { pub root: PathBuf, pub glob: Option<String>,
                   pub file_type: Option<String>, pub hidden: bool,
                   pub no_ignore: bool, pub follow: bool }

pub enum Verb {
    Lexical { text: String, regex: bool },
    Path { terms: Vec<String>, max_files: usize },
    Outline { file: String, max_items: Option<usize> },
    Structural { query: StructuralQuery, max_files: usize,
                 max_regions: usize, full_region: FullRegionMode },
}

// One call per verb. Three return Packet; outline returns OutlineResult,
// deliberately, because the two answer shapes differ.
kgrep::lexical::run_grep(&query, budget) -> Result<Packet, String>
kgrep::find::run_find(&query, budget)    -> Result<Packet, String>
kgrep::trace::run_trace(&query, budget)  -> Result<Packet, String>
kgrep::outline::run_outline(&query)      -> Result<OutlineResult, String>

// Rendering reads the answer, not the flags.
kgrep::packet::render_grep_text(&packet)                         // no options
kgrep::packet::render_find_text(&packet, &options)
kgrep::packet::render_trace_text(&packet, &options)
kgrep::packet::render_outline_text(&result)

// The DSL parses at the edge, so the library takes a finished query.
kgrep::trace::parse_query(&terms) -> Result<StructuralQuery, String>
```

Three things worth knowing about it:

- **`Verb::Outline` keeps its own `file`.** An outline resolves that name *within*
  the root, so root and file are genuinely different things.
- **`paths_only` is not a print flag.** It is part of the question, and the packet
  records it, so a renderer honours it without being told twice.
- **Printing options did not ride along.** `RenderOptions` holds one field,
  `debug_score`, because that is the only thing printing needs that the packet
  does not already carry. `paths_only` and `full_region` turned out to be query
  options, not print options, so they live in `Query` and `Verb`.

### Two deltas from the brief that was handed over

If you are working from an earlier copy, these changed once the shape froze:

| earlier brief | frozen |
|---|---|
| `kgrep::run(root, &query, budget)` | four entry points, one per verb |
| "outline may fold into Packet" | it does not; it keeps `OutlineResult` |
| `Verb::Outline` folding its file into `Where` | it keeps its own `file` |
| `RenderOptions` holding `paths_only` and `full_region` | one field, `debug_score`; the other two are query options |

## Review checklist for `cd9e9147`

Two things to confirm rather than assume:

1. **`debug_plan` was dropped** on the grounds that no caller can observe it.
   Confirm that is true rather than convenient.
2. **`max_regions` now maps onto `Budget::max_total_matches`** in `grep_budget`.
   That is correct as far as it goes, but it is only the first of the three budget
   changes below, and on its own it makes the model's detail knob a count of match
   *records*.

## Next: the budget axis

kgrep's `Budget` has three knobs, and the third is the unit the whole objective is
measured in:

| field | default | bounds |
|---|---|---|
| `max_total_matches` | 20,000 | match records kept |
| `max_hits` | `Some(500)` | files listed, which is coverage |
| `max_detail_tokens` | `Some(8_000)` | tokens of detail |

What the model can reach today:

| model knob | maps to | verdict |
|---|---|---|
| `max_regions` | `Budget::max_total_matches` | works |
| — | `Budget::max_detail_tokens` | **unreachable** |
| `max_files` | `Verb::Path` / `Verb::Structural` only | **ignored for grep**, so `max_hits` is unreachable too |

Why it matters: a match line can be 240 characters, so "200 regions" ranges over
about a factor of five in output size. That is the finding in
`~/kgrep/bench/README.md`. The model's only detail knob is denominated in the unit
we measured to be the wrong one, and the token knob, which is what caps the
pathological 154k-token case, cannot be raised at all.

Three steps, all of them in kcode, because the schema and the mapping are both
here and kgrep already has every field:

1. **Add a token knob** to the schema (`max_tokens`), mapped to
   `Budget::max_detail_tokens`. This is the deliberate opt-out in the right unit.
2. **Decide `max_regions`.** Drop it, or keep it as a secondary record cap.
   Dropping is model-visible, so it is a deliberate call, not a tidy-up.
3. **Map `max_files` to `Budget::max_hits`** so coverage is controllable for
   grep, not only for find and trace.

No kgrep change is needed for any of it. `Budget`'s fields are public and
`..Budget::default()` sets one without naming the others.

## Gates

- **Do not rename the model-facing tool as part of this.** The name reaches the
  schema, the alias table, config flags like `show_agentgrep_output`, display
  strings and tests. That is a separate, surveyed change.
- **Do not build a search index inside a search.** A first-use build is ~514 ms on
  a normal repo and minutes on a monorepo, and anything over 5 s is promoted to a
  background task. The index will be an explicit API that kcode schedules on its
  own. Until it exists, searches run without one and nothing is worse than today.
- **Do not add a second way to get file structure.** kgrep owns that precedence
  (index, then a per-file fallback, then its own scanner) in one place. If you
  find yourself branching on structure in kcode, stop and raise it.

## How to verify

- **Parity on the edges.** `~/kgrep/scripts/parity.py`, 13 cases: non-UTF-8
  names, symlinks, globs, glob+type, hidden, no-ignore, binary, empty,
  extension-less. All agreed as of Stage 3.
- **The benchmark.** `~/kgrep/scripts/bench.py`, 25 verified tasks, recall and
  tokens-to-answer. Head-to-head in `~/kgrep/bench/baseline.json`: kgrep at a
  median of 28.8 tokens to the answer against agentgrep's 285.4.
- Do not run these casually. They walk the whole repository.

## What to report back

- Whether the `Query` mapping was as clean as it looks, or whether a tool param
  has no honest home in it.
- What the 5 s budget did in practice now that the seam moved. Did anything new
  get promoted to the background?
- Anything in this brief that was wrong. It was written from reading the tree, and
  reading is not running.
