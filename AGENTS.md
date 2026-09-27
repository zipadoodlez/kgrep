# graphgrep

One Rust tool for how an agent finds its way around a codebase.

## What this is

graphgrep fuses two existing tools into a single slim form factor:

- **[agentgrep](https://github.com/1jehuang/agentgrep)** — Rust, index-free lexical
  search for agents: `grep`, `find`, `outline`, `trace`. One-shot, daemon-free,
  returns a compact structured packet instead of raw match noise.
- **[graphify](https://github.com/Graphify-Labs/graphify)** — Python, builds a
  persistent knowledge graph from source: extract (tree-sitter) → build →
  cluster → analyze → report, then answers structural questions (god nodes,
  communities, paths, neighbours).

Both answer the same agent question: *where is X, and what connects it to Y.*
agentgrep answers it lexically and instantly; graphify answers it structurally
and durably. graphgrep is the grain of each, in one crate.

The end goal is to replace the `agentgrep` dependency inside **kcode** with
graphgrep, refactoring kcode's integration site wherever graphgrep's shape is
the cleaner one. What is fixed is kcode's behaviour, not agentgrep's API. kcode
keeps agentgrep until graphgrep is complete and tested; the swap is the last
step, not the first.

### How the two halves arrive

Asymmetric effort, and it matters:

- **agentgrep is already Rust.** It is not ported, it is *absorbed*: vendor its
  `src/` as the lexical core and reshape it onto the core representation below.
  It compiles today and has a real consumer, so this half is low risk.
- **graphify is Python.** Its grain is *ported* to Rust. This is the only real
  Rust-writing work, and it is small if scope is fixed before the port starts.

## Philosophy

Two skills govern all work here. Both are always on.

**Ponytail** governs the diff: stdlib over custom, one line over fifty, delete
over add. The laziest thing that actually works.

**Zonytail** governs the codebase: one obvious home per concept, legible blast
radius, uniformity, locality of reasoning, a small sharp core. Centralize
representation, localize behaviour. Default local. Couple on change, not on
looks. The ratchet points one way: leave it cheaper than you found it.

They pull opposite ways on purpose. When they conflict, zonytail decides *where*
code lives and *what shape the codebase is*; ponytail decides how much of it
there is. Neither ever lowers the bar on understanding: read the region fully,
trace it end to end, then move.

**Capability is fixed.** We reach a slim form factor by cutting *husk*, never by
cutting what the tool must do. An elegant tool that can no longer answer the
questions agents actually ask is a smaller broken thing.

## The core idea

One query surface, two backends behind it. `docs/DESIGN.md` holds the longer
form of this: how the two halves synergize, and how the graph cache is meant to
build itself. This file stays the settled truth; that one is the aim.

- **Lexical backend** — agentgrep's index-free scan. Always available, instant,
  no setup. This is the floor.
- **Graph backend** — graphify's extracted graph, persisted on disk. Used when
  the graph exists and is fresh; it upgrades an answer from "these files match"
  to "this is the structure and here is the shortest path between the two".

A query picks the best backend it can afford, and degrades to lexical when the
graph is absent or stale. The graph is an *upgrade*, never a prerequisite. No
command may require the graph to exist in order to work.

### The small sharp core

Shared representations, one home each. Everything else stays boringly local.

- **Node** — a symbol or file with an identity and a source location.
- **Edge** — a typed relation (`calls`, `imports`, `uses`, …) with a confidence
  (`EXTRACTED` | `INFERRED` | `AMBIGUOUS`).
- **Graph** — nodes + edges + communities. One representation, not one per
  stage.
- **Query** — lexical terms or structural intent (subject/relation/support).
- **Packet** — the ranked, compact result an agent reads. Text and JSON.

The pipeline stages (`build`, `cluster`, `analyze`) are operations over the
graph, not owners of it. If a stage needs a second graph representation, that is
the god module being born: stop and fix the seam.

## Scope: grain and husk

Cut deliberately, region by region. Start from the core representation.

**Keep (grain):**

- Lexical search, ranking, symbol grouping, outline, the trace DSL.
- tree-sitter extraction for the languages agents actually meet, with a
  call-graph second pass.
- The graph pipeline: build, cluster, analyze (god nodes, communities, paths,
  neighbours, cycles, diff).
- Structured graph query over the persisted graph.
- Compact text + JSON packets for every command.

**Cut (husk) unless proven load-bearing:**

- The long tail of niche extractors (terraform, verilog, fortran, powershell,
  commonlisp, …). Add a language when a real user needs it, not to hit parity.
- LLM-based extraction and semantic cleanup. Lexical + AST covers the floor.
- Installers, hooks, editor wiring, `install.py`, `hooks.py`.
- MCP/HTTP servers, watch mode, HTML/callflow/canvas/SVG/wiki exporters.
- Video, PDF, Office, Google Workspace, URL ingest.
- `Prs`, `serve`, `cache` and other machinery around the hot path.

Every "keep" is a capability contract. Every "cut" is a reversible decision: a
cut feature can return as a small operation over the core if it proves itself.

## Consumers and the kcode seam

The consumer that matters is kcode. Today it depends on agentgrep as a library
and calls:

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
`args.rs`, `context.rs`).

**kcode is in scope to refactor.** We do not preserve agentgrep's API shape for
its own sake. If a cleaner graphgrep API makes kcode's integration site simpler
too, change both. The refactor is real but bounded: the agentgrep integration
site, not the rest of kcode.

**No dependency swap until graphgrep is done and tested.** kcode keeps its
agentgrep dependency throughout. Repointing it at graphgrep is the last step,
after graphgrep is complete and its tests pass.

### Clean core, thin CLI

Inside graphgrep there is one shape: a `Query` in, a `Packet` out. `grep`,
`find`, `outline`, and `trace` are thin constructors over that core, not four
parallel implementations. The CLI keeps those four verbs for humans and
scripts; kcode calls the core API directly. One way in, one way out, however
many front doors.

### Parity while kcode is untouched

Because the swap is deferred, parity with agentgrep is proven independently:
graphgrep's own tests, plus a differential check that runs agentgrep and
graphgrep over the same corpus and compares output. Absorbing code without a
real oracle is how silent regressions land.

## Layout

The crate does not exist yet. This is the intended shape; let structure emerge
from the core, do not pre-create empty modules.

```
src/
  lib.rs        // the public API kcode embeds
  model.rs      // the core: Node, Edge, Confidence, Graph, Query, Packet
  cli.rs        // clap surface: grep | find | outline | trace | graph *
  scan.rs       // file walking, ignore rules, candidate collection
  lexical.rs    // grep matching, find ranking, symbol grouping
  outline.rs    // known-file structural scan
  extract/      // tree-sitter -> nodes/edges, one file per language
  graph/        // build, cluster, analyze, query over the core graph
  packet.rs     // ranking + text/json rendering
```

## Commands

Standard Rust, no exotic tooling:

```bash
cargo build            # debug build
cargo build --release  # release build
cargo test             # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

These become real with the first code commit. Until then AGENTS.md is the only
source of truth for intent.

## Working rules for agents

- **Read before you claim.** A structural claim is not a fact until you have
  read the code behind it. Never dress a guess as a finding.
- **Finish what you start.** Half-migrated is the most expensive state there
  is. Cut a pass to what you will complete, ship it whole.
- **One home per concept.** Before adding a new module, path, or representation,
  find whether the canonical home already exists. A parallel path is how a
  codebase grows a second way to do everything.
- **No capability regressions.** Do not delete what the system needs to do to
  make a file smaller.
- **No artifacts, no state.** You read the code fresh; the structure is the
  memory. Keep docs (including this file) in sync, not aspirational.
- **Commit as you go.** Small, whole, working commits.

## Decisions

Settled:

1. **Language: Rust.** One crate, one binary. The Python side is a source to
   read, not a runtime to keep.
2. **API: clean core, refactor kcode.** One `Query`/`Packet` core. The four CLI
   verbs are thin constructors over it, and kcode's integration site is
   refactored onto the same core when graphgrep is ready.
3. **No dependency swap until done.** kcode keeps agentgrep until graphgrep is
   complete and tested. Parity is proven by tests plus a differential run
   against agentgrep.
4. **Extraction languages, initial set:** Rust, Python, TypeScript/JavaScript,
   Go. More on demand, never for parity's sake.

Open:

5. **Graph freshness.** How stale is too stale to prefer the graph backend over
   lexical. Decide when the graph backend lands.
6. **Attribution (deferred, not optional).** Neither upstream is ours.
   agentgrep is MIT (`1jehuang`); graphify is Apache-2.0 (`Safi Shamsi and the
   Graphify contributors`). Absorbing agentgrep's source and porting graphify's
   design both make graphgrep a derivative work, so a `NOTICE`/`LICENSE`
   attribution is owed. Deferred to the end of the build by choice, but it must
   land before graphgrep is released or pointed at by kcode.
