#!/usr/bin/env python3
"""Print the ignored-tests shard matrix: every shard, or the ones a pull request needs.

Usage:
  ignored_shards.py --matrix --all      `matrix=`, every shard and the daily-rows legs (the daily run)
  ignored_shards.py --matrix BASE HEAD  `matrix=`, the shards this pull request must run

The ignored-tests check is a matrix with one package per shard; this file is the single
place that names them. A shard is selected when a changed crate is its package or a
transitive dependency of it. Workspace-wide files, and any path no crate owns, select
every shard; pushes to main always run everything (the `changes` job passes --all).
crates/sunrise-client holds sunrise-client-lib's sources too (its [lib] path reaches
into that tree), so that directory counts for both packages.
"""

import json
import sys
import tomllib
from functools import lru_cache
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools.ci import rust_changes  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]

# One package per shard, balanced on the measured test times of the single-job run this
# plan replaced (dyna-ci run #952, e8752e146f: sunrise-client 380s, sunrise-client-render
# 279s, sunrise-client-lib 189s, 16m53s wall); sunrise-world and sunrise-items joined when every library
# test started running with the data, not only the #[ignore]d ones, and the three gameplay crates
# (sunrise-gameplay-common, sunrise-gameplay, sunrise-client-gameplay) joined when gameplay split out of
# sunrise-client so their library tests keep running with prepared vanilla data. Rebalance by editing
# this tuple only.
SHARDS = (
    "sunrise-client",
    "sunrise-gameplay-common",
    "sunrise-gameplay",
    "sunrise-client-gameplay",
    "sunrise-client-lib",
    "sunrise-client-render",
    "sunrise-world",
    "sunrise-items",
    "sunrise-level",
    "sunrise-sim",
)
# The leg that runs the daily rows (tools/ci/ignored-tests/daily-*.txt) that no package shard above runs: the moved
# tests of every other package, and of the shard packages' targets their shard does not build (tools/ci/ignored_tests.py
# daily_rows_groups). Coverage, not speed: without it those tests would run nowhere. Main's full matrix only.
DAILY_ROWS = "daily-rows"
# The leg that runs each workspace's daily rows.
DAILY_ROWS_LEGS = {".": DAILY_ROWS}
# What a package's ignored tests need built that `cargo test` does not build itself, by package: the command the daily
# run (tools/ci/ignored_tests.py) runs before them. sunrise-mod-package's holder tests start the `holder_actor` example
# from beside their test executable (target/debug/examples, the test profile's folder); every one of them is a daily
# row, so `verify` builds nothing for them.
PREPARE = {
    "sunrise-mod-package": ["cargo", "build", "--locked", "--profile", "test", "-p", "sunrise-mod-package",
                            "--features", "runtime-process", "--example", "holder_actor"],
    # tests/shader_fuzz_cli.rs drives the `shader_fuzz` example through its command line.
    "sunrise-render-shaderpack": ["cargo", "build", "--locked", "--profile", "test", "-p", "sunrise-render-shaderpack",
                                  "--example", "shader_fuzz"],
}
# A package too big for one test process's memory runs as several legs, `PACKAGE#NofM` (tools/ci/ignored_tests.py
# gives each test to exactly one leg). sunrise-client's single process was killed (signal 9) on three main runs;
# after the gameplay split it still holds ~4,437 tests (4,114 ignored game-boot/resource tests), so it keeps 2 legs,
# whereas each gameplay crate (<900 tests each) fits comfortably in a single leg on dyna-large.
LEGS = {"sunrise-client": 2}

# Files at the repository root that every package's build reads.
ROOT_WIDE_FILES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml"}
# The crates directory whose sources build more than one of the shard packages.
SHARED_SOURCES = {"sunrise-client": ("sunrise-client", "sunrise-client-lib")}
# Every Cargo workspace whose tests the daily rows may name, by folder: the game's.
WORKSPACES = (".",)


def expand(packages):
    """The matrix entries for these shard packages: a package with legs becomes one entry per leg."""
    return [
        f"{package}#{number}of{LEGS[package]}" if package in LEGS else package
        for package in packages
        for number in range(1, LEGS.get(package, 1) + 1)
    ]


@lru_cache(maxsize=1)
def workspace_members():
    """{package: (its folder, its workspace's folder)} for every member of WORKSPACES, relative to the root."""
    members = {}
    for workspace in WORKSPACES:
        table = tomllib.loads((ROOT / workspace / "Cargo.toml").read_text())
        for member in ([workspace] if "package" in table else []) + [
                Path(workspace, folder).as_posix() for folder in table["workspace"].get("members", ())]:
            name = tomllib.loads((ROOT / member / "Cargo.toml").read_text())["package"]["name"]
            members[name] = (member, workspace)
    return members


def daily_rows_leg(package):
    """The leg that runs `package`'s daily rows that no package shard runs (DAILY_ROWS_LEGS, by its workspace)."""
    return DAILY_ROWS_LEGS[workspace_members()[package][1]]


def workspace_targets():
    """Each member's test targets as the daily rows name them, in every workspace (WORKSPACES):
    {package: {"lib", "bin:NAME", "test:NAME"}}.

    Read from the manifests and Cargo's default layout (src/lib.rs, src/main.rs, src/bin/, tests/), not by building."""
    targets = {}
    for member, _ in workspace_members().values():
        folder = ROOT / member
        table = tomllib.loads((folder / "Cargo.toml").read_text())
        name = table["package"]["name"]
        found = set()
        if "lib" in table or (folder / "src/lib.rs").is_file():
            found.add("lib")
        found.update(f"bin:{entry['name']}" for entry in table.get("bin", []))
        found.update(f"test:{entry['name']}" for entry in table.get("test", []))
        if table["package"].get("autobins", True):
            if (folder / "src/main.rs").is_file():
                found.add(f"bin:{name}")
            found.update(f"bin:{path.stem}" for path in (folder / "src/bin").glob("*.rs"))
            found.update(f"bin:{path.parent.name}" for path in (folder / "src/bin").glob("*/main.rs"))
        if table["package"].get("autotests", True):
            found.update(f"test:{path.stem}" for path in (folder / "tests").glob("*.rs"))
            found.update(f"test:{path.parent.name}" for path in (folder / "tests").glob("*/main.rs"))
        targets[name] = found
    return targets


def workspace_dependencies():
    """Each workspace member's dependency names (its manifest's dependencies and dev-dependencies)."""
    dependencies = {}
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        table = tomllib.loads(manifest.read_text())
        names = set()
        for section in ("dependencies", "dev-dependencies"):
            names.update(table.get(section, {}))
        dependencies[table["package"]["name"]] = names
    return dependencies


def depends_on(start, target, dependencies):
    """Whether `target` is `start` itself or a transitive dependency of `start`."""
    seen, todo = {start}, [start]
    while todo:
        current = todo.pop()
        if current == target:
            return True
        for name in dependencies.get(current, ()):
            if name not in seen:
                seen.add(name)
                todo.append(name)
    return False


def select(paths, dependencies):
    """The shard packages whose tests a changed path set can affect."""
    if not rust_changes.needs_rust(paths):
        return []
    owners = set()
    for path in paths:
        if not (path.startswith("docs/") or path.endswith(".md")):
            if ("/" not in path and path in ROOT_WIDE_FILES) or path.startswith(".cargo/"):
                return list(SHARDS)
            if path.startswith("crates/") and path.count("/") >= 2:
                crate = path.split("/")[1]
                owners.update(SHARED_SOURCES.get(crate, (crate,)))
            else:
                return list(SHARDS)  # no crate owns it: run everything, as the single job did
    return [
        package for package in SHARDS
        if any(depends_on(package, owner, dependencies) for owner in owners)
    ]


if __name__ == "__main__":
    arguments = sys.argv[1:]
    every = arguments[:2] == ["--matrix", "--all"]
    selected = len(arguments) == 3 and arguments[0] == "--matrix"
    if every:
        shards = [*expand(SHARDS), *DAILY_ROWS_LEGS.values()]
    elif selected:
        shards = expand(select(rust_changes.changed_paths(arguments[1], arguments[2]), workspace_dependencies()))
    else:
        sys.exit(__doc__)
    print("matrix=" + json.dumps({"shard": shards}))
