#!/usr/bin/env python3
"""Give block-entity table fixtures their explicit native type membership."""

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1] / 'crates'
FIXTURES = {
    'sunrise-game-data/src/block_entity_types.rs': [
        ['mod:own', 'mod:other'], ['test:machine', 'test:single', 'test:empty'], ['test:machine'],
    ],
    'sunrise-client/src/block_entity_item.rs': [['example:crate', 'example:tank']],
    'sunrise-client/src/content_identity.rs': [[]],
    'sunrise-client/src/structure_blocks.rs': [['test:machine']],
    'sunrise-client/src/save.rs': [['test:machine', 'test:other']],
    'sunrise-client/src/lan/chunks.rs': [['test:machine']] * 4,
    'sunrise-client/src/lan/hub.rs': [['test:machine']],
    'sunrise-client/src/game/network/remote/block_entity_data_tests.rs': [['notice:counter', 'notice:silent']],
    'sunrise-client/src/game/administration/commands/data/mod_block_entity_tests.rs': [[None, 'fixture:replacement']],
    'sunrise-client/tests/lan_bot.rs': [['test:machine_entity']],
    'sunrise-command/src/item.rs': [['example:crate']],
}
CALL = re.compile(r'(BlockEntityTypes::new\(\n.*?\n)( +)\)', re.DOTALL)


def main():
    for name, rows in FIXTURES.items():
        path = ROOT / name
        text = path.read_text()
        matches = list(CALL.finditer(text))
        assert len(matches) == len(rows), name
        for match, keys in reversed(list(zip(matches, rows))):
            values = ', '.join(f'"{key}".into()' if key else 'ENTITY.to_owned()' for key in keys)
            pos = match.start(2)
            text = text[:pos] + f'{match[2]}    [{values}],\n' + text[pos:]
        path.write_text(text.replace(
            'BlockEntityTypes::new(BTreeMap::new(), Vec::new())',
            'BlockEntityTypes::new(BTreeMap::new(), Vec::new(), [])',
        ))


if __name__ == '__main__':
    main()
