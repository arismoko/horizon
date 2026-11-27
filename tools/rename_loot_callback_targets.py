#!/usr/bin/env python3
"""Distinguish engine loot callback invokers from API hook targets."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATHS = [
    "crates/sunrise-world/src/loot.rs",
    "crates/sunrise-world/tests/native_load_hooks.rs",
    "crates/sunrise-items/src/items.rs",
    "crates/sunrise-gameplay-common/src/game/native_hooks/shared.rs",
]

for name in PATHS:
    path = ROOT / name
    source = path.read_text()
    for kind in ("function", "condition"):
        source = source.replace(f"native_loot_{kind}", f"native_invoke_loot_{kind}")
    path.write_text(source)
