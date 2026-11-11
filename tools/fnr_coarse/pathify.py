#!/usr/bin/env python3
"""Turn the generator's single-function copies into one path copy (fnr-coarse prototype).
pathify.py LIB.rs INSTALLS.txt ENTRIES.txt OUT_INSTALLS.txt
- every copied method is renamed `__fnr_path_<name>` in its helper trait, so calls between copies
  can name the copy (an inherent method would otherwise win over the helper trait's);
- calls to a copied method inside the copies (`.name(`, `Self::name(`) are rewritten to the copy;
- free functions need nothing: the mirror module's own copy shadows the game's re-export;
- only the entries are installed (through their own checks); the rest of the path is reached only
  from inside the copies."""
import re, sys
lib_path, installs_path, entries_path, out_installs = sys.argv[1:5]
text = open(lib_path).read()
rows = [l.split('\t') for l in open(installs_path).read().splitlines() if l]
entries = {l.strip() for l in open(entries_path) if l.strip()}
methods = set()
for target, ident, install in rows:
    m = re.search(r'as (__Fnr_\w+?)(?:<[^>]*>)?>::(\w+)$', ident)
    if m and not m.group(2).startswith('__fnr_'):
        methods.add((m.group(1), m.group(2)))
for trait, name in sorted(methods):
    new = f'__fnr_path_{name}'
    # trait declaration and impl inside `trait <trait>` / `impl <trait> for`
    text = re.sub(rf'(pub trait {trait}\b[^{{]*\{{\s*fn ){name}\b', rf'\g<1>{new}', text)
    m = re.search(rf'impl(?:<[^>]*>)? {trait}\b', text)
    f = re.compile(rf'\bfn {name}\b').search(text, m.end())
    text = text[:f.start()] + f'fn {new}' + text[f.end():]
    text = re.sub(rf'(as {trait}(?:<[^>]*>)?\s*>::){name}\b', rf'\g<1>{new}', text)
calls = 0
for _, name in sorted(methods):
    new = f'__fnr_path_{name}'
    text, a = re.subn(rf'\.{name}\s*\(', f'.{new}(', text)
    text, b = re.subn(rf'\bSelf::{name}\s*\(', f'Self::{new}(', text)
    calls += a + b
for target, ident, install in rows:
    if target in entries:
        continue
    fn = install.rsplit('::', 1)[1].rstrip('()')
    m = re.search(rf'#\[doc\(hidden\)\]\s*pub fn {fn}\(\) -> bool \{{', text)
    d, i = 1, m.end()
    while d:
        d += {'{': 1, '}': -1}.get(text[i], 0)
        i += 1
    text = text[:m.start()] + text[i:]
uses = []
for target, ident, install in rows:
    m = re.match(r'horizon_patch((?:::\w+)*)::<.* as (__Fnr_\w+?)(?:<[^>]*>)?>::', ident)
    if m:
        uses.append(f'pub use crate{m.group(1)}::{m.group(2)};')
# every mirror module sees every path trait, so calls between copies resolve to the copies
text = re.sub(r'^(\s*pub mod \w+ \{)', r'\1\n use crate::__fnr_path::*;', text, flags=re.M)
text += '\n#[doc(hidden)]\npub mod __fnr_path {\n' + '\n'.join(sorted(set(uses))) + '\n}\n'
open(lib_path, 'w').write(text)
kept = [r for r in rows if r[0] in entries]
open(out_installs, 'w').write('\n'.join('\t'.join(r) for r in kept) + '\n')
print(f'{len(methods)} methods renamed, {calls} call sites redirected, {len(kept)} of {len(rows)} copies installed')
