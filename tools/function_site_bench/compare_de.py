"""Compare actual D/E cell/check placement and compiled feature/profile records."""
from collections import Counter
import json
from pathlib import Path
import re

root = Path(__file__).resolve().parents[2]
base = root / 'target/wholegame/kit-out'
check = re.compile(r'if let ::std::option::Option::Some\(__fnr\) = [^;]*?__fnr_read_slot_(\d+)\(\) \{ return __fnr\(')
point = re.compile(r'#\[horizon_mod_api_macros::point\(([^\]]*)\)\]')
fn = re.compile(r'\bfn\s+(?:r#)?(\w+)')

def placement(stage):
    native, api = Counter(), Counter()
    for path in (stage / 'crates').rglob('*.rs'):
        text = path.read_text()
        relative = str(path.relative_to(stage))
        for match in check.finditer(text):
            name = list(fn.finditer(text[:match.start()]))[-1][1]
            native[relative, int(match[1]), name] += 1
        for match in point.finditer(text):
            slot = re.search(r'\bslot\s*=\s*(\d+)', match[1])
            if slot:
                name = fn.search(text, match.end())[1]
                api[relative, int(slot[1]), name] += 1
    def ordinalize(entries):
        grouped = Counter()
        for file, slot, name in entries:
            grouped[file, name] += entries[file, slot, name]
        return grouped
    return ordinalize(native), ordinalize(api)

def artifacts(stage):
    records = {}
    for line in (stage / 'build.json').read_text().splitlines():
        event = json.loads(line)
        if event['reason'] != 'compiler-artifact':
            continue
        package = event['package_id'].replace(str(stage), '$STAGE')
        key = (package, event['target']['name'], tuple(event['target']['kind']), json.dumps(event['profile'],sort_keys=True))
        records[key] = (event['features'], event['profile'])
    return records

d, e = base / 'stage-D', base / 'stage-E'
def cells(stage):
    pattern = r'pub static __REPLACE: \[::std::sync::atomic::AtomicPtr<\(\)>; (\d+)\]'
    return {str(p.relative_to(stage)): int(m[1]) for p in (stage / 'crates').rglob('*.rs') for m in [re.search(pattern,p.read_text())] if m}

def build_cfgs(stage):
    return Counter((e['package_id'].replace(str(stage), '$STAGE'), tuple(e['cfgs'])) for line in (stage / 'build.json').read_text().splitlines() for e in [json.loads(line)] if e['reason'] == 'build-script-executed')

dn, da = placement(d)
en, ea = placement(e)
ad, ae = artifacts(d), artifacts(e)
config = [p.relative_to(d) for p in d.rglob('*') if p.is_file() and (p.name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml') or '.cargo' in p.relative_to(d).parts) and 'zc-game' not in p.parts and '.git' not in p.parts]
changed = [str(p) for p in config if not (e / p).exists() or (d / p).read_bytes() != (e / p).read_bytes()]
result = dict(d_native_checks=dn.total(), d_api_slots=da.total(), e_native_checks=en.total(), e_api_slots=ea.total(), d_placement_only=list((dn + da - en - ea).elements()), e_placement_only=list((en + ea - dn - da).elements()), d_compile_records=len(ad), e_compile_records=len(ae), compile_records_equal=ad==ae, config_files_compared=len(config), changed_configs=changed)
result.update(d_cells=cells(d), e_cells=cells(e), build_script_cfgs_equal=build_cfgs(d)==build_cfgs(e))
path = Path('/box/shared/fn-replace/de-comparison.json')
path.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
assert result['build_script_cfgs_equal'], 'build-script cfg values differ'
assert dn.total() == 12388 and not da
assert en.total() + ea.total() == 12388
assert dn == en + ea, 'entry placement differs by source file/function and multiplicity'
assert ad == ae, 'compiled feature/profile records differ'
assert not changed, 'Cargo/config/toolchain inputs differ'
