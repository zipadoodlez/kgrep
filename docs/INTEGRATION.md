# The kcode seam

How `kgrep` replaces `agentgrep` inside kcode, and what has to be true first.
Everything here was read out of the kcode tree at the paths cited, not assumed.

Status: **proposal.** The facts are verified; the decisions marked open are not.

## What the seam is today

One tool, four modes. Verified in `crates/jcode-app-core/src/tool/agentgrep.rs`:

- `AgentGrepTool` is registered once, at `tool/mod.rs:346`, under the name
  `agentgrep` (`agentgrep.rs:186-188`).
- The model mostly does not say `agentgrep`: `tool-core/src/lib.rs:385-396` maps
  `grep`, `file_grep` and `Grep` onto it. The native grep tool was deleted in
  favour of it, so **`grep` is already agentgrep** from the model's view.
- The four verbs are a `mode` parameter on one schema (`agentgrep.rs:194-247`),
  not four tools: `grep`, `find`, `outline`, `trace`.
- Execution is `execute` -> `spawn_blocking` -> `run_agentgrep_blocking` ->
  `execute_linked_agentgrep` (`agentgrep.rs:249-271`, `335`, `381`), which
  dispatches per mode to `run_grep` / `run_find` / `run_outline` / `run_smart`
  and then renders with `render_*_output`.
- Linkage is **in-process, as a library.** There is no subprocess, which is the
  same shape kgrep should keep.

### Five behaviours in the caller that are load-bearing

These are integration, not library, and a swap must preserve them:

1. **A 5 second foreground budget** (`agentgrep.rs:23`). Past it the search is
   *adopted into a background task* and the model gets a task id to poll with
   `bg` (`agentgrep.rs:274-314`).
2. **Bounds live in the caller.** `DEFAULT_GREP_MAX_REGIONS = 200`
   (`agentgrep.rs:88`), and find/outline default to 5 files and 6 regions.
   `max_regions` is model-settable, so the model can raise the cap.
3. **Single-file filtering.** `filter_{grep,find,smart}_result_to_exact_file`
   narrows a result when the model passed one file as `path`.
4. **`context.json` is written per call**, via `maybe_write_context_json`
   (`agentgrep.rs:344`). This is harness state, and it is the one thing an
   external tool cannot have.
5. **Slow calls are logged** above 2 s (`agentgrep.rs:356-361`).

## Decision 1: kgrep exposes the shaper, not four run/render pairs

Today's interface is `run_grep(&root, &args) -> GrepResult` plus
`render_grep_output(&result, &args, cap)`, and three more like it. kgrep's core is
already `Query` in, `Packet` out, with ranking and the budget inside and rendering
as a method on the packet.

So the new seam should be: the tool builds a `Query`, kgrep returns a `Packet`,
and the tool renders it. That deletes the four-way dispatch and the four render
functions from kcode, and it puts the budget in one home instead of in every
caller, which is the bug the CLI already hit once.

- **Open:** whether kcode keeps `run_*`-shaped wrappers for a smaller diff, or
  takes the `Query`/`Packet` shape directly. The second is better and is bigger.

## Decision 2: the budget moves into the core, and is expressed in tokens

The measured win came from ranking plus a **token** budget. The seam currently
passes a **match count** (`max_regions = 200`). Those are not the same control: a
match line can be 240 characters, so "200 matches" ranges over about a factor of
five in output size, which is exactly the finding in `bench/README.md`.

- kgrep should own the default token budget.
- If the model is to raise it, the schema should expose a token budget, not a
  region count.
- `max_files` / `paths_only` stay as the coverage control, which is a different
  axis: coverage first, detail second.

## Decision 3: the index is an explicit API, and the harness schedules it

This is the significant new work, and the 5 second budget is why it cannot be
implicit.

A blind `ctags -R` over kcode is **30,814 ms and 1,320 MB**, because `target/` is
77 GB and ignore-listed. Built from the file list the search already walks, the
same repository is **514 ms and 5.4 MB**, with a **2.6 ms** name lookup against our
whole-repo scan at 29.8 ms. One file can be retagged into an existing index in
**20.9 ms**.

So the index is cheap, but a first-use build inside a search would blow the 5 s
budget on any large repository and degrade into "running in background" for a
one-line grep. Therefore:

- **kgrep exposes index operations explicitly**: build or refresh for a root, and
  report status. A search *uses* a fresh index and never blocks to create one.
- **kcode owns the scheduling**, because it already has a background task
  mechanism and knows when a workspace opens. Build eagerly at workspace open,
  or lazily in the background, but never inside a search.
- **Freshness is a rule, not a hope**: rebuild when a tracked file is newer than
  the index. A stale tag is a silently wrong answer.
- **It lives outside the repository**, keyed by workspace root, since it is
  derived state and must not appear in `git status`.
- **It stays mmapped.** 5.4 MB on disk is nothing, but a parsed in-memory map is
  several times that and would breach the 32 MB ceiling. The ceiling is what
  forces mmap-and-search-in-place rather than load-and-parse.
- **The generation must use the walker's own file list**, never `ctags -R`. That
  single difference is 60x in time and 245x in size, and it is the difference
  between a good tool and a disk-filling one.

Payoff: this is what gives structure to `grep`, `find` **and** `trace` on every
language, which is the whole of "support all types of code". The per-file
subprocess that shipped can only ever serve `outline`.

- **Open:** how big a repository gets an index at all. A monorepo build is
  minutes, and the honest answer may be to skip the index above a size and stay
  on the scanner.

## Decision 4: take the two harness-only signals

Neither is available to an external tool, and both are already in hand.

- **`intent`.** The schema requires the model to state *why* it is searching
  (`tool-core/src/lib.rs:25-30`). Ranking currently has one profile for
  everything, and `bench/README.md` records the exact tension: a definition query
  wants the sparse file, a topic query wants the dense one, and no single weight
  can serve both. Intent is the discriminator, it is free, and it is the
  cheapest real improvement on the list.
- **`context.json`.** What the agent has already read and what changed. kgrep
  accepts `--context-json` today and ignores it, which is recorded in the known
  gaps.

## Decision 5: the name, and the blast radius

The model sees `agentgrep`, and `grep` aliases to it. The name should be settled
here, because it is in the schema, the alias table, the config flags
(`show_agentgrep_output`), display strings, and tests. Renaming is a survey, not
a find-and-replace, and it is part of the swap rather than a follow-up.

## What the swap must not regress

- The four modes, their schemas, and the `grep` alias.
- The 5 s budget and the background adoption path.
- Single-file filtering.
- The slow-call warning.
- `context.json` being written, even if unread at first.

## Order

1. Settle the shape (Decision 1) and the budget (Decision 2). Small, and it
   changes what everything else looks like.
2. Index as an explicit API with the harness scheduling it (Decision 3).
3. `intent` (Decision 4), because it is cheap and it addresses a measured
   weakness.
4. Attribution, then the name, then the swap.

## Open questions

- Does the index earn its place on a monorepo, or does the scanner stay for
  everything above some size?
- Where exactly does the cache live, and how is it invalidated when the workspace
  root moves?
- Does `intent` change ranking by profile, or by weights within one profile? The
  first is testable on the existing 25 tasks and the second is a bigger change.
- Does `kgrep` stay a library dependency, as agentgrep is, or become a binary?
  Library keeps the current shape; a binary would isolate the ctags subprocess but
  add a process boundary per call.
