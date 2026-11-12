#!/usr/bin/env python3
"""Carry the no-rebuild launcher baseline, retaining API point implementation files."""
import subprocess
from pathlib import Path

REF = 'origin/no-points'
KEEP = {'api.rs', 'inject.rs', 'mod_points.rs', 'internals.rs', 'prebuilt.rs', 'planning.rs', 'registry_compile.rs', 'slots.rs'}
for line in subprocess.check_output(['git', 'diff', '--name-status', '90db1b460b', REF, '--', 'launcher/'], text=True).splitlines():
    status, name = line.split('\t')
    path = Path(name)
    if path.parent == Path('launcher/src/native_mods') and path.name in KEEP:
        continue
    if status == 'D':
        path.unlink(missing_ok=True)
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(subprocess.check_output(['git', 'show', f'{REF}:{name}']))
