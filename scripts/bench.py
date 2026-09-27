#!/usr/bin/env python3
"""Measure retrieval quality: does the tool surface the answer, and how much
output must be read before it does?

We measure the tool, not the model. For each task we run the tool, then find how
much of its output a reader must get through before the ground truth appears.
That is the quantity that actually costs an agent: tokens spent looking.

Token counts are estimates at 4 characters per token, so they are comparable
across tools and honest about being an approximation. Character counts are exact
and always reported alongside.

Usage:
    scripts/bench.py --bin PATH [--bin PATH ...] [--corpus DIR] [--json OUT]
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import statistics
import sys

CHARS_PER_TOKEN = 4.0
TASKS_DEFAULT = os.path.join(os.path.dirname(__file__), "..", "bench", "tasks.json")


def est_tokens(text: str) -> float:
    return len(text) / CHARS_PER_TOKEN


def tokens_before(text: str, needle: str) -> float | None:
    """Estimated tokens up to and including the first line containing `needle`."""
    if not needle:
        return None
    offset = 0
    for line in text.splitlines(keepends=True):
        offset += len(line)
        if needle in line:
            return offset / CHARS_PER_TOKEN
    return None


def run_task(binary: str, corpus: str, task: dict, timeout: float) -> dict:
    cmd = [binary] + task["args"]
    result = {
        "id": task["id"],
        "args": task["args"],
        "exit": None,
        "found": False,
        "tokens_to_answer": None,
        "output_tokens": 0.0,
        "output_chars": 0,
        "error": None,
    }
    try:
        proc = subprocess.run(
            cmd,
            cwd=corpus,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        result["error"] = f"timeout after {timeout}s"
        return result

    out = proc.stdout
    result["exit"] = proc.returncode
    result["output_chars"] = len(out)
    result["output_tokens"] = round(est_tokens(out), 1)

    needle = task.get("expect_path") or task.get("expect_symbol")
    if proc.returncode != 0 and not out:
        result["error"] = proc.stderr.strip()[:200] or "non-zero exit with no output"
        return result

    position = tokens_before(out, needle)
    if position is not None:
        result["found"] = True
        result["tokens_to_answer"] = round(position, 1)
    return result


def summarize(rows: list[dict]) -> dict:
    found = [r for r in rows if r["found"]]
    positions = [r["tokens_to_answer"] for r in found if r["tokens_to_answer"] is not None]
    return {
        "tasks": len(rows),
        "found": len(found),
        "recall": round(len(found) / len(rows), 3) if rows else 0.0,
        "mean_tokens_to_answer": round(statistics.fmean(positions), 1) if positions else None,
        "median_tokens_to_answer": round(statistics.median(positions), 1) if positions else None,
        "total_output_tokens": round(sum(r["output_tokens"] for r in rows), 1),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", action="append", required=True, help="binary to measure; repeatable")
    parser.add_argument("--corpus", default=os.path.expanduser("~/kcode"))
    parser.add_argument("--tasks", default=TASKS_DEFAULT)
    parser.add_argument("--json", default=None, help="write full results here")
    parser.add_argument("--timeout", type=float, default=60.0)
    args = parser.parse_args()

    with open(args.tasks) as handle:
        spec = json.load(handle)
    tasks = spec["tasks"]

    if not os.path.isdir(args.corpus):
        print(f"error: corpus not found: {args.corpus}", file=sys.stderr)
        return 2

    report = {"corpus": args.corpus, "tasks": len(tasks), "tools": {}}

    for binary in args.bin:
        rows = [run_task(binary, args.corpus, task, args.timeout) for task in tasks]
        summary = summarize(rows)
        report["tools"][binary] = {"summary": summary, "rows": rows}

        name = os.path.basename(binary)
        print(f"\n=== {name} ===")
        print(f"{'task':<26} {'found':<6} {'tok->ans':>9} {'out tok':>9}")
        for row in rows:
            position = row["tokens_to_answer"]
            mark = "yes" if row["found"] else "NO"
            print(
                f"{row['id']:<26} {mark:<6} "
                f"{(position if position is not None else '-'):>9} "
                f"{row['output_tokens']:>9}"
            )
            if row["error"]:
                print(f"  ! {row['error']}")
        print(
            f"recall {summary['recall']:.0%}  "
            f"mean tokens->answer {summary['mean_tokens_to_answer']}  "
            f"total output tokens {summary['total_output_tokens']}"
        )

    if args.json:
        with open(args.json, "w") as handle:
            json.dump(report, handle, indent=2)
        print(f"\nwrote {args.json}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
