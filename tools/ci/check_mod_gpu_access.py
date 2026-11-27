#!/usr/bin/env python3
"""verify-static's guard of the renderer-out window: the files below run inside `Renderer::render`'s frame hooks,
where `Game::render` has taken `Game::renderer` out of the game (sunrise#5858), so they reach the GPU through
`Game::mod_gpu()` and never through `self.renderer`, `self.client.renderer`, `self.game.renderer`, or
`self.game.client.renderer`.

Fails naming each such access in the non-test part of a guarded file (comments are ignored; a file's `#[cfg(test)]`
module is not read). Reads source only; builds nothing.

Usage: check_mod_gpu_access.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GUARDED = (
    "crates/sunrise-client-gameplay/src/game/modding/native_custom_geometry.rs",
    "crates/sunrise-client-gameplay/src/game/host/draw.rs",
    "crates/sunrise-client-gameplay/src/game/host/draw_client.rs",
    # The deleted gameplay-common copy had no draw code of its own.
    "crates/sunrise-client-gameplay/src/game/modding/native_block_entity_draw.rs",
)
ACCESS = re.compile(r"\bself\s*\.\s*(?:game\s*\.\s*)?(?:client\s*\.\s*)?renderer\b")
COMMENT = re.compile(r"//[^\n]*|/\*.*?\*/", re.S)
TEST_MODULE = re.compile(
    r'^#\[cfg\((?:test|any\(test, feature = "test-support"\))\)\]\n'
    r'(?:pub(?:\(crate\))? )?mod tests\s*\{',
    re.M,
)


def offenders(text):
    """(line, text) of each `self[.game][.client].renderer` outside comments and the test module."""
    blanked = COMMENT.sub(lambda m: re.sub(r"[^\n]", " ", m.group()), text)
    tests = TEST_MODULE.search(blanked)
    if tests:
        blanked = blanked[:tests.start()]
    lines = text.splitlines()
    return [
        (n, lines[n - 1].strip())
        for n in (blanked.count("\n", 0, m.start()) + 1 for m in ACCESS.finditer(blanked))
    ]


def main():
    failed = False
    for name in GUARDED:
        for number, line in offenders((ROOT / name).read_text()):
            failed = True
            print(f"{name}:{number}: {line}: reach the GPU through Game::mod_gpu(), not the renderer", file=sys.stderr)
    if not failed:
        print("The frame-hook files never read Game::renderer.")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
