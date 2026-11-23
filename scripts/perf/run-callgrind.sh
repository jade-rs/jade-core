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

# Do not spend a full run collecting zero events when optimized code has lost
# the function that controls collection. Both binaries use this head harness.
python3 - "$bin" <<'PY_SYMBOL'
import subprocess
import sys
symbols = subprocess.check_output(['nm', '--defined-only', sys.argv[1]], text=True)
found = [line for line in symbols.splitlines()
         if len(parts := line.split()) >= 3
         and parts[-1] == 'jade_docx_perf_measured_region'
         and parts[-2] in ('T', 't')]
if len(found) != 1:
    raise SystemExit('Missing unique Callgrind measurement boundary: ' + sys.argv[1])
PY_SYMBOL
read -r -a scenarios <<< "${PERF_SCENARIOS:-load layout repaint keystroke keystroke-marks export}"
fixture_dir="${PERF_FIXTURE_DIR:-$(cd "$(dirname "$0")/../.." && pwd)/public/fixtures}"

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
