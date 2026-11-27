"""Read native hook implementations across the game's area modules."""
from pathlib import Path


def native_hook_sources():
    crates = Path(__file__).resolve().parents[1] / "crates"
    directories = (
        crates / "sunrise-gameplay-common/src/game/native_hooks",
        crates / "sunrise-gameplay/src/game/native_hooks",
        crates / "sunrise-client-gameplay/src/game/native_hooks",
    )
    paths = [path for directory in directories for path in sorted(directory.glob("*.rs"))]
    return "\n".join(path.read_text(encoding="utf-8") for path in paths).replace(
        "pub(in crate::game) fn", "pub(super) fn"
    )
