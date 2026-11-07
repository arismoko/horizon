#!/usr/bin/env python3
"""Apply rustc's ItemStack count/damage getter suggestions, recording every edit.

Usage: cargo test ... --message-format=json > target/stack-diagnostics.json
       apply_field_reads.py target/stack-diagnostics.json EDIT_LOG.json
Only E0616 diagnostics naming this owned stack are eligible. Item identities,
assignments without a rustc getter suggestion, and generated files are not
rewritten. Parts item tests are deferred until the registrar split is merged.
"""
import json
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[3]
edits = {}
for line in Path(sys.argv[1]).read_text().splitlines():
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    if message.get('reason') != 'compiler-message':
        continue
    diagnostic = message['message']
    if (diagnostic.get('code') or {}).get('code') != 'E0616':
        continue
    text = diagnostic['message']
    if "item_stack::ItemStack" not in text or not any(f"field `{field}`" in text for field in ('count', 'damage')):
        continue
    for child in diagnostic['children']:
        for span in child['spans']:
            if span.get('suggested_replacement') != '()':
                continue
            path = Path(span['file_name']).resolve()
            if not path.is_relative_to(root / 'crates') or path.name == 'items.rs':
                continue
            edits.setdefault(path, set()).add((span['byte_start'], span['byte_end'], '()'))
log = []
for path, changes in sorted(edits.items()):
    content = path.read_bytes()
    for start, end, replacement in sorted(changes, reverse=True):
        log.append({'path': str(path.relative_to(root)), 'start': start, 'end': end,
                    'old': content[start:end].decode(), 'new': replacement})
        content = content[:start] + replacement.encode() + content[end:]
    path.write_bytes(content)
Path(sys.argv[2]).write_text(json.dumps(log, indent=2) + '\n')
print(f'Applied {len(log)} compiler-suggested getter edits')
