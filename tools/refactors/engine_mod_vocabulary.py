#!/usr/bin/env python3
"""Switch engine imports to vocabulary; keep explicit platform test operations."""
from pathlib import Path
import collections
import json
import re

ROOT = Path(__file__).resolve().parents[2]
CRATES = ('horizon-client', 'horizon-client-lib', 'horizon-client-render')
# Explicit platform operations, including deliberately shadowed vocabulary install.
ITEMS = ('testing::with_mods', 'testing::FakeHost', 'testing::FakeServerHost',
         'testing::FakeClientHost', 'testing::ItemKeyError', '__private::part_tests',
         '__private::install', '__private::install_parts', '__private::initialize',
         '__private::install_packages', '__private::install_registration_data',
         '__private::with_test_packages', '__private::with_test_registration_data',
         '__private::profiler')


def main():
    counts = collections.Counter()
    for crate in CRATES:
        folder = ROOT / 'crates' / crate
        manifest = folder / 'Cargo.toml'
        text = manifest.read_text().replace(
            'horizon-mod-api = { path = "../horizon-mod-api" }',
            'mod_api = { package = "horizon-mod-vocab", path = "../horizon-mod-vocab" }').replace(
            'horizon-mod-api = { path = "../horizon-mod-api", features = ["testing"] }',
            'mod_platform = { package = "horizon-mod-api", path = "../horizon-mod-api", features = ["testing"] }')
        manifest.write_text(text)
        for path in folder.rglob('*.rs'):
            text = original = path.read_text()
            for item in ITEMS:
                text, n = re.subn(r'\bmod_api::' + re.escape(item) + r'\b', 'mod_platform::' + item, text)
                counts[item] += n
            # Split mixed imports, preserving vocabulary items in their original group.
            def split(match):
                body = match[1]
                for item in ITEMS:
                    pattern = r'(?<![\w:])' + re.escape(item) + r'\s*,?'
                    body, n = re.subn(pattern, '', body)
                    if n:
                        counts[item] += n
                        return 'use mod_platform::' + item + ';\nuse mod_api::{' + body.strip().lstrip(',').strip() + '};'
                return match[0]
            text = re.sub(r'use mod_api::\{([^{}]+)\};', split, text)
            if text != original:
                path.write_text(text)
    path = ROOT / 'tools/layers.json'
    config = json.loads(path.read_text())
    removed = {edge for edge in config['allowed'] if edge.split(' -> ')[0] in CRATES and edge.split(' -> ')[1] in ('horizon-mod-api', 'horizon-mod-api-gen', 'horizon-mod-api-macros')}
    # Preserve the original formatting of this hand-maintained config.
    text = path.read_text()
    for edge in removed:
        text = re.sub(r'^    "' + re.escape(edge) + r'"[^\n]*\n', '', text, flags=re.M)
    path.write_text(text)
    print(json.dumps(dict(counts), indent=2))


if __name__ == '__main__':
    main()
