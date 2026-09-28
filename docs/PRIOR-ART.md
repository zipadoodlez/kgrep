# Prior art: what other code does, and what we absorb

Two questions, one file, because they produce the same thing: the reasons kgrep is
shaped the way it is.

- **The field.** How other harnesses find code, with sources, so the settled
  choices below rest on evidence rather than on impressions.
- **Our upstream.** The tool we absorb, what was wrong with it for our purposes,
  and what we changed.

Confidence is marked throughout. **verified** means a primary source, a vendor doc,
an engineering blog or the repository itself. **partial** means a secondary source
or a reasonable inference. **unverified** means we could not confirm it. Claims
about our own code are not marked, because they are checkable in the tree.

## The layers

kgrep's design leans on a layered retrieval model. The industry converged on the
same layers independently, which is a reason to think the shape is right rather
than merely ours.

| Layer | Claude Code | Cursor | Copilot / VS Code | Aider | Sourcegraph | opencode | Serena |
|---|---|---|---|---|---|---|---|
| Lexical scan | yes, verified | yes, verified | yes, verified | yes, partial | yes, partial | yes, partial | yes, verified |
| N-gram regex index | no | yes, verified | yes (Blackbird), verified | no | yes (Zoekt), verified | no | no |
| Vector embeddings | no | yes, verified | yes, verified | no | yes, verified | no | no |
| tree-sitter | no | yes (chunking), verified | "language intelligence", partial | yes, verified | "search-based nav", verified | no | no |
| LSP | no | no | yes ("usages"), verified | no | no | diagnostics only, verified | yes, verified |
| Compiler-accurate index | no | no | no | no | yes (invented SCIP), verified | no | no |
| Symbol graph and PageRank | no | no | no | yes, verified | no | no | no |
| Content hashing for freshness | no | yes (Merkle), verified | server-side | no | commit-level | no | n/a |

### Per harness

**Claude Code.** Lexical-first and deliberately bare: read, write, edit, bash, glob,
grep. Ripgrep is bundled, confirmed by a settings reference that exposes
`sandbox.ripgrep`, "use your own ripgrep binary inside the sandbox". No published
evidence of tree-sitter, an LSP client, embeddings, or a code graph in the agent
path. *(verified; `docs.claude.com` settings reference)*

**Cursor.** The most layered stack, and the only one that indexed *lexical* search
itself. A semantic index of tree-sitter-derived chunks, embedded, requiring roughly
80% of the index before semantic search is available. A Merkle tree of file and
directory hashes for freshness, keyed off a commit with user and agent edits layered
on top. And a local sparse n-gram inverted index that narrows the file set before a
real regex scan, explicitly motivated by `rg` taking over 15 seconds on large
monorepos, living in two mmapped files. *(verified; `cursor.com/blog`)*

Cursor matters to us for one reason above the others: **they state the regex index
must be very fresh**, unlike the semantic index, because if the agent cannot find
text it just wrote, it goes on a wild goose chase. That is our freshness rule,
independently arrived at.

**GitHub Copilot / VS Code.** The clearest published taxonomy, and it matches ours
almost exactly: semantic search requiring a workspace index; text search; grep,
which "works without an index"; file search; "usages" backed by LSP; and
list-directory plus read-file. GitHub's Blackbird engine (Rust, sparse n-grams)
powers repository text search. The docs also admit that for small projects the
whole workspace is simply read into context. *(verified; VS Code docs, `github.blog`)*

**Aider.** The strongest validation of extracting with tree-sitter queries: a
`tags.scm` per language declaring what counts as a definition versus a reference,
a symbol graph, and PageRank to fit the most important symbols into a token budget
of about 1k, sent with every request. *(verified; `aider.chat/docs/repomap.html`)*

**Sourcegraph.** The only major player with compiler-accurate persisted code
intelligence, and their framing is our two-tier split: "Search-based code navigation
is powered by tools like ctags and tree-sitter. Precise code navigation ... requires
custom configuration ... the results are compiler-accurate." They invented SCIP, a
protobuf index of definitions and references roughly 8x smaller than LSIF. *(verified;
`sourcegraph.com/blog/announcing-scip`)*

**Serena.** Not a harness, but the definitive proof that LSP in the agent loop works,
wrapping language servers to give agents symbol tools across 40+ languages. Its own
evaluation quotes agents saying the symbol tools were the single most impactful
addition, collapsing 8 to 12 unreliable text steps into one call. Notably it starts
its tools disabled when the surrounding harness already covers file and search work.
*(verified; `github.com/oraios/serena`)*

**opencode.** Integrates LSP for **diagnostics only**, not symbol navigation: about
25 language servers, disabled by default. Their measured code sizes are the useful
part, because they show where the cost sits:

| File | Size | Role |
|---|---|---|
| `lsp/server.ts` | 55 KB | the per-language table: command, extensions, init options |
| `lsp/client.ts` | 23 KB | protocol client |
| `lsp/lsp.ts` | 17 KB | lifecycle, dispatch, diagnostics |
| `lsp/language.ts` | 2.5 KB | extension detection |
| `lsp/diagnostic.ts`, `lsp/launch.ts` | 1.7 KB | glue |

More than half the code is the language table, not the protocol, and this is the
cheapest possible use: push notifications, no definition or references requests.
Their own warning is the strongest evidence on the cost:

> "LSP can help the agent find and fix issues ... but it is not always a net
> positive. Language servers can get out of sync, use significant memory, vary by
> version or project, and slow down agent workflows. In many projects it is better
> to have the agent run lint, typecheck, or other diagnostic CLI tools directly."

*(verified; `opencode.ai/docs/lsp`)*

**Not verified.** Codex CLI, almost certainly ripgrep plus shell, no primary source
in hand. Windsurf/Codeium, proprietary ("Fast Context"). Cline/Roo, no indexing or
graph described. Zed is tree-sitter and LSP based as an editor and runs agents
through ACP, but we could not confirm what its agent uses for retrieval.

## The cost of LSP, in three parts

This is the evidence behind a settled decision: **no language servers in-process,
ever.** The decision is recorded in `AGENTS.md`; the reasoning is here.

1. **Protocol client: light.** JSON-RPC over stdio, `Content-Length` framing,
   request and response correlation, the initialize handshake, document sync.
   Roughly 500 to 800 lines of Rust, with `lsp-types` supplying the messages.
2. **Language table: moderate and boring.** Which binary, which extensions, which
   init options, per language. opencode spent 55 KB on 25 languages, so four
   languages is a couple of hundred lines. It is also the part that rots.
3. **Runtime: heavy, and this is the real cost.** A language server holds a
   whole-project index. rust-analyzer on a large workspace is hundreds of MB to GBs
   of RAM and tens of seconds to warm up. It gets out of sync, its version matters,
   and it must be installed. Diagnostics are cheap push notifications; definition
   and references need the full index, which is the expensive part.

Two classic bug sources: positions are line plus UTF-16 code units rather than
bytes, and headless use means the server indexes the whole workspace rather than a
few open buffers.

**Cheaper routes to the same win**, which is what we took:

- **ctags.** One parseable file, name to file and line. Roughly 90% of "go to
  definition" for roughly 1% of the weight. No process, no warm-up, no staleness.
- **An index over our own extraction.** In-process ctags, free.
- **SCIP.** Read a file, no process, exact. Needs an indexer to have run.
- **MCP.** kcode is already an MCP client, and Serena is an MCP server exposing LSP
  symbol tools, so real LSP can arrive as user configuration rather than as code we
  own. That is why nothing is lost by declining it: a user who wants it can have it
  without us.

## Our upstream: agentgrep

agentgrep is the code we absorb, so its weaknesses became ours unless deliberately
fixed. Every one below is a fair reading of a tool that works well; most are
deliberate tradeoffs that were right for its context and wrong for ours.

All but one are now addressed, and the one left open is resolution. That is why this
reads as a record rather than a work list.

| # | What it did | What we did |
|---|---|---|
| 1 | Capped the first N matches **in path order** (`results.sort_by(\|a, b\| a.path.cmp(&b.path))`), so the cap kept the alphabetically earliest files, not the best | Rank before spending the budget. Four cheap signals, and 3.8x better on median tokens-to-answer |
| 2 | Counted **matches** rather than tokens, while a match line can run to 240 characters, so "200 matches" spanned roughly 5x in output | Budget in tokens. `max_detail_tokens`, defaulted and justified against measured medians |
| 3 | Applied the cap at **render time**, after collecting every match, so it guarded context and did nothing for peak memory | Bound during the scan, per worker, and merge bounded tops |
| 4 | Dropped whole files once capped, which throws away the cheapest information to keep | Coverage first: name every matching file, expand the top few, count the tail, and report the true total |
| 5 | Left the safety default with the **caller**, so every consumer had to remember it, and a new one reintroduced a bug already found once | The bounded packet is the library default, with an explicit opt-out. **Still open upstream:** the contribution offered to agentgrep |
| 6 | Ran `rg` as a fast path and then kept a large body of code forcing the two searchers to agree, about 560 lines in v0.1.6 | One searcher, in-process, streamed and parallel |
| 7 | Extracted structure by line-based keyword spotting, so end lines were approximations and items were flat, and everything outside four languages fell to a generic scanner | ctags for coverage, in roughly 100 languages, with the scanner as the fallback. Nesting depth is still not available to a caller |
| 8 | Had no symbol resolution: it could say where a name appears, never which definition it means | **Open.** Resolution is a later source, never in-process via a language server. See the LSP cost above |
| 9 | Measured 1.39x slower than `rg` overall, attributed to the structure pass, file re-reads and the `rg` fork, not to the lexical scan | Context rather than a defect. Their own designed next step was to fuse the passes, which is the direction we took |

Two carry forward, and both are in the roadmap rather than here: the upstream
contribution for weakness 5, and resolution for weakness 8.

### Worth keeping, unchanged

Credit where it is due, because these are the reasons agentgrep is worth building on
rather than replacing:

- **The header always reports the true total**, so a caller raising the cap knows
  what it is asking for. Progressive disclosure done properly.
- **Grouping matches under their enclosing symbol**, the single biggest usability
  win over raw grep output.
- **`--paths-only` early exit**, the cheapest possible answer to "which files".
- **Deterministic ordering** by native path bytes, so output never wobbles with
  filesystem read order.
- **Binary refusal and lossy UTF-8 decoding**, so a stray byte does not hide a whole
  file and non-UTF-8 names stay addressable.
- **Ignore semantics that already match `rg`**: the `ignore` crate, `.rgignore`,
  symlink handling, and glob matching.

## What the field changed for us

Four conclusions drawn from the survey, kept because they explain decisions rather
than because they are still open:

1. **The layered model is consensus, not invention.** VS Code publishes nearly our
   exact taxonomy, which lowers the risk of the shape.
2. **An index-free lexical floor is the right default.** Claude Code ships ripgrep
   and tools and is the most used harness. Our rule that structure is an upgrade and
   never a dependency matches what the market tolerates.
3. **Indexing lexical search itself is a real fourth tier we had not planned.**
   Cursor's sparse n-gram index attacks exactly our scaling weakness, since a scan
   must read every file. Optional, later, and the mmapped-artifact design we would
   copy.
4. **LSP is the tier we decline, not the tier we missed.** It is the biggest
   unexploited lever in the field, and the runtime cost is why we decline it rather
   than the capability being unavailable: a user who wants it can point kcode at an
   MCP server.
