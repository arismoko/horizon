#!/usr/bin/env python3
"""Move concrete payload authoring to PartRegistrar's post-3038 crate.

Keep the public registration vocabulary and frozen lookup storage below the engine.
Generator ownership conversions and test adaptations are separate corrections.
"""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-mod-vocab/src/parts/payloads.rs"
target = root / "crates/horizon-mod-api/src/parts/payloads.rs"
tests = root / "crates/horizon-mod-vocab/src/parts/payloads/tests.rs"
new_tests = root / "crates/horizon-mod-api/src/parts/payloads/tests.rs"
text = source.read_text()
start = text.index("fn indices(protocol:")
registration = text[start:]
assert "impl PayloadsRegistrar for PartRegistrar" in registration
assert not target.exists()
text = text[:start].replace(
    '#[cfg(test)]\n#[path = "payloads/tests.rs"]\nmod tests;\n', "", 1
)
source.write_text(text)
target.write_text(
    '//! Concrete payload authoring; frozen lookup storage stays in the vocabulary.\n'
    'use super::*;\n\n#[cfg(test)]\n#[path = "payloads/tests.rs"]\nmod tests;\n\n'
    + registration
)
new_tests.parent.mkdir(parents=True, exist_ok=True)
tests.rename(new_tests)
