#!/usr/bin/env python3
"""Place incoming sync 28 modules in the fork's discovered game areas."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
GAME = Path("crates/horizon-client/src/game")
MOVES = {
    "extinguish.rs": "entities/extinguish.rs",
    "extinguish_tests.rs": "entities/extinguish_tests.rs",
    "frame_autotune.rs": "runtime/frame_autotune.rs",
    "smithing_icons.rs": "interface/smithing_icons.rs",
}

if __name__ == "__main__":
    for source, destination in MOVES.items():
        src, dst = GAME / source, GAME / destination
        if (ROOT / dst).exists() and not (ROOT / src).exists():
            continue
        subprocess.run(["git", "mv", str(src), str(dst)], cwd=ROOT, check=True)
