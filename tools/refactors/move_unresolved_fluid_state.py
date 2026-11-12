#!/usr/bin/env python3
"""Relocate #2587's fluid-name storage verbatim before callback integration."""
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
source = ROOT / 'crates/horizon-mod-vocab/src/block/unresolved_fluid.rs'
s = source.read_text()
a = s.index('#[derive(Default)]\nstruct Table')
b = s.index('/// Compare an identity', a)
c = s.index('/// Resolve an engine-issued process-lifetime handle.')
(ROOT / 'crates/horizon-mod-api/src/fluid_state.rs').write_text(
    '//! Platform-owned identities for retained unavailable-fluid names.\n'
    'use crate::UnresolvedFluid;\n'
    'use std::{collections::HashMap, sync::{LazyLock, RwLock}};\n\n'
    + s[a:b] + s[c:])
source.write_text(s[:a] + s[b:c])
