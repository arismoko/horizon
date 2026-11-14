#!/usr/bin/env python3
"""Move what `cargo-horizon-mod` reads of a mod's own source out of the generator.

`horizon-mod-api-gen` builds the game's API; the mod tool used it for two things only: the mod
folder reader (`mod_uses`) and the scan of a mod's public Wire types (`wire_types`), which walks
a crate's module files. This script moves, unchanged apart from paths and visibility:

- `src/mod_uses.rs` and `src/wire_types.rs` to the new crate `horizon-mod-source` (lib
  `mod_source`);
- the module walk of `src/source.rs` (`GameFile`, renamed `ModuleFile`, `conditions`,
  `test_only`, `gated`, `walk_module_files`, `children`) to `mod_source::modules`; the walk
  cache and `module_files` stay in the generator;

and points every user at the new paths. The walk still calls the generator's parser here; the
hand-written commit after this one passes the parser in.

Run once from anywhere: `python3 tools/refactors/unlink_mod_source.py`.
"""
from pathlib import Path
import re

root = Path(__file__).resolve().parents[2]
gen = root / 'crates/horizon-mod-api-gen'
new = root / 'crates/horizon-mod-source'
tool = root / 'crates/cargo-horizon-mod'


def edit(path, *pairs):
    text = path.read_text()
    for old, replacement in pairs:
        assert text.count(old) == 1, (path, old)
        text = text.replace(old, replacement)
    path.write_text(text)


def cut(text, start, end):
    """The block from `start` up to (not including) `end`, and the text without it."""
    i = text.index(start)
    j = text.index(end, i)
    return text[i:j], text[:i] + text[j:]


# The new crate.
(new / 'src').mkdir(parents=True, exist_ok=True)
(new / 'Cargo.toml').write_text('''[package]
name = "horizon-mod-source"
version = "0.1.0"
edition.workspace = true
license.workspace = true
publish = false
description = "A mod's own source as the mod tools read it: the mod folder, its module files and the Wire types it lets cross"

[lib]
name = "mod_source"

[dependencies]
horizon-mod-surface = { path = "../horizon-mod-surface" }
proc-macro2 = { version = "1", features = ["span-locations"] }
quote = "1"
syn = { version = "2", features = ["full"] }
toml = "1.1"

[dev-dependencies]
tempfile = "3.27.0"

[lints]
workspace = true
''')
(new / 'src/lib.rs').write_text('''//! A mod's own source as the mod tools read it, without the generator that builds the game's
//! API: the mod folder the uses scan reads ([`mod_uses`]), a crate's module files
//! ([`modules`]) and the Wire types a mod lets cross ([`wire_types`]).
pub mod mod_uses;
pub mod modules;
pub mod wire_types;
''')
for name in ['mod_uses.rs', 'wire_types.rs']:
    (new / 'src' / name).write_text((gen / 'src' / name).read_text())
    (gen / 'src' / name).unlink()

# The module walk.
source = gen / 'src/source.rs'
text = source.read_text()
head, text = cut(text, '/// A source file of the game\'s crates', '/// The files of the module `module`')
walk, text = cut(text, 'fn walk_module_files(', '/// The files of `horizon_client::game`')
gated, text = cut(text, '/// Whether `attrs` hold a `cfg` (or `cfg_attr`)', '/// `r#type` as `type`.')
moved = head + 'pub ' + walk + gated
moved = moved.replace('pub(crate) ', 'pub ')
(new / 'src/modules.rs').write_text('''//! A crate's source tree as the tools walk it: each file with its module path and the `cfg`s it
//! is compiled under, following `mod` declarations, `#[path]`s and inline modules.

use quote::ToTokens;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::Item;

''' + moved)
text = text.replace(
    'use syn::{Item, Type};\n',
    'use mod_source::modules::{ModuleFile, conditions, gated, test_only, walk_module_files};\n'
    'use syn::{Item, Type};\n',
)
source.write_text(re.sub(r'\bGameFile\b', 'ModuleFile', text))

# Every user, at the new paths.
MOVED = {'ModuleFile', 'conditions', 'test_only', 'gated'}


def split_use(match):
    """`use crate::source::{a, gated, b};` as the names that stay and the names that moved."""
    names = [name.strip() for name in match.group(1).split(',') if name.strip()]
    stay = [name for name in names if name not in MOVED]
    gone = [name for name in names if name in MOVED]
    lines = []
    if stay:
        lines.append('use crate::source::{' + ', '.join(stay) + '};')
    if gone:
        lines.append('use mod_source::modules::{' + ', '.join(gone) + '};')
    return '\n'.join(lines)


for path in list(gen.rglob('*.rs')) + list((new / 'src').glob('*.rs')):
    if path == source:
        continue
    text = path.read_text()
    changed = re.sub(r'\bGameFile\b', 'ModuleFile', text)
    changed = re.sub(r'use crate::source::\{([^}]*)\};', split_use, changed)
    changed = re.sub(r'(?<![\w:])(?:crate::)?source::(ModuleFile|conditions|test_only|gated)\b',
                     r'mod_source::modules::\1', changed)
    changed = changed.replace('use horizon_mod_api_gen::mod_uses;', 'use mod_source::mod_uses;')
    if changed != text:
        path.write_text(changed)
edit(gen / 'src/lib.rs', ('pub mod mod_uses;\n', ''), ('pub mod wire_types;\n', ''))
edit(gen / 'Cargo.toml', (
    'horizon-mod-surface = { path = "../horizon-mod-surface" }\n',
    'horizon-mod-surface = { path = "../horizon-mod-surface" }\n'
    'horizon-mod-source = { path = "../horizon-mod-source" }\n',
))
for path in (tool / 'src').rglob('*.rs'):
    text = path.read_text()
    changed = text.replace('horizon_mod_api_gen::', 'mod_source::')
    if changed != text:
        path.write_text(changed)
edit(tool / 'Cargo.toml', (
    'horizon-mod-api-gen = { path = "../horizon-mod-api-gen" }\n',
    'horizon-mod-source = { path = "../horizon-mod-source" }\n',
))
edit(root / 'Cargo.toml', (
    '    "crates/horizon-mod-surface",\n',
    '    "crates/horizon-mod-source",\n    "crates/horizon-mod-surface",\n',
))
