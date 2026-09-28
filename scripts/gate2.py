#!/usr/bin/env python3
"""Gate 2, as a bound rather than a spike.

Question: could harness familiarity pay on top of kgrep's ranking?

This does not implement the mechanism. It measures the ceiling. For each
multi-turn task it reports how many tokens sit before the answer, and how much of
that sits in blocks belonging to files the agent had already seen. That second
number is an UPPER BOUND on the saving, because it credits familiarity with
removing those blocks entirely, when the real mechanism replaces them with a note
line and keeps six structure items.

A bound is enough to decide, and it is much cheaper than a spike. If the best
possible saving is under the kill threshold, no implementation can clear it.

It also computes the two numbers the brief asked for:

  gold     the declared `gold_known` set, handed in directly
  derived  the files the earlier turns actually returned, which is what kcode
           would have to infer, and the gap between them is the value of the
           inference

Usage:
  scripts/gate2.py --bin target/release/kgrep --corpus ~/kcode
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

TASKS_DEFAULT = os.path.join(os.path.dirname(__file__), "..", "bench", "tasks-multiturn.json")
CHARS_PER_TOKEN = 4.0


def blocks(text: str) -> list[tuple[str, int]]:
    """Split rendered output into (path, chars) hit blocks.

    The header is everything before the first blank line. After it, a hit block
    starts at every line that is not indented, which is how the renderer writes a
    path. Verified against the real format rather than assumed.
    """
    lines = text.splitlines(keepends=True)
    index = 0
    while index < len(lines) and lines[index].strip() != "":
        index += 1

    out: list[tuple[str, int]] = []
    start: int | None = None
    path = ""
    for cursor in range(index, len(lines)):
        line = lines[cursor]
        if line.strip() and not line.startswith((" ", "\t")):
            if start is not None:
                out.append((path, sum(len(part) for part in lines[start:cursor])))
            start, path = cursor, line.strip()
    if start is not None:
        out.append((path, sum(len(part) for part in lines[start:])))
    return out


def run(binary: str, corpus: str, args: list[str]) -> str:
    proc = subprocess.run(
        [binary] + args, cwd=corpus, capture_output=True, text=True, timeout=120
    )
    return proc.stdout


def report(binary: str, corpus: str, task: dict) -> dict:
    turns = task["turns"] if "turns" in task else [{"args": task["args"]}]
    outputs = [run(binary, corpus, turn["args"]) for turn in turns]
    final = outputs[-1]

    # Derived state: what the earlier turns actually surfaced. This is a
    # generous proxy for "what the agent read", since a real harness marks a file
    # known when it is read rather than when it is merely listed, and this counts
    # everything listed.
    derived = {path for out in outputs[:-1] for path, _ in blocks(out)}

    found = blocks(final)
    answer = task["expect_path"]
    answer_at = next((n for n, (path, _) in enumerate(found) if answer in path), None)

    row = {
        "id": task["id"],
        "kind": task["kind"],
        "turns": len(turns),
        "files": len(found),
        "answer_rank": None if answer_at is None else answer_at + 1,
        "tokens_to_answer": None,
        "gold_bound": 0.0,
        "derived_bound": 0.0,
        "gold_hits": [],
        "derived_hits": [],
    }
    if answer_at is None:
        row["error"] = "answer path absent from the final output"
        return row

    prefix = sum(chars for _, chars in found[:answer_at])
    row["tokens_to_answer"] = round(prefix / CHARS_PER_TOKEN, 1)

    gold = set(task.get("gold_known", []))
    for path, chars in found[:answer_at]:
        if path in gold:
            row["gold_bound"] += chars / CHARS_PER_TOKEN
            row["gold_hits"].append(path)
        if path in derived:
            row["derived_bound"] += chars / CHARS_PER_TOKEN
            row["derived_hits"].append(path)
    row["gold_bound"] = round(row["gold_bound"], 1)
    row["derived_bound"] = round(row["derived_bound"], 1)
    return row


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", required=True)
    parser.add_argument("--corpus", default=os.path.expanduser("~/kcode"))
    parser.add_argument("--tasks", default=TASKS_DEFAULT)
    parser.add_argument("--json", default=None)
    args = parser.parse_args()

    with open(args.tasks) as handle:
        tasks = json.load(handle)["tasks"]

    print(f"{'task':<34}{'kind':<12}{'rank':>5}{'tok->ans':>10}{'gold':>8}{'derived':>9}  share")
    rows = []
    for task in tasks:
        row = report(os.path.abspath(args.bin), args.corpus, task)
        rows.append(row)
        share = (
            ""
            if not row.get("tokens_to_answer")
            else f"{100 * row['gold_bound'] / row['tokens_to_answer']:.0f}%"
        )
        print(
            f"{row['id']:<34}{row['kind']:<12}"
            f"{row.get('answer_rank') or '-':>5}"
            f"{row.get('tokens_to_answer') or 0:>10.1f}"
            f"{row['gold_bound']:>8.1f}{row['derived_bound']:>9.1f}  {share}"
        )

    benign = [r for r in rows if r["kind"] == "benign"]
    controls = [r for r in rows if r["kind"] == "control"]
    print()
    print("Controls must show a bound of zero. Any other number means the known set")
    print("intersects the prefix for a task where it provably must not.")
    for row in controls:
        flag = "OK" if row["gold_bound"] == 0 else "UNEXPECTED"
        print(f"  {row['id']:<34} bound={row['gold_bound']:<8} {flag}")
    print()
    if benign:
        makes = [100 * r["gold_bound"] / r["tokens_to_answer"] for r in benign if r["tokens_to_answer"]]
        print(
            f"Benign ceiling: best case {max(makes):.0f}%, "
            f"median {sorted(makes)[len(makes) // 2]:.0f}% of tokens-to-answer."
        )
        print("That is an upper bound: the real mechanism keeps a note and six items.")

    if args.json:
        with open(args.json, "w") as handle:
            json.dump({"tasks": rows}, handle, indent=2)
        print(f"\nwrote {args.json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
