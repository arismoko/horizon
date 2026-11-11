#!/usr/bin/env bash
# Published-loop benchmark (fnr-loops), host side: the points-tick kit's tick scene on the bundle's builds.
#   FNR_INPUT=BUNDLE [FNR_RES=PREPARED_26.3] [FNR_SAVE=SAVE] [BENCH_CPUS=..] [PERF=perf] [FNR_VARIANTS=base,L0,L1c,L1,L3] run-on-host.sh
# BUNDLE holds kit/ (the points-tick kit) and builds/<V>/zc-game/ (horizon-client beside its libraries).
# Every run gets fresh scratch homes (kit scene.py); nothing outside ~/bench-fnr-loops is written but the
# results copy. Read before running: it calls only kit/points-tick/scene.py (setup, tick) and summary.py.
set -euo pipefail
: "${FNR_INPUT:?set FNR_INPUT to the bundle}"
: "${FNR_RES:=/box/shared/vanilla/horizon-client/26.3}"
: "${FNR_RESULTS:=/box/shared/fn-replace/loops-bench/results}"
INPUT=$(realpath "$FNR_INPUT")
BASE=$HOME/bench-fnr-loops
mkdir -p "$BASE/results"
WORK=$(mktemp -d "$BASE/run.XXXXXX")
trap 'rm -rf -- "$WORK"' EXIT
cp -a "$INPUT/kit" "$WORK/kit"
# A private copy: the game may write beside its resources.
cp -a "$FNR_RES" "$WORK/resources"
VARIANTS=${FNR_VARIANTS:-base,L0,L1c,L1,L3}
IFS=',' read -r -a builds <<< "$VARIANTS"
for v in "${builds[@]}"; do
  case "$v" in base|L0|L1c|L1|L3) ;; *) echo "unknown variant $v" >&2; exit 2;; esac
  mkdir -p "$WORK/out/stage-$v"
  cp -a "$INPUT/builds/$v/zc-game" "$WORK/out/stage-$v/zc-game"
done
export OUT=$WORK/out HORIZON_RES=$WORK/resources TMPDIR=$WORK
export HORIZON_LAUNCHER_TRASH=$WORK/trash HORIZON_LAUNCHER_CLIPBOARD=$WORK/clipboard HORIZON_LAUNCHER_DESKTOP=$WORK/desktop
mkdir -p "$HORIZON_LAUNCHER_TRASH" "$HORIZON_LAUNCHER_CLIPBOARD" "$HORIZON_LAUNCHER_DESKTOP"
RESULT=$BASE/results/$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$RESULT"
{ date -u; uname -a; lscpu; echo "BENCH_CPUS=${BENCH_CPUS:-}"; cat /proc/sys/kernel/perf_event_paranoid; "${PERF:-perf}" --version; } > "$RESULT/environment.txt"
if [ -n "${FNR_SAVE:-}" ]; then
  mkdir -p "$WORK/out/points-setup"
  cp -a "$FNR_SAVE" "$WORK/out/points-setup/game"
else
  python3 "$WORK/kit/points-tick/scene.py" setup "$WORK/out/stage-${builds[0]}/zc-game" "$WORK/out/points-setup" > "$RESULT/setup.log" 2>&1
fi
# Five rounds in rotating order, 500 warm-up and 2,000 measured ticks each, as the kit's campaign.
python3 - "$WORK/kit/points-tick/scene.py" "$VARIANTS" <<'PY' | tee "$RESULT/campaign.log"
import os, pathlib, subprocess, sys
scene, names = sys.argv[1], sys.argv[2].split(',')
out = pathlib.Path(os.environ['OUT'])
for r in range(1, 6):
    for v in names[r % len(names):] + names[:r % len(names)]:
        print('round', r, 'variant', v, flush=True)
        subprocess.run(['python3', scene, 'tick', str(out / f'stage-{v}' / 'zc-game'), str(out / 'points-setup' / 'game'),
                        str(out / 'runs' / 'loops' / f'tick-{v}-{r}'), '500', '2000'], check=True)
PY
python3 "$WORK/kit/points-tick/summary.py" loops > "$RESULT/summary.txt"
cp -a "$WORK/out/runs" "$RESULT/runs"
mkdir -p "$FNR_RESULTS"
cp -a "$RESULT" "$FNR_RESULTS/"
echo "Results: $FNR_RESULTS/$(basename "$RESULT")"
