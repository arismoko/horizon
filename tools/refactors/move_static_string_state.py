#!/usr/bin/env python3
"""Stage #2587's verbatim static-string implementation and test relocation.

Run against the pre-relocation tree; callback integration follows separately.
Only the two intern-owning tests are removed from the shared Wire test module.
"""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools'))
from layering.rust_text import blank, item_end

source_path = ROOT / 'crates/horizon-mod-vocab/src/wire/mod.rs'
source = source_path.read_text()
start = source.index('pub fn intern(value:')
stop = item_end(blank(source), start)[0]
implementation = source[start:stop]
target = ROOT / 'crates/horizon-mod-api/src/wire/static_strings.rs'
target.write_text('//! Platform-owned bounded storage for static Wire strings.\n'
                  'use super::{WireError, MAX_STATIC_STRINGS, MAX_STATIC_STRING_BYTES};\n'
                  'use std::collections::HashSet;\n\n' + implementation + '\n')
functions = []
for name in ('static_string_rows_roundtrip_through_intern',
             'attachment_refusals_use_bounded_interned_static_text'):
    start = source.index('    #[test]\n    fn ' + name)
    stop = item_end(blank(source), source.index('fn ' + name, start))[0]
    functions.append(source[start:stop])
    source = source[:start] + source[stop:]
helper = source.index('    fn round<T: Wire')
helper_end = item_end(blank(source), helper)[0]
functions.append(source[helper:helper_end])
(ROOT / 'crates/horizon-mod-api/tests/static_string_state.rs').write_text(
    'pub use mod_api::*;\npub use mod_api::wire::*;\n\nmod tests {\n    use super::*;\n'
    + '\n\n'.join(functions) + '\n}\n')
source_path.write_text(source)
