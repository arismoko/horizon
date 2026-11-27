#!/usr/bin/env python3
"""The repository's formatting check, the one CI's `format` and `lint` jobs and `tools/pr_ready.py` run.

    python3 tools/ci/format_scan.py

`cargo fmt --all` reaches only the modules a `mod` line names, so it misses the gameplay fragments an `include!`
reads, the API and vocabulary files a build script includes, and the generator's
fixtures. Each of those is checked by path as well. Every check runs, so one run names every unformatted file; the
exit status is 1 if any of them failed. The launcher is a separate workspace with its own check.
"""

import os
import subprocess
import sys
import tomllib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

if __package__:
    from .rust_changes import tokens
    from .surface_report import source_layout
else:
    from rust_changes import tokens
    from surface_report import source_layout

ROOT = Path(__file__).resolve().parents[2]
# Every `.rs` file under these folders, checked one file at a time (no child modules followed).
TREES = ("crates/sunrise-mod-api/src", "crates/sunrise-mod-vocab/src",
         "crates/sunrise-client/src/game", "crates/sunrise-client/src/cut_layout",
         "crates/sunrise-sim/src/health", "crates/sunrise-sim/src/mobs/goals",
         "crates/sunrise-sim/src/save/carrier")
GAMEPLAY_OWNERS = ("sunrise-gameplay-common", "sunrise-gameplay", "sunrise-client-gameplay")
# Files no `mod` line reaches.
DISCOVERED = ("crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/*.rs",
              "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures/*/*.rs")
RUSTFMT = ["rustfmt", "--edition", "2024", "--check", "--config", "skip_children=true"]
# Windows limits a command line to 32,767 characters (CHUNK=200).
# On Linux and macOS, ARG_MAX is at least 2 MiB (MAX_ARG_STRLEN is 128 KiB); 500 relative
# paths averaging ~60 characters total ~30 KiB, staying well within Unix limits while reducing process spawn overhead.
CHUNK = 200 if os.name == "nt" else 500


def literal_fragments(root: Path, files: set[Path]) -> set[Path]:
    pending = list(files)
    found = set()
    while pending:
        source = pending.pop()
        text = source.read_text(encoding="utf-8")
        if "include" not in text:
            continue
        lexed = list(tokens(text))
        for index, token in enumerate(lexed):
            if token != ("ident", "include"):
                continue
            tail = lexed[index + 1:index + 5]
            if len(tail) != 4 or tail[0] != ("punct", "!") or tail[2][0] != "str":
                continue
            if (tail[1], tail[3]) not in {
                (("punct", "("), ("punct", ")")),
                (("punct", "["), ("punct", "]")),
                (("punct", "{"), ("punct", "}")),
            }:
                continue
            fragment = (source.parent / tail[2][1]).resolve()
            if (fragment.is_relative_to(root) and fragment.suffix == ".rs" and fragment.is_file()
                    and fragment not in files and fragment not in found):
                found.add(fragment)
                pending.append(fragment)
    return found


def split_sources(root: Path) -> set[Path]:
    files = set()
    for owner in GAMEPLAY_OWNERS:
        package = root / "crates" / owner
        manifest = tomllib.loads((package / "Cargo.toml").read_text(encoding="utf-8"))
        library = (package / manifest.get("lib", {}).get("path", "src/lib.rs")).resolve()
        if not library.is_relative_to(root) or not library.is_file():
            raise ValueError(f"{owner}: gameplay library source is missing or outside the workspace")
        files.update(library.parent.rglob("*.rs"))
    return files


def commands(root: Path) -> list[list[str]]:
    files = sorted(path.relative_to(root).as_posix() for tree in TREES for path in (root / tree).rglob("*.rs"))
    files += sorted(path.relative_to(root).as_posix() for pattern in DISCOVERED for path in root.glob(pattern))
    if source_layout(root) == "split":
        root = root.resolve()
        checked = {root / path for path in files}
        owners = split_sources(root)
        extra = (owners | literal_fragments(root, owners | checked)) - checked
        files += sorted(path.relative_to(root).as_posix() for path in extra)
    return [["cargo", "fmt", "--all", "--", "--check"]] + [
        RUSTFMT + files[start:start + CHUNK] for start in range(0, len(files), CHUNK)]


def main() -> int:
    all_commands = commands(ROOT)
    # Run formatting checks in parallel across available cores, overlapping cargo fmt and rustfmt chunks.
    max_workers = min(len(all_commands), os.cpu_count() or 4)
    with ThreadPoolExecutor(max_workers=max_workers) as pool:
        results = list(pool.map(lambda cmd: subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True), all_commands))
    failed = [cmd for cmd, result in zip(all_commands, results) if result.returncode != 0]
    # Replay differences in deterministic command order to avoid interleaved outputs across threads.
    for result in results:
        if result.stdout:
            sys.stdout.write(result.stdout)
        if result.stderr:
            sys.stderr.write(result.stderr)
    sys.stdout.flush()
    sys.stderr.flush()
    if failed:
        print(f"format_scan: {len(failed)} formatting check(s) failed; the differences are above.", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
