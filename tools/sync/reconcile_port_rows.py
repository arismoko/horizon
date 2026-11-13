#!/usr/bin/env python3
"""Merge sync 27 ledger cells, retaining fork relocations on overlapping edits."""
import pathlib
import subprocess
import re

symbols = {}
for file in pathlib.Path('crates').glob('**/*.rs'):
    if 'target' in file.parts:
        continue
    for name in re.findall(r'\bfn\s+(\w+)', file.read_text()):
        symbols.setdefault(name, set()).add(file)

BASE = '7b42678bcd'
FORK = '600782248b'
UPSTREAM = '501446443c'
SCAFFOLD = 'b8881bd88e'
FILES = '''animal-1 classes-01 client-001 client-002 client-003 client-004
client-005 client-008 client2-015 client2-016 game-003 game-005 game2-001
game2-004 game2-007 game2-009 game2-014 game2-017 gui-inventory-1 rest-004'''.split()

def rows(ref, path):
    data = subprocess.check_output(['git', 'show', f'{ref}:{path}'], text=True)
    return {line.split('\t')[0]: line.split('\t') for line in data.splitlines()}

for name in FILES:
    path = pathlib.Path('tools/port_map') / (name + '.tsv')
    base, fork, upstream = (rows(ref, path) for ref in (BASE, FORK, UPSTREAM))
    original = subprocess.check_output(['git', 'show', f'{SCAFFOLD}:{path}'], text=True)
    raw = {line.split('\t')[0]: line for line in original.splitlines()}
    current = {key: line.split('\t') for key, line in raw.items()}
    for key, incoming in upstream.items():
        old = base.get(key)
        if incoming == old:
            continue
        own = fork.get(key)
        if own is None or own == old:
            current[key] = incoming
        elif old is not None and len(old) == len(own) == len(incoming):
            current[key] = [u if f == b else f for b, f, u in zip(old, own, incoming)]
    # Overlapping source-cell edits need semantic relocation decisions: the
    # minecart NBT methods remain in the fork's minecarts.rs; these others moved.
    relocations = {
        'ClientLevel.removeEntity': 'crates/horizon-client/src/game/network/remote_removal.rs::Game::remove_entity',
        'ClientLevel.EntityCallbacks.onTrackingEnd': 'crates/horizon-client/src/game/network/remote_removal.rs::Game::remove_entity',
        'ClientPacketListener.handleRemoveEntities': 'crates/horizon-client/src/game/network/remote_removal.rs::Game::remove_entity',
        'WorldOptionsScreen.updatePortControlsState': 'crates/horizon-client/src/game/network/lan_screen.rs::WorldOptions::update_port_controls',
        'Piglin.holdInOffHand': 'crates/horizon-client/src/mobs/piglin.rs::hold_in_offhand',
        'AnvilScreen.subInit': 'crates/horizon-client/src/game/blocks/stations/anvil.rs::rename_box',
    }
    for key, target in relocations.items():
        if key in current:
            current[key][2] = target
    # Resolve moved source files by unique basename, never invent a target.
    for row in current.values():
        if len(row) < 3 or not row[2]:
            continue
        source, sep, symbol = row[2].partition('::')
        if source.endswith('.rs') and not pathlib.Path(source).exists():
            matches = list(pathlib.Path('crates').glob('**/' + pathlib.Path(source).name))
            if len(matches) == 1:
                row[2] = str(matches[0]) + sep + symbol
            else:
                candidates = symbols.get(symbol.split('::')[-1], set())
                if len(candidates) == 1:
                    row[2] = str(next(iter(candidates))) + sep + symbol
                elif row[0] in upstream and upstream[row[0]] != base.get(row[0]):
                    print(f'Unresolved incoming path: {path}: {row[0]}: {row[2]}')
    path.write_text('\n'.join(
        raw[key] if raw.get(key, '').split('\t') == row
        else '\t'.join(row).rstrip()
        for key, row in current.items()
    ) + '\n')
