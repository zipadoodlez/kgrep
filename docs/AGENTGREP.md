# Notes on agentgrep: what we take, and what we fix

agentgrep is the code we absorb, so its weaknesses become ours unless we
deliberately fix them. This file records them with evidence, so the work list is
grounded in the source rather than in impressions.

Everything here is a fair reading of a tool that works well and is thoughtfully
built. Most of these are deliberate tradeoffs that were right for their context.
They are listed because they are wrong for ours.

## Known weaknesses

### 1. The output cap truncates an unranked list

The cap takes the first N matches **in file-path order**, because `run_grep_native`
sorts results by path, not by relevance:

```rust
results.sort_by(|a, b| a.path.cmp(&b.path));
```

So for a broad query the surviving matches come from the alphabetically earliest
files, not the most useful ones. The cap selects an arbitrary N rather than the
best N. This is a recall problem, not a display problem: the file holding the
answer can be dropped because its path sorts late.

**Fix:** rank before spending the budget.

### 2. The cap counts matches, not tokens

The unit is match count (200 in jcode, unbounded in the CLI), but the currency is
tokens. A match line can run to 240 characters, and each file also carries a
symbol header and an "other:" summary. So 200 matches ranges from a few thousand
to roughly fifteen thousand tokens depending on the query. The bound is real but
imprecise, and it is not the bound the caller cares about.

**Fix:** budget in tokens.

### 3. It is a context guard, not a memory guard

The cap is applied at **render time**, after every match has been collected. So it
protects the model's context window and does nothing for peak memory, which stays
proportional to the total number of matches. That directly conflicts with the
flat-memory constraint this project is built on.

**Fix:** bound the work during the scan, not the string at the end.

### 4. Truncation drops whole files rather than reducing detail

Once the cap is reached, the per-file loop breaks and remaining files are not
rendered at all. The existence of a matching file is information, and it is the
cheapest information to keep, so dropping it is the most expensive thing to drop
per token saved.

**Fix:** name every matching file, expand the top few, and let the tail be a
count.

### 5. The safety default lives in the caller, not the library

`run_grep` has no cap. The cap is passed at render time, so every consumer must
remember it. jcode does remember (`DEFAULT_GREP_MAX_REGIONS = 200`), and its own
comment records why, a single call producing 923k characters across 2,027
benchmark transcripts. The CLI does not remember, and `run_grep`'s default does
not either, so a new caller reintroduces the bug that was already found once.

**Fix:** make the bounded packet the library default, with an explicit opt-out.

### 6. Two searchers, and the machinery to keep them agreeing

The native scan is preceded by an `rg` subprocess fast path, and because the two
make slightly different decisions there is a large body of code forcing parity:
symlink handling, `--glob` semantics, rg config isolation, glob+type filtering,
non-UTF-8 path parsing. In v0.1.6 the `rg` path and its parsing is about 560
lines, and the v0.1.7 release is largely parity fixes. Their own release notes
call it the "parity swarm".

**Fix:** one searcher. Already done in kgrep.

### 7. The structure sketch is approximate and flat

Extraction is line-based keyword spotting, not a parse. Two consequences:

- **End lines are not real.** An item ends on the line before the next item
  begins, so a function containing a nested item is described too short. The code
  documents this as intentional.
- **Items are flat.** There is no nesting, so it cannot express "this method is
  inside this impl", which is what a zoomed answer needs.

It covers Rust, TypeScript/JavaScript, Python, and Markdown. Everything else falls
to a generic scanner that finds only ALL-CAPS section lines, so for most languages
the structure information is close to empty.

**Fix:** consume ctags as a declaration index, with the line-based scanner kept
as the fallback where no index exists. It emits the fields and methods the
line-scanner never sees, and it covers roughly a hundred languages, so the
coverage hole closes at index time rather than by compiling grammars in.

### 8. No symbol resolution

It can tell you where a name appears, but not which definition a name refers to.
"Who calls this" and "what implements this" are therefore out of reach, and they
are the questions an agent needs answered before editing anything.

**Fix:** resolution as a later source, never in-process via a language server. See
`docs/SOTA.md` for the cost analysis.

### 9. Measured standing versus `rg`

Their own status document records the geometric mean at **1.39x slower than `rg`**
overall, with `--paths-only` at or near parity. Their profile attributes the gap
to the structure pass, matched-file re-reads, and the `rg` subprocess fork/exec,
rather than to the lexical scan. Notably, their designed next step is to drop the
`rg` subprocess and fuse the passes, which is the direction this project already
took.

## Worth keeping, unchanged

Credit where it is due, because these are the reasons agentgrep is worth building
on rather than replacing:

- **The header always reports the true total**, so a caller who raises the cap
  knows what they are asking for. Progressive disclosure done properly.
- **Grouping matches under their enclosing symbol.** The single biggest usability
  win over raw grep output.
- **`--paths-only` early exit**, which is the cheapest possible answer to "which
  files".
- **Deterministic ordering** by native path bytes, so output never wobbles with
  filesystem read order.
- **Binary refusal and lossy UTF-8 decoding**, so a stray byte in a comment does
  not hide a whole file, and non-UTF-8 names stay addressable.
- **The architecture ignores verdict**: `ignore` crate semantics, `.rgignore`,
  symlink handling, and glob matching that is already aligned with `rg`'s.
