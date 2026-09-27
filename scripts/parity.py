#!/usr/bin/env python3
"""Differential parity against the harness-faithful agentgrep oracle.

Builds a corpus that is nasty on purpose, runs the same invocations through both
binaries, and compares **which files each one found**. The point is the scan:
ignore rules, symlinks, globs, file types, and names that are not valid UTF-8 are
where two implementations drift apart, and none of that shows up on a normal
repository.

Comparison is on the set of matched files, with the display suffix stripped,
because the two tools deliberately spell non-UTF-8 names differently (agentgrep
uses a `#b=` token scheme, kgrep uses `#raw=`, and both are injective). Whether
they find the same files is the question; how they render an unrepresentable
name is a documented difference.

Usage:
    scripts/parity.py --kgrep target/release/kgrep \
                      --oracle bench/oracle/target/release/agentgrep-harness
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile

# Every invocation to compare, as argument lists appended to the binary.
# `token` is the word planted in the corpus.
CASES: list[tuple[str, list[str]]] = [
    ("grep literal", ["grep", "PARITYTOKEN", "--paths-only"]),
    ("grep regex", ["grep", "PARITY(TOKEN|WORD)", "--paths-only", "--regex"]),
    ("grep --type rs", ["grep", "PARITYTOKEN", "--paths-only", "--type", "rs"]),
    ("grep --glob '*.rs'", ["grep", "PARITYTOKEN", "--paths-only", "--glob", "*.rs"]),
    (
        "grep glob + type",
        ["grep", "PARITYTOKEN", "--paths-only", "--glob", "*.rs", "--type", "rs"],
    ),
    ("grep --hidden", ["grep", "PARITYTOKEN", "--paths-only", "--hidden"]),
    ("grep --no-ignore", ["grep", "PARITYTOKEN", "--paths-only", "--no-ignore"]),
    ("grep no extension", ["grep", "EXTENSIONLESS", "--paths-only"]),
    ("grep in symlink", ["grep", "VIASYMLINK", "--paths-only"]),
    ("grep non-utf8 name", ["grep", "ODDNAME", "--paths-only"]),
    ("grep binary file", ["grep", "BINARYTOKEN", "--paths-only"]),
    ("find path terms", ["find", "parity", "dir", "--paths-only"]),
    ("find --type rs", ["find", "parity", "--paths-only", "--type", "rs"]),
]


def build_corpus(root: str) -> None:
    """A corpus where each edge case has its own distinctive token."""

    def write(relative: str, body: str) -> None:
        path = os.path.join(root, relative)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as handle:
            handle.write(body)

    # Ordinary files, one per token, so a case can be attributed to a cause.
    write("src/parity_dir/plain.rs", "// PARITYTOKEN\nfn plain() {}\n")
    write("src/parity_dir/other.py", "PARITYTOKEN = 1\n")
    write("src/parity_dir/notes.md", "PARITYTOKEN in docs\n")
    # No extension at all, which only the fallback scanner handles.
    write("src/parity_dir/extensionless", "// EXTENSIONLESS\n")
    # A hidden file, which only --hidden should reach.
    write(".hidden_parity.rs", "// PARITYTOKEN hidden\n")
    # Binary: a NUL byte means the file is refused entirely.
    with open(os.path.join(root, "src/parity_dir/binary.bin"), "wb") as handle:
        handle.write(b"BINARYTOKEN\x00\x01\x02\n")
    # Empty. Nothing to find, but it must not crash anything.
    write("src/parity_dir/empty.rs", "")
    # Ignored by git, which only --no-ignore should reach.
    write("src/parity_dir/ignored.rs", "// PARITYTOKEN ignored\n")
    write(".gitignore", "src/parity_dir/ignored.rs\n")
    # A symlink to a file, and a symlink to a directory.
    os.symlink(
        os.path.join(root, "src/parity_dir/plain.rs"),
        os.path.join(root, "src/parity_dir/links_to_plain.rs"),
    )
    os.makedirs(os.path.join(root, "src/parity_dir/real_dir"), exist_ok=True)
    write("src/parity_dir/real_dir/target.rs", "// VIASYMLINK here\n")
    os.symlink(
        os.path.join(root, "src/parity_dir/real_dir"),
        os.path.join(root, "src/parity_dir/linked_dir"),
    )
    # A name that is not valid UTF-8. Only reachable on Unix.
    if os.name == "posix":
        odd = os.path.join(root.encode(), b"src/parity_dir/odd_\xff_name.rs")
        with open(odd, "wb") as handle:
            handle.write(b"// ODDNAME\n")


def normalise(display: str) -> str:
    """Drop the display-only suffix the two tools spell differently."""
    for marker in ("#b=", "#raw="):
        if marker in display:
            return display.split(marker)[0]
    return display


def run(binary: str, args: list[str], root: str) -> tuple[set[str], str]:
    proc = subprocess.run(
        [binary] + args,
        cwd=root,
        capture_output=True,
        text=True,
        errors="replace",
        timeout=60,
    )
    if proc.returncode != 0:
        return set(), proc.stderr.strip()[:200]
    found = {normalise(line.strip()) for line in proc.stdout.splitlines() if line.strip()}
    return found, ""


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--kgrep", required=True)
    parser.add_argument("--oracle", required=True)
    parser.add_argument("--keep", action="store_true", help="keep the corpus for inspection")
    args = parser.parse_args()

    # Absolute, because every run happens with the corpus as the working
    # directory.
    args.kgrep = os.path.abspath(args.kgrep)
    args.oracle = os.path.abspath(args.oracle)

    for name, path in (("kgrep", args.kgrep), ("oracle", args.oracle)):
        if not os.path.exists(path):
            print(f"error: {name} not found at {path}", file=sys.stderr)
            return 2

    root = tempfile.mkdtemp(prefix="kgrep-parity-")
    build_corpus(root)
    # A git repository, so `.gitignore` is honoured the same way by both.
    subprocess.run(["git", "init", "-q"], cwd=root, check=False)

    print(f"{'case':<26} {'files':>6} {'parity':<8} notes")
    failures = 0
    for label, case in CASES:
        theirs, their_error = run(args.oracle, case, root)
        ours, our_error = run(args.kgrep, case, root)
        if their_error or our_error:
            note = (their_error or our_error).replace("\n", " ")[:60]
            print(f"{label:<26} {'-':>6} {'ERROR':<8} {note}")
            failures += 1
            continue
        if theirs == ours:
            print(f"{label:<26} {len(ours):>6} {'same':<8}")
            continue
        missing = sorted(theirs - ours)
        extra = sorted(ours - theirs)
        note = []
        if missing:
            note.append(f"kgrep missed {len(missing)}: {', '.join(missing[:2])}")
        if extra:
            note.append(f"kgrep extra {len(extra)}: {', '.join(extra[:2])}")
        print(f"{label:<26} {len(ours):>6} {'DIFF':<8} {'; '.join(note)}")
        failures += 1

    print()
    if failures == 0:
        print(f"all {len(CASES)} cases agree on the set of files found")
    else:
        print(f"{failures} of {len(CASES)} cases differ")
    if args.keep:
        print(f"corpus kept at {root}")
    else:
        shutil.rmtree(root, ignore_errors=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
