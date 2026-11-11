"""Audit the fixed Dw exclusions against current manifests, listener references and examples."""
import json
from pathlib import Path
import re
import sys
import tomllib

root = Path(sys.argv[1])
out = Path('/box/shared/fn-replace')
excluded = set((out / 'dw-excluded.txt').read_text().splitlines())
points = json.loads((out / 'current-api-points.json').read_text())
by_name = {p['name']: p['target'] for p in points}
for p in json.loads((root.parents[0] / 'kit-out/stage-B/points.json').read_text())['points']:
    by_name.setdefault(p['name'], p['target'])
normalize = lambda target: re.sub(r'<[^>]*>', '', target).replace(' ', '')
canonical = {normalize(t): t for t in excluded}
hits = {'fixture_native_mixin': {}, 'fixture_api_listener': {}, 'examples': {}, 'portizon': {}}

def record(group, target, path):
    target = canonical.get(normalize(target))
    if target is not None:
        hits[group].setdefault(target, []).append(str(path))


def scan(directory, example=False):
    for path in directory.rglob('*'):
        if not path.is_file():
            continue
        if path.name == 'horizon-mod.toml':
            manifest = tomllib.loads(path.read_text())
            for mixin in manifest.get('native', {}).get('mixin', []):
                target = mixin.get('target')
                if target is not None:
                    assert isinstance(target, str), path
                    record('examples' if example else 'fixture_native_mixin', target, path.relative_to(root))
            for inject in manifest.get('inject', []):
                if inject.get('access', 'api') == 'api':
                    target = inject.get('at', inject.get('target'))
                    if target is not None:
                        assert isinstance(target, str), path
                        record('examples' if example else 'fixture_api_listener', target, path.relative_to(root))
        if path.suffix == '.rs':
            text = path.read_text()
            # Constructor names are generated catalog names; suffixes name phase variants.
            for name in re.findall(r'mod_points::(?:listeners::)?([A-Za-z_]\w*)', text):
                for candidate in (name, re.sub(r'_(?:return|replace)$', '', name)):
                    if candidate in by_name:
                        record('examples' if example else 'fixture_api_listener', by_name[candidate], path.relative_to(root))
            # Examples can also show literal source targets in Rust-generated manifests.
            if example:
                for target in re.findall(r'horizon_\w+::[^\s"`]+', text):
                    record('examples', target.rstrip(',;)'), path.relative_to(root))

scan(root / 'tools/ci/fn-replace-check')
scan(root / 'crates/horizon-mod-wasm/tests')
for directory in root.rglob('examples'):
    if 'target' not in directory.relative_to(root).parts:
        scan(directory, example=True)
port = Path('/box/shared/portizon-emitter/points-used.md')
text = port.read_text()
# Match full audited spellings; aliases are expanded below from the audit's declared vocabulary.
expanded = text.replace('N::GameLevel', "horizon_client::game::native_hooks::base::GameLevel<'_>").replace('N::Game', "horizon_client::game::native_hooks::base::Game<'_>")
expanded = expanded.replace('T::', "horizon_client::game::transactions::Game<'_>::").replace('C::', "horizon_client::game::creatures::Game<'_>::").replace('E::Game::', "horizon_client::game::native_hooks::Game<'_>::")
for target in re.findall(r'horizon_\w+::[^\s"`|]+', expanded):
    record('portizon', target.rstrip(',;)'), port)
api = sorted(p['target'] for p in points if p['target'] in excluded and p['api_refusal'] is None)
result = dict(source_commit='c76b3b5f75', excluded=len(excluded), groups=hits, api_records=api)
(out / 'dw-audit.json').write_text(json.dumps(result, indent=2) + '\n')
for group, targets in hits.items():
    print(group, len(targets))
    for target in sorted(targets):
        print(' ', target)
print('api_records', len(api))
