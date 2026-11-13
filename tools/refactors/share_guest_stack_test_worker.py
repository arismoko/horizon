#!/usr/bin/env python3
"""Consolidate the 14 slow-shard guest workers without changing test bodies."""
from pathlib import Path
import re

root = Path(__file__).resolve().parents[2]
files = [
    "mobs/creatures/death_weapon_review_tests.rs",
    "mobs/creatures/death_weapon_tests.rs",
    "modding/original_player.rs",
    "modding/block_entity_tick_items_tests.rs",
    "modding/block_use_completion_tests.rs",
    "world/villager_professions/tests.rs",
]
pattern = re.compile(
    r"(?P<indent> +)horizon_guest_stack::builder\(\)\n"
    r"(?:(?P=indent)    \.name\(std::thread::current\(\)\.name\(\)\.unwrap\(\)\.to_owned\(\)\)\n)?"
    r"(?P=indent)    \.spawn\(\|\| \{(?P<body>.*?)\n"
    r"(?P=indent)    \}\)\n(?P=indent)    \.unwrap\(\)\n"
    r"(?P=indent)    \.join\(\)\n(?P=indent)    "
    r"(?:\.unwrap\(\)|\.unwrap_or_else\(\|panic\| std::panic::resume_unwind\(panic\)\));",
    re.DOTALL,
)
changes = []
count = 0
for relative in files:
    path = root / "crates/horizon-client/src/game" / relative
    source = path.read_text()
    result, n = pattern.subn(
        lambda m: m["indent"]
        + "crate::game::native_entity_refs::tests::run_on_guest_stack(|| {"
        + m["body"] + "\n" + m["indent"] + "});",
        source,
    )
    changes.append((path, result))
    count += n
assert count == 14, f"expected 14 wrappers, found {count}"
for path, result in changes:
    path.write_text(result)
