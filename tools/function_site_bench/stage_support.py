"""Corrections needed when transplanting current staging onto the fixed kit base."""
import json
from pathlib import Path
import re


def exclude_generated_point_helpers(stage):
    """Do not rediscover replacement installers as public game injection points."""
    path = stage / 'crates/horizon-mod-point-shape/src/lib.rs'
    source = path.read_text()
    old = '    !sig.ident.to_string().starts_with("native_")'
    assert source.count(old) == 1
    source = source.replace(old, '    let name = sig.ident.to_string();\n    !name.starts_with("native_")\n        && !name.starts_with("__fnr_")', 1)
    path.write_text(source)


def mark_benchmark_exclusions(root, base, stage, excluded):
    """Exclude selected functions before staging, without changing Rust optimizer hints.

    A false cfg_attr is erased by rustc and changes no body or inline attribute.
    The existing source-level eligibility rule refuses conditional functions, so
    it suppresses checks AND readers/installers/catalog entries, not silent no-op
    replacement registrations. This is a benchmark-only eligibility marker.
    """
    shapes = json.loads(Path('/box/shared/fn-replace/d-target-shapes.json').read_text())
    names = {p['target']: p['name'] for p in json.loads((base / 'points.json').read_text())['points']}
    for target in sorted(excluded):
        row = shapes[target]
        relative = (root / row['file']).relative_to(base)
        path = stage / relative
        text = path.read_text()
        candidates = []
        for match in re.finditer(r'#\[horizon_mod_api_macros::point\(([^\]]*)\)\]', text):
            named = re.search(r'\bname\s*=\s*"' + re.escape(names[target]) + '"', match[1])
            default = not re.search(r'\bname\s*=', match[1]) and re.match(
                r'\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+(?:r#)?' + re.escape(target.rsplit('::', 1)[1]) + r'\b', text[match.end():])
            if named or default:
                candidates.append(match.start())
        assert len(candidates) == 1, (target, path, len(candidates))
        offset = candidates[0]
        path.write_text(text[:offset] + '#[cfg_attr(any(), doc(hidden))]\n' + text[offset:])
    prune_benchmark_point_lists(stage, excluded)
    (stage / 'fnr-benchmark-excluded.txt').write_text('\n'.join(sorted(excluded)) + '\n')


def prune_benchmark_point_lists(stage, excluded):
    """Remove excluded lower-crate declarations from the old base's explicit point lists."""
    plain = lambda value: re.sub(r'<[^>]*>', '', value).replace(' ', '')
    targets = {plain(t) for t in excluded}
    for path in (stage / 'crates/horizon-mod-api-gen/slots').glob('*.txt'):
        lines = path.read_text().splitlines()
        path.write_text('\n'.join(line for line in lines if plain(line) not in targets) + '\n')
