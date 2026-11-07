#!/usr/bin/env python3
"""Move the raw renderer and rewrite its paths after the destination is approved."""
import argparse
from pathlib import Path
import subprocess

SOURCE = "crates/horizon-mod-api/src/render.rs"
EXPORT = "crates/horizon-mod-api/src/exports/render.rs"


def replacements(lib):
    return {
        "mod_api::render::": f"{lib}::render::",
        "mod_api::render as native_render": f"{lib}::render as native_render",
    }


def rewrite(text, lib):
    for old, new in replacements(lib).items():
        text = text.replace(old, new)
    return text


def tracked(root):
    return subprocess.check_output(
        ["git", "ls-files", "-z"], cwd=root
    ).decode().split("\0")[:-1]


def edits(root, crate, lib):
    destination = f"crates/{crate}/src/render.rs"
    if (root / destination).exists():
        raise ValueError(f"destination already exists: {destination}")
    # Read every input before writing; the export disappears, not a compatibility shim.
    changes = {
        SOURCE: None,
        EXPORT: None,
        destination: (root / SOURCE).read_text(),
    }
    for name in tracked(root):
        if name in changes or not name.endswith((".rs", ".md")):
            continue
        text = (root / name).read_text()
        changed = rewrite(text, lib)
        if changed != text:
            changes[name] = changed
    return changes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--destination-crate", required=True)
    parser.add_argument("--destination-lib", required=True)
    parser.add_argument("--apply", action="store_true", help="default only lists changes")
    args = parser.parse_args()
    changes = edits(args.root, args.destination_crate, args.destination_lib)
    for name, text in sorted(changes.items()):
        print(f"{'delete' if text is None else 'write'} {name}")
    if args.apply:
        for name, text in changes.items():
            path = args.root / name
            if text is None:
                path.unlink()
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)


if __name__ == "__main__":
    main()
