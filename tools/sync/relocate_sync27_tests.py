#!/usr/bin/env python3
"""Move incoming game-root tests beside their relocated owning modules."""
from pathlib import Path
root = Path('crates/horizon-client/src/game')
placements = {
 'campfire_tests': ('blocks', 'blocks/campfire.rs', 'campfire_tests.rs'),
 'farm_animal_interaction_tests': ('blocks', 'blocks/sculk.rs', '../farm_animal_interaction_tests.rs'),
 'fireworks_events_tests': ('entities', 'entities/fireworks.rs', 'fireworks_events_tests.rs'),
 'item_count_predicate_tests': ('administration/commands', 'administration/commands/mod.rs', '../../item_count_predicate_tests.rs'),
 'machines_chest_cat_tests': ('blocks/machines', 'blocks/machines/mod.rs', '../../machines_chest_cat_tests.rs'),
 'phantom_spawn_tests': ('interface', 'interface/stats.rs', '../phantom_spawn_tests.rs'),
 'portal_cooldown_tests': ('blocks', 'blocks/nether_portal.rs', 'portal_cooldown_tests.rs'),
 'sniffers_tests': ('mobs', 'mobs/sniffers.rs', '../sniffers_tests.rs'),
 'villager_witness_tests': ('mobs', 'mobs/villagers.rs', '../villager_witness_tests.rs'),
}
for name, (area, owner, old) in placements.items():
 source = root / (name + '.rs')
 source.rename(root / area / source.name)
 file = root / owner
 text = file.read_text()
 assert f'#[path = "{old}"]' in text
 file.write_text(text.replace(f'#[path = "{old}"]', f'#[path = "{name}.rs"]'))
(root / 'shield_ram_tests.rs').rename(root / 'items/shield_ram_tests.rs')
shield = root / 'items/shield.rs'
shield.write_text(shield.read_text() + '\n#[cfg(test)]\n#[path = "shield_ram_tests.rs"]\nmod ram_tests;\n')
(root / 'sign_chain_tests.rs').rename(root / 'interface/sign_chain_tests.rs')
game = root.parent / 'game.rs'
game.write_text(game.read_text().replace('game/sign_chain_tests.rs', 'game/interface/sign_chain_tests.rs'))
