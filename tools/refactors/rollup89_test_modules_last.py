#!/usr/bin/env python3
"""Move appended production helpers before the existing test modules.

One-time mechanical carry for rollup 89's items_after_test_module diagnostics.
The module text is retained byte-for-byte; no Rust parser is needed because the
reviewed helper doc comments uniquely delimit the trailing production items.
"""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
for relative, helper in (
    ("crates/horizon-command/src/item.rs", "\n/// `LodestoneTracker.CODEC`: optional GlobalPos and a strict optional boolean."),
    ("crates/horizon-items/src/item_data/ingredient_components.rs", "\n/// Read the record without cloning its payload."),
):
    path = ROOT / relative
    source = path.read_text()
    marker = "\n#[cfg(test)]\nmod tests {"
    assert source.count(marker) == 1, relative
    assert source.count(helper) == 1, relative
    start = source.index(marker)
    tail = source.index(helper)
    if tail < start:
        continue
    tests = source[start:tail]
    assert tests.rstrip().endswith("}"), relative
    path.write_text(source[:start] + source[tail:].rstrip() + "\n" + tests.rstrip() + "\n")
