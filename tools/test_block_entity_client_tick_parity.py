#!/usr/bin/env python3
"""Compare the client ticks of spawners, trial spawners, vaults, campfires, beacons, potent sulfur
and animated block entities with vanilla 26.3.

The Java oracle (VanillaBlockEntityClientTickOracle) runs their client tickers as
ClientLevel.tickBlockEntities does: every loaded one, each tick, in the level's ticker order,
sharing the level random. Columns arrive
as chunk packets carrying update tags, whose order the tickers follow (LevelChunk.replaceWithPacketData),
and level events 3011 to 3021 run through LevelEventHandler. The game test TEST runs the same cases on a
server's client: chunk packets (`remote_chunk`), forgotten columns, level events and
`tick_client_block_entities`. Each tick compares one ordered stream of particles and local sounds
(with each sound's seed), each block entity's spin and the level random's state, bit for bit.

Each tick also compares what it left of every beacon (levels, beam sections) and of the chest lids,
shulker lids, enchanting books, bells and conduits (`states`), with block events driving the lids and bells.

Cases: campfires, a conduit, a beacon and potent sulfur mixed with the rest in one packet order, through
`tick_client_block_entities` and through `tick_remote_block_entities`; a spawner at its required range's edge (inside, on, outside), with a dead or spectating
player; every trial spawner state, ominous or not, around a spawn and a 20-tick boundary; vaults
with and without a display item and a connected player around the particle range; each kind at
Chebyshev distance 63, 64 and 65 and far beyond, several sharing the level random, a mixed column
forgotten and sent again in another order, a shulker lid pushing a player out of spawner range
before or after the spawner and book tick, and level events 3011 to 3021.

docs/parity/block-entity-client-ticks.jsonl holds the cases and a hash of vanilla's log of each, which
the game test checks alone in the daily run; `--write` makes it again, and a run fails while it is
stale. Diagnostics go to .local/block-entity-client-tick-parity/report.txt.

  test_block_entity_client_tick_parity.py [--filter SUBSTRING] [--write]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base, test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity'
TEST = 'game::client_side::spawner_client_tick_tests::client_ticks_match_the_vanilla_cases'
COMMITTED = lab.ROOT / 'docs/parity/block-entity-client-ticks.jsonl'
NAME = 'block_entity_client_tick'
TRIAL_STATES = ['inactive', 'waiting_for_players', 'active', 'waiting_for_reward_ejection', 'ejecting_reward', 'cooldown']
FACINGS = ['north', 'south', 'west', 'east']


def player(x, y, z, dead=False, spectator=False):
    return {'pos': [x, y, z], 'dead': dead, 'spectator': spectator}


def spawner(pos, range_=16, delay=20):
    return {'kind': 'spawner', 'pos': pos, 'range': range_, 'delay': delay}


def trial(pos, state, ominous=False, next_mob=0):
    return {'kind': 'trial_spawner', 'pos': pos, 'state': state, 'ominous': ominous, 'nextMobSpawnsAt': next_mob}


def vault(pos, facing='north', ominous=False, display=True, connected=True, state='active', particles_range=4.5):
    return {'kind': 'vault', 'pos': pos, 'facing': facing, 'ominous': ominous, 'display': display,
            'connected': connected, 'state': state, 'particlesRange': particles_range}


def cases():
    out = []

    def add(name, entities, who, ticks=24, game_time=1000, events=None, fills=None, path=None):
        case = {'name': name, 'seed': 0x5EED0000 + len(out), 'gameTime': game_time, 'ticks': ticks,
                'player': who, 'entities': entities}
        if events:
            case['events'] = events
        if fills:
            case['fills'] = fills
        if path:
            case['path'] = path
        out.append(case)

    # BaseSpawner.isNearPlayer: strictly closer than the range to the block's centre.
    for dx in (15.5, 16.0, 16.5, -15.25):
        add(f'spawner range {dx}', [spawner([8, 64, 8])], player(8.5 + dx, 64.5, 8.5))
    add('spawner diagonal', [spawner([8, 64, 8], range_=6)], player(8.5 + 4.2, 64.5, 8.5 - 4.2))
    add('spawner dead player', [spawner([8, 64, 8])], player(9.5, 64.0, 8.5, dead=True))
    add('spawner spectator', [spawner([8, 64, 8])], player(9.5, 64.0, 8.5, spectator=True))
    add('spawner counts down', [spawner([8, 64, 8], delay=3)], player(9.5, 64.0, 8.5), ticks=8)
    add('spawner any range', [spawner([8, 64, 8], range_=-1)], player(500.5, 64.0, 8.5))
    for state in TRIAL_STATES:
        for ominous in (False, True):
            add(f'trial {state} {ominous}', [trial([8, 64, 8], state, ominous, next_mob=1010)], player(9.5, 64.0, 8.5),
                ticks=24, game_time=995)
    for facing in FACINGS:
        add(f'vault {facing}', [vault([8, 64, 8], facing)], player(10.5, 64.0, 9.5), game_time=1000)
    add('vault ominous inactive', [vault([8, 64, 8], ominous=True, state='inactive', display=False)], player(9.5, 64.0, 8.5))
    # VaultBlockEntity.Client.isWithinConnectionRange: block positions, <= the range.
    for dx in (4, 5, -3):
        add(f'vault connection {dx}', [vault([8, 64, 8])], player(8.5 + dx, 64.0, 8.5), game_time=1000)
    add('vault not connected', [vault([8, 64, 8], connected=False)], player(9.5, 64.0, 8.5))
    # ClientLevel ticks every loaded block entity, however far from the player (the storage around
    # the view centre still holds their columns).
    far = [trial([8 + 70, 64, 8], 'cooldown'), vault([8 - 130, 64, 8 + 150]), trial([8, 64, 8 + 200], 'active')]
    add('far block entities', far, player(8.5, 64.0, 8.5), game_time=1000)
    # Each kind at Chebyshev distance 63, 64 and 65 from the player's block, along x, z and y, in
    # three columns: none of them stops at 64.
    for kind, make in (('spawner', lambda p: spawner(p, range_=-1)), ('trial', lambda p: trial(p, 'active')),
                       ('vault', vault)):
        edge = [make(p) for d in (63, 64, 65) for p in ([8 + d, 64, 8], [8, 64, 8 - d], [8, 64 + d, 8])]
        add(f'{kind} chebyshev 63 64 65', edge, player(8.5, 64.0, 8.5), game_time=1000)
    # Several draw from the one level random, in the level's ticker order.
    many = [trial([1, 64, 1], 'active'), vault([3, 64, 1]), spawner([5, 64, 1]), trial([1, 64, 5], 'waiting_for_players'),
            vault([20, 64, 3], 'east', ominous=True), trial([3, 64, 12], 'cooldown', True), spawner([12, 64, 12])]
    add('shared random', many, player(6.5, 64.0, 4.5), game_time=1000)
    # A mixed column the server forgets and sends again, then sends again while it is loaded, each
    # packet listing its block entities in another order: the tickers follow the packet, not the
    # column's map. Column (1, 0) goes and comes back within one tick.
    mixed = many + [spawner([9, 70, 14]), vault([14, 66, 2], 'south'), trial([18, 64, 2], 'active'),
                    vault([30, 64, 9], 'west'), spawner([17, 64, 14])]
    first = [i for i, e in enumerate(mixed) if e['pos'][0] < 16]
    second = [i for i, e in enumerate(mixed) if e['pos'][0] >= 16]
    add('chunk reload', mixed, player(6.5, 64.0, 4.5), ticks=30, game_time=990,
        events={'6': [['forget', 0, 0]], '9': [['send', 0, 0, first[::-1]]],
                '14': [['send', 0, 0, first[2:] + first[:2]]],
                '20': [['forget', 1, 0], ['send', 1, 0, second[::-1]]]})
    # LevelEventHandler.levelEvent 3011 to 3021 draws from the same level random as the ticks, a
    # sound's seed included. Within 10 blocks of the player: a farther sound's distance delay is not
    # compared here.
    at, vault_at = [10, 64, 8], [8, 64, 8]
    level_events = [[3011, at, 0], [3012, at, 1], [3013, at, 2], [3014, at, 0], [3015, vault_at, 0],
                    [3016, vault_at, 1], [3017, at, 0], [3018, at, 0], [3019, at, 3], [3020, at, 0],
                    [3020, at, 1], [3021, at, 0], [3015, vault_at, 1], [3012, at, 0]]
    add('level events', [vault(vault_at), trial(at, 'active')], player(9.5, 64.0, 10.5), ticks=len(level_events) + 2,
        game_time=1000, events={str(t): [['event', *e]] for t, e in enumerate(level_events)})
    # Campfires, a conduit, a beacon and potent sulfur in one ticker list with spawners, a vault and a
    # trial spawner: vanilla ticks them all from Level.blockEntityTickers in packet order, so their
    # particles, sounds and level random draws interleave. The conduit's shape (water around it and
    # prismarine on its frame) and a geyser's water column are the level's blocks. `path: remote`
    # runs the client's whole block entity phase (tick_remote_block_entities), not only its list.
    water = {'from': [5, 61, 5], 'to': [11, 67, 11], 'block': 'minecraft:water'}
    # ConduitBlockEntity.updateShape's frame positions.
    ring = [[ox, oy, oz] for ox in range(-2, 3) for oy in range(-2, 3) for oz in range(-2, 3)
            if (abs(ox) > 1 or abs(oy) > 1 or abs(oz) > 1)
            and (ox == 0 and 2 in (abs(oy), abs(oz)) or oy == 0 and 2 in (abs(ox), abs(oz))
                 or oz == 0 and 2 in (abs(ox), abs(oy)))]
    assert len(ring) == 42, len(ring)

    def frame(n, block='minecraft:prismarine'):
        return [{'from': [8 + dx, 64 + dy, 8 + dz], 'to': [8 + dx, 64 + dy, 8 + dz], 'block': block}
                for dx, dy, dz in ring[:n]]

    def campfire(pos, items, lit=True, signal=False, facing='north'):
        return {'kind': 'campfire', 'pos': pos, 'items': items, 'lit': lit, 'signal': signal, 'facing': facing}

    def sulfur(pos, state):
        return {'kind': 'potent_sulfur', 'pos': pos, 'state': state}

    column_water = [{'from': [x, 65, z], 'to': [x, 67, z], 'block': 'minecraft:water'} for x, z in ((13, 13), (3, 13))]
    who = player(6.5, 64.0, 2.5)
    for n, label in ((20, 'active'), (42, 'hunting')):
        mix = [campfire([1, 64, 1], [True, False, True, False]), spawner([5, 64, 1]),
               {'kind': 'conduit', 'pos': [8, 64, 8]}, vault([3, 64, 1]), sulfur([13, 64, 13], 'erupting'),
               {'kind': 'beacon', 'pos': [14, 64, 2]}, trial([1, 64, 5], 'waiting_for_players'),
               campfire([14, 64, 6], [False, True, True, True], signal=True, facing='east'),
               campfire([12, 64, 1], [True, True, False, False], lit=False), sulfur([3, 64, 13], 'wet'),
               campfire([2, 64, 10], [False, False, False, True], facing='west'), sulfur([14, 64, 10], 'dormant')]
        fills = [water, *frame(n), *column_water, {'from': [14, 65, 10], 'to': [14, 66, 10], 'block': 'minecraft:water'}]
        for path in (None, 'remote'):
            add(f'mixed list {label} {path or "list"}', mix, who, ticks=45, game_time=1000, fills=fills, path=path)
    add('campfires lit and unlit', [campfire([1, 64, 1], [True] * 4), campfire([3, 64, 1], [True] * 4, lit=False),
                                    campfire([5, 64, 1], [True, False, True, False], signal=True, facing='south'),
                                    spawner([7, 64, 1], range_=-1), campfire([9, 64, 1], [False] * 4)],
        who, ticks=30, path='remote')
    add('continuous geyser', [sulfur([13, 64, 13], 'continuous'), spawner([5, 64, 1], range_=-1)], who, ticks=50,
        fills=column_water[:1], path='remote')
    # The animated block entities the client's list took over (chest lids, shulker lids, enchanting
    # books, bells), a beacon's beam and a conduit's turn: their state after each tick is compared
    # as well (`states`), with block events (`blockEvent`) where vanilla has them.
    def at(kind, pos, **fields):
        return {'kind': kind, 'pos': pos, **fields}

    def open_close(pos, opens, closes):
        return {str(t): [['blockEvent', pos, 1, n]] for t, n in ((opens, 1), (closes, 0))}

    add('chest lid', [at('chest', [8, 64, 8], facing='north')], who, ticks=30, path='remote',
        events=open_close([8, 64, 8], 2, 14))
    add('shulker lid', [at('shulker', [8, 64, 8], facing='up')], player(20.5, 64.0, 8.5), ticks=40, path='remote',
        events=open_close([8, 64, 8], 1, 20))
    add('shulker lid east', [at('shulker', [8, 64, 8], facing='east')], player(20.5, 64.0, 8.5), ticks=40,
        path='remote', events=open_close([8, 64, 8], 3, 25))
    add('book near the player', [at('enchanting_table', [8, 64, 8])], player(9.5, 64.0, 9.5), ticks=40, path='remote')
    add('book far from the player', [at('enchanting_table', [8, 64, 8])], player(30.5, 64.0, 8.5), ticks=40,
        path='remote')
    add('bell rung', [at('bell', [8, 64, 8], facing='north')], who, ticks=60, path='remote',
        events={'2': [['blockEvent', [8, 64, 8], 1, 2]], '40': [['blockEvent', [8, 64, 8], 1, 4]]})
    base = {'from': [7, 63, 7], 'to': [9, 63, 9], 'block': 'minecraft:iron_block'}
    glass = {'from': [8, 66, 8], 'to': [8, 66, 8], 'block': 'minecraft:red_stained_glass'}
    more = {'from': [8, 67, 8], 'to': [8, 67, 8], 'block': 'minecraft:blue_stained_glass'}
    add('beacon beam', [at('beacon', [8, 64, 8])], who, ticks=130, fills=[base, glass, more], path='remote')
    add('beacon without a base', [at('beacon', [8, 64, 8])], who, ticks=100, fills=[glass], path='remote')
    # Lids, books, bells and a beam in the ticker list with the rest, their block events between ticks.
    crowd = [at('chest', [1, 64, 1], facing='south'), spawner([5, 64, 1], range_=-1),
             at('shulker', [3, 64, 1], facing='up'), at('enchanting_table', [3, 64, 4]),
             at('bell', [14, 64, 2], facing='east'), campfire([12, 64, 1], [True, False, True, True]),
             at('beacon', [14, 64, 6]), {'kind': 'conduit', 'pos': [8, 64, 8]}, vault([1, 64, 9]),
             at('chest', [1, 64, 12], facing='west')]
    add('animated block entities mixed', crowd, player(4.5, 64.0, 3.5), ticks=70, game_time=1000, path='remote',
        fills=[water, *frame(42), base | {'from': [13, 63, 5], 'to': [15, 63, 7]}, glass | {'from': [14, 66, 6], 'to': [14, 66, 6]}],
        events={'3': [['blockEvent', [1, 64, 1], 1, 1]], '4': [['blockEvent', [3, 64, 1], 1, 1]],
                '6': [['blockEvent', [14, 64, 2], 1, 3]], '9': [['blockEvent', [1, 64, 12], 1, 2]],
                '30': [['blockEvent', [3, 64, 1], 1, 0]], '33': [['blockEvent', [1, 64, 1], 1, 0]]})
    # ShulkerBoxBlockEntity.moveCollidedEntities pushes the player east out of BaseSpawner's
    # required range on its first tick. Compare the spawner on both sides of that push, and the
    # book's target angle (EnchantingTableBlockEntity.bookAnimationTick) at the new position.
    lid = at('shulker', [8, 64, 8], facing='east')
    book = at('enchanting_table', [9, 64, 10])
    nearby = spawner([7, 64, 8], range_=2)
    push = {'0': [['blockEvent', [8, 64, 8], 1, 1]]}
    for label, entities in (('before', [lid, nearby, book]), ('after', [nearby, book, lid])):
        add(f'shulker pushes {label} spawner ticks', entities, player(9.3, 64.0, 9.15), ticks=1,
            events=push, path='remote')
    return out


def canonical(log):
    """A log as the game test hashes its own: compact, keys sorted."""
    return json.dumps(log, separators=(',', ':'), sort_keys=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    parser.add_argument('--write', action='store_true', help=f'write {COMMITTED.relative_to(lab.ROOT)} again')
    parser.add_argument('--resources', type=Path, default=Path(os.environ.get('SUNRISE_TEST_RESOURCES', lab.RESOURCES)))
    args = parser.parse_args()
    chosen = [c for c in cases() if args.filter in c['name']]
    if not chosen:
        sys.exit(f'{NAME}: --filter {args.filter!r} selects no case')
    if args.write and args.filter:
        sys.exit(f'{NAME}: --write takes every case, not a filter')
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'block-entity-client-tick-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.txt'
    report.write_text('')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))

    def run(command, name, cwd=lab.ROOT, env=None):
        path = build / (name + '.log')
        with path.open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=cwd, env=env, timeout=3600, stdout=log,
                                    stderr=subprocess.STDOUT)
        if result.returncode:
            sys.exit(f'{NAME}: {name} failed (log: {path})')
        return path

    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', dependencies, '-d', classes,
         PARITY / 'ParityData.java', PARITY / 'VanillaBlockEntityClientTickOracle.java'], 'compile-java')
    run(['java', '-Xmx1G', '-Djava.awt.headless=true', '-cp', str(classes) + os.pathsep + dependencies,
         'dev.sunriselab.parity.VanillaBlockEntityClientTickOracle', inputs, build / 'vanilla.jsonl'], 'vanilla')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(chosen), (len(vanilla), len(chosen))
    notes = []
    for case, v in zip(chosen, vanilla):
        if 'error' in v:
            notes.append(f'ORACLE ERROR {case["name"]}: {v["error"]}')
    if notes:
        report.write_text('\n'.join(notes) + '\n')
        sys.exit(f'{NAME}: the oracle failed on {len(notes)} cases (report: {report})')
    rows = [{'case': c, 'vanilla': hashlib.sha256(canonical(v).encode()).hexdigest()} for c, v in zip(chosen, vanilla)]
    if args.write:
        COMMITTED.write_text(''.join(json.dumps(r, separators=(',', ':')) + '\n' for r in rows))
    with tempfile.TemporaryDirectory(prefix='sunrise-client-tick-') as tmp:
        scratch = Path(tmp)
        homes = {k: scratch / k for k in ('SUNRISE_HOME', 'SUNRISE_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')}
        env = lab.game_env(os.environ, **homes, RUST_MIN_STACK='16777216')
        # The shared vanilla build (no Extras, no controllers), as the other harnesses of this test
        # binary take it; the parity workflow warms this feature variant before running the harnesses.
        binaries = [lab.parity_fixtures(default_features=False)]
        env['SUNRISE_TEST_RESOURCES'] = str(args.resources.resolve())
        env['SUNRISE_CLIENT_TICK_CASES'] = str(inputs)
        env['SUNRISE_CLIENT_TICK_OUT'] = str(build / 'rust.jsonl')
        log = run([binaries[0], TEST, '--exact', '--ignored', '--nocapture'], 'game', cwd=scratch, env=env)
        assert '1 passed; 0 failed' in log.read_text(encoding='utf-8'), 'the game test did not run'
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(rust) == len(chosen), (len(rust), len(chosen))
    failed = 0
    for case, v, r in zip(chosen, vanilla, rust):
        where = first_difference(v['ticks'], r['ticks'])
        if where:
            notes.append(f'MISMATCH {case["name"]}: {where}')
            failed += 1
    if not args.filter and not args.write:
        committed = [json.loads(l) for l in COMMITTED.read_text().splitlines()] if COMMITTED.exists() else []
        if committed != rows:
            notes.append(f'{COMMITTED.relative_to(lab.ROOT)} is not these cases and vanilla\'s logs: run with --write')
            failed += 1
    report.write_text(''.join(n + '\n' for n in notes) + f'{len(chosen) - failed} of {len(chosen)} cases match\n')
    if failed:
        sys.exit(f'{NAME}: FAILED, {failed} of {len(chosen)} cases differ (report: {report})')
    print(f'{NAME}: ok')


def first_difference(vanilla, rust):
    """Where two logs first part: the tick and field, and the entry, never vanilla's values."""
    for t, (vt, rt) in enumerate(zip(vanilla, rust)):
        for field in ('events', 'spins', 'states', 'random'):
            if vt[field] == rt[field]:
                continue
            if field == 'random':
                return f'tick {t} leaves the level random elsewhere'
            at = next((i for i, (a, b) in enumerate(zip(vt[field], rt[field])) if a != b),
                      min(len(vt[field]), len(rt[field])))
            return f'tick {t} {field} differ from entry {at} ({len(vt[field])} vanilla, {len(rt[field])} Sunrise)'
    if len(vanilla) != len(rust):
        return f'{len(vanilla)} vanilla ticks, {len(rust)} Sunrise ticks'
    return None


if __name__ == '__main__':
    main()
