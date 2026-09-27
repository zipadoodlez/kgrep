# Design notes

Direction, not decisions. `AGENTS.md` holds what is settled; this file holds what
we are aiming at and why. If the two ever disagree, AGENTS.md wins and this file
is out of date.

> **Revised 2026-09-27.** The framing moved from "fuse two tools" to **one shaper
> with pluggable sources**, and the binding constraint became **flat memory with
> respect to repository size**. The graph is now source three of several, not half
> the product. The sections below on how the halves synergize remain useful as the
> long-term direction, but the build order at the bottom is the current plan.

## The objective, and the memory guardrail

See `AGENTS.md`. In one line: **optimise tokens and latency, keep memory inside a
declared ceiling.** Memory is a guardrail, not an objective, and it matters in
classes: unbounded or orders-of-magnitude growth is out, a capped or mmapped
budget that buys tokens or latency is in.

The consequence for everything below is that a source is only allowed in if it is
streaming and bounded, and any persisted artifact is mmapped and queried by
offset rather than deserialized. Cursor's regex index is the one shipped example
of doing this deliberately; we adopt it for memory rather than for latency.

## The cache: the map that builds itself

### The shape

The graph is not a thing you set up. It is a thing that accumulates while you
work.

- **First search on a repo answers immediately, from the plain-text sweep, and
  starts building the map in the background.** The search never waits for the
  map. By the time the user asks their third question, the map is warm.
- **The map is never required.** Every command must be correct and useful with
  no map at all. The map is always an upgrade, never a dependency.
- **Initialize once per repo, then keep it warm incrementally.** A full build
  happens once. Everything after that is a small patch.

### Keeping it warm

Two triggers, because neither is sufficient alone.

**Eager: a git hook.** `post-commit`, `post-checkout`, `post-merge`, and
`post-rewrite` run a silent incremental update. This is the cheap, timely path.
Keep the hook tiny, and make it opt-in. Installers and hook managers were husk
in graphify; the hook itself is worth keeping, the machinery around it is not.

**Lazy: a staleness check on every search.** Hooks miss work that is never
committed, which is most work. So each search cheaply asks "has anything changed
since I last looked", using git's view of the repo first (`HEAD` plus the dirty
set, which git already tracks) and file mtimes as a fallback for non-git trees.
If something changed, patch it before answering, or answer and patch
immediately after.

### What "incremental" actually means

Not a rebuild of changed files. Three steps:

1. **Re-read the files that changed**, and replace only the facts that came
   from them. Facts record their source file and a content hash, so eviction is
   exact.
2. **Repair the facts that pointed at things that changed or disappeared.**
   Deleting a function breaks every note that referenced it, including notes in
   files that did not change. This requires a reverse index: who points at this
   node. Without it, incremental updates rot silently.
3. **Let the cheap text sweep verify.** A plain scan over the changed files
   detects claims the structural source got wrong or relations it never saw,
   including the ones wired up by a name in quotes. The always-fresh half keeps
   the sometimes-stale half honest.

### Budgets and failure

- If the first full build would exceed a time or size budget, build partially
  and stay lexical for the rest, rather than making the user wait.
- The build is resumable. Interrupting it keeps what is done.
- Storage is a gitignored directory of derived data. It never travels with the
  repo and is always disposable.

## How the two halves synergize

From loosest to tightest.

### 0. Side by side

Two engines, one binary, sharing nothing. The least interesting outcome.

### 1. One report shape

Both halves produce the same packet. The reader cannot tell which produced it.
Cheap and clearly worth doing.

### 2. One shared homework assignment

Describing a file depends only on the file, never on the query. So it is
computed once and remembered. **This remembered homework is exactly what the
graph is.** The lexical mode computes it on the fly and discards it; the graph
mode computed it earlier and kept it.

### 3. They grade each other

**The map makes the search smarter.** Ranking stops being textual and becomes
structural: how connected a thing is, which cluster you are in, which of the
twelve things named `render` you actually mean. This is the best available
answer to the main complaint about plain search, which is noise.

**The search fills the map's holes.** A map built from declarations is blind to
relations wired by a name in quotes: command tables, plugin registries, config
keys, string dispatch. A plain scan finds them and draws them in, marked as
lower confidence. Without this the map has holes exactly where the code is most
indirect.

### 4. One shared unit

Files and symbols cannot merge, because they are different things. The shared
unit is **a named item with a location and a text surface**. Then a text match
is a statement about the surface and a map edge is a statement about
relatedness, and both jobs operate on the same objects. One node type, two kinds
of link: looks-alike, and relates-to.

### 5. One walk

If text is just another kind of link, the query becomes a node and every command
becomes a short route of steps, where a step is either "match this text" or
"follow this relation".

**A map with no roads drawn is plain search.** The lexical floor is not a
separate machine; it is the same walk over a map that has no relation links yet.
The graph is that walk with the links filled in.

### What this unlocks

- **Disambiguation.** "Where is this rendered" resolves which one you mean.
- **Aliases.** Wrappers, re-exports and alternate names collapse into one thing.
- **Neighbourhoods in the packet.** The hit arrives with what it uses and what
  uses it, so no follow-up search is needed.
- **Structural noise removal.** Generated code, vendored code and unrelated
  tests drop out for a real reason, not a guess from the folder name.
- **A pointer to the next question.** Retrieval becomes navigation.
- **Cheap freshness.** The plain scan detects which map facts broke, so keeping
  the map current never requires a rebuild.

### The honest costs

- **Staleness.** Every structural signal is gated on freshness. Pure lexical
  must always be correct alone.
- **Recall.** The map has holes, so it only narrows the search when confident,
  and the plain sweep is always available to widen it.
- **Setup.** A full build is expensive, so it is optional, budgeted and
  incremental.
- **Over-structuring.** Structure must never bury an exact literal match. A
  purely lexical mode must remain available.

## Build order

> **Superseded by the roadmap in `AGENTS.md`**, which is authoritative and has
> exit criteria. Kept here because the reasoning below still explains why the
> order is what it is.

Do not build the unified walk or the graph first. Each earlier step must be
independently useful and must not raise peak memory.

1. **The shaper.** `Query` in, `Packet` out. The four CLI verbs become
   constructors over it. Absorb agentgrep's lexical code here and reshape it.
2. **Remove the memory spikes in the code being absorbed.** One fused pass per
   file instead of several, no whole-file lowercased copy, no whole-file line
   vector, and a bounded top-K instead of collecting every match before sorting.
   This is a free win, and it is the first proof of the memory thesis.
3. **`ctags` for coverage**, run per file and only for the languages the line
   scanner cannot parse. Measured at 14.5 ms for one file against 3.2 ms for the
   scanner, and 2.9 s for a whole-repo index, which is why there is no index: the
   question is per file, so the index, the artifact and the freshness problem all
   disappear. It is the difference between an outline and nothing on a Go, Ruby or
   Java repository.
4. **An mmapped sparse n-gram index** for lexical scaling on large repos.
   Cursor's published design, adopted for memory rather than latency. This is the
   "best in the world" claim.
5. **The graph, last**, as an mmapped artifact queried on demand, answering only
   the architectural questions: what is central, what clusters, how things link.

Cut on purpose: an in-process parse for real ranges. ctags gives the declarations
and none of the extents, and the only things extents bought were trace region
bodies and symbol-scoped edits, neither of which has a measured need.

Explicitly not built: language servers in-process, embedding models, vector
stores, and any resident structure proportional to repository size.

Steps 1 and 2 can be one milestone. Steps 3 to 6 are each optional and each
shippable on their own, and none of them may raise peak memory.
