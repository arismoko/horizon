#!/usr/bin/env python3
"""One-shot move: each stand-in of `mod stubs` and `mod api` in dispatch-tests/src/fixtures/shared.rs gets its own
file, fixtures/stubs/<Name>.rs or fixtures/api/<Name>.rs, which the generator reads into the same scope.

The grouping is tools/hotspots/carry.py's `stand_ins` (the same one `carry.py dispatch-stub` uses on old-style
PRs): an item, its impls and a `use` under it, with the comments above them, dedented one level. `use` lines of
other paths stay in a `mod stubs` in shared.rs. Every output file and shared.rs then go through rustfmt.

    python3 tools/refactors/split_dispatch_stand_ins.py
"""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/hotspots"))
import carry  # noqa: E402

shared = ROOT / carry.SHARED
found, rest = carry.stand_ins(shared.read_text(encoding="utf-8"))
written = [shared]
for (scope, name), text in sorted(found.items()):
    path = shared.parent / scope / f"{name}.rs"
    if path.exists():
        sys.exit(f"{path} exists already")
    path.parent.mkdir(exist_ok=True)
    path.write_text(text, encoding="utf-8")
    written.append(path)
shared.write_text(rest, encoding="utf-8")
subprocess.run(["rustfmt", "--edition", "2024", *map(str, written)], check=True)
print(f"moved {len(found)} stand-ins into {len(written) - 1} files")
