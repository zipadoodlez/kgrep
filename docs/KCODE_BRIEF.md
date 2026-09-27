# Brief: putting kgrep behind kcode's search tool

For the agent working in the kcode tree. You do not have the kgrep-side session
context, so this is self-contained. Read the whole thing before touching code:
the ordering at the end matters more than any individual edit.

**Status of this brief.** The facts about kcode are verified with file and line
references. The kgrep interface in "The contract" is **built and frozen** as of
commit `f0b747f`, so it is safe to build against. Additive changes only from
here. Anything that would rename or reshape it is a conversation first, not a
commit.

## What is being done, in one paragraph

kcode's search tool is backed by `agentgrep` today, linked in-process as a
library. It is being replaced by `kgrep`, which absorbs agentgrep's engine and
adds ranking, a token-bounded packet, and structure in about a hundred languages.
The tool keeps working the way the model expects. What changes is the library
behind it, and the shape of the seam between them.

## Where things are

| what | where |
|---|---|
| the tool | `crates/jcode-app-core/src/tool/agentgrep.rs` |
| its params | `crates/jcode-app-core/src/tool/agentgrep/args.rs` |
| harness state | `crates/jcode-app-core/src/tool/agentgrep/context.rs` |
| its tests | `crates/jcode-app-core/src/tool/agentgrep_tests.rs` |
| registration | `crates/jcode-app-core/src/tool/mod.rs:346` |
| the dependency | `crates/jcode-app-core/Cargo.toml:97` |
| the alias table | `crates/jcode-tool-core/src/lib.rs:385-396` |

Five things about the current seam that shaped this design:

1. **The model mostly does not say `agentgrep`.** The alias table maps `grep`,
   `file_grep` and `Grep` onto it, and kcode's native grep tool was deleted in
   favour of agentgrep. So from the model's point of view, **`grep` already is
   this tool.**
2. **It is one tool with four modes**, not four tools: `grep`, `find`, `outline`,
   `trace`, selected by a `mode` parameter on one schema
   (`agentgrep.rs:194-247`).
3. **There is a 5 second foreground budget** (`agentgrep.rs:23`). Past it the
   search is adopted into a background task and the model is told to poll it with
   `bg` (`agentgrep.rs:274-314`). A slow tool is still a working tool here, but
   being promoted to the background for a one-line grep is a bad experience, so
   nothing may become slow by default.
4. **Bounds live in the caller**: `DEFAULT_GREP_MAX_REGIONS = 200`
   (`agentgrep.rs:88`), with find and outline defaulting to 5 files and 6
   regions.
5. **Everything is synchronous and blocking**, offloaded with `spawn_blocking`
   (`agentgrep.rs:262`). Keep it that way: no async inside the search path.

## The contract

kgrep's entry points used to take clap structs, which is why this tool has four
`build_*_args` functions: it was reverse-engineering a command line to call a
library. They now take one `Query`, and the CLI is a thin adapter over it. These
signatures are frozen.

```rust
use kgrep::model::{Budget, FullRegionMode, Query, RenderOptions, StructuralQuery, Verb, Where};

// The one input: the shared narrowing, plus exactly one verb.
let query = Query {
    where_: Where { root, glob, file_type, hidden, no_ignore, follow },
    paths_only: false,
    verb: Verb::Lexical { text: "foo".into(), regex: false },
};

// The one call per verb, all taking the same input shape.
let packet = kgrep::lexical::run_grep(&query, Budget::default())?;  // -> Packet
let packet = kgrep::find::run_find(&query, Budget::default())?;     // -> Packet
let packet = kgrep::trace::run_trace(&query, Budget::default())?;   // -> Packet
let result = kgrep::outline::run_outline(&query)?;                  // -> OutlineResult

// Rendering reads the answer, not the flags.
let text = kgrep::packet::render_grep_text(&packet);                       // grep needs no options
let text = kgrep::packet::render_find_text(&packet, &render_options);
let text = kgrep::packet::render_trace_text(&packet, &render_options);
let text = kgrep::packet::render_outline_text(&result);
```

The four verbs are not folded into one entry point, and that is deliberate:
three return `Packet` and `outline` returns `OutlineResult`, because the two
output shapes genuinely differ. One entry would have meant one renderer that
branches four ways. Dispatch on the mode, as this tool already does.

### Why four functions rather than one

The verb is the only thing that differs, and the outputs are not interchangeable.
If that changes, it changes by adding an entry point, not by reshaping the input.

### The mapping from tool params stays in kcode

The shared thing is the type, not the mapping. The CLI has its own adapter
(`cli.rs`, `to_query` per verb) and this tool's params are a different shape, so
the mapping belongs here:

```rust
fn query_from_params(params: &AgentGrepInput, root: PathBuf) -> Result<Query, String> {
    let where_ = Where { root, glob: params.glob.clone(), file_type: params.r#type.clone(),
                         hidden: false, no_ignore: false, follow: false };
    let paths_only = params.paths_only.unwrap_or(false);
    let verb = match params.mode.as_str() {
        "grep"    => Verb::Lexical { text: params.query.clone().unwrap_or_default(),
                                     regex: params.regex.unwrap_or(false) },
        "find"    => Verb::Path { terms: params.query.clone().into_iter().collect(),
                                  max_files: params.max_files.unwrap_or(10) },
        "outline" => Verb::Outline { file: params.file.clone().unwrap_or_default(),
                                     max_items: None },
        // The mode the model says is `smart`; kgrep calls it `trace`.
        "trace" | "smart" => Verb::Structural {
            query: kgrep::trace::parse_query(&params.terms.clone().unwrap_or_default())?,
            max_files: params.max_files.unwrap_or(5),
            max_regions: params.max_regions.unwrap_or(6),
            full_region: FullRegionMode::Auto,
        },
        other => return Err(format!("unknown mode: {other}")),
    };
    Ok(Query { where_, paths_only, verb })
}
```

Two things about it:

- **`paths_only` is not a print flag.** It is part of the question, and the
  packet records it, so a renderer honours it without being told twice.
- **`Verb::Outline` keeps its own `file`** rather than folding it into `Where`,
  because an outline resolves that name *within* the root, so the two are
different things.

### The two decisions, now closed

- **`outline` keeps its own output type.** It was not folded into `Packet`.
- **`intent` is not a `Query` field.** It is still dropped at the seam, and can
  be added later as an optional field without breaking anything.

## What to change, and what it replaces

Net effect: **delete four arg builders, four run calls and four render calls;
add one mapping function, one call and one render.** The tool's schema, mode
names and behaviour do not change.

| in kcode | becomes |
|---|---|
| `build_grep_args`, `build_find_args`, `build_outline_args`, `build_smart_args_and_query` | one `query_from_params` |
| `run_grep` / `run_find` / `run_outline` / `run_smart` | the same four names, each taking `&Query` |
| `render_grep_output` / `render_find_output` / `render_outline_output` / `render_smart_output` | the render functions above, taking `&RenderOptions` where they need it |

Keep `execute_linked_agentgrep`'s structure: the mode dispatch stays (it builds a
different `Query` per mode), and so do the `filter_*_to_exact_file` helpers, which
are a caller-side narrowing and have nothing to do with the library.

## Five behaviours you must not regress

1. The four modes, their mode names, and the `grep` / `file_grep` / `Grep` alias.
2. The 5 s foreground budget and the background adoption path.
3. Single-file filtering when the model passes one file as `path`.
4. The >2 s slow-call warning (`agentgrep.rs:356-361`).
5. `context.json` still being written per call, even though kgrep does not read it
   yet. It is harness state and it is the thing no external tool can have.

Also: the tool's params schema must keep every property it has today. Adding a
`max_tokens` budget is expected; removing `max_regions` is a model-visible change
and needs a decision, not a drive-by.

## Hard gates

- **Do not point kcode at kgrep before the attribution lands.** agentgrep is MIT
  and kgrep absorbs its source, so the notice is required. The kgrep side owns
  writing it.
- **Do not build the search index inside a search.** A first-use build is ~514 ms
  on a normal repo but minutes on a monorepo, and anything over 5 s gets promoted
  to the background. The index will be an explicit API that kcode schedules on its
  own, using the background machinery that already exists. Until that API exists,
  searches simply run without an index and nothing is worse than today.
- **Do not add a second way to get file structure.** kgrep will own the
  precedence (index, then a per-file fallback, then its own scanner) in one place.
  If you find yourself branching on structure in kcode, stop and raise it.
- **Do not rename the model-facing tool as part of this work.** The name is
  deliberately undecided, and it reaches the schema, the alias table, config flags
  like `show_agentgrep_output`, display strings and tests. That is a separate,
  surveyed change.

## Development setup

kcode has no git remote for the kgrep repo, so develop against a path dependency
and swap it later:

```toml
# crates/jcode-app-core/Cargo.toml — replace the git dep at line 97
kgrep = { path = "../../../kgrep" }
```

Build and test:

```bash
cargo build -p jcode-app-core
cargo test -p jcode-app-core agentgrep     # the tool's own tests
```

The tool's tests in `agentgrep_tests.rs` are the contract for the behaviours
above. They are the first thing that should fail if the swap breaks something, and
they should keep passing **without being rewritten**, apart from the wiring.

## How to verify

- **Parity on the edges.** `~/kgrep/scripts/parity.py` covers 13 cases
  (non-UTF-8 names, symlinks, globs, glob+type, hidden, no-ignore, binary, empty,
  extension-less) and all of them agree today.
- **The benchmark.** `~/kgrep/scripts/bench.py` runs 25 verified navigation tasks
  and reports recall and tokens-to-answer. `~/kgrep/bench/baseline.json` is the
  current head-to-head, where kgrep is at a median of 28.8 tokens to the answer
  against agentgrep's 285.4.
- Do not run these casually: they walk the whole repository.

## Order

1. Read `agentgrep.rs` end to end. Understand the four modes and the five
   behaviours above before changing anything.
2. The kgrep shape is frozen. Build the mapping against it.
3. Refactor the seam in kcode.
4. Run the tool's tests and the parity script.
5. Report back: what you changed, what broke, what the tests said, and anything in
   the interface that was wrong in practice. The last one is the most useful.

## What to report back

- Whether the `Query` mapping was as clean as it looks, or whether a tool param
  has no honest home in it.
- Whether the four render calls were enough, or whether one of them wanted
  something `RenderOptions` does not carry.
- What the 5 s budget did in practice. Did anything new get promoted to the
  background?
- Anything the brief got wrong. It was written from reading the tree, and reading
  is not running.
