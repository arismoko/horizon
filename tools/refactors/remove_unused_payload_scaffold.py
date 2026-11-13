#!/usr/bin/env python3
"""Remove the unimplemented token transport scaffold superseded by raw payloads."""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
for relative in ("crates/horizon-mod-vocab/src/content/payloads.rs",
                 "crates/horizon-mod-vocab/src/exports/payloads.rs"):
    (ROOT / relative).unlink(missing_ok=True)
path = ROOT / "crates/horizon-mod-vocab/src/content/mod.rs"
path.write_text(path.read_text().replace("pub(crate) mod payloads;\n", ""))
path = ROOT / "docs/modding/src/concepts/typed-payloads.md"
source = path.read_text()
if "## Stage 1:" in source:
    start = source.index("## Stage 1:")
    end = source.index("## Stage 2:", start)
    source = source[:start] + source[end:]
path.write_text(source)

# Retire deleted identities under the existing golden generator contract.
retired = {"crate::content::payloads::" + name for name in
           ("ConnectionPhase", "PayloadDirection", "PayloadPhase", "PeerPolicy")}
path = ROOT / "tools/fixtures/wire_enum_layouts.json"
manifest = json.loads(path.read_text())
manifest["enums"] = [entry for entry in manifest["enums"] if entry["canonical_path"] not in retired]
manifest["retired_identities"] = sorted(set(manifest.get("retired_identities", [])) | retired)
path.write_text(json.dumps(manifest, indent=2) + "\n")
path = ROOT / "tools/fixtures/wire_enum_baseline_identities.json"
path.write_text(json.dumps([entry for entry in json.loads(path.read_text()) if entry not in retired], indent=2) + "\n")
sys.path.insert(0, str(ROOT / "tools"))
from generate_wire_enum_goldens import render
(ROOT / "crates/horizon-mod-vocab/src/wire/fieldless_enum_goldens.rs").write_text(render(manifest))
