#!/usr/bin/env python3
"""Move horizon-mod-wasm's wire_loopback test target into its own test-only package, with discovered modules.

The package crates/horizon-mod-wasm/wire-loopback has a build.rs that writes one `#[path = "<absolute>"] mod <stem>;`
per file of tests/wire_loopback/ into OUT_DIR. Its own build script watches that folder, so editing a test reruns
only the test package's script, never horizon-mod-wasm's (whose rerun rebuilds the library and everything above it).
This script, run once from the repository root:
1. moves crates/horizon-mod-wasm/tests/wire_loopback.rs and tests/wire_loopback/ to
   crates/horizon-mod-wasm/wire-loopback/tests/ (git mv);
2. renames wire_loopback/loot_tags.rs to loot_tags_wire.rs, the one module whose name differed from its file, so
   every test keeps its path;
3. deletes every `#[path = "wire_loopback/<file>.rs"]` + `mod <name>;` pair from the root, after checking that the
   pairs name each file of the folder exactly once and that each name is its file's stem;
4. puts the `include!` of the generated list where the first pair stood (module order does not matter: the root
   and its children define no macro, and items resolve wherever they are declared);
5. points prepared_equip.rs's `#[path]` to horizon-mod-wasm's src/prepared.rs one folder further up;
6. runs rustfmt on the root, which removes the blank lines the deleted pairs leave behind.
"""

from pathlib import Path
import re
import subprocess

OLD = Path("crates/horizon-mod-wasm/tests")
NEW = Path("crates/horizon-mod-wasm/wire-loopback/tests")
FOLDER = NEW / "wire_loopback"
ROOT = NEW / "wire_loopback.rs"
PAIR = re.compile(r'^#\[path = "wire_loopback/(\w+)\.rs"\]\nmod (\w+);\n', re.M)
INCLUDE = ("// One module per file of tests/wire_loopback/, written by build.rs: a new test goes in a file there.\n"
           'include!(concat!(env!("OUT_DIR"), "/wire_loopback_modules.rs"));\n')
PREPARED = ('#[path = "../../src/prepared.rs"]', '#[path = "../../../src/prepared.rs"]')

NEW.mkdir(parents=True)
for name in ("wire_loopback.rs", "wire_loopback"):
    subprocess.run(["git", "mv", str(OLD / name), str(NEW / name)], check=True)
subprocess.run(["git", "mv", str(FOLDER / "loot_tags.rs"), str(FOLDER / "loot_tags_wire.rs")], check=True)
text = ROOT.read_text()
pairs = PAIR.findall(text)
renamed = [(file, name) for file, name in pairs if file != name]
assert renamed == [("loot_tags", "loot_tags_wire")], renamed
names = sorted(name for _, name in pairs)
assert names == sorted(path.stem for path in FOLDER.glob("*.rs")), names
first = PAIR.search(text).start()
text = text[:first] + INCLUDE + PAIR.sub("", text[first:])
ROOT.write_text(text)
prepared = FOLDER / "prepared_equip.rs"
source = prepared.read_text()
assert source.count(PREPARED[0]) == 1
prepared.write_text(source.replace(*PREPARED))
subprocess.run(["rustfmt", "--edition", "2024", str(ROOT)], check=True)
print(f"moved to {NEW}; {len(pairs)} module lines removed; loot_tags.rs renamed to loot_tags_wire.rs")
