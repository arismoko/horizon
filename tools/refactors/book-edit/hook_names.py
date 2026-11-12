#!/usr/bin/env python3
"""Use public hook names in Rust doc comments and catalog notes, never code."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SOURCES = [ROOT / 'crates' / name / 'src' for name in
           ('horizon-mod-vocab', 'horizon-mod-api', 'horizon-mod-runtime')]
# Scoped Part declarations use the same zero-argument catalog syntax as Hooks.
hooks = set()
for path in (SOURCES[0] / 'hooks').rglob('*.rs'):
    hooks.update(re.findall(r'\bfn\s+(\w+)\s*\(\s*\)\s*;', path.read_text()))
region = re.compile(r'(?m)^\s*//[/!][^\n]*|#\[notes\s*=\s*(?:r(?P<hashes>#+)?"[\s\S]*?"(?P=hashes)|"(?:\\.|[^"\\])*")\s*\]')
name = re.compile(r'\bnative_(\w+)\b')
replaced = 0
left = []
calls = []
for root in SOURCES:
    for path in sorted(root.rglob('*.rs')):
        original = path.read_text()
        def rewrite(match):
            def rename(hit):
                global replaced
                line = original.count('\n', 0, match.start() + hit.start()) + 1
                location = f'{path.relative_to(ROOT)}:{line}'
                if hit[1] not in hooks:
                    left.append(f'{location}: {hit[0]}')
                    return hit[0]
                if re.match(r'\s*\(', match[0][hit.end():]):
                    calls.append(f'{location}: {match[0][hit.start():]}')
                replaced += 1
                return hit[1]
            return name.sub(rename, match[0])
        updated = region.sub(rewrite, original)
        if updated != original:
            path.write_text(updated)
print(f'Replaced {replaced}; left {len(left)}; call forms {len(calls)}')
print('LEFT:\n' + '\n'.join(left))
print('CALL FORMS:\n' + '\n'.join(calls))
