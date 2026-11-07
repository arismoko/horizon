#!/usr/bin/env python3
"""Give stack-taking host writes an explicit typed conversion refusal.

The draw file's stack methods belong to DrawContext, not a generated host area.
Existing typed Results retain their error type and implement From<StackWriteError>.
All other mutable host methods taking ItemStack gain Result<Old, StackWriteError>.
This script emits the exact signature edit log, including unchanged Result methods.
"""
import json
import re
from pathlib import Path

root = Path(__file__).resolve().parents[3]
log = []
pattern = re.compile(r'fn\s+(\w+)\s*\((.*?)\)\s*(?:->\s*([^;{]+))?[;{]', re.S)
for path in sorted((root / 'crates/horizon-mod-api/src/host').glob('*.rs')):
    if path.name == 'draw.rs':
        continue
    source = path.read_text()
    edits = []
    for match in pattern.finditer(source):
        if 'ItemStack' not in match[2] or '&mut self' not in match[2]:
            continue
        old = match[0]
        result = (match[3] or '()').strip()
        if result.startswith('Result<'):
            continue
        new = f'fn {match[1]}({match[2]}) -> Result<{result}, StackWriteError>{old[-1]}'
        edits.append((match.start(), match.end(), new))
        log.append({'path': str(path.relative_to(root)), 'method': match[1], 'old': old, 'new': new})
    for start, end, new in reversed(edits):
        source = source[:start] + new + source[end:]
    if edits:
        path.write_text(source)
output = root / 'tools/refactors/api-pr3a/stack-write-results.json'
output.write_text(json.dumps(log, indent=2) + '\n')
print(f'Changed {len(log)} stack-taking write signatures')
