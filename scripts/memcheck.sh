#!/usr/bin/env bash
# Peak memory, against repo size and against query weight.
#
# Two measurements:
#   1. Scaling: the same query over synthetic repos of increasing size. The
#      result should be flat, because the funnel works one file at a time.
#   2. Real repo: a real tree, with light and heavy queries, to find the actual
#      ceiling that a memory budget has to live under.
#
# Peak RSS is read from the tool's own /proc/self/status high-water mark, which
# is the only trustworthy source. External wrappers inflate the number badly:
# a forked parent shares its pages with the child, so a Python or shell wrapper
# reports its own footprint as the child's peak. An early version of this script
# did exactly that and reported a flat 13 MB for every query and every binary.
# Do not measure this with a wrapper unless the wrapper is tiny.
#
# Usage:
#   scripts/memcheck.sh                 # scaling, small by default
#   scripts/memcheck.sh 100 1000 10000  # scaling, bigger
#   scripts/memcheck.sh --repo ~/kcode  # a real tree
set -euo pipefail

cd "$(dirname "$0")/.."
cargo build --release --quiet
BIN=./target/release/kgrep

# A query that matches nothing, so the measurement is the walk itself.
NULL_QUERY="zzz_never_present_token_zzz"

measure() { # measure <root> <args...>
  local root="$1"; shift
  ( cd "$root" && KGREP_PEAK_RSS=1 "$OLDPWD/$BIN" "$@" 2>&1 >/dev/null \
    | awk '/^peak_rss_kb:/{print $2}' )
}

if [ "${1:-}" = "--repo" ]; then
  root="${2:?--repo needs a directory}"
  root=$(cd "$root" && pwd)
  files=$(git -C "$root" ls-files 2>/dev/null | wc -l || find "$root" -type f | wc -l)
  echo "repo: $root  ($files tracked files)"
  echo
  printf '%-32s %10s\n' "query" "peak RSS"
  printf '%-32s %10s\n' "grep (matches nothing)" "$(measure "$root" grep "$NULL_QUERY") KB"
  printf '%-32s %10s\n' "grep --paths-only fn" "$(measure "$root" grep --paths-only fn) KB"
  printf '%-32s %10s\n' "grep fn (very heavy result)" "$(measure "$root" grep fn) KB"
  printf '%-32s %10s\n' "grep --type rs fn" "$(measure "$root" grep --type rs fn) KB"
  exit 0
fi

SIZES=("$@")
if [ ${#SIZES[@]} -eq 0 ]; then
  SIZES=(100 1000)
fi

echo "scaling: the same query over synthetic repos of growing size."
echo "The claim is that the number stays flat, because work is per file."
echo
printf '%-10s %14s\n' "files" "peak RSS"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

for size in "${SIZES[@]}"; do
  repo="$WORK/repo-$size"
  mkdir -p "$repo/src"
  python3 - "$repo" "$size" <<'PY'
import os, sys
root, n = sys.argv[1], int(sys.argv[2])
body = "".join(f"pub fn helper_{i}() -> u32 {{ {i} }}\n" for i in range(40))
for i in range(n):
    d = os.path.join(root, "src", f"mod_{i % 100:03d}")
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, f"file_{i:06d}.rs"), "w") as f:
        f.write(f"// file {i}\n{body}")
PY
  printf '%-10s %14s\n' "$size" "$(measure "$repo" grep "$NULL_QUERY") KB"
done

echo
echo "Flat means the memory claim holds. Growth means a regression."
