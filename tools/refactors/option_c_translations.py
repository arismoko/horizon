#!/usr/bin/env python3
"""Restore retained API point translations from the option C base."""
import json
from pathlib import Path
import subprocess
source = '\n'.join(p.read_text() for p in Path('launcher/src').rglob('*.rs'))
for path in Path('launcher/lang').glob('*.json'):
    original = subprocess.check_output(['git', 'show', f'90db1b460b:{path}'], text=True)
    current = json.loads(subprocess.check_output(['git', 'show', f'HEAD:{path}'], text=True))
    base = json.loads(original)
    keep = {key for key in base if key in current or key in source}
    lines = []
    for line in original.splitlines()[1:-1]:
        key = json.loads('{' + line.rstrip(',') + '}')
        if next(iter(key)) in keep:
            lines.append(line.rstrip(','))
    path.write_text('{\n' + ',\n'.join(lines) + '\n}\n')
