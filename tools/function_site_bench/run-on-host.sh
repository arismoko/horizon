#!/usr/bin/env bash
# Host-only driver. No installs, network, sudo, or writes outside ~/bench-fnr and final results.
# INPUT must be the host-visible PRIVATE artifact bundle exported by the worker, never vanilla.
# Kit scripts called: points-tick/scene.py (setup, tick), points-tick/summary.py,
# points-tick/census/census.py, points-tick/census/match.py; scene.py imports common/kitenv.py.
# This is the tick-only campaign (same rotating order as campaign.py); no frame benchmark.
set -euo pipefail
: "${FNR_INPUT:?Set FNR_INPUT to the host-visible worker artifact bundle}"
: "${FNR_RESULTS:=$HOME/bench-fnr/results}"
INPUT=$(realpath "$FNR_INPUT")
case "$INPUT" in *vanilla*) echo 'Refusing shared vanilla input' >&2; exit 2;; esac
BASE=$HOME/bench-fnr
mkdir -p "$BASE/results"
WORK=$(mktemp -d "$BASE/run.XXXXXX")
cleanup() {
  local status=$?
  # A failed perf attach can leave the kit's server blocked on its control FIFO.
  # Terminate only processes with an argv path inside this run's private directory.
  python3 - "$WORK" <<'PY'
import os, pathlib, signal, sys
prefix = os.fsencode(sys.argv[1] + '/')
for proc in pathlib.Path('/proc').glob('[0-9]*'):
    try:
        args = (proc / 'cmdline').read_bytes().split(b'\0')
        if any(arg.startswith(prefix) for arg in args):
            os.kill(int(proc.name), signal.SIGTERM)
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        pass
PY
  if [ "$status" -ne 0 ] && [ -n "${RESULT:-}" ]; then
    [ ! -d "$WORK/out/runs" ] || cp -a "$WORK/out/runs" "$RESULT/partial-runs" || true
    [ ! -f "$WORK/out/points-setup/game/server.log" ] || cp "$WORK/out/points-setup/game/server.log" "$RESULT/setup-server.log" || true
    [ ! -f "$WORK/perf-invocations.txt" ] || cp "$WORK/perf-invocations.txt" "$RESULT/perf-invocations.txt" || true
  fi
  rm -rf -- "$WORK"
}
trap cleanup EXIT
# All copied data is private and disposable. No game is run against INPUT.
cp -a "$INPUT/kit" "$WORK/kit"
cp -a "$INPUT/resources" "$WORK/resources"
# Only the three signed distribution packages approved by root, privately extracted.
# Arch calls the libpfm.so.4 provider `libpfm`, not Debian's `libpfm4`.
: "${PERF_PACKAGE_DIR:=$BASE/perf-7.2.11-1}"
: "${LIBPFM_PACKAGE_DIR:=$BASE/libpfm}"
: "${LIBTRACEEVENT_PACKAGE_DIR:=$BASE/libtraceevent}"
export FNR_PERF_BINARY=$PERF_PACKAGE_DIR/root/usr/bin/perf
export FNR_PERF_LIBS=$LIBPFM_PACKAGE_DIR/root/usr/lib:$LIBTRACEEVENT_PACKAGE_DIR/root/usr/lib
export PERF=$WORK/perf
export FNR_PERF_LOG=$WORK/perf-invocations.txt
cat > "$PERF" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
export LD_LIBRARY_PATH=$FNR_PERF_LIBS
printf '%s\t' "$FNR_PERF_BINARY" "$@" >> "$FNR_PERF_LOG"
printf '\n' >> "$FNR_PERF_LOG"
exec "$FNR_PERF_BINARY" "$@"
SH
chmod +x "$PERF"
LD_LIBRARY_PATH=$FNR_PERF_LIBS ldd "$FNR_PERF_BINARY" > "$WORK/perf-ldd.txt"
if grep -q 'not found' "$WORK/perf-ldd.txt"; then
  cat "$WORK/perf-ldd.txt" >&2
  echo 'Distribution perf has missing libraries; stop without fetching dependencies.' >&2
  exit 78
fi
mkdir -p "$WORK/out"
VARIANTS=${FNR_VARIANTS:-A,B,C}
IFS=',' read -r -a builds <<< "$VARIANTS"
for v in "${builds[@]}"; do
  case "$v" in A|B|C|D|E|Ma|Mb|Mc|Mall) ;; *) echo "Invalid variant: $v" >&2; exit 2;; esac
  mkdir -p "$WORK/out/stage-$v"
  cp -a "$INPUT/builds/$v/zc-game" "$WORK/out/stage-$v/zc-game"
  # The cross-host file copy can drop executable mode; change only our private copy.
  chmod u+x "$WORK/out/stage-$v/zc-game/horizon-client"
  cp "$INPUT/builds/$v/points.json" "$WORK/out/stage-$v/points.json"
done
# Check only Dyna CI containers, not the owner's other sessions or game clients.
# Exit 75 on a busy runner; the agent waits for that container to exit before retrying.
command -v podman >/dev/null
runners=$(podman ps --filter label=dyna.ci --format '{{.Names}}')
while IFS= read -r runner; do
  [ -z "$runner" ] && continue
  processes=$(podman top "$runner" args)
  if grep -q '[R]unner.Worker' <<< "$processes"; then
    printf 'CI_BUSY_CONTAINER=%s\n' "$runner" >&2
    exit 75
  fi
done <<< "$runners"
# AMD desktop: select exactly four physical cores with their online SMT siblings,
# leaving the other four physical cores free. Read topology, never assume CPU numbers.
BENCH_CPUS=$(python3 - <<'PY'
from pathlib import Path
def parse(text):
    out = set()
    for part in text.strip().split(','):
        bounds = list(map(int, part.split('-')))
        out.update(range(bounds[0], bounds[-1] + 1))
    return out
online = parse(Path('/sys/devices/system/cpu/online').read_text())
groups = {tuple(sorted(parse(p.read_text()) & online)) for p in Path('/sys/devices/system/cpu').glob('cpu[0-9]*/topology/thread_siblings_list')}
groups.discard(())
assert len(groups) == 8, f'Expected 8 physical cores, got {groups}'
selected = sorted(groups, key=min)[:4]
print(','.join(map(str, sorted({cpu for group in selected for cpu in group}))))
PY
)
export BENCH_CPUS
export OUT=$WORK/out HORIZON_RES=$WORK/resources
export TMPDIR=$WORK
export HORIZON_LAUNCHER_TRASH=$WORK/trash HORIZON_LAUNCHER_CLIPBOARD=$WORK/clipboard HORIZON_LAUNCHER_DESKTOP=$WORK/desktop
mkdir -p "$HORIZON_LAUNCHER_TRASH" "$HORIZON_LAUNCHER_CLIPBOARD" "$HORIZON_LAUNCHER_DESKTOP"
TAG=$(date -u +%Y%m%dT%H%M%SZ)
RESULT=$BASE/results/$TAG
mkdir -p "$RESULT"
{
  date -u; uname -a; lscpu; lscpu --extended
  echo "BENCH_CPUS=$BENCH_CPUS"; cat /proc/sys/kernel/perf_event_paranoid
  "$PERF" --version
  for package in "$PERF_PACKAGE_DIR" "$LIBPFM_PACKAGE_DIR" "$LIBTRACEEVENT_PACKAGE_DIR"; do
    grep -E '^(pkgname|pkgver|arch) = ' "$package/root/.PKGINFO"
    cat "$package/package.sha256" "$package/signature-check.txt"
  done
  cp "$WORK/perf-ldd.txt" "$RESULT/perf-ldd.txt"
  echo 'Kit reference machine: Ryzen 9 7900X; this run uses Ryzen 7 9800X3D, four physical cores and their SMT siblings.'
  echo 'Only tick campaign; no frame or hot-loop wall timing on a non-quiet host.'
} > "$RESULT/environment.txt"
taskset -c "$BENCH_CPUS" "$PERF" stat -e instructions:u -- true > "$RESULT/perf-probe.txt" 2>&1
if [ -n "${FNR_SAVE:-}" ]; then
  mkdir -p "$WORK/out/points-setup"
  cp -a "$FNR_SAVE" "$WORK/out/points-setup/game"
  printf 'Reused private fixed save: %s\n' "$FNR_SAVE" > "$RESULT/setup.log"
else
  python3 "$WORK/kit/points-tick/scene.py" setup "$WORK/out/stage-B/zc-game" "$WORK/out/points-setup" > "$RESULT/setup.log" 2>&1
fi
# Interleaved rotating order, five rounds, exactly 500 warm-up and 2,000 measured ticks.
# scene.py creates fresh scratch homes and copies the identical setup save every time.
export FNR_VARIANTS=$VARIANTS
python3 - "$WORK/kit/points-tick/scene.py" <<'PY' | tee "$RESULT/campaign.log"
import os, pathlib, subprocess, sys
scene=sys.argv[1]; out=pathlib.Path(os.environ['OUT']); names=os.environ['FNR_VARIANTS'].split(',')
for r in range(1,6):
    for v in names[r%len(names):]+names[:r%len(names)]:
        run=out/'runs'/'reproduced'/f'tick-{v}-{r}'
        print('round',r,'variant',v,flush=True)
        subprocess.run(['python3',scene,'tick',str(out/f'stage-{v}'/'zc-game'),str(out/'points-setup'/'game'),str(run),'500','2000'],check=True)
PY
cp "$FNR_PERF_LOG" "$RESULT/perf-invocations.txt"
python3 "$WORK/kit/points-tick/summary.py" reproduced > "$RESULT/summary.txt"
mkdir -p "$RESULT/census"
for v in "${builds[@]}"; do
  python3 "$WORK/kit/points-tick/census/census.py" "$WORK/out/stage-$v/zc-game/libhorizon_mod_host.so" "$RESULT/census/$v.json" >> "$RESULT/census/log.txt"
done
for v in "${builds[@]}"; do
  [ "$v" = A ] && continue
  (
    cd "$RESULT/census"
    python3 "$WORK/kit/points-tick/census/match.py" A.json "$v.json" "$WORK/out/stage-$v/points.json" > "match-$v.txt"
    mv kept_out_of_line.json "kept-out-of-line-$v.json"
  )
done
# Preserve raw perf CSV, tick times, setup/run logs and the common generated save for later variants.
# Build trees and copied resource trees are removed by the EXIT trap.
cp -a "$WORK/out/runs" "$RESULT/runs"
cp -a "$WORK/out/points-setup" "$RESULT/points-setup"
mkdir -p "$FNR_RESULTS"
if [ "$(realpath "$FNR_RESULTS")" != "$(dirname "$RESULT")" ]; then
  cp -a "$RESULT" "$FNR_RESULTS/"
fi
printf 'Results: %s\n' "$RESULT"
