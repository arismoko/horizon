#!/usr/bin/env python3
"""Check the generated prebuilt game's Windows crate graph, without linking a game."""
import argparse
import fnmatch
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
PATHS = ("crates/**", "Cargo.*", "packaging/windows/**", "tools/prebuilt_game.py",
         ".cargo/**", "rust-toolchain*")


def applies(paths):
    return any(fnmatch.fnmatchcase(path, pattern) for path in paths for pattern in PATHS)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--select", nargs=2, metavar=("BASE", "HEAD"))
    parser.add_argument("--measure", action="store_true")
    args = parser.parse_args()
    if args.select:
        paths = subprocess.check_output(
            ["git", "diff", "--no-renames", "--name-only", "-z", *args.select], cwd=ROOT,
        ).decode().split("\0")
        print(f"required={str(applies(paths)).lower()}")
        return
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve() / "windows-prebuilt-check"
    if args.measure:
        print("Fresh Cargo target; compiler/download caches remain shared, NOT compiler-cache-cold.", flush=True)
        for command in (["git", "rev-parse", "HEAD"], ["rustc", "-vV"], ["cargo", "-V"],
                        ["uname", "-a"], ["x86_64-w64-mingw32-gcc", "--version"]):
            subprocess.run(command, cwd=ROOT, check=True)
        for key in sorted(os.environ):
            if key in ("CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "RUSTC_WRAPPER",
                       "RUNNER_NAME", "RUNNER_OS", "RUNNER_ARCH"):
                print(f"{key}={os.environ[key]}", flush=True)
        # Only this job's private check artifacts are cleared; tool/compiler caches remain.
        shutil.rmtree(target / "check", ignore_errors=True)
        for label, bound in (("fresh-target complete pipeline", 1200),
                             ("warm complete pipeline", 360)):
            started = time.monotonic()
            try:
                subprocess.run([sys.executable, str(Path(__file__).resolve())],
                               cwd=ROOT, check=True, timeout=bound)
            finally:
                print(f"{label}: {time.monotonic() - started:.3f} seconds", flush=True)
        return
    sys.path.insert(0, str(ROOT / "tools"))
    import prebuilt_game

    started = time.monotonic()
    try:
        staged = prebuilt_game.stage(target)
        environment = {key: value for key, value in os.environ.items()
                       if key != "CARGO_ENCODED_RUSTFLAGS"}
        environment.update(RUSTFLAGS="-C prefer-dynamic -D warnings",
                           CARGO_TARGET_DIR=str(target / "check"))
        subprocess.run(
            ["cargo", "check", "--locked", "--target", "x86_64-pc-windows-gnu",
             "-p", "horizon-mod-host", "--lib",
             *prebuilt_game.cargo_features(False, "win32")],
            cwd=staged, env=environment, check=True,
        )
    finally:
        shutil.rmtree(target / "stage", ignore_errors=True)
        print(f"Windows prebuilt check elapsed: {time.monotonic() - started:.2f} seconds", flush=True)


if __name__ == "__main__":
    main()
