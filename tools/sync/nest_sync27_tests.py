#!/usr/bin/env python3
"""Keep owner-nested sync fixtures out of game-area root discovery."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GAME = ROOT / "crates/horizon-client/src/game"
PAIRS = {
    "blocks/campfire.rs": "campfire_tests.rs",
    "blocks/sculk.rs": "farm_animal_interaction_tests.rs",
    "blocks/nether_portal.rs": "portal_cooldown_tests.rs",
    "entities/fireworks.rs": "fireworks_events_tests.rs",
    "interface/stats.rs": "phantom_spawn_tests.rs",
    "items/shield.rs": "shield_ram_tests.rs",
    "mobs/sniffers.rs": "sniffers_tests.rs",
    "mobs/villagers.rs": "villager_witness_tests.rs",
}
for owner, filename in PAIRS.items():
    source = GAME / owner
    old = source.parent / filename
    new = source.parent / "tests" / filename
    new.parent.mkdir(exist_ok=True)
    old.rename(new)
    source.write_text(source.read_text().replace(f'#[path = "{filename}"]', f'#[path = "tests/{filename}"]'))
old = GAME / "interface/sign_chain_tests.rs"
new = old.parent / "tests" / old.name
new.parent.mkdir(exist_ok=True)
old.rename(new)
game = GAME.parent / "game.rs"
game.write_text(game.read_text().replace('game/interface/sign_chain_tests.rs', 'game/interface/tests/sign_chain_tests.rs'))
