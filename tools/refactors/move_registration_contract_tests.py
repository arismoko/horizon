#!/usr/bin/env python3
"""Move registration-dependent contracts above the vocabulary boundary (#2587).

The move is verbatim; caller paths are corrected in a later integration commit.
Pure Wire and lookup-context tests remain in the vocabulary.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from layering.rust_text import blank, item_end

VOCAB = ROOT / "crates/horizon-mod-vocab/src"
API = ROOT / "crates/horizon-mod-api/src/parts"


def modules(source, target):
    path = VOCAB / source
    text = path.read_text()
    code = blank(text)
    regions = []
    for match in re.finditer(r"#\[cfg\(test\)\]\s*mod\s+\w+\s*\{", code):
        end, _ = item_end(code, code.index("mod", match.start()))
        regions.append((match.start(), end))
    assert regions, source
    moved = "\n".join(text[start:end] for start, end in regions)
    for start, end in reversed(regions):
        text = text[:start] + text[end:]
    path.write_text(text)
    (API / target).write_text(moved + "\n")


def main():
    for source, target in [
        ("unit_tests.rs", "registration_value_tests.rs"),
        ("item/brewing_dispatch_tests.rs", "registration_brewing_tests.rs"),
        ("item/item_model_dispatch_tests.rs", "registration_item_model_tests.rs"),
    ]:
        path = VOCAB / source
        (API / target).write_text("#![cfg(test)]\n" + path.read_text())
        path.unlink()
    for source, target in [
        ("content/registry_keys.rs", "registration_identity_tests.rs"),
        ("client/particle_keys.rs", "registration_particle_tests.rs"),
        ("__private/registry_installation.rs", "registration_epoch_tests.rs"),
        ("block/block_metadata.rs", "registration_block_metadata_tests.rs"),
        ("block/fluid_capability.rs", "registration_fluid_capability_tests.rs"),
        ("testing.rs", "registration_fake_host_tests.rs"),
    ]:
        modules(source, target)
    path = VOCAB / "world/phase.rs"
    text = path.read_text()
    start = text.index("#[test]\nfn client_only_attribute_does_not_shift_both_side_registry")
    end, _ = item_end(blank(text), text.index("fn client_only_attribute", start))
    (API / "registration_attribute_side_tests.rs").write_text("#![cfg(test)]\n" + text[start:end] + "\n")
    path.write_text(text[:start] + text[end:])
    path = VOCAB / "exports/domains.rs"
    path.write_text(path.read_text().replace("#[cfg(test)]\nmod unit_tests;", ""))
    path = VOCAB / "lib.rs"
    text = path.read_text()
    for name in ["brewing_dispatch_tests", "item_model_dispatch_tests"]:
        text = text.replace(f'#[cfg(test)]\nmod {name} {{\n    include!("item/{name}.rs");\n}}\n', "")
    path.write_text(text)


if __name__ == "__main__":
    main()
