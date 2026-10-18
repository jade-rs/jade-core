#!/usr/bin/env bash
# Runs every jade-docx-perf scenario on every fixture under callgrind.
#
#   scripts/perf/run-callgrind.sh <jade-docx-perf binary> <out dir> <fixture>...
#
# Writes <out dir>/<scenario>@<fixture>.out per run. Only the scenario's
# measured region is collected (see `perf_measured_region`), so setup such as
# loading the fixture does not count.
set -euo pipefail

bin=$1
out=$2
shift 2
fixtures=("$@")
scenarios=(load layout repaint keystroke export)
fixture_dir="$(cd "$(dirname "$0")/../.." && pwd)/public/fixtures"

mkdir -p "$out"
jobs=()
for fixture in "${fixtures[@]}"; do
  for scenario in "${scenarios[@]}"; do
    jobs+=("$scenario $fixture")
  done
done

run_one() {
  local scenario=$1 fixture=$2
  valgrind --tool=callgrind \
    --collect-atstart=no \
    --toggle-collect='*perf_measured_region*' \
    --callgrind-out-file="$out/$scenario@$fixture.out" \
    "$bin" "$scenario" "$fixture_dir/$fixture" \
    >"$out/$scenario@$fixture.log" 2>&1 || {
    echo "failed: $scenario $fixture" >&2
    tail -20 "$out/$scenario@$fixture.log" >&2
    return 1
  }
}
export -f run_one
export bin out fixture_dir

parallel=${PERF_JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu)}
printf '%s\n' "${jobs[@]}" | xargs -P "$parallel" -L 1 bash -c 'run_one $0 $1'
