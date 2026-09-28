# kgrep

Code search and retrieval for agents: one shaper, several sources, a bounded
packet.

Not a standalone tool yet. It exists to replace `agentgrep` behind kcode's search
tool, and it is a library first: `Query` in, `Packet` out, with ranking, a
token-denominated budget, and structure in about a hundred languages.

- **`AGENTS.md`** is the plan of record. What is settled, what has been measured,
  where the code is, and what is next.
- **`bench/README.md`** is the measurement record, including three claims that were
  made and then retracted, kept in place so the next reader can see how they died.
- **`docs/`** holds the design, the survey of how other harnesses retrieve code,
  the review of the tool this absorbs, and the kcode integration brief.

Two numbers, measured through the same call path the harness uses: a median of
28.8 tokens to reach an answer against agentgrep's 285.4, and recall 25/25 against
24/25, on 25 verified navigation tasks.

MIT, with agentgrep's notice retained. It is a derivative of
[agentgrep](https://github.com/1jehuang/agentgrep); see `NOTICE`.
