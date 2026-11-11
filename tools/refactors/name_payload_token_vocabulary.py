#!/usr/bin/env python3
"""Separate staged token transport vocabulary from the shipped raw transport API."""
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
RENAMES = {"PayloadDirection": "PayloadTokenDirection", "PayloadSendError": "PayloadTokenSendError"}
for relative in ("crates/horizon-mod-vocab/src/content/payloads.rs",
                 "crates/horizon-mod-vocab/src/exports/payloads.rs"):
    path = ROOT / relative
    source = path.read_text()
    for old, new in RENAMES.items():
        source = re.sub(r"\b" + old + r"\b", new, source)
    path.write_text(source)

path = ROOT / "docs/modding/src/concepts/typed-payloads.md"
source = path.read_text()
start = source.index("## Stage 1:")
end = source.index("## Stage 2:", start)
stage = source[start:end].replace("`PayloadSendError`", "`PayloadTokenSendError`")
stage = stage.replace("| `PayloadToken` |", "| `PayloadTokenDirection` (`ToServer`, `ToClient`) | The direction of a staged token, distinct from raw registration's bidirectional `PayloadDirection` |\n| `PayloadToken` |")
path.write_text(source[:start] + stage + source[end:])

# A renamed enum is not the frozen baseline identity. Retire that pin using
# the golden generator's explicit retirement contract, never reuse its old hash.
identity = "crate::content::payloads::PayloadDirection"
path = ROOT / "tools/fixtures/wire_enum_layouts.json"
manifest = json.loads(path.read_text())
manifest["enums"] = [entry for entry in manifest["enums"] if entry["canonical_path"] != identity]
manifest["retired_identities"] = sorted(set(manifest.get("retired_identities", [])) | {identity})
path.write_text(json.dumps(manifest, indent=2) + "\n")
path = ROOT / "tools/fixtures/wire_enum_baseline_identities.json"
path.write_text(json.dumps([entry for entry in json.loads(path.read_text()) if entry != identity], indent=2) + "\n")
sys.path.insert(0, str(ROOT / "tools"))
from generate_wire_enum_goldens import render
(ROOT / "crates/horizon-mod-vocab/src/wire/fieldless_enum_goldens.rs").write_text(render(manifest))
