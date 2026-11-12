#!/usr/bin/env python3
"""Move platform-owned implementations out of the engine vocabulary (#2587).

Run from the repository root. This step deliberately does not rewrite callers;
subsequent commits install engine-owned callbacks and adapt the public facade.
--verify proves that the relocated implementations match their original bytes.
"""
import argparse
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
BASE = "30b15b134c9edc62d9698d5040e093cc26058cab"
MOVES = (
    ("profiler.rs", "profiler.rs"),
    ("profiler/firefox.rs", "profiler/firefox.rs"),
    ("content/crash.rs", "content/crash.rs"),
    ("wire/wire_resources.rs", "wire/wire_resources.rs"),
    ("world/phase.rs", "load_state.rs"),
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--only", help="Move or verify just this relative source path")
    args = parser.parse_args()
    for source, target in MOVES:
        if args.only and source != args.only:
            continue
        old = f"crates/horizon-mod-vocab/src/{source}"
        new = f"crates/horizon-mod-api/src/{target}"
        if args.verify:
            expected = subprocess.check_output(["git", "show", f"{BASE}:{old}"], cwd=ROOT)
            if (ROOT / new).read_bytes() != expected:
                raise SystemExit(f"Not a verbatim move: {new}")
        else:
            (ROOT / new).parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(["git", "mv", old, new], cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
