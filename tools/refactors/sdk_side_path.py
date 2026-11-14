#!/usr/bin/env python3
"""One path for the native mod inventory's types (PR 3b review): `BuiltMod` is `mod_api::__private::native::BuiltMod`
and its side is the API's own `Side`, so `horizon_mod_package` re-exports neither. Rewrites every Rust caller and the
Python generators that write inventory rows. Crates without `mod_api` (the launcher, `horizon-items`) name `Side`
through `mod_vocab`, which `mod_api` re-exports.

    python3 tools/refactors/sdk_side_path.py
"""
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Rust files under these use `mod_vocab`, not `mod_api`.
VOCAB = ("launcher/", "crates/horizon-items/", "crates/horizon-mod-package/")


def rewrite(path: str, text: str) -> str:
    api = "mod_vocab" if path.startswith(VOCAB) else "mod_api"
    text = text.replace("horizon_mod_package::runtime::BuiltMod", f"{api}::__private::native::BuiltMod")
    text = re.sub(r"horizon_mod_package::(manifest::)?Side\b", f"{api}::Side", text)
    if not path.endswith(".rs"):
        return text

    def split(match: re.Match) -> str:
        names = [name.strip() for name in match["names"].split(",") if name.strip()]
        moved = [name for name in names if name in ("Side", "runtime::BuiltMod")]
        if not moved:
            return match[0]
        kept = [name for name in names if name not in moved]
        lines = []
        if kept:
            lines.append(f"{match['indent']}use horizon_mod_package::{{{', '.join(kept)}}};")
        for name in moved:
            lines.append(f"{match['indent']}use {api}::" + ("Side;" if name == "Side" else "__private::native::BuiltMod;"))
        return "\n".join(lines)

    return re.sub(r"(?m)^(?P<indent>[ \t]*)use horizon_mod_package::\{(?P<names>[^}]*)\};", split, text)


def main() -> None:
    files = subprocess.run(["git", "ls-files", "*.rs", "*.py", "*.template"], cwd=ROOT, capture_output=True,
                           text=True, check=True).stdout.split()
    for name in files:
        # Historical refactor scripts and this one describe old trees; they are not rewritten.
        if name.startswith("tools/refactors/") or name == "tests/test_rename_crate.py":
            continue
        path = ROOT / name
        text = path.read_text()
        new = rewrite(name, text)
        if new != text:
            path.write_text(new)
            print(name)


if __name__ == "__main__":
    main()
