# graphgrep

One shaper for how an agent finds its way around a codebase, built so that
memory stays flat no matter how large the repository is.

## What this is

A single tool for the questions agents ask constantly: where is X defined, who
calls X, what is inside this file, which files are about this topic. It answers
with a **packet**: ranked, structured, budgeted, and carrying its reasons, so the
agent can act on one call instead of ten.

Behind that one interface sit several **sources** of evidence. Each is
independently useful, each is optional, and each must be streaming and bounded
to be allowed in.

| Source | Answers | Cost |
|---|---|---|
| Lexical scan | exact text and regex | none, always available |
| tree-sitter outline | structure, nesting, symbols | bounded per file |
| Tag index | where is X defined | one mmapped file |
| mmapped n-gram index | fast regex on huge repos | mmapped, evictable |
| Graph | what is central, what clusters, how things link | optional artifact |
| LSP (via MCP only) | exact references and implementations | never ours to run |

The shaper is ours and is the product. The sources are interchangeable.

## The hard constraint: memory

This is the differentiator, not a detail. Other harnesses and other indexes hold
the codebase in RAM. We do not. Every design decision is judged by capability
per byte.

**The rules:**

1. **Nothing resident.** Work is per file and the memory is released per file.
2. **Peak memory may scale with the largest file, never with the repository.**
   A hundred-file repo and a half-million-file repo must peak at the same place.
3. **Any persistent artifact is mmapped, never deserialized.** Touch only the
   lookup table and read the payload off disk by offset, so the OS pages it in
   and can evict it under pressure. Never build a `Vec` of the whole index or the
   whole graph.
4. **Bounded results.** Ranked output keeps a bounded top-K. No tool collects
   every match before sorting.
5. **No language servers and no embedding models in-process.** A language server
   is hundreds of megabytes to gigabytes and would dwarf the entire harness.
   Embeddings carry a model. Both are excluded. LSP is consumed over MCP, if and
   only if the user already runs one.
6. **Every tool call has a memory budget.** Exceeding it truncates or spills. It
   never grows unbounded.

The last one is what makes the claim testable: a search over a huge repo must
not allocate in proportion to the repo.

## The shaper

One interface, whatever is behind it.

- **Query** in. Lexical terms, a path query, a file to outline, or a structural
  intent (subject, relation, support).
- **Packet** out. Ranked hits, trimmed to budget, text and JSON, each hit
  carrying the reasons it ranked where it did.

`grep`, `find`, `outline`, and `trace` are thin constructors over `Query`, not
four parallel implementations. One way in, one way out, however many front doors.

## Origin: what comes from where

- **agentgrep is absorbed (code).** It is already Rust, so its source is taken,
  reshaped onto the shaper, and its memory spikes removed. Its MIT notice follows
  it.
- **graphify is ideas only (no code).** We take the concepts: edge confidence
  (`EXTRACTED`/`INFERRED`/`AMBIGUOUS`), god nodes, communities. We write our own
  extraction, driven by **tree-sitter queries**, one small file per language.
  This is the approach aider already ships, and it replaces graphify's thousands
  of lines of hand-walking and cross-file resolution.

Neither upstream is ours. Attribution is owed and is deferred, not optional. See
Decisions.

## Scope: grain and husk

**Keep (grain):**

- Lexical search, ranking, symbol grouping, outline, the trace DSL.
- The shaper: `Query`, `Hit`, `Packet`, budgets, reasons, text and JSON.
- tree-sitter extraction for the languages agents actually meet, via queries.
- A memory-mapped index for lexical scaling, later.
- A memory-mapped graph artifact, last and optional.

**Cut (husk) unless proven load-bearing:**

- Language servers in-process, embedding models, vector stores.
- Installers, hooks managers, editor wiring, MCP/HTTP servers, watch mode,
  HTML/callflow/canvas/SVG/wiki exporters.
- Video, PDF, Office, Google Workspace, URL ingest.
- The long tail of niche extractors. Add a language when a real user needs it.
- Any resident structure proportional to repository size.

Every "keep" is a capability contract. Every "cut" is reversible: a cut feature
can return as a bounded source over the shaper if it proves itself.

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
too, change both, bounded to that integration site.

**No dependency swap until graphgrep is done and tested.** kcode keeps agentgrep
throughout. Repointing it is the last step.

### Clean core, thin CLI

Inside graphgrep there is one shape: a `Query` in, a `Packet` out. The CLI keeps
`grep`, `find`, `outline`, `trace` for humans and scripts; kcode calls the core
API directly.

### Parity while kcode is untouched

Because the swap is deferred, parity with agentgrep is proven independently:
graphgrep's own tests, plus a differential check that runs agentgrep and
graphgrep over the same corpus and compares output. Absorbing code without a real
oracle is how silent regressions land.

## Layout

```
src/
  lib.rs        // the public API kcode embeds
  model.rs      // the core: Node, Edge, Confidence, Query, Hit, Packet
  cli.rs        // clap surface: grep | find | outline | trace
  scan.rs       // file walking, ignore rules, candidate collection
  lexical.rs    // grep matching, find ranking, symbol grouping
  outline.rs    // file structure, tree-sitter query driven
  packet.rs     // ranking, budgets, text and JSON rendering
```

Let structure emerge from the core. Do not pre-create empty modules.

## Commands

```bash
cargo build            # debug build
cargo build --release  # release build
cargo test             # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Working rules for agents

- **Memory is the product.** Any change that makes peak memory scale with repo
  size is a regression, however fast or neat it is.
- **Read before you claim.** A structural claim is not a fact until you have read
  the code behind it. Never dress a guess as a finding.
- **Finish what you start.** Half-migrated is the most expensive state there is.
- **One home per concept.** Before adding a module, path, or representation, find
  whether the canonical home already exists.
- **No capability regressions.** Do not delete what the system needs to do to
  make a file smaller.
- **No artifacts, no state.** You read the code fresh; the structure is the
  memory. Keep docs in sync, not aspirational.
- **Commit as you go.** Small, whole, working commits.

## Decisions

Settled:

1. **Language: Rust.** One crate, one binary.
2. **Shape: one shaper, pluggable sources.** Not two tools glued together.
3. **Memory: flat with respect to repository size.** The differentiator.
4. **No language servers or embedding models in-process.** LSP is consumed via
   MCP only, if the user already has one.
5. **graphify contributes ideas, not code.** Extraction is our own, via
   tree-sitter queries.
6. **agentgrep contributes code**, absorbed and reshaped.
7. **No dependency swap until done.** kcode keeps agentgrep until graphgrep is
   complete and tested, proven by a differential run.
8. **Extraction languages, initial set:** Rust, Python, TypeScript/JavaScript,
   Go. More on demand, never for parity's sake.

Open:

9. **Graph freshness.** How stale is too stale to prefer the graph over lexical.
   Decide when the graph source lands.
10. **Attribution (deferred, not optional).** agentgrep is MIT (`1jehuang`);
    graphify is Apache-2.0 (`Safi Shamsi and the Graphify contributors`).
    Absorbing agentgrep's source makes graphgrep a derivative work of it, so its
    notice is owed. Must land before graphgrep is released or pointed at by
    kcode.
