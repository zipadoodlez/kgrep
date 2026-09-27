# State of the art: how coding harnesses retrieve code

Researched 2026-09. What each agent harness actually uses to find code, with
sources. Confidence is marked: **verified** means a primary source (vendor doc,
engineering blog, or repo) says it; **partial** means a secondary source or a
reasonable inference; **unverified** means we could not confirm it.

This matters because graphgrep's design leans on a layered retrieval model, and
the industry has independently converged on the same layers.

## The layers, and who uses each

| Layer | Claude Code | Cursor | GitHub Copilot / VS Code | Aider | Sourcegraph Cody / Amp | Serena (MCP) |
|---|---|---|---|---|---|---|
| Lexical scan (ripgrep) | yes, verified | yes, verified | yes, verified | yes, partial | yes, partial | yes (regex tool) verified |
| Trigram / n-gram regex index | no, not found | yes, verified | yes (Blackbird), verified | no | yes (Zoekt), verified | no |
| Vector embeddings ("semantic") | no, not found | yes, verified | yes, verified | no | yes, verified | no |
| tree-sitter | no, not found | yes (AST chunking), verified | "language intelligence", partial | yes, verified | "search-based nav", verified | no |
| ctags / tag index | no | no | no | no | named as the classic baseline | no |
| LSP (language server) | no, not found | no, not found | yes ("usages"), verified | no | no | yes, verified |
| SCIP / static code-intel index | no | no | no | no | yes (they invented SCIP), verified | no |
| Symbol graph + PageRank | no | no | no | yes, verified | no | no (uses LSP instead) |
| Content hashing for freshness | no | yes (Merkle tree), verified | index managed server-side | no | commit-level consistency | n/a |

## Per-harness notes

### Claude Code (Anthropic)

Lexical-first and deliberately bare. The tool set is read, write, edit, bash,
glob, and grep. Ripgrep is the search engine, shipped with the CLI: the settings
reference exposes `sandbox.ripgrep`, "use your own ripgrep binary inside the
sandbox", which confirms a bundled ripgrep. No published evidence of
tree-sitter, an LSP client, embeddings, or a code graph in the agent path.

Source: `docs.claude.com/en/docs/claude-code/settings-reference`.

### Cursor

The most layered stack of any harness, and the only one that indexed *lexical*
search itself.

- **Semantic index**: tree-sitter-derived syntactic chunks, embedded, stored in
  Turbopuffer, cached by chunk content. Requires roughly 80% of the index before
  semantic search is available. **(verified, Cursor engineering blog)**
- **Freshness**: a Merkle tree of file and directory hashes detects exactly what
  changed so it does not reprocess everything. Client-controlled, keyed off a
  git commit with user and agent edits layered on top. **(verified)**
- **Regex index**: a *local* sparse n-gram inverted index that narrows the file
  set before a real regex scan. Explicitly motivated by `rg` taking over 15
  seconds on large monorepos. Index lives in two mmapped files, built from a
  commit with a layer on top for edits. **(verified)**

Source: `cursor.com/blog/secure-codebase-indexing`,
`cursor.com/blog/fast-regex-search`.

### GitHub Copilot / VS Code

The clearest published taxonomy, and it matches ours almost exactly. VS Code's
docs enumerate its search tools:

| Tool | What it is |
|---|---|
| Semantic search (`#codebase`) | embeddings, requires a workspace index |
| Text search | substring over file content |
| Grep | exact text or regex, "works without an index" |
| File search | glob over names |
| Usages | "Find All References, Find Implementation, and Go to Definition" |
| List directory / Read file | the classic pair |

"Usages" is LSP-backed language intelligence. The index is built and maintained
by Copilot, partly local and partly remote, and for GitHub repositories it is
built server-side so it is often instantly available. GitHub's own Blackbird
search engine (Rust, sparse n-grams, built for regex) powers repository text
search. Note the doc's own admission that for small projects the whole workspace
is just read into context.

Source: `code.visualstudio.com/docs/agents/reference/workspace-context`,
`/tools-reference`, `github.blog` Blackbird post.

### Aider

The closest thing in the wild to the design in `docs/DESIGN.md`, and the
strongest validation of the tree-sitter-query approach.

Its repo map extracts definitions and references with **tree-sitter**, using a
`tags.scm` query file per language to declare what counts as a definition versus
a reference. It builds a symbol dependency graph and runs **PageRank** to select
the most important symbols that fit a token budget (default 1k tokens), which is
sent with every request.

Source: `aider.chat/docs/repomap.html`, `aider.chat/2023/10/22/repomap.html`.

### Sourcegraph Cody / Amp

The only major player with compiler-accurate, persisted code intelligence. Their
own framing is the two-tier split this project uses:

> "Search-based code navigation is powered by tools like ctags and tree-sitter.
> Precise code navigation ... requires custom configuration ... the results are
> compiler-accurate."

They invented **SCIP** to replace LSIF, and it is a protobuf index of symbol
definitions and references, roughly 8x smaller than LSIF. Zoekt provides the
trigram regex index.

Source: `sourcegraph.com/blog/announcing-scip`.

### Serena (MCP toolkit, not a harness)

Not a harness, but the definitive proof that LSP-in-the-agent-loop works. It
wraps language servers to give agents symbol-level tools: find symbol, file
outline, find referencing symbols, find declaration, find implementations, and
symbolic edits, across 40+ languages. Its own evaluation quotes agents in Claude
Code and Codex CLI saying the symbol tools are the single most impactful
addition, collapsing 8 to 12 unreliable text steps into one call. It also
starts its tools disabled when the surrounding harness already covers file and
search work.

Source: `github.com/oraios/serena`.

### Not verified this session

- **Codex CLI**: almost certainly ripgrep plus shell, but we have no primary
  source in hand.
- **Windsurf / Codeium**: proprietary retrieval ("Fast Context"); no primary
  source found.
- **Cline / Roo**: the repo describes plan/act, file edits, and terminal work,
  but no indexing or graph. Historically it offered optional embedding-based
  codebase search.
- **OpenHands, Continue, Zed**: Zed is tree-sitter and LSP based as an editor
  and runs agents through ACP, but we did not confirm what its agent uses for
  retrieval. Continue and OpenHands not confirmed.

## What this means for graphgrep

1. **The layered model is the industry consensus, not our invention.** VS Code
   publishes nearly our exact taxonomy: lexical grep, glob, symbol-level
   language intelligence, and semantic search as an optional index. We are
   building the well-trodden shape, which lowers risk.

2. **The lexical floor being index-free is the right default.** Claude Code
   ships only ripgrep plus tools and is the most-used harness. Grep with no
   setup is the baseline everyone keeps. Our rule that the graph is an upgrade,
   never a dependency, matches what the market actually tolerates.

3. **Indexing the lexical search itself is a real fourth tier we had not
   planned.** Cursor's sparse n-gram index attacks exactly agentgrep's weakness:
   `rg` must read every file, which costs 15+ seconds on a large monorepo. This
   is optional and later, but it is the missing tier between "scan everything"
   and "structural graph".

4. **Cursor independently confirms our freshness principle.** They state the
   regex index must be very fresh, unlike the semantic index, because if the
   agent cannot find text it just wrote, it goes on a wild goose chase. And they
   key the index to a commit with local edits layered on top. That is our
   "commit-anchored, never blocks, patch over the top" design, already in
   production, and their Merkle tree is our content-hash manifest.

5. **Aider is the proof that our extraction plan works, and it validates
   tree-sitter queries specifically.** Aider uses `tags.scm` query files per
   language to extract definitions and references, then PageRank to rank, then a
   token budget. That is the cheap interpretation layer we proposed instead of
   porting graphify's resolution engine. It is a small, shipped Python
   implementation doing exactly what we want.

6. **LSP is the big unexploited lever.** Only Serena and VS Code's "usages"
   tool use it, and Serena's own agent testimonials call it the highest-impact
   addition. Every other harness leans on text. It is also exactly the tier we
   ranked as "optional, consume if present, never install".
