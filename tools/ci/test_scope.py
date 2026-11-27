#!/usr/bin/env python3
"""The crates a pull request's `cargo test` must cover, for GITHUB_OUTPUT.

Prints `packages=-p a -p b ...`, or `packages=` when the whole workspace must run (main and nightly never call this).
A package is selected when a changed file is in its directory, when it depends (normal, dev or build) on a selected one,
or when it reads the changed file at run time from outside its own directory (READERS: tests read other crates' files).
A change outside every package directory, such as Cargo.lock, .github/ or tools/, selects the whole workspace.
"""

import argparse
import json
import os
import posixpath
import re
import subprocess
import sys
from pathlib import Path

# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(Path(__file__).resolve().parents[2]))
from tools.ci.rust_changes import changed_paths  # noqa: E402

# Files and folders a package reads from outside its own directory (tests at run time, build scripts and
# include_str!/include_bytes! at compile time). A changed path equal to or under an entry selects the package, as if its
# own files had changed: whatever depends on it is selected too.
# tests/test_ci_test_scope.py scans crates/ and xtask/ and fails on a read missing here. Entries outside every package
# directory only document the read: a change there selects the whole workspace.
SUNRISE_CLIENT = [
    # The build script and the native hook tests read the hook targets' sources.
    "crates/sunrise-client-render/src", "crates/sunrise-world/src", "crates/sunrise-items/src",
    "docs/api/launch-reference.md", "docs/sunrise-ctl-commands.md", "docs/modding/examples/src/worldgen.rs",
    "docs/parity/block-entity-client-ticks.jsonl", "docs/parity/random-chance-seeded.jsonl",
    "research-lock.json", "tests/fixture_support/item_resource_properties.rs", "tests/fixtures",
    # mobs::render's compile_part tests compile the fn-replace-check `lower` fixture's helpers (`include!`).
    "tools/ci/fn-replace-check/lower/src",
]
# The generator scans these to write the mod API and the host boundary: its own sources, the vocabulary, the
# engine crates holding hook targets and their manifests.
MOD_API_GEN = [
    "Cargo.lock", "crates/sunrise-assets/src", "crates/sunrise-client", "crates/sunrise-client-gameplay",
    "crates/sunrise-game-data/src", "crates/sunrise-client-render", "crates/sunrise-core/src",
    "crates/sunrise-gameplay", "crates/sunrise-gameplay-common", "crates/sunrise-items",
    "crates/sunrise-mod-api", "crates/sunrise-mod-api-macros/src/lib.rs", "crates/sunrise-mod-package/src",
    "crates/sunrise-mod-render", "crates/sunrise-mod-runtime/src", "crates/sunrise-mod-vocab/behaviours",
    "crates/sunrise-mod-vocab/src", "crates/sunrise-sim/src", "crates/sunrise-world",
    "tools/layering/mod_vocab_state.json",
]
READERS = {
    "cargo-sunrise-mod": [
        "Cargo.lock", "crates/sunrise-attestation/tests/fixtures", "crates/sunrise-mod-api",
        "crates/sunrise-mod-package/src/sdk/fixture.tar.xz",
        # The page checks read the package crate's screenshot fixtures.
        "crates/sunrise-mod-package/tests/fixtures",
        "crates/sunrise-mod-package/tests/index", "crates/sunrise-mod-vocab/changes/debt-re-wl-re-wl.toml",
        "docs/modding/src/how-to/native-mods.md", "tools/ci/rust-version",
        # The installed-client daily builds and installs the fn-replace-check `lower` fixture.
        "tools/ci/fn-replace-check/lower",
    ],
    # The pinned Rust, the mods library's template and the packager's game/SDK lock pair.
    "sunrise-native-build": [
        "crates/sunrise-client/src/native_mod_entry.rs.template", "tools/ci/rust-version",
        "tools/fixtures/sdk-git-patches",
    ],
    # The pipeline bench example links the bench mod's source in.
    "sunrise-mod-host": ["tools/fixtures/mod-pipeline-bench"],
    "sunrise-client": SUNRISE_CLIENT,
    # Shares sunrise-client's sources, so its unit tests run the same reads.
    "sunrise-client-lib": SUNRISE_CLIENT,
    "sunrise-gameplay-common": [
        "crates/sunrise-gameplay/src", "crates/sunrise-mod-vocab/src/hooks", "crates/sunrise-world/src",
        "docs/api/launch-reference.md", "docs/modding/examples/src/worldgen.rs",
    ],
    "sunrise-gameplay": ["crates/sunrise-mod-vocab/src/hooks", "tests/fixtures"],
    "sunrise-client-gameplay": ["crates/sunrise-mod-vocab/src/hooks"],
    "sunrise-parity-fixtures": [*SUNRISE_CLIENT, "crates/sunrise-gameplay-common/src/game/parity_fixtures.rs"],
    "sunrise-sim": [
        "crates/sunrise-client/tests/fixtures/advancement-icons/data",
        "crates/sunrise-client/tests/fixtures/dialog_codec_cases.json",
        "docs/parity/random-chance-seeded.jsonl",
    ],
    # lang.rs embeds the bundled language files.
    "sunrise-game-data": ["crates/sunrise-client/assets/sunrise/lang"],
    "sunrise-client-render": [
        "crates/sunrise-client/assets/sunrise/lang", "crates/sunrise-client/assets/sunrise/textures/gui",
        # Its WGSL validation compiles the shader pack crate's shaders too.
        "crates/sunrise-render-shaderpack/src",
        "tools/light_storage_scenario.txt", "tools/light_storage_witness.txt",
    ],
    # The daily Character comparator reads the original public-JDK API helper.
    "sunrise-core": ["tools/JavaCharacterTables.java"],
    "sunrise-game": ["packaging/windows/exe.rs", "packaging/macos/exe.rs"],
    # The ignored-test inventory reads the game's modules and the vocabulary's export fragments.
    "sunrise-game-discovery": [
        "crates/sunrise-client/src", "crates/sunrise-gameplay-common/src", "crates/sunrise-gameplay/src",
        "crates/sunrise-client-gameplay/src", "crates/sunrise-mod-vocab/src",
    ],
    "sunrise-mod-api": MOD_API_GEN,
    "sunrise-mod-api-gen": [
        *MOD_API_GEN,
        "crates/sunrise-mod-surface/tests/fixtures/occultism-0.9.0-alpha.1/uses.json",
        "crates/sunrise-mod-vocab/changes",
        "crates/sunrise-mod-vocab/extension-tags",
    ],
    "sunrise-mod-api-gen-tests": MOD_API_GEN,
    # content_keys' source guard searches these crates' sources for the files that define declaration types.
    "sunrise-mod-package": [
        "Cargo.lock", "crates/sunrise-core/src", "crates/sunrise-mod-vocab/src", "docs/developer-install.md",
        "tools/mixins/narrow-reference-rule.patch",
    ],
    "sunrise-mod-render": MOD_API_GEN,
    "sunrise-official": ["packaging/windows/exe.rs", "packaging/macos/exe.rs"],
    "sunrise-perf": ["crates/sunrise-perf-format/tests/fixtures", "tests/fixtures/pacing_slow_frames.hzperf"],
    "sunrise-port-pack": ["docs/modpacks/atm10-mods.csv"],
    "sunrise-prepare": [
        "crates/sunrise-gameplay-common/src/game/runtime/launch_api.rs", "mods/sunrise/src/test/java", "research-lock.json",
    ],
    # The RSA key tests verify the client's signed-textures fixture.
    "sunrise-protocol": ["crates/sunrise-client/src/cut_layout/game/network/remote/fixtures/signed_textures.json"],
    "sunrise-release-sign": [
        "tests/install_ps1_signature.ps1", "tools/ci/sign_native_build.sh", "tools/ci/sign_release.sh",
    ],
    "sunrise-server": [
        "crates/sunrise-theme/fonts", "crates/sunrise-theme/icons", "docs/api/server-reference.md",
    ],
    # Its daily stable-release test drives the release gate's Python through the real surface differ.
    "sunrise-stage": [
        "tests/stable_mod_api_integration.py", "tests/test_stable_mod_api.py", "tools/ci/nightly.py",
        "tools/release_train.py",
    ],
    # Commands (tokens, icons, package), not tests: listed so the scan finds nothing unexplained.
    "xtask": [
        "crates/sunrise-client/src", "crates/sunrise-mod-vocab/src/hooks", "crates/sunrise-perf/web",
        "crates/sunrise-prepare", "crates/sunrise-theme", "docs/media/brand", "mods/sunrise/src/test/java", "packaging",
        "research-lock.json", "tools", "LICENSES/MIT-Horizon.txt",
    ],
}
# What a package's tests only read at run time and nothing else uses: the package is selected, its dependents are not.
# sunrise-game-data's lang.rs tests read the sources of every crate for the language keys the code names.
SCANNED = {"sunrise-game-data": ["crates"]}
# Literals the scan takes for a walk up to crates/ that are not one (a ".." among path-traversal checks).
# "machines/.." in a test's sample paths, "mods/" in an instance's file names: no read either.
NOT_READS = {
    "sunrise-client": {"crates"}, "sunrise-client-lib": {"crates"}, "sunrise-parity-fixtures": {"crates"},
    "sunrise-gameplay-common": {"crates"}, "sunrise-sim": {"crates"},
    "sunrise-level": {"crates/sunrise-client-render"},
    "sunrise-mod-package": {"crates"},
    # The retained-SDK test writes a synthetic render.rs into its temporary archive, not this checkout.
    "cargo-sunrise-mod": {"crates/sunrise-mod-render/src/render.rs"},
    "sunrise-mod-vocab": {"."}, "sunrise-port-pack": {"mods"}, "sunrise-prepare": {"crates"},
    # A package's source-file names (`package::source_files`), never this checkout's lock.
    "sunrise-mod-source": {"Cargo.lock"},
    # shader_fuzz's `..` path parts, in the packs it writes and reads, beside its own seed pack.
    "sunrise-render-shaderpack": {"crates"},
    # Server init uses the filename as a checkout-root sentinel; only sunrise-prepare reads its contents.
    "sunrise-server": {"research-lock.json"},
    # Paths inside a staged game source or a test's scratch folder, never this checkout's files.
    "sunrise-native-build": {
        "Cargo.lock", "crates/sunrise-client/Cargo.toml", "crates/sunrise-client/build.rs", "crates/sunrise-client/src",
        "crates/sunrise-client/src/native_main.rs.template", "crates/sunrise-core/src", "crates/sunrise-core/src/storage.rs",
        "crates/sunrise-mod-api", "crates/sunrise-mod-host", "crates/sunrise-mod-host/Cargo.toml", "crates/sunrise-official",
        "crates/sunrise-official/Cargo.toml",
        # The discovery tests' scratch game source (`discover_tests.rs`).
        "crates/sunrise-client/src/entry.rs", "crates/sunrise-gameplay-common/src/game.rs", "crates/sunrise-client/src/lib.rs",
        "crates/sunrise-gameplay/src", "crates/sunrise-gameplay/src/game.rs",
        "crates/sunrise-mod-api-gen", "crates/sunrise-mod-api-gen/internals.txt",
    },
}


def under(path, folder):
    return path == folder or path.startswith(folder + "/")


class Workspace:
    def __init__(self, metadata):
        root = metadata["workspace_root"]

        def relative(path):
            # Repository paths in `/` form on every platform, as git and under() name them.
            return Path(os.path.relpath(os.path.normpath(path), root)).as_posix()

        names = [package["name"] for package in metadata["packages"]]
        home = {p["name"]: relative(os.path.dirname(p["manifest_path"])) for p in metadata["packages"]}
        self.dirs = {name: {home[name]} for name in names}
        for package in metadata["packages"]:
            for target in package["targets"]:
                source = relative(target["src_path"])
                # sunrise-client-lib builds sunrise-client's files: all of that package's folder counts.
                folder = max((d for d in home.values() if under(source, d)), key=len, default=None)
                self.dirs[package["name"]].add(folder or posixpath.dirname(source))
        self.home = home
        self.dependents = {name: set() for name in names}
        for package in metadata["packages"]:
            for dependency in package["dependencies"]:
                if dependency["name"] in self.dependents:
                    self.dependents[dependency["name"]].add(package["name"])

        # The stable mod API auditor reads every workspace package manifest, then indexes library
        # sources reachable from the facade and its three seam crates. Discover both sets from
        # Cargo metadata so adding a workspace member or an internal dependency needs no scope-list edit.
        packages = {package["name"]: package for package in metadata["packages"]}
        self.mod_api_gen_inputs = {relative(os.path.join(root, "Cargo.toml"))}
        self.mod_api_gen_inputs.update(relative(package["manifest_path"]) for package in metadata["packages"])
        pending = ["sunrise-mod-api", "sunrise-mod-vocab", "sunrise-core", "sunrise-mod-runtime"]
        indexed = set()
        while pending:
            name = pending.pop()
            if name in indexed or name not in packages:
                continue
            indexed.add(name)
            package = packages[name]
            for dependency in package["dependencies"]:
                # OutsideSeam follows normal dependencies (including target-specific ones), not
                # build or dev dependencies. Cargo metadata resolves workspace package names here.
                if dependency.get("kind") in (None, "normal") and dependency["name"] in packages:
                    pending.append(dependency["name"])
            for target in package["targets"]:
                if {"lib", "proc-macro"}.intersection(target.get("kind", ())):
                    self.mod_api_gen_inputs.add(posixpath.dirname(relative(target["src_path"])))

    def owners(self, path):
        return {name for name, dirs in self.dirs.items() if any(under(path, d) for d in dirs)}

    def readers(self, path, table=None):
        dynamic = table is None
        table = READERS if table is None else table
        readers = {name for name, paths in table.items()
                   if name in self.dirs and any(under(path, p) for p in paths)}
        if dynamic and "sunrise-mod-api-gen" in self.dirs and any(
            under(path, p) for p in self.mod_api_gen_inputs
        ):
            readers.add("sunrise-mod-api-gen")
        return readers

    def scope(self, paths):
        """The packages to test, or None for the whole workspace."""
        chosen, scanners = set(), set()
        for path in paths:
            chosen |= self.readers(path)
            scanners |= self.readers(path, SCANNED)
            owners = self.owners(path)
            chosen |= owners
            # Documentation nobody builds from, unless a test reads it (READERS) or a crate owns it.
            inert = (path.startswith("docs/") or path.endswith(".md")) and not path.startswith(".github/")
            if not owners and not inert:
                return None
        todo = list(chosen)
        while todo:
            for dependent in self.dependents[todo.pop()] - chosen:
                chosen.add(dependent)
                todo.append(dependent)
        chosen |= scanners
        return None if chosen >= set(self.dirs) else chosen


def flags(chosen):
    return "packages=" + " ".join(f"-p {name}" for name in sorted(chosen or ()))


def cargo_metadata():
    return json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"]))


LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"')
ROOT_FILES = {"native-hooks.json", "research-lock.json", "Cargo.lock"}
ROOT_JOIN = re.compile(r"^/?(\.\./?)+$")


def reads(workspace):
    """{package: {path: where}}: tracked paths outside the package that its sources name as string literals.

    A literal is resolved against the package folder (run time, CARGO_MANIFEST_DIR) and its file's folder (include_str!).
    "../.." in a file with CARGO_MANIFEST_DIR is the repository root or crates/, reported as "." or "crates".
    """
    tracked = subprocess.check_output(["git", "ls-files", "-z"]).decode().split("\0")
    known = {"."} | {t for path in tracked if path for t in [path, *(p.as_posix() for p in Path(path).parents)]}
    found = {}
    for file in tracked:
        owners = workspace.owners(file) if file.endswith(".rs") else ()
        if not owners:
            continue
        text = Path(file).read_text(errors="replace")
        manifest = "CARGO_MANIFEST_DIR" in text
        for name in owners:
            home = workspace.home[name]
            for literal in set(LITERAL.findall(text)):
                stripped = literal.strip("/")
                if re.search(r"[\s{}]|://", literal):
                    continue
                if ROOT_JOIN.match(literal):
                    # A walk up from the manifest folder: the repository root, or crates/ for its siblings.
                    candidates = {posixpath.normpath(posixpath.join(home, stripped))} & {".", "crates"} if manifest else set()
                    candidates = {c for c in candidates if c != home}
                elif stripped.strip("./"):
                    candidates = {posixpath.normpath(posixpath.join(base, stripped)) for base in (home, posixpath.dirname(file))}
                    if "/" in literal and not literal.startswith(".") or literal in ROOT_FILES:
                        candidates.add(posixpath.normpath(stripped))
                    candidates = {c for c in candidates if c in known and not c.startswith("..")
                                  and not any(under(c, d) or under(d, c) for d in workspace.dirs[name])}
                else:
                    continue
                for candidate in candidates:
                    found.setdefault(name, {})[candidate] = f"{file}: {literal}"
    return found


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("head")
    args = parser.parse_args()
    chosen = Workspace(cargo_metadata()).scope(changed_paths(args.base, args.head))
    print(flags(chosen))
