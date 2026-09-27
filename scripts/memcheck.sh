#!/usr/bin/env bash
# Prove that peak memory does not scale with repository size.
#
# Builds synthetic repositories of increasing size, runs the same query against
# each, and reports peak RSS. The claim is that the numbers stay flat, because
# the funnel works one file at a time and releases it.
#
# Usage: scripts/memcheck.sh [sizes...]
#
# The default is deliberately small. It writes a few thousand short files, so
# run it on purpose, not casually. Add bigger sizes when you want a stronger
# claim: scripts/memcheck.sh 100 1000 10000
set -euo pipefail

cd "$(dirname "$0")/.."

SIZES=("$@")
if [ ${#SIZES[@]} -eq 0 ]; then
  SIZES=(100 1000)
fi

cargo build --release --quiet
BIN=./target/release/graphgrep
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# A query that matches nothing, so the measurement is the walk itself and not
# the size of the result.
QUERY="zzz_never_present_token_zzz"

printf '%-10s %-14s %s\n' "files" "peak_rss_kb" "notes"
for size in "${SIZES[@]}"; do
  repo="$WORK/repo-$size"
  mkdir -p "$repo/src"
  python3 - "$repo" "$size" <<'PY'
import os, sys
root, n = sys.argv[1], int(sys.argv[2])
# ~40 short lines per file, so file count is the only thing that varies much.
body = "".join(f"pub fn helper_{i}() -> u32 {{ {i} }}\n" for i in range(40))
for i in range(n):
    d = os.path.join(root, "src", f"mod_{i % 100:03d}")
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, f"file_{i:06d}.rs"), "w") as f:
        f.write(f"// file {i}\n{body}")
PY

  peak=$(GRAPHGREP_PEAK_RSS=1 "$BIN" grep --path "$repo" "$QUERY" 2>&1 >/dev/null \
    | awk '/^peak_rss_kb:/{print $2}')
  printf '%-10s %-14s %s\n' "$size" "${peak:-n/a}" "query matches nothing"
done

echo
echo "Flat across sizes means the memory claim holds. Growth means a regression."
