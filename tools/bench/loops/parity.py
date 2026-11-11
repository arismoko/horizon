#!/usr/bin/env python3
"""Fixed-tick full-state parity for published loops (bench only).

  parity.py run GAME SAVE OUT TICKS   # copy SAVE, freeze the server before it ticks, step exactly TICKS, save, stop
  parity.py compare A B               # every decoded chunk, entity and POI record and level.dat of two runs

The server freezes on its first console line (`tick freeze`, written before it starts), so a run ticks the
save exactly TICKS times (`tick step`) before `save-all flush`. Needs HORIZON_RES and the points-tick kit
(KIT, default /box/shared/root/bench-kit/horizon-bench-kit) for its scene helpers.
"""
import os, shutil, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
KIT = Path(os.environ.get('KIT', '/box/shared/root/bench-kit/horizon-bench-kit'))
sys.path.insert(0, str(KIT / 'points-tick'))
sys.path.insert(0, str(KIT / 'common'))
sys.path.insert(0, str(ROOT / 'tools'))
import kitenv  # noqa: E402
import scene  # noqa: E402
import anvil_dump  # noqa: E402

# Wall-clock stamps, not simulation state.
VOLATILE = {'LastPlayed', 'LastUpdate'}


def run(game, save: Path, out: Path, ticks: int):
    out.mkdir(parents=True)
    env = kitenv.scratch_env()
    game_dir = out / 'game'
    shutil.copytree(save, game_dir)
    scene.properties(game_dir, 25613)
    proc = scene.server(game, game_dir, env)

    def say(line):
        proc.stdin.write(line + '\n')
        proc.stdin.flush()

    say('tick freeze')
    log = game_dir / 'server.log'
    scene.wait_for(log, 'Done (', 600, proc)
    time.sleep(2)
    say('tick query')
    say(f'tick step {ticks}')
    time.sleep(ticks / 20 + 15)
    say('tick query')
    say('save-all flush')
    time.sleep(10)
    say('stop')
    proc.wait(timeout=300)
    print(f'{out}: exit {proc.returncode}')


def strip(value):
    if isinstance(value, dict):
        return {k: strip(v) for k, v in value.items() if k not in VOLATILE}
    if isinstance(value, list):
        return [strip(v) for v in value]
    if isinstance(value, tuple):
        return tuple(strip(v) for v in value)
    return value


def records(world: Path):
    found = {}
    for path in sorted(world.rglob('*')):
        rel = path.relative_to(world).as_posix()
        if path.suffix == '.mca' and path.stat().st_size:
            for pos, chunk in anvil_dump.region_chunks(path).items():
                chunk = strip(chunk)
                if '/entities/' in f'/{rel}':
                    # Mobs wander differently from run to run of one build (two runs of the same
                    # executable differ), so entities count by type and UUID only.
                    for entity in chunk.get('Entities', []):
                        found.setdefault('entities', set()).add((entity.get('id'), repr(entity.get('UUID'))))
                    continue
                # Block entities are saved in no fixed order either.
                if isinstance(chunk.get('block_entities'), list):
                    chunk['block_entities'].sort(key=lambda e: (e.get('x'), e.get('y'), e.get('z')))
                found[f'{rel} {pos}'] = chunk
        elif path.suffix == '.dat' and path.name != 'session.lock':
            found[rel] = strip(anvil_dump.read_dat(path))
    return found


def diff(a, b, at=''):
    if type(a) is not type(b):
        return [f'{at}: {a!r:.80} != {b!r:.80}']
    if isinstance(a, dict):
        out = []
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                out.append(f'{at}/{k}: only in {"B" if k not in a else "A"}')
            else:
                out += diff(a[k], b[k], f'{at}/{k}')
        return out
    if isinstance(a, set):
        return [f'{at}: {len(a - b)} only in A, {len(b - a)} only in B'] if a != b else []
    if isinstance(a, (list, tuple)) and len(a) == len(b):
        return [d for i, (x, y) in enumerate(zip(a, b)) for d in diff(x, y, f'{at}[{i}]')]
    return [] if a == b else [f'{at}: {a!r:.80} != {b!r:.80}']


def compare(a: Path, b: Path):
    ra, rb = records(a / 'game' / 'world'), records(b / 'game' / 'world')
    differences = []
    for key in sorted(set(ra) | set(rb)):
        if key not in ra or key not in rb:
            differences.append(f'{key}: only in {"B" if key not in ra else "A"}')
        else:
            differences += [f'{key}{d}' for d in diff(ra[key], rb[key])]
    print(f'{len(ra)} records in A, {len(rb)} in B, {len(differences)} differences')
    for line in differences[:200]:
        print(line)
    return 1 if differences else 0


if __name__ == '__main__':
    what = sys.argv[1]
    if what == 'run':
        run(str(Path(sys.argv[2]).resolve()), Path(sys.argv[3]).resolve(), Path(sys.argv[4]).resolve(), int(sys.argv[5]))
    elif what == 'compare':
        sys.exit(compare(Path(sys.argv[2]), Path(sys.argv[3])))
    else:
        sys.exit(__doc__)
