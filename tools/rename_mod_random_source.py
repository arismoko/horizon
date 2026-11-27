#!/usr/bin/env python3
"""Rename the mod API's random types (#1531): `StructureRandom` becomes `RandomSource`, and the
mod-facing `LegacyRandom` becomes `LegacyRandomSource`. Engine code keeps `sunrise_core::random`'s
own names. Also drops the two `sunrise_core::random` re-exports the API no longer has."""
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SELF = Path(__file__).resolve()
# Files where a bare `LegacyRandom` already means the API's re-export.
BARE_LEGACY = [
    *map(str, (ROOT / "crates/sunrise-mod-vocab/src").rglob("*.rs")),
    str(ROOT / "crates/sunrise-gameplay/src/game/modding/native_goal_ticks/mod.rs"),
]

tracked = subprocess.run(
    ["git", "ls-files", "*.rs", "*.md"], cwd=ROOT, capture_output=True, text=True, check=True
).stdout.split()

for name in tracked:
    path = ROOT / name
    if path.resolve() == SELF or name.startswith("crates/sunrise-core/"):
        continue
    old = path.read_text()
    new = re.sub(r"\bStructureRandom\b", "RandomSource", old)
    new = re.sub(r"\bmod_api::LegacyRandom\b", "mod_api::LegacyRandomSource", new)
    if str(path.resolve()) in BARE_LEGACY:
        new = re.sub(r"\bLegacyRandom\b", "LegacyRandomSource", new)
    if name == "crates/sunrise-mod-vocab/src/exports/sunrise_core.rs":
        new = new.replace("pub use sunrise_core::random::LegacyRandomSource;\n", "")
        new = new.replace(", random::RandomSource,", ",")
    if new != old:
        path.write_text(new)
