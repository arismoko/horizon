#!/usr/bin/env python3
"""The prebuilt game (#589): sunrise-client as a Rust dylib plus every crate's metadata, so the launcher builds
native trait mods against it with rustc alone, in seconds, without the game's source or a game build.

    python3 tools/prebuilt_game.py build OUT [--server]   # build and collect the folder, write sunrise-prebuilt.json
    python3 tools/prebuilt_game.py smoke OUT              # build the starter mod and the game executable against OUT only
    python3 tools/prebuilt_game.py start OUT              # start smoke's executable, as installed, as far as no game data allows
    python3 tools/prebuilt_game.py exports OUT            # each DLL's exported names, toward Windows' limit
    python3 tools/prebuilt_game.py pack OUT DIST          # the SDK's assets from OUT: runtime-, api- and
                                                          # engine-<target>[-server].tar.xz, build-data.tar.xz
                                                          # and surface.json, in DIST; for the client side also
                                                          # tool-<target>.tar.xz (cargo-sunrise-mod); --stage-tool the
                                                          # sunrise-stage `build` compiled, instead of compiling it;
                                                          # --programs also programs-<target>[-server].tar.xz
    python3 tools/prebuilt_game.py tool DIST              # just this computer's tool-<target>.tar.xz, as `pack` makes it
                                                          # (the rolling mod-tool release, tools/ci/mod_tool.py)
    python3 tools/prebuilt_game.py gather LEGS DIST       # the release folder from every packing leg's DIST, once
                                                          # their build data and surface are checked equal
    python3 tools/prebuilt_game.py index DIST [--nightly NAME] [--previous BUILD DIFF]  # DIST/sdk.json

Each OS's folder is built on that OS: the proc-macro libraries in it run inside the player's own rustc.
The folder holds compiler outputs and game-owned registry patch crates, never vanilla data; `build` refuses any other file.
"""

import argparse
from concurrent.futures import ProcessPoolExecutor
import faulthandler
import hashlib
import io
import json
import lzma
import os
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import time
import urllib.request
from pathlib import Path

if __package__:
    from .ci.surface_report import source_layout
else:
    from ci.surface_report import source_layout


ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools import file_lock, lab
PROFILE = "release"
# What rustc writes for a dylib or proc macro on each OS, plus metadata and the MinGW import libraries.
SUFFIXES = (".rmeta", ".so", ".dll", ".dll.a", ".dylib")
IDENTITY = "sunrise-prebuilt.json"
# The crate the game's library is built from; its build resolves what mods compile against.
GAME = "sunrise-mod-host"
OFFICIAL = "sunrise-official"
LOCK = "Cargo.lock"
# `git archive` of the whole commit takes seconds on every runner; past this it is stuck, not slow.
ARCHIVE_SECONDS = 600
# While the commit is copied out, every thread's stack is printed this often: a stall there names its line.
COPY_WATCH_SECONDS = 120
# Where `smoke` installs the executable it builds, as the launcher does, for `start`: beside OUT, never inside it,
# since the archive is OUT alone.
SMOKE = "prebuilt-smoke"
# Where `smoke` installs the same executable without selected mods, for `start`.
SMOKE_API = "prebuilt-smoke-api"
# SUNRISE_PREBUILT_TARGET builds for another target than this computer's (a cross build): the platform below is the target's.
CROSS_TARGET = os.environ.get("SUNRISE_PREBUILT_TARGET")
PLATFORM = ("darwin" if CROSS_TARGET.endswith("-apple-darwin") else "win32" if "-windows-" in CROSS_TARGET else "linux") if CROSS_TARGET else sys.platform
EXECUTABLE = "sunrise-client.exe" if PLATFORM == "win32" else "sunrise-client"
# The furthest a game started without its prepared files gets (`loading_screen::preflight_resources`): past loading the
# game's libraries and the mods' runtime environment (`entry::main_with`), where a split allocator aborts (#3877).
NO_RESOURCES = "Sunrise's prepared game files are missing"
# The MinGW runtime the game's GNU DLLs import on Windows, installed beside the executable.
MINGW_RUNTIME = ("libgcc_s_seh-1.dll", "libwinpthread-1.dll")
# Windows' STATUS_DLL_NOT_FOUND, the exit status of a process whose imports could not all be loaded.
DLL_NOT_FOUND = 0xC0000135
# The launcher's generated `main.rs` comes from the game it links against.
TEMPLATE = "native_main.rs.template"
NATIVE_MODS_TEMPLATE = "native_mod_entry.rs.template"
API_MAIN = ROOT / "crates/sunrise-client/src/native_main.rs.template"
# The fixed executable folder in the runtime slice.
RUNTIME = "bin"
# The required inputs (`sunrise_mod_package::sdk::Inputs`): `members` lists what each slice must hold.
INPUTS = ROOT / "crates/sunrise-mod-package/src/sdk/inputs.json"
# The game libraries outside the API closure, in the runtime archive.
GAME_LIBRARIES = ("sunrise_mod_host", "sunrise_official")
# The lowest of the libraries the prebuilt game is split into: every other one links it.
SDK_BASE = "sunrise-core"
# Source files the folder carries besides compiler outputs.
CARRIED = {"Cargo.toml": None, IDENTITY: None, LOCK: ROOT / LOCK, TEMPLATE: ROOT / "crates/sunrise-client/src" / TEMPLATE,
           NATIVE_MODS_TEMPLATE: ROOT / "crates/sunrise-client/src" / NATIVE_MODS_TEMPLATE}
# The one Git source the game can ship as SDK-owned path patches.
WGPU_FORK = "https://github.com/sunrise-org/sunrise-wgpu"
WGPU_PATCHES = ("naga", "naga-types", "wgpu-hal", "wgpu-naga-bridge", "wgpu-types")
WGPU_ROOT_FILES = ("Cargo.toml", "LICENSE.APACHE", "LICENSE.MIT", "SUNRISE.md")
# These dev-only workspace members live inside naga but are not part of its patch payload.
WGPU_OMIT = ("naga/fuzz/", "naga/hlsl-snapshots/", "naga/xtask/")
# Native libraries the build scripts point the final link at (the `windows` crates' MinGW import libraries):
# cargo passes their folders to rustc, a plain rustc gets them from here (`-L native=OUT/native`).
NATIVE = "native"
NATIVE_SUFFIXES = (".a", ".lib")
# The staged internals signatures, checked against a mod's declared access list.
INTERNALS = "internals.json"
# The published loops (`#[sunrise_loop]`), which a mod's `[[native.loop]]` rows name.
LOOPS = "loops.json"
# The function index: where each function a mixin may target is, and whether it takes a replacement; the
# launcher's patch step reads it instead of walking the game's source. Only the engine route patches functions, so it
# is in the `engine` slice, not with the rest of the build data in `api`.
FUNCTIONS = "functions.json"
# The SDK's build data: each file opens with `schema` and `build.commit`.
BUILD_DATA = (FUNCTIONS, INTERNALS, LOOPS)
# The build data the API route reads, in the `api` slice.
API_BUILD_DATA = (INTERNALS, LOOPS)
BUILD_DATA_SCHEMA = 1
BUILD_DATA_ARCHIVE = "build-data.tar.xz"
# The surface export (`sunrise-stage surface`), an SDK asset of its own.
SURFACE = "surface.json"
# xz's default preset: the dictionary covers the large metadata files; the publish legs' time is measured by PR 5.
XZ_PRESET = 6
# The SDK's `tool` asset: `cargo sunrise-mod native-build` compiles native mods as the launcher does.
TOOL = "cargo-sunrise-mod"
TOOL_PROGRAM = TOOL + (".exe" if PLATFORM == "win32" else "")
# The SDK's `programs` asset: what the player folder holds besides the game program, so the runtime slice's game installs
# from the SDK alone.
PROGRAMS = "programs"


LAUNCHER_FILES = ("src/game_app.rs", "assets/icons/sunrise.icns")  # game_app.rs include_bytes!es the icon
# The sunrise-launcher commit the bundle preparation is fetched at (tools/ci/launcher_pin.sh reads the same file).
LAUNCHER_PIN = ROOT / "tools/ci/launcher-pin"


def launcher_pin() -> str:
    pin = LAUNCHER_PIN.read_text().strip()
    if not re.fullmatch(r"[0-9a-f]{40}", pin):
        sys.exit("tools/ci/launcher-pin must be a full 40-character lowercase commit SHA")
    return pin


def fetch_launcher_files(destination: Path, pin: str, fetch=lambda url: urllib.request.urlopen(url, timeout=60).read()) -> Path:
    """The files of the bundle preparation at the sunrise-launcher commit `pin`, in their own layout under `destination`
    (game_app.rs reads the icon by the relative path ../assets/icons/)."""
    for name in LAUNCHER_FILES:
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(fetch(f"https://raw.githubusercontent.com/sunrise-org/sunrise-launcher/{pin}/{name}"))
    return destination


def run(command: list[str], **kwargs) -> str:
    print("+", " ".join(command), flush=True)
    # No tool here reads standard input; a child left holding the runner's would wait on it with no output.
    return subprocess.run(command, check=True, text=True, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                          **kwargs).stdout


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def rustc_host() -> str:
    verbose = run(["rustc", "-vV"])
    return next(line.split(": ", 1)[1] for line in verbose.splitlines() if line.startswith("host: "))


def package_format(root: Path) -> int:
    """The package layout's one number, from its one place (`sdk::PACKAGE_FORMAT` includes the same file)."""
    text = (root / "crates/sunrise-mod-package/src/sdk/package-format").read_text().rstrip()
    if not re.fullmatch(r"[0-9]+", text):
        raise ValueError(f"sdk/package-format must hold one number, not {text!r}")
    return int(text)


def tracked_files(root: Path, commit: str, paths: tuple[str, ...] = ()):
    """Yield the name, committed bytes and archive mode of each tracked regular file under `paths` (all if empty),
    as GitHub's archive of the commit serves them, whatever line endings the working tree was checked out with (Git
    for Windows' core.autocrlf) and whatever it has that is not committed. Links are left out, as the launcher's
    download leaves them out."""
    # `git archive` converts line endings as a checkout would (core.autocrlf, core.eol); both are turned off so the
    # bytes are the commit's on every runner, and tar.umask keeps Git's executable bit. Git writes a file, read once
    # it has exited, with no pipe between the two processes: Windows SDK legs streaming it through a pipe sat silent
    # here until their timeout. A git that never ends fails the step with a traceback after ARCHIVE_SECONDS.
    with tempfile.TemporaryDirectory(prefix="archive-") as folder:
        tar = Path(folder) / "commit.tar"
        run(["git", "-c", "core.autocrlf=false", "-c", "core.eol=lf", "-c", "tar.umask=002", "archive",
             "--format=tar", "-o", str(tar), commit, *(["--", *paths] if paths else [])],
            cwd=root, timeout=ARCHIVE_SECONDS)
        with tarfile.open(tar, mode="r:") as archive:
            for member in archive:
                if member.isfile():
                    yield member.name, archive.extractfile(member).read(), member.mode


def tracked_blobs(root: Path, commit: str, paths: tuple[str, ...] = ()):
    """The bytes GitHub's archive serves, which the launcher downloads and checks against the function index."""
    for name, data, _ in tracked_files(root, commit, paths):
        yield name, data


def head() -> str:
    return run(["git", "rev-parse", "HEAD"], cwd=ROOT).strip()


def sync_stage(source: Path, destination: Path) -> None:
    """Mirror directories, file bytes and POSIX execute bits, leaving symlinks out.
    Don't rewrite identical bytes: Cargo uses mtimes for path crate freshness."""
    if destination.is_symlink():
        destination.unlink()
    destination.mkdir(exist_ok=True)
    entries = {path.name: path for path in source.iterdir() if not path.is_symlink()}
    for path in destination.iterdir():
        incoming = entries.get(path.name)
        if path.is_symlink():
            path.unlink()
        elif incoming is None or path.is_dir() != incoming.is_dir():
            if path.is_dir():
                shutil.rmtree(path)
            else:
                path.unlink()
    for path in entries.values():
        dest = destination / path.name
        if path.is_dir():
            sync_stage(path, dest)
        else:
            data = path.read_bytes()
            if not dest.exists() or dest.read_bytes() != data:
                dest.write_bytes(data)
            if os.name == "posix":
                mode = stat.S_IMODE(dest.stat().st_mode)
                expected = (mode & ~0o111) | (path.stat().st_mode & 0o111)
                if mode != expected:
                    dest.chmod(expected)


def stage(target_dir: Path, stage_tool: Path | None = None, *, sdk_dylibs: bool = False) -> Path:
    """Stage HEAD's tracked files on an exclusively owned target. Run the staging pass (#589) on a fresh copy so
    it can edit the source and emit build data, then sync the result. SDK builds also stage dylib manifests."""
    setup_started = time.monotonic()
    faulthandler.dump_traceback_later(COPY_WATCH_SECONDS, repeat=True)
    target_dir.mkdir(parents=True, exist_ok=True)
    staged = target_dir / "stage"
    commit = head()
    with tempfile.TemporaryDirectory(prefix="next-", dir=target_dir) as scratch:
        scratch = Path(scratch)
        for name, data, mode in tracked_files(ROOT, commit):
            path = scratch / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            if os.name == "posix" and mode & 0o111:
                path.chmod(stat.S_IMODE(path.stat().st_mode) | (mode & 0o111))
        arguments = [str(scratch), str(scratch / INTERNALS), commit]
        if stage_tool is None:
            # Build the tool from a stable copy of the unmodified commit, not the disposable rewrite tree.
            source = target_dir / "stage-in"
            sync_stage(scratch, source)
            command = ["cargo", "run", "--locked", "--release", "-q", "-p", "sunrise-stage",
                       "--bin", "sunrise-stage", "--"]
            env = {**os.environ, "CARGO_TARGET_DIR": str(target_dir / "tools")}
            label = "Stage release build and execution"
        else:
            source = scratch
            command = [str(stage_tool.resolve())]
            env = None
            label = "Stage binary execution"
        faulthandler.cancel_dump_traceback_later()
        print(f"stage_phase=copy_setup seconds={time.monotonic() - setup_started:.3f}", flush=True)
        started = time.monotonic()
        print(run(command + arguments, cwd=source, env=env), end="")
        print(f"{label}: {time.monotonic() - started:.3f}s (source copy/setup excluded)", flush=True)

        if sdk_dylibs:
            prepare_sdk_manifests(scratch)
        sync_stage(scratch, staged)
    return staged


def prepare_sdk_manifests(staged: Path) -> None:
    """Build the host and official layer as dylibs in one Cargo graph, and split out the API libraries. With prefer-dynamic, the official layer uses the host's engine state."""
    metadata = json.loads(run(["cargo", "metadata", "--format-version", "1", "--filter-platform", rustc_host()],
                              cwd=staged))
    split = sdk_dylibs(metadata)
    # A warning, not a refusal: this reads declared dependencies of every feature, and rustc links fewer.
    if clash := shared_twice(metadata, split | {GAME, OFFICIAL}):
        print(f"warning: {clash}", file=sys.stderr, flush=True)
    for package in metadata["packages"]:
        name = package["name"]
        root_dylib = name in (GAME, OFFICIAL)
        if not root_dylib and name not in split:
            continue
        if not any(target["kind"] == ["lib"] for target in package["targets"]):
            continue
        manifest = Path(package["manifest_path"])
        if not manifest.is_relative_to(staged):
            continue  # Registry dependencies are owned by Cargo, not rewritten.
        text = manifest.read_text()
        crate_types = '["dylib"]' if root_dylib else '["rlib", "dylib"]'
        if "[lib]" in text:
            text = text.replace("[lib]", f"[lib]\ncrate-type = {crate_types}", 1)
        else:
            text += f"\n[lib]\ncrate-type = {crate_types}\n"
        manifest.write_text(text)


def normal_dependencies(metadata: dict) -> dict[str, list[str]]:
    """Each package's normal dependencies that link into it, by package ID: never a proc macro, which runs in the
    compiler and keeps its own dependencies inside its library."""
    macros = {package["id"] for package in metadata["packages"]
              if any("proc-macro" in target["kind"] for target in package["targets"])}
    return {package["id"]: [] for package in metadata["packages"]} | {
        node["id"]: [dependency["pkg"] for dependency in node["deps"] if dependency["pkg"] not in macros
                     and any(kind["kind"] is None for kind in dependency["dep_kinds"])]
        for node in metadata["resolve"]["nodes"]}


def sdk_dylibs(metadata: dict) -> set[str]:
    """The names of the API closure's crates split out of the game as libraries of their own: those that stand on
    `SDK_BASE`. A library holds every crate below it that no library it links
    holds, so another crate beside the base would hold a second copy of whatever both
    use, and rustc refuses to link two. Such a crate stays inside the first library above it."""
    dependencies = normal_dependencies(metadata)
    closure = api_closure(metadata)
    base = {package["id"] for package in metadata["packages"] if package["name"] == SDK_BASE}

    def stands_on_base(package_id: str) -> bool:
        pending, seen = [package_id], set()
        while pending:
            current = pending.pop()
            if current in base:
                return True
            if current not in seen:
                seen.add(current)
                pending.extend(dependencies[current])
        return False

    return {package["name"] for package in metadata["packages"]
            if package["id"] in closure and stands_on_base(package["id"])}


def shared_twice(metadata: dict, libraries: set[str]) -> str | None:
    """What is wrong when two of `libraries` (package names) would each hold a copy of one crate, else None.
    A library holds every crate it reaches without passing through another library, except what the libraries
    it links hold."""
    dependencies = normal_dependencies(metadata)
    names = {package["id"]: package["name"] for package in metadata["packages"]}
    ids = sorted(package_id for package_id, name in names.items() if name in libraries)

    def reach(library: str) -> tuple[set[str], set[str]]:
        """What `library` reaches without passing through another library, and every library it links."""
        inside, linked, pending = set(), set(), list(dependencies[library])
        while pending:
            current = pending.pop()
            if current in ids:
                if current not in linked:
                    linked |= {current} | reach(current)[1]
            elif current not in inside:
                inside.add(current)
                pending.extend(dependencies[current])
        return inside, linked

    reached = {library: reach(library) for library in ids}
    holds = {library: inside - set().union(*(reached[other][0] for other in linked))
             for library, (inside, linked) in reached.items()}
    for first in ids:
        for second in ids:
            if first < second and (both := holds[first] & holds[second]):
                return (f"{names[first]} and {names[second]} are separate libraries of the prebuilt game and may "
                        f"each hold a copy of {', '.join(sorted(names[crate] for crate in both))}: rustc refuses to "
                        "link two copies into one program. If the build fails on these crates, one library must "
                        "depend on the other, or stay out of `sdk_dylibs`.")
    return None


def cargo_features(server: bool, platform: str) -> list[str]:
    """The game crate's features for this archive. The archive's std is a shared library with its own
    default-allocator `__rust_alloc`. Only ELF's
    dynamic linker (Linux) resolves every library's `__rust_alloc` to the first definition, the game's mimalloc.
    Windows (#2394) and macOS (#3877) bind each import to the library it was linked against, so a game allocator
    there can leave other libraries and std's own code on std's heap beside it, and a block
    freed on the other heap aborts. So everywhere but Linux the game keeps no allocator of its own and every
    library shares std's (the system heap)."""
    features = ["--no-default-features"] if server else []
    if platform != "linux":
        features += ["--features", "system-allocator"]
    return features


def copy_artifacts(messages: list[dict], deps: Path, host: Path, target_dir: Path) -> dict[str, list[str]]:
    """Keep target metadata separate from the host proc-macros loaded by rustc. Returns each package's copied files,
    relative to the package folder.

    An explicit Cargo --target keeps host build dependencies outside target_dir;
    those libraries are not part of the target SDK. Proc-macros alone run on the host.
    """
    copied: dict[str, list[str]] = {}
    for message in messages:
        if message["reason"] != "compiler-artifact":
            continue
        proc_macro = "proc-macro" in message["target"]["kind"]
        filenames = list(message["filenames"])
        # Cargo does not inventory standalone metadata emitted alongside a dylib.
        for name in message["filenames"]:
            if proc_macro or not any(file.endswith((".so", ".dll", ".dylib")) for file in message["filenames"]):
                continue
            metadata = Path(name).with_suffix(".rmeta")
            if metadata.is_file() and str(metadata) not in filenames:
                filenames.append(str(metadata))
        for name in filenames:
            file = Path(name)
            if not name.endswith(SUFFIXES):
                continue
            if proc_macro:
                if name.endswith((".so", ".dll", ".dylib", ".dll.a")):
                    shutil.copy2(file, host / file.name)
                    copied.setdefault(message["package_id"], []).append(f"{host.name}/{file.name}")
            elif file.is_relative_to(target_dir):
                shutil.copy2(file, deps / file.name)
                copied.setdefault(message["package_id"], []).append(f"{deps.name}/{file.name}")
    return copied


def lower_crate_cfgs(metadata: dict, messages: list[dict], staged: Path, target_dir: Path,
                     target_cfg: set[str]) -> dict[str, list[str]]:
    """The source-indexed target libraries' own cfgs, from this Cargo build, not a resolution
    of the workspace's manifests (which includes other feature activations)."""
    packages = {package["id"]: Path(package["manifest_path"]).parent.name.replace("-", "_")
                for package in metadata["packages"]
                if Path(package["manifest_path"]).is_relative_to(staged / "crates")
                and package["name"] != GAME}
    script_cfg = {message["package_id"]: message["cfgs"] for message in messages
                  if message["reason"] == "build-script-executed"
                  and Path(message["out_dir"]).is_relative_to(target_dir)}
    target_cfg = target_cfg - {"debug_assertions"}
    result = {}
    for message in messages:
        if message["reason"] != "compiler-artifact" or message["package_id"] not in packages:
            continue
        if not {"lib", "rlib", "dylib"}.intersection(message["target"]["kind"]):
            continue
        if not any(Path(name).is_relative_to(target_dir) for name in message["filenames"]):
            continue
        package_id = message["package_id"]
        cfg = target_cfg.union(script_cfg.get(package_id, ()),
                               (f"feature={json.dumps(name, ensure_ascii=False)}" for name in message["features"]))
        if message["profile"]["debug_assertions"]:
            cfg.add("debug_assertions")
        name = packages[package_id]
        cfg = sorted(cfg)
        if result.setdefault(name, cfg) != cfg:
            raise ValueError(f"target library {name} was built under conflicting cfgs")
    return result


def api_closure(metadata: dict) -> set[str]:
    """The package IDs an API-only mod's build reads: sunrise-mod-api and its normal dependencies.
    A proc macro is a leaf, since its own dependencies are inside its library."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [package["id"] for package in metadata["packages"] if package["name"] == "sunrise-mod-api"]
    closure = set()
    while pending:
        package_id = pending.pop()
        if package_id in closure:
            continue
        closure.add(package_id)
        if any("proc-macro" in target["kind"] for target in packages[package_id]["targets"]):
            continue
        pending.extend(dependency["pkg"] for dependency in nodes[package_id]["deps"]
                       if any(kind["kind"] is None for kind in dependency["dep_kinds"]))
    return closure


def slices(metadata: dict, copied: dict[str, list[str]], api_route: list[str]) -> dict[str, list[str]]:
    """Inventory the runtime dynamic libraries and API metadata from Cargo's build records.
    All remaining files belong to the engine slice."""
    names = {package["id"]: package["name"] for package in metadata["packages"]}
    libraries = {crate.replace("_", "-") for crate in GAME_LIBRARIES}
    closure = api_closure(metadata)
    # Each slice unpacks to its own folder: the API route's crates must all be in `api`.
    inside = sorted(names[package_id] for package_id in closure if names.get(package_id) in libraries)
    if inside:
        sys.exit(f"the API closure holds game libraries {inside}, which the api slice cannot carry")
    dynamic = lambda file: file.endswith((".so", ".dylib", ".dll", ".dll.a"))
    runtime = [IDENTITY, *api_route] + [file for package_id, files in copied.items()
                if package_id in closure or names.get(package_id) in libraries
                for file in files if dynamic(file) and not file.startswith("host/")]
    api = [file for package_id in closure for file in copied.get(package_id, [])
           if not dynamic(file) or file.startswith("host/")] + [*API_BUILD_DATA, NATIVE_MODS_TEMPLATE]

    return {"runtime": sorted(set(runtime)), "api": sorted(set(api))}


def official_externs(deps: Path) -> list[str]:
    """Enable the generated entry point's official feature when its layer is in the build."""
    library = dylib(deps, "sunrise_official")
    return ["--cfg", 'feature="official"', "--extern", f"sunrise_official={library}"] if library else []


def api_route(out: Path, host: str) -> list[str]:
    """Build the fixed executable shared by both native routes, once for this SDK."""
    deps = out / "deps"
    folder = out / RUNTIME
    folder.mkdir()
    common = ["--edition", "2024", "-C", "prefer-dynamic", "-C", "opt-level=3", "-L", f"dependency={deps}",
              "-L", f"dependency={out / 'host'}", "-L", f"native={out / NATIVE}"]
    link_args = {"linux": ["-Wl,-rpath,$ORIGIN"], "darwin": ["-Wl,-rpath,@executable_path"]}.get(PLATFORM, [])
    if PLATFORM == "win32":
        # Match packaging/windows/exe.rs at this executable's link, not in SDK metadata.
        link_args = ["/STACK:8388608" if host.endswith("-msvc") else "-Wl,--stack,8388608"]
    # A cross build names the target and the linker Cargo would use, which a bare rustc does not read.
    linker = os.environ.get("CARGO_TARGET_" + host.upper().replace("-", "_") + "_LINKER")
    cross_args = ["--target", host, *(["-C", f"linker={linker}"] if linker else [])] if CROSS_TARGET else []
    run(["rustc", *common, *cross_args, "--crate-type", "bin", "--crate-name", "sunrise_native_game",
         *(flag for arg in link_args for flag in ("-C", f"link-arg={arg}")),
         *extern_args(deps, "sunrise_client"), *extern_args(deps, "mod_api"), *extern_args(deps, "sunrise_mod_package"),
         *extern_args(deps, "sunrise_mod_host"), *official_externs(deps),
         "-o", str(folder / EXECUTABLE), str(API_MAIN)])
    built = sorted(path.relative_to(out).as_posix() for path in folder.iterdir())
    if not {path.name for path in folder.iterdir()} <= api_route_files():
        sys.exit(f"the API route's build left {built}, not only {sorted(api_route_files())}")
    for library in runtime_support():
        shutil.copy2(library, folder / library.name)
    return sorted(path.relative_to(out).as_posix() for path in folder.iterdir())


def api_route_files() -> set[str]:
    """What the build leaves in the runtime binary folder: only the fixed executable."""
    return {EXECUTABLE}


def runtime_support() -> list[Path]:
    """What the fixed executable loads besides the game's libraries: Rust's standard library, and on Windows the MinGW
    runtime the GNU DLLs import. The runtime slice carries them beside the executable in `bin/`, so it runs as is with no
    Rust toolchain. Not in `deps/`: the build tool installs every runtime `deps/` library as the game's and adds these
    from the player's toolchain itself (sunrise-native-build's `runtime_libraries`), so a second copy there would be
    listed twice."""
    libdir = Path(run(["rustc", "--print", "target-libdir", *(["--target", CROSS_TARGET] if CROSS_TARGET else [])]).strip())
    libraries = [path for path in libdir.iterdir() if runtime_support_file(path.name) and path.name not in MINGW_RUNTIME]
    if len(libraries) != 1:
        sys.exit(f"{libdir} holds {len(libraries)} dynamic Rust standard libraries, not one")
    if PLATFORM == "win32":
        gcc = shutil.which("x86_64-w64-mingw32-gcc" if CROSS_TARGET else "gcc")
        if gcc is None:
            sys.exit("no gcc on PATH, so no MinGW runtime to put beside the executable")
        libraries += [Path(gcc).parent / name for name in MINGW_RUNTIME]
    return libraries


def runtime_support_file(name: str) -> bool:
    """Whether `name` is one of [`runtime_support`]'s files."""
    return name in MINGW_RUNTIME or (name.startswith(("libstd-", "std-")) and name.endswith((".so", ".dll", ".dylib")))


def holder_mark() -> bytes:
    """The bytes the launcher looks for in an executable before it runs it under the holder (`holder::capable`),
    read from their one definition, `holder.rs`'s `CAPABILITY`."""
    source = ROOT / "crates/sunrise-mod-package/src/holder.rs"
    found = re.search(rb'CAPABILITY: \[u8; \d+\] = \*b"([^"]+)"', source.read_bytes())
    if not found:
        sys.exit(f"{source.relative_to(ROOT)} no longer defines CAPABILITY as `[u8; N] = *b\"...\"`: "
                 "update holder_mark() in tools/prebuilt_game.py to read it")
    return found.group(1)


def holds(names: list[str], member: str) -> bool:
    """Whether the slice file list `names` holds `member`, an entry of the required inputs' `members`."""
    folder, _, name = member.rpartition("/")
    prefix = f"{folder}/" if folder else ""
    if name.startswith("@program:"):
        return f"{prefix}{name[9:]}{'.exe' if PLATFORM == 'win32' else ''}" in names
    if name.startswith("@dylib:"):
        return f"{prefix}{dylib_prefix()}{name[7:]}{dylib_suffix()}" in names
    if name.startswith("@rmeta:"):
        if f"{prefix}lib{name[7:]}.rmeta" in names:
            return True
        return any(re.fullmatch(re.escape(f"{prefix}lib{name[7:]}-") + r"[0-9a-f]+\.rmeta", entry) for entry in names)
    return member in names


def dylib_prefix() -> str:
    return "" if PLATFORM == "win32" else "lib"


def dylib_suffix() -> str:
    return {"win32": ".dll", "darwin": ".dylib"}.get(PLATFORM, ".so")


def runtime_packages(metadata: dict, root: str) -> set[str]:
    """Normal SDK library package IDs, excluding host build/proc-macro trees."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [package["id"] for package in metadata["packages"] if package["name"] == root]
    visited = set()
    runtime = set()
    while pending:
        package_id = pending.pop()
        if package_id in visited:
            continue
        visited.add(package_id)
        if not any(any(kind in ("lib", "rlib", "dylib") for kind in target["kind"])
                   for target in packages[package_id]["targets"]):
            continue
        runtime.add(package_id)
        pending.extend(dependency["pkg"] for dependency in nodes[package_id]["deps"]
                       if any(kind["kind"] is None for kind in dependency["dep_kinds"]))
    return runtime


def runtime_crates(metadata: dict, root: str) -> set[str]:
    runtime = runtime_packages(metadata, root)
    return {target["name"] for package in metadata["packages"] if package["id"] in runtime
            for target in package["targets"]
            if any(kind in ("lib", "rlib", "dylib") for kind in target["kind"])}


def native_links(metadata: dict, root: str) -> list[dict]:
    runtime = runtime_packages(metadata, root)
    return [{"name": package["name"], "version": package["version"], "library": package["links"]}
            for package in metadata["packages"] if package["id"] in runtime and package.get("links")]


def runtime_role_facts(metadata: dict, root: str, artifacts: list[dict], target_dir: Path, side: str) -> dict:
    packages = {package["id"]: package for package in metadata["packages"]}
    runtime = runtime_packages(metadata, root)
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    libraries = {}
    compiled = {}
    files: dict[str, set[str]] = {}
    for artifact in artifacts:
        package = artifact.get("package_id")
        if artifact["reason"] != "compiler-artifact" or package not in runtime:
            continue
        target = artifact["target"]
        if (not set(target["kind"]) & {"lib", "rlib", "dylib"}
                or not any(Path(path).is_relative_to(target_dir) for path in artifact["filenames"])):
            continue
        features = sorted(artifact["features"])
        identity = (target["name"], features)
        if package in compiled and compiled[package] != identity:
            raise ValueError(f"{side} SDK has conflicting runtime artifacts for {package}")
        compiled[package] = identity
        files.setdefault(package, set()).update(
            Path(filename).name for filename in artifact["filenames"]
            if Path(filename).is_relative_to(target_dir)
            and Path(filename).suffix in (".rmeta", ".so", ".dll", ".dylib")
        )
    if not any(packages[package]["name"] == root for package in compiled):
        raise ValueError(f"{side} SDK is missing the {root} runtime library")
    identities = {
        package: f"{packages[package]['name']}@{packages[package]['version']}:{library}"
        for package, (library, _) in compiled.items()
    }
    if len(set(identities.values())) != len(identities):
        raise ValueError(f"{side} SDK has ambiguous runtime package identities")
    for package in sorted(compiled):
        library, features = compiled[package]
        aliases = {}
        for dep in nodes[package]["deps"]:
            if dep["pkg"] in compiled and any(kind["kind"] is None for kind in dep["dep_kinds"]):
                extern = identities[dep["pkg"]]
                if aliases.setdefault(dep["name"], extern) != extern:
                    raise ValueError(f"{side} SDK has conflicting dependency alias {dep['name']} in {library}")
        libraries[identities[package]] = {
            "package": packages[package]["name"], "library": library, "features": features,
            "aliases": dict(sorted(aliases.items())), "artifacts": sorted(files[package]),
        }
    return {"side": side, "libraries": dict(sorted(libraries.items()))}


def facade_crates(deps: Path, runtime: set[str]) -> list[str]:
    """Only runtime crates with a unique extern can be exposed by an internals facade."""
    eligible = []
    for crate in sorted(runtime):
        if dylib(deps, crate) is not None or len(list(deps.glob(f"lib{crate}-*.rmeta"))) == 1:
            eligible.append(crate)
    return eligible


def assert_externs(out: Path, eligible: list[str]) -> None:
    """Runs in the SDK build on every OS, including macOS, before publication."""
    for crate in set(eligible) | {"mod_api", "sunrise_mod_package"}:
        extern_args(out / "deps", crate)


def build(out: Path, server: bool) -> None:
    # Resolve before entering the staged copy. Keep its source and Cargo outputs exclusively owned until collection
    # and validation are done. Closing the file releases the lock; leave its name in place for future builds.
    configured_target = lab.cargo_target().resolve() / "prebuilt"
    target_dir = configured_target.resolve()
    # Collection empties the output. It must neither remove a path to the build's lock and stage
    # nor overlap the actual work directory when `prebuilt` is a symlink or junction.
    output = out.resolve()
    for target in (configured_target, target_dir):
        if target.is_relative_to(output) or output.is_relative_to(target):
            sys.exit(f"E_SDK_OUT_OVERLAPS_TARGET: The SDK output {out} overlaps the build's target folder {target}: "
                     "choose another output.")
    target_dir.mkdir(parents=True, exist_ok=True)
    with (target_dir / ".sdk-build.lock").open("a+b") as lock:
        if not file_lock.lock(lock, wait=False):
            sys.exit(f"Cannot acquire the SDK build lock in {target_dir}. Use a separate CARGO_TARGET_DIR.")
        _build_locked(output, server, target_dir)


def runtime_feature_fields(root: Path, metadata: dict, artifacts: list[dict], target_dir: Path, server: bool) -> dict:
    layout = source_layout(root)
    if layout == "split":
        side = "server" if server else "client"
        role = runtime_role_facts(metadata, GAME, artifacts, target_dir, side)
        shells = [library for library in role["libraries"].values() if library["package"] == "sunrise-client"]
        if len(shells) != 1:
            raise ValueError(f"{side} SDK needs exactly one sunrise-client runtime library, found {len(shells)}")
        return {"game_features": shells[0]["features"], "source_layout": layout, "runtime_role": role}
    if layout != "legacy":
        raise ValueError(f"unknown SDK source layout: {layout}")
    return {"game_features": sorted(next(
        message["features"] for message in artifacts
        if message["reason"] == "compiler-artifact"
        and message["package_id"].split("#")[0].endswith("/sunrise-client")
        and "lib" in message["target"]["kind"]
    ))}


def _build_locked(out: Path, server: bool, target_dir: Path) -> None:
    layout = package_format(ROOT)
    staged = stage(target_dir, sdk_dylibs=True)
    host = CROSS_TARGET or rustc_host()

    # One std for the game, every mod and the executable: the game links std dynamically, as mods will.
    rustflags = "-C prefer-dynamic --emit=metadata"
    # On macOS this also gives both dylibs an @rpath install name instead of an absolute build path.
    if PLATFORM == "darwin":
        rustflags += " -C rpath"
    features = cargo_features(server, PLATFORM)
    # Linux's executable RUNPATH doesn't cover transitive dependencies, so the dylibs must find
    # their dependencies beside themselves. macOS's @rpath install names use the executable's rpath.
    if PLATFORM == "linux":
        rustflags += " -C link-arg=-Wl,-rpath,$ORIGIN"
    # The locked resolution names the checkout Cargo compiles from: refuse a modified one before it does.
    metadata = json.loads(run([
        "cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", host,
        *features,
    ], cwd=staged))
    verify_wgpu_checkout(staged, metadata)
    # No official layer in a dedicated server.
    official = not server
    packages = ["-p", GAME, "-p", OFFICIAL] if official else ["-p", GAME]
    # Encoded flags take precedence over RUSTFLAGS in Cargo.
    environment = {key: value for key, value in os.environ.items() if key != "CARGO_ENCODED_RUSTFLAGS"}
    # The SDK game is the game players run with native mods: it keeps the release profile's single codegen unit,
    # whatever the environment says. Against 16 units, the bare dedicated server ran 8.6% fewer instructions per
    # tick and its mean tick fell measurably; the cost is the build's final step (422 s against 270 s on Linux).
    environment["CARGO_PROFILE_RELEASE_CODEGEN_UNITS"] = "1"
    messages = run(
        ["cargo", "build", "--locked", *packages, "--lib",
         "--target", host, "--profile", PROFILE, "--message-format", "json-render-diagnostics", *features],
        cwd=staged,
        env={**environment, "SUNRISE_GAME_BUILD_ID": head(), "RUSTFLAGS": rustflags + " -D warnings", "CARGO_TARGET_DIR": str(target_dir)},
    )
    artifacts = list(map(json.loads, messages.splitlines()))
    scripts = [
        message for message in artifacts
        if message["reason"] == "build-script-executed"
    ]
    if out.exists():
        shutil.rmtree(out)
    deps = out / "deps"
    deps.mkdir(parents=True)
    macros = out / "host"
    macros.mkdir()
    copied = copy_artifacts(artifacts, deps, macros, target_dir / host)
    eligible = facade_crates(deps, runtime_crates(metadata, GAME))
    assert_externs(out, eligible)
    native = out / NATIVE
    native.mkdir()
    # Only libraries that come with a crate (its source in Cargo's home, or its build script's output): a
    # system folder a script names (pkg-config's /usr/lib) is the player's own system's to provide.
    crates = (target_dir, Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")))
    for script in scripts:
        # A static library a script names is already inside its crate's rlib, so inside the game.
        bundled = {lib.split("=")[-1] for lib in script["linked_libs"] if lib.startswith("static")}
        for search in script["linked_paths"]:
            folder = Path(search.split("=", 1)[-1]).resolve()
            if not any(folder.is_relative_to(root.resolve()) for root in crates):
                continue
            for file in sorted(folder.iterdir()):
                stem = file.name.removeprefix("lib").split(".")[0]
                if file.is_file() and file.name.endswith(NATIVE_SUFFIXES) and stem not in bundled:
                    shutil.copy2(file, native / file.name)
    # The commit's bytes, as the build data's are, not a checkout's (CRLF under Git for Windows).
    carried = {name: source.relative_to(ROOT).as_posix() for name, source in CARRIED.items() if source is not None}
    blobs = dict(tracked_blobs(ROOT, head(), tuple(carried.values())))
    for name, path in carried.items():
        if name != LOCK:
            (out / name).write_bytes(blobs[path])
    patches, registry_lock = carry_cargo_inputs(staged, out, metadata, blobs[carried[LOCK]])
    for manifest in BUILD_DATA:
        shutil.copy2(staged / manifest, out / manifest)
    if official and dylib(deps, "sunrise_official") is None:
        sys.exit("prebuilt client is missing the sunrise-official dylib")
    sliced = slices(metadata, copied, api_route(out, host))
    files = {str(path.relative_to(out).as_posix()): sha256(path) for path in sorted(out.rglob("*")) if path.is_file()}
    identity = {
        "package_format": layout,
        "commit": head(),
        "rustc": run(["rustc", "-vV"]).strip(),
        "target": host,
        "profile": PROFILE,
        "side": "server" if server else "client",
        "rustflags": rustflags,
        "facade_crates": eligible,
        # The game crate's features: a `[[native.mixin]]` build compiles functions copied out of its source
        # under the same cfgs (function replacement).
        **runtime_feature_fields(staged, metadata, artifacts, target_dir / host, server),
        # The common function index covers all legs; each leg's identity says which of its
        # declarations actually exist in the lower libraries this patch can link against.
        "lower_crate_cfg": lower_crate_cfgs(metadata, artifacts, staged, target_dir / host,
                                           set(run(["rustc", "--print", "cfg", "--target", host]).splitlines())),
        "registry_patches": patches,
        "patch_files": {name: digest for name, digest in files.items() if name.startswith("patches/")},
        "registry_lock": registry_lock,
        "native_links": native_links(metadata, GAME),
        "native_log": True,
        # The API route: the mods library's crate name and metadata, and which files the `runtime` and `api` archives
        # hold (every other file is the `engine` archive's).
        "slices": sliced,
        "files": files,
    }
    (out / IDENTITY).write_text(json.dumps(identity, indent=2) + "\n")
    check(out)


def localize_game_lock(text: str, label: str, source: str | None) -> tuple[str, dict[str, str]]:
    """Lower the five Git identities without re-resolving the graph or reserializing the lock."""
    expected = tomllib.loads(text)
    packages = expected.get("package", [])
    unused = expected.get("patch", {}).get("unused", [])
    versions = {}
    counts = {}
    for record in packages:
        name, version = record["name"], record["version"]
        counts[name, version] = counts.get((name, version), 0) + 1
        lock_source = record.get("source", "")
        if (source is not None and name in ("wgpu", "wgpu-core")
                and lock_source != "registry+https://github.com/rust-lang/crates.io-index"):
            raise ValueError(f"{label}: {name} must stay on crates.io when carrying the wgpu Git patches")
        if not lock_source.startswith("git+"):
            continue
        if source is None or name not in WGPU_PATCHES:
            raise ValueError(f"{label}: unsupported Git package {name} {version}: {lock_source}")
        if lock_source != source:
            raise ValueError(f"{label}: {name} Git source must match the manifest rev: {lock_source} != {source}")
        if name in versions or "checksum" in record:
            raise ValueError(f"{label}: ambiguous or invalid Git identity for {name} {version}")
        versions[name] = version
        del record["source"]
    for record in unused:
        if record.get("source", "").startswith("git+"):
            raise ValueError(f"{label}: unused Git patch {record['name']}; all five wgpu patches must be resolved")
    if source is not None:
        missing = set(WGPU_PATCHES) - versions.keys()
        if missing:
            raise ValueError(f"{label}: missing locked wgpu Git packages: {', '.join(sorted(missing))}")
        unused_ids = {(record["name"], record["version"]) for record in unused}
        for name, version in versions.items():
            if counts[name, version] != 1 or (name, version) in unused_ids:
                raise ValueError(f"{label}: {name} {version} collides with another package identity")

    refs = {f"{name} {version} ({source})": f"{name} {version}" for name, version in versions.items()}
    for record in (*packages, *unused):
        if "dependencies" in record:
            record["dependencies"] = [refs.get(dep, dep) for dep in record["dependencies"]]
    # Reject even escaped Git strings in an unrecognized reference (for example a legacy metadata key).
    pending = [expected]
    while pending:
        value = pending.pop()
        if isinstance(value, dict):
            pending.extend(value)
            pending.extend(value.values())
        elif isinstance(value, list):
            pending.extend(value)
        elif isinstance(value, str) and "git+" in value:
            raise ValueError(f"{label}: unsupported Git source or dependency reference: {value}")
    if not versions:
        return text, versions

    # Cargo writes one [[package]] section per row, and one dependency string per line.
    # The semantic comparison below refuses to emit a partial rewrite if that layout changes.
    prefix, *sections = re.split(r"(?m)(?=^[ \t]*\[\[package\]\][ \t]*(?:#[^\r\n]*)?\r?$)", text)
    if len(sections) != len(packages):
        raise ValueError(f"{label}: cannot locate Cargo-generated package sections for Git patch localization")
    source_line = re.compile(rf'(?m)^([ \t]*)source[ \t]*=[ \t]*"{re.escape(source)}"[ \t]*(#[^\r\n]*)?(\r?\n|$)')
    for index, record in enumerate(packages):
        if versions.get(record["name"]) == record["version"]:
            sections[index] = source_line.sub(lambda m: m[1] + m[2] + m[3] if m[2] else "", sections[index], count=1)
    ref_values = "|".join(map(re.escape, refs))
    result = re.sub(rf'(?m)^([ \t]*")({ref_values})("[ \t]*,[ \t]*(?:#[^\r\n]*)?\r?$)',
                    lambda m: m[1] + refs[m[2]] + m[3], prefix + "".join(sections))
    if tomllib.loads(result) != expected:
        raise ValueError(f"{label}: cannot localize Git sources without changing unrelated lock records")
    return result, versions


def pinned_git(checkout: Path, *args: str, request: bytes = b"") -> bytes:
    """Read local Git objects only, including with older Git that lacks GIT_NO_LAZY_FETCH."""
    return subprocess.run(["git", *args], cwd=checkout, input=request, stdout=subprocess.PIPE,
                          check=True, timeout=ARCHIVE_SECONDS,
                          env={**os.environ, "GIT_NO_LAZY_FETCH": "1", "GIT_ALLOW_PROTOCOL": "",
                               "GIT_NO_REPLACE_OBJECTS": "1"}).stdout


def pinned_wgpu_tree(checkout: Path, rev: str) -> list[tuple[str, bytes, bytes, bytes]]:
    """The name, mode, kind and object ID of every pinned entry of the root files and the five package directories,
    read from the checkout's Git objects once it is the top level at HEAD `rev`."""
    if (pinned_git(checkout, "rev-parse", "--show-prefix").strip()
            or pinned_git(checkout, "rev-parse", "HEAD").strip() != rev.encode("ascii")):
        raise ValueError(f"wgpu Git checkout {checkout} must be the Git top level at HEAD {rev}")
    tree = pinned_git(checkout, "ls-tree", "-r", "-z", rev, "--", *WGPU_ROOT_FILES, *WGPU_PATCHES)
    entries = []
    for entry in tree.rstrip(b"\0").split(b"\0") if tree else ():
        header, raw_path = entry.split(b"\t", 1)
        entries.append((os.fsdecode(raw_path), *header.split(b" ")))
    return entries


def pinned_blobs(checkout: Path, rev: str, files: dict[str, tuple[bytes, bytes]]):
    """Yield the name, mode and bytes of each of `files` (name to mode and object ID) from the checkout's local Git
    blobs, not an archive subject to attributes or the mutable worktree."""
    request = b"".join(oid + b"\n" for _, oid in files.values())
    with io.BytesIO(pinned_git(checkout, "cat-file", "--batch", request=request)) as blobs:
        for name, (mode, oid) in files.items():
            header = blobs.readline().split()
            if len(header) != 3 or header[:2] != [oid, b"blob"]:
                raise ValueError(f"wgpu Git blob {name!r} is unavailable locally at {rev}; restore Cargo's checkout")
            size = int(header[2])
            data = blobs.read(size)
            if len(data) != size or blobs.read(1) != b"\n":
                raise ValueError(f"incomplete wgpu Git blob {name!r} at {rev}")
            yield name, mode, data


def is_link(info: os.stat_result) -> bool:
    """A symlink, or on Windows any reparse point (a junction included), whatever else its mode says."""
    return stat.S_ISLNK(info.st_mode) or bool(
        getattr(info, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)


def wgpu_worktree(checkout: Path) -> dict[str, os.stat_result]:
    """The `lstat` of every path that is not a directory under the root files and the five package directories, ignored
    files included. A link or reparse point is recorded, never followed or listed."""
    found = {}
    pending = [*WGPU_ROOT_FILES, *WGPU_PATCHES]
    while pending:
        name = pending.pop()
        path = checkout / name
        if not os.path.lexists(path):
            continue
        info = path.lstat()
        if stat.S_ISDIR(info.st_mode) and not is_link(info):
            pending.extend(f"{name}/{child}" for child in os.listdir(path))
        else:
            found[name] = info
    return found


def verify_wgpu_worktree(checkout: Path, rev: str) -> None:
    """Refuse unless the checkout's actual files for the root files and the five package directories are the tree at
    `rev`: the same paths (no untracked or ignored file), bytes and executable bit. Neither Git's index, its filters
    and attributes, nor `core.filemode` is consulted, since Cargo compiles the bytes on disk."""
    entries = pinned_wgpu_tree(checkout, rev)
    for name, mode, kind, _ in entries:
        if kind != b"blob" or mode not in (b"100644", b"100755"):
            raise ValueError(f"wgpu Git checkout cannot be verified: {name!r} at {rev} is not a regular file")
    files = {name: (mode, oid) for name, mode, _, oid in entries}
    pinned = {name: (mode, data) for name, mode, data in pinned_blobs(checkout, rev, files)}
    found = wgpu_worktree(checkout)
    for name in sorted(files.keys() | found.keys()):
        if name in found and is_link(found[name]):
            problem = "is a symlink or reparse point, which is never followed"
        elif name not in files:
            problem = "is not in the pinned tree"
        elif name not in found:
            problem = "is missing"
        elif not stat.S_ISREG(found[name].st_mode):
            problem = "is not a regular file"
        elif (checkout / name).read_bytes() != pinned[name][1]:
            problem = "has other bytes than the pinned file"
        elif os.name == "posix" and bool(found[name].st_mode & stat.S_IXUSR) != (pinned[name][0] == b"100755"):
            problem = "has another executable mode than the pinned file"
        else:
            continue
        raise ValueError(f"Cargo's wgpu Git checkout is not the pinned tree at {rev}: {name!r} {problem}. "
                         f"Remove Cargo's checkout {checkout} so Cargo restores it, then build again.")


def carry_wgpu(checkout: Path, rev: str, destination: Path) -> None:
    """Copy blobs, not an archive subject to attributes or files from the mutable checkout."""
    files = {}
    for name, mode, kind, oid in pinned_wgpu_tree(checkout, rev):
        parts = name.split("/")
        if ".cargo" in parts or name.startswith(WGPU_OMIT):
            continue
        if kind != b"blob" or mode not in (b"100644", b"100755"):
            raise ValueError(f"wgpu Git patch {name!r} at {rev} is not a regular file (symlink or submodule)")
        if (name not in WGPU_ROOT_FILES and (parts[0] not in WGPU_PATCHES or len(parts) < 2)
                or any(part in ("", ".", "..", ".git") for part in parts) or "\\" in name or ":" in name):
            raise ValueError(f"unexpected wgpu Git patch path {name!r} at {rev}")
        if len(parts) > 2 and parts[-1] == "Cargo.toml":
            raise ValueError(f"unexpected nested crate in wgpu Git patch: {name}")
        files[name] = (mode, oid)
    required = {*WGPU_ROOT_FILES, *(f"{name}/Cargo.toml" for name in WGPU_PATCHES)}
    if missing := required - files.keys():
        raise ValueError(f"wgpu Git patch {rev} is missing {', '.join(sorted(missing))}")

    destination.mkdir(parents=True)
    for name, mode, data in pinned_blobs(checkout, rev, files):
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as file:
            file.write(data)
        if os.name == "posix" and mode == b"100755":
            path.chmod(stat.S_IMODE(path.stat().st_mode) | 0o111)


def staged_patches(staged: Path) -> dict:
    return tomllib.loads((staged / "Cargo.toml").read_text()).get("patch", {})


def wgpu_pin(patches: dict) -> tuple[dict, str | None, str | None]:
    """The game's Git patch specs, their one full rev and the lock source it gives, all None without Git patches."""
    git_specs = {(registry, name): spec for registry, entries in patches.items() for name, spec in entries.items()
                 if "git" in spec}
    rev = None
    source = None
    if git_specs:
        if git_specs.keys() != {("crates-io", name) for name in WGPU_PATCHES}:
            raise ValueError(f"only the five crates.io wgpu Git patches may be carried, together; got {sorted(git_specs)}")
        for (_, name), spec in git_specs.items():
            if spec.keys() != {"git", "rev"} or spec["git"] != WGPU_FORK:
                raise ValueError(f"{name}: SDK Git patches require only git = {WGPU_FORK!r} and a full rev")
            pin = spec["rev"]
            if not isinstance(pin, str) or not re.fullmatch(r"[0-9a-f]{40}", pin) or (rev and rev != pin):
                raise ValueError(f"{name}: all wgpu Git patches must use the same full lowercase rev")
            rev = pin
        source = f"git+{WGPU_FORK}?rev={rev}#{rev}"
    return git_specs, rev, source


def locked_git_packages(metadata: dict) -> list[dict]:
    return [package for package in metadata["packages"] if (package.get("source") or "").startswith("git+")]


def wgpu_checkout(resolved: list[dict]) -> Path:
    """The one Cargo checkout the locked metadata resolves every Git package of `resolved` from."""
    roots = set()
    for package in resolved:
        name = package["name"]
        manifest = Path(package["manifest_path"])
        root = manifest.parent.parent.resolve()
        if (not manifest.is_absolute() or manifest.name != "Cargo.toml" or manifest.parent.name != name
                or manifest.resolve() != root / name / "Cargo.toml"):
            raise ValueError(f"{name}: locked Cargo metadata must point to <wgpu checkout>/{name}/Cargo.toml")
        roots.add(root)
    if len(roots) != 1:
        raise ValueError("locked Cargo metadata must resolve all wgpu Git patches from one checkout")
    return roots.pop()


def verify_wgpu_checkout(staged: Path, metadata: dict) -> None:
    """Before the locked game build: refuse a Cargo checkout of the pinned wgpu patches whose files are not the pin's.
    Cargo reuses a checkout at the right HEAD whatever its worktree holds, so the compiled game and the blobs
    `carry_wgpu` ships could otherwise differ. Without Git packages in the locked resolution there is nothing to verify."""
    resolved = locked_git_packages(metadata)
    if not resolved:
        return
    _, rev, _ = wgpu_pin(staged_patches(staged))
    if rev is not None:  # Git packages the manifest does not pin are refused by `carry_cargo_inputs`.
        verify_wgpu_worktree(wgpu_checkout(resolved), rev)


def carry_cargo_inputs(staged: Path, out: Path, metadata: dict, committed_lock: bytes) -> tuple[dict, str]:
    """Carry the committed lock and the staged registry lock separately, using the resolved game metadata."""
    patches = staged_patches(staged)
    git_specs, rev, source = wgpu_pin(patches)

    stage_lock = (staged / LOCK).read_bytes().decode("utf-8") if rev is not None else (staged / LOCK).read_text()
    registry_lock, versions = localize_game_lock(stage_lock, f"staged {LOCK}", source)
    lock, committed_versions = localize_game_lock(committed_lock.decode("utf-8"), f"committed {LOCK}", source)
    if versions != committed_versions:
        raise ValueError(f"the staged and committed {LOCK} disagree on wgpu Git package identities")
    resolved = locked_git_packages(metadata)
    identities = sorted((package["name"], package["version"], package["source"]) for package in resolved)
    if identities != sorted((name, version, source) for name, version in versions.items()):
        raise ValueError(f"locked Cargo metadata does not match the game Git lock identities: {identities}")
    if rev is not None:
        carry_wgpu(wgpu_checkout(resolved), rev, out / "patches/wgpu")
        for registry, name in git_specs:
            patches[registry][name] = {"path": f"patches/wgpu/{name}"}

    patches = carry_patches(staged, out, patches=patches, copied=git_specs.keys())
    (out / LOCK).write_bytes(lock.encode("utf-8") if rev is not None else committed_lock)
    return patches, registry_lock


def carry_patches(staged: Path, out: Path, *, patches: dict | None = None, copied=()) -> dict:
    """Carry only game-owned registry patch crates, never the rest of the game source."""
    if patches is None:
        patches = staged_patches(staged)
    for registry, entries in patches.items():
        for name, spec in entries.items():
            if (registry, name) in copied:
                continue
            if "path" not in spec or spec.keys() & {"git", "rev", "tag", "branch", "registry", "registry-index"}:
                raise ValueError(f"{name}: unsupported game SDK patch source; expected a game-owned path or the pinned wgpu group")
            source = staged / spec["path"]
            if source.is_symlink() or any(path.is_symlink() for path in source.rglob("*")):
                raise ValueError("game patch contains a symlink")
            source = source.resolve()
            if not source.is_relative_to(staged.resolve()):
                raise ValueError("game patch escapes the staged source")
            destination = out / "patches" / source.name
            shutil.copytree(source, destination, symlinks=True)
            if any(path.is_symlink() for path in destination.rglob("*")):
                raise ValueError("copied game patch contains a symlink")
            spec["path"] = destination.relative_to(out).as_posix()
    # This deliberately is not the game's buildable workspace manifest.
    lines = []
    for registry, entries in patches.items():
        lines.append(f"[patch.{json.dumps(registry)}]")
        for name, spec in entries.items():
            lines.append(f"{json.dumps(name)} = {{ path = {json.dumps(spec['path'])} }}")
    (out / "Cargo.toml").write_text("\n".join(lines) + "\n")
    return patches


def dylib(deps: Path, crate: str) -> Path | None:
    """A dylib carries its own metadata, so it has no `.rmeta` beside it."""
    names = {f"{prefix}{crate}.{ext}" for prefix in ("lib", "") for ext in ("so", "dll", "dylib")}
    return next((path for path in deps.iterdir() if path.name in names), None)


def extern_args(deps: Path, crate: str) -> list[str]:
    """Resolve a native SDK extern by name, refusing ambiguous metadata rather than choosing a variant."""
    library = dylib(deps, crate)
    if library is None:
        candidates = list(deps.glob(f"lib{crate}-*.rmeta")) + list(deps.glob(f"lib{crate}.rmeta"))
        if len(candidates) != 1:
            raise ValueError(f"expected exactly one SDK metadata for {crate}: {candidates}")
        library = candidates[0]
    return ["--extern", f"{crate}={library}"]


def check(out: Path) -> None:
    """Only compiler outputs and exactly inventoried patch sources may ship."""
    identity = json.loads((out / IDENTITY).read_text()) if (out / IDENTITY).is_file() else {}
    patch_files = identity.get("patch_files", {})
    links = [path for path in out.rglob("*") if path.is_symlink()]
    if links:
        sys.exit(f"prebuilt folder contains symlinks: {links}")
    for name, digest in patch_files.items():
        path = out / name
        if not name.startswith("patches/") or ".." in Path(name).parts or not path.is_file() or sha256(path) != digest:
            sys.exit(f"prebuilt patch inventory mismatch: {name}")
    stray = [
        path for path in out.rglob("*")
        if path.is_file() and not (path.parent == out and (path.name in CARRIED or path.name in BUILD_DATA))
        and str(path.relative_to(out).as_posix()) not in patch_files
        and not (path.parent == out / "deps" and path.name.endswith(SUFFIXES))
        and not (path.parent == out / "host" and path.name.endswith((".so", ".dll", ".dylib", ".dll.a")))
        and not (path.parent == out / NATIVE and path.name.endswith(NATIVE_SUFFIXES))
        and not (path.parent == out / RUNTIME and (path.name in api_route_files() or runtime_support_file(path.name)))
    ]
    if stray:
        sys.exit(f"prebuilt folder holds files that aren't compiler outputs: {stray}")
    if dylib(out / "deps", "sunrise_mod_host") is None:
        sys.exit("prebuilt folder has no sunrise-mod-host dylib")

    commit = identity.get("commit")
    for name in BUILD_DATA:
        path = out / name
        header = json.loads(path.read_text()) if path.is_file() else {}
        header = header if isinstance(header, dict) else {}
        build = header.get("build")
        if header.get("schema") != BUILD_DATA_SCHEMA or not isinstance(build, dict) or build.get("commit") != commit:
            sys.exit(f"prebuilt {name} is missing or has no build-data header for commit {commit}")
    sliced = identity.get("slices", {})
    listed = [name for kind in ("runtime", "api") for name in sliced.get(kind, [])]
    absent = [name for name in listed if not (out / name).is_file()]
    if absent or len(listed) != len(set(listed)):
        sys.exit(f"prebuilt slices name absent files {absent} or overlap")
    lacking = [f"{kind}: {member}" for kind, members in json.loads(INPUTS.read_text())["members"].items()
               for member in members if not holds(sliced.get(kind, []), member)]
    if lacking:
        sys.exit(f"prebuilt slices lack required members: {lacking}")
    mark = holder_mark()
    if mark not in (out / RUNTIME / EXECUTABLE).read_bytes():
        sys.exit(f"{RUNTIME}/{EXECUTABLE} does not carry the holder marker {mark.decode()}")


def smoke(out: Path) -> None:
    """Build two starter mod dylibs, the second calling into the first, and install the fixed executable from OUT
    alone."""
    deps = out / "deps"
    assert_externs(out, json.loads((out / IDENTITY).read_text())["facade_crates"])

    def extern(crate: str) -> list[str]:
        return extern_args(deps, crate)

    common = ["--edition", "2024", "-C", "prefer-dynamic", "-C", "opt-level=1", "-L", f"dependency={deps}",
              "-L", f"dependency={out / 'host'}", "-L", f"native={out / NATIVE}"]
    installed = out.parent / SMOKE
    if installed.exists():
        shutil.rmtree(installed)
    installed.mkdir()
    # Use the same dylib link arguments as sunrise-native-build, not the executable's link policy.
    platform = {"darwin": "macos", "win32": "windows"}.get(PLATFORM, PLATFORM)
    prefix = f"{platform} "
    policy = ROOT / "crates/sunrise-native-build/src/dylib-link-args.txt"
    link_args = [line.removeprefix(prefix) for line in policy.read_text(encoding="utf-8").splitlines()
                 if line.startswith(prefix)]
    identity = json.loads((out / IDENTITY).read_text())
    release = next(line.removeprefix("release: ") for line in identity["rustc"].splitlines() if line.startswith("release: "))
    starter = (ROOT / "crates/cargo-sunrise-mod/template/native/lib.rs").read_text()
    template = (out / NATIVE_MODS_TEMPLATE).read_text()
    smoke_library = installed / f"{dylib_prefix()}smoke{dylib_suffix()}"
    # `smoke_user` calls `smoke` through its library, as a mod calls a declared dependency: rustc links it against
    # `smoke`'s library (on windows-gnu through its import library) and the loader resolves the call at start.
    mods = {
        "smoke": (SMOKE_LIBRARY_ITEM, [], "", []),
        "smoke_user": (SMOKE_USER_ITEM, ["--extern", f"smoke={smoke_library}"], '\n[dependencies]\nsmoke = "^0.1"\n', ["smoke"]),
    }
    records = []
    with tempfile.TemporaryDirectory(dir=out.parent) as scratch:
        scratch = Path(scratch)
        for mod, (item, externs, dependencies, links) in mods.items():
            source = starter.replace("{id}", mod) + item
            # The identity names each linked mod's recorded identity (`library_identity` in sunrise-mod-package).
            recorded = {r["mod"]: r["identity"] for r in records}
            linked = ",".join(f"{m}={hashlib.sha256(recorded[m].encode()).hexdigest()}" for m in links) or "none"
            build = f"{identity['commit']} {mod} 0.1.0 {release} {hashlib.sha256(source.encode()).hexdigest()} {linked}"
            rows = {"NATIVE_BUILD": build, "MOD_ENTRIES": f'"{mod}" => crate::hooks,',
                    "BUILT_MODS": f'mod_api::__private::native::BuiltMod {{ id: "{mod}", version: "0.1.0", side: mod_api::Side::Both }},',
                    "MOD_REDIRECTS": ""}
            entry = template
            for marker, value in rows.items():
                entry = entry.replace(f"/* {marker} */", value)
            (scratch / f"{mod}_entry.rs").write_text(entry)
            (scratch / f"{mod}.rs").write_text(source + f'\n#[path = "{mod}_entry.rs"] mod entry;\n')
            library = installed / f"{dylib_prefix()}{mod}{dylib_suffix()}"
            run(["rustc", *common, "-L", f"dependency={installed}", "--crate-type", "dylib", "--crate-name", mod,
                 *extern("mod_api"), "--extern", f"sunrise_mod_host={dylib(deps, 'sunrise_mod_host')}", *externs,
                 *(flag for arg in link_args for flag in ("-C", f"link-arg={arg}")),
                 "-o", str(library), str(scratch / f"{mod}.rs")])
            records.append({"file": library.name, "sha256": sha256(library), "kind": "mod", "mod": mod,
                            "identity": f"{build} unwind"})
            install_package(installed / "sunrise-native-mods" / mod, mod, source, dependencies)
    shutil.copy2(out / RUNTIME / EXECUTABLE, installed / EXECUTABLE)
    (installed / "native-mods.json").write_text(json.dumps({"libraries": records}))
    if holder_mark() not in (installed / EXECUTABLE).read_bytes():
        sys.exit("smoke: holder capability missing from the executable")
    for library in runtime_libraries(out):
        shutil.copy2(library, installed / library.name)
    # The same executable as shipped, with no native libraries selected.
    api = out.parent / SMOKE_API
    if api.exists():
        shutil.rmtree(api)
    shutil.copytree(out / RUNTIME, api)
    for library in runtime_libraries(out):
        shutil.copy2(library, api / library.name)
    print(f"smoke: two starter mods, one calling the other, and the game executable built against {out} alone, "
          f"installed in {installed}")


# What `smoke` adds to the starter for `smoke_user` to call; never inlined, so the call crosses the libraries.
SMOKE_LIBRARY_ITEM = """
/// Called from `smoke_user`'s library. Named apart from the starter's own items (its private `greeting`).
#[inline(never)]
pub fn library_name() -> &'static str {
    "smoke"
}
"""
SMOKE_USER_ITEM = """
/// A call into `smoke`'s library.
pub fn library_name_length() -> usize {
    smoke::library_name().len()
}
"""


def install_package(folder: Path, mod: str, source: str, dependencies: str) -> None:
    """Mod MOD's package as the launcher leaves it beside the executable, which reads it back at start
    (sunrise-mod-package `runtime::Runtime::load`): the starter manifest with its DEPENDENCIES table and its
    [native] pins, and its source."""
    manifest = (ROOT / "crates/cargo-sunrise-mod/template/native/sunrise-mod.toml").read_text()
    manifest = manifest.replace("{id}", mod).replace("{name}", mod) + dependencies
    mixins = re.search(r'pub const REV: &str = "([0-9a-f]{40})";',
                       (ROOT / "crates/sunrise-mod-package/src/mixins_tool.rs").read_text())[1]
    game = re.search(r'pub const VERSION_ID: &str = "([^"]+)";',
                     (ROOT / "crates/sunrise-core/src/version.rs").read_text())[1]
    sunrise = run(["git", "rev-parse", "HEAD"], cwd=ROOT).strip()
    manifest += f'\n[native]\ngame = "{game}"\nsunrise = "{sunrise}"\nmixins = "{mixins}"\n'
    (folder / "src").mkdir(parents=True)
    (folder / "sunrise-mod.toml").write_text(manifest)
    (folder / "Cargo.toml").write_text(f'[package]\nname = "{mod}"\nversion = "0.1.0"\nedition = "2024"\n')
    (folder / "src/lib.rs").write_text(source)


def runtime_libraries(out: Path) -> list[Path]:
    """What is installed beside the executable: every shared library the identity's `runtime` slice records under
    `deps/` (sunrise-native-build's `runtime_libraries` rule), and std and the MinGW runtime from the slice's own `bin/`
    ([`runtime_support`]; the build tool takes the toolchain's copies of the same files). So `start` fails when the
    slice misses a library the game loads, and the game it starts needs no Rust toolchain."""
    runtime = json.loads((out / IDENTITY).read_text())["slices"]["runtime"]
    return [out / name for name in runtime
            if (name.startswith("deps/") and name.endswith((".so", ".dll", ".dylib")))
            or (name.startswith(f"{RUNTIME}/") and runtime_support_file(Path(name).name))]


def start(out: Path) -> None:
    """Start two installations of the fixed executable (with smoke's two libraries and without mods)
    with no game data: it loads the game's libraries and sets up the mods' runtime, then stops at the missing
    prepared files. A client SDK starts as a headless client, whose refusal names the launcher's download page
    (the hook only its official layer registers); a server SDK starts as a dedicated server, which never
    advertises the launcher. Fails on a crash,
    a hang or any other stop, so an archive whose libraries disagree (two allocators, #3877) cannot ship. On Windows
    it runs without MinGW on PATH, as on a player's computer, so a runtime DLL the launcher would not install fails
    here (#3521)."""
    client = json.loads((out / IDENTITY).read_text())["side"] == "client"
    for installed in (out.parent / SMOKE, out.parent / SMOKE_API):
        with tempfile.TemporaryDirectory(dir=out.parent) as scratch:
            scratch = Path(scratch)
            # Never a real player's folders, even ones this process inherited: every home is a folder of the scratch.
            homes = {key: scratch / key.lower() for key in
                     ("SUNRISE_HOME", "SUNRISE_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME")}
            for home in homes.values():
                home.mkdir()
            environment = lab.game_env(RUST_BACKTRACE="1")
            environment.update({key: str(home) for key, home in homes.items()})
            if sys.platform == "win32":
                # A player has no MinGW: Windows must find every DLL beside the executable or in the system, so a
                # folder of PATH that holds the MinGW runtime (the runner's MinGW, Git's, a toolchain's) is left out.
                environment["PATH"] = os.pathsep.join(
                    folder for folder in environment.get("PATH", "").split(os.pathsep)
                    if folder and not any((Path(folder) / name).is_file() for name in MINGW_RUNTIME))
            game = scratch / "game"
            game.mkdir()
            # A dedicated server stops at an unaccepted EULA before it reaches the prepared files (the EULA is accepted).
            (game / "eula.txt").write_text("eula=true\n")
            executable = installed / EXECUTABLE
            if sys.platform == "darwin":
                # Use the launcher's actual bundle preparation, not a second list of libraries: its source from the
                # sunrise-launcher commit pinned in tools/ci/launcher-pin.
                bundle_source = scratch / "bundle.rs"
                pin = launcher_pin()
                game_app = fetch_launcher_files(scratch / "launcher", pin) / "src/game_app.rs"
                module = json.dumps(str(game_app))
                bundle_source.write_text(
                    'mod i18n { pub fn tr(s: &str) -> &str { s } }\n'
                    'mod shortcut { pub fn xml(s: &str) -> String { s.to_owned() } }\n'
                    f'#[path = {module}] mod game_app;\n'
                    'fn main() { game_app::ensure(std::path::Path::new(&std::env::args().nth(1).unwrap())).unwrap(); }')
                bundle_tool = scratch / "bundle"
                run(["rustc", "--edition=2021", str(bundle_source), "-o", str(bundle_tool)])
                run([str(bundle_tool), str(installed)])
                bundled = installed / "Sunrise.app/Contents/MacOS/sunrise-client"
                executable = bundled
                probe = subprocess.run([str(bundled), "--launch-api"], cwd=game, env=environment,
                                       capture_output=True, text=True, timeout=60)
                if probe.returncode != 0:
                    sys.exit(f"bundled launch API failed: {probe.stderr}")
                json.loads(probe.stdout)
            role = ["--headless"] if client else ["--dedicated", "--nogui"]
            command = [str(executable), "--resources", str(scratch / "no-resources"), *role]
            print("+", " ".join(command), flush=True)
            result = subprocess.run(command, cwd=game, env=environment, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, encoding="utf-8", errors="replace", timeout=300)
        print(result.stdout, end="")
        print(f"start: exit {result.returncode}")
        if result.returncode == DLL_NOT_FOUND:
            sys.exit("start: Windows found no copy of a DLL the executable or one of its libraries imports "
                     f"(0xC0000135): it is neither beside the executable in {installed} nor a system DLL")
        # The prepared-files check exits with status 1; any other status is a different stop.
        if result.returncode != 1 or NO_RESOURCES not in result.stdout or "panicked" in result.stdout:
            sys.exit(f"start: {installed.name} stopped before the prepared-files check (exit {result.returncode})")
        shows_launcher = bool(re.search(r"launcher: download it at https?://\S+ \(Download\)", result.stdout))
        if shows_launcher != client:
            problem = "client omitted" if client else "server showed"
            sys.exit(f"start: {installed.name}: {problem} the launcher download URL")
        print(f"start: {installed.name} loaded the game's libraries and the mods' runtime and stopped at: "
              f"{NO_RESOURCES}")


def tar_xz(folder: Path, names: list[str], archive: Path) -> None:
    """A tar of `names` (paths under `folder`, folders included) the same on every OS and run: sorted `./` entries,
    zero times and owners, GNU format, xz-compressed with no timestamp of its own. Modes are 0o755 for folders and
    executables and 0o644 for other files, whatever the host's umask or file system gave them (Windows reports
    0o666 and 0o777)."""
    def clean(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.mtime, info.uid, info.gid, info.uname, info.gname = 0, 0, 0, "", ""
        info.mode = 0o755 if info.isdir() or info.mode & 0o111 else 0o644
        return info
    with lzma.open(archive, "wb", format=lzma.FORMAT_XZ, check=lzma.CHECK_CRC64, preset=XZ_PRESET) as xz, \
            tarfile.open(fileobj=xz, mode="w", format=tarfile.GNU_FORMAT) as tar:
        for name in sorted(set(names)):
            arcname = "." if name == "." else "./" + name
            tar.add(folder / name, arcname=arcname, recursive=False, filter=clean)


def tree(folder: Path) -> list[str]:
    """Every path under `folder`, relative, with `/`, and `.` itself."""
    return ["."] + [path.relative_to(folder).as_posix() for path in folder.rglob("*")]


def archives(out: Path, sliced: dict[str, list[str]]) -> dict[str, list[str]]:
    """Each archive's entries: the identity's `runtime` and `api` files, and every other file for `engine`, each with
    the folders above it, so the three unpacked into one folder are the whole package."""
    files = {name: kind for kind in ("runtime", "api") for name in sliced[kind]}
    every = [name for name in tree(out) if (out / name).is_file()]
    chosen = {"runtime": [], "api": [], "engine": []}
    for name in every:
        chosen[files.get(name, "engine")].append(name)
    return {kind: sorted({".", *(parent.as_posix() for name in names for parent in Path(name).parents
                                if parent != Path("."))} | set(names)) for kind, names in chosen.items()}


def pack(out: Path, dist: Path, stage_tool: Path | None = None, with_tool: bool = False,
         with_programs: bool = False) -> None:
    """The SDK release's assets from a checked folder OUT, into DIST: the package as its `runtime`, `api` and `engine`
    archives for its target and side ([`archives`]), the build data, the surface export of this source and, for the
    client side, the target's `tool` ([`tool`]): one per target, as it serves both sides. `with_tool` packs it on a server
    leg too, for a server-only SDK that no client leg completes (the fn-replace fixtures): the launcher needs the tool
    even to export a server. `with_programs` packs the target and side's `programs` too ([`programs`])."""
    check(out)
    identity = json.loads((out / IDENTITY).read_text())
    if identity.get("package_format") != package_format(ROOT) or identity.get("commit") != head():
        sys.exit(f"{out} is not this source's package (format {package_format(ROOT)}, commit {head()})")
    if with_programs:
        from tools import release_file

        if not lists_programs(release_file.LAUNCHER_MIN, release_file.UNKNOWN_KINDS_LAUNCHER):
            sys.exit(f"{PROGRAMS}-* waits for the launcher floor: an sdk.json listing a kind a launcher doesn't know "
                     "stops that launcher reading the whole SDK, so tools/release_file.py's LAUNCHER_MIN must first "
                     "reach UNKNOWN_KINDS_LAUNCHER, the first launcher that skips such kinds")
    dist.mkdir(parents=True, exist_ok=True)
    suffix = "-server" if identity["side"] == "server" else ""
    jobs = {dist / f"{kind}-{identity['target']}{suffix}.tar.xz": (out, names)
            for kind, names in archives(out, identity["slices"]).items()}
    jobs[dist / BUILD_DATA_ARCHIVE] = (out, [".", *BUILD_DATA])
    if with_programs:
        folder = programs(identity, out.parent / f"{PROGRAMS}-{identity['target']}{suffix}")
        jobs[dist / f"{folder.name}.tar.xz"] = (folder, tree(folder))
    packs_tool = identity["side"] == "client" or with_tool
    # Each archive keeps its single xz stream; only independent archives run concurrently.
    with ProcessPoolExecutor(max_workers=len(jobs) + packs_tool) as pool:
        pending = {archive: pool.submit(tar_xz, folder, names, archive) for archive, (folder, names) in jobs.items()}
        # The tool compiles while the package's archives compress, then joins them: compressing the engine archive
        # leaves cores idle for minutes, which the tool's build took after it (macOS, 2026-11-25: 5m52 then 7m04).
        if packs_tool:
            archive = dist / f"tool-{identity['target']}.tar.xz"
            pending[archive] = pool.submit(tar_xz, tool(identity["target"]), [".", TOOL_PROGRAM], archive)
        failures = [f"{archive.name}: {error}" for archive, future in pending.items()
                    if (error := future.exception()) is not None]
    if failures:
        sys.exit("SDK archive compression failed: " + "; ".join(failures))
    command = ([str(stage_tool.resolve())] if stage_tool is not None else
               ["cargo", "run", "--locked", "--release", "-q", "-p", "sunrise-stage", "--bin", "sunrise-stage", "--"])
    run(command + ["surface", str(ROOT), str(dist / SURFACE)], cwd=ROOT)
    for asset in sorted(dist.iterdir()):
        print(f"{asset.name}: {asset.stat().st_size} bytes")


def lists_programs(launcher_min: str, unknown_kinds: str) -> bool:
    """Whether an SDK may list the `programs` kind: only once every launcher that may install the build
    (`launcher_min` on) skips SDK asset kinds it doesn't know, which `unknown_kinds` names ("" until one is released)."""
    version = lambda text: tuple(int(part) for part in text.split("."))
    return bool(unknown_kinds) and version(launcher_min) >= version(unknown_kinds)


def programs(identity: dict, folder: Path) -> Path:
    """FOLDER, the `programs` slice for the package's target and side: the player folder without its game program, from
    the one list of what it holds (the `package` route of `sdk/inputs.json`, `package-server` for a server leg). Each
    companion program is built from this source for this computer the way the list says to make it, then `xtask package
    --no-game` lays the folder out as it does the player archive's; on Windows x64 Microsoft's DXC goes in beside them,
    as in the player archive (tools/ci/fetch_dxc.py). The checkout's `exporters/` is a required input."""
    if rustc_host() != identity["target"]:
        sys.exit(f"the programs for {identity['target']} are built on that computer, not on {rustc_host()}")
    route = "package-server" if identity["side"] == "server" else "package"
    inputs = json.loads(INPUTS.read_text())
    for name in inputs["routes"][route]["program"]:
        # The `how` strings are whole commands (the packager's refusals and the install guide print them as written).
        command = shlex.split(inputs["how"][name].replace("PROFILE", "--release"))
        if command[:2] != ["cargo", "build"]:
            sys.exit(f"{INPUTS.name}: how[{name!r}] is {inputs['how'][name]!r}; a program is built by "
                     "`cargo build ...`")
        run([*command[:2], "--locked", *command[2:]], cwd=ROOT)
    # Where the player folder's game program would be: its companions are beside it.
    game = lab.cargo_target().resolve() / PROFILE / EXECUTABLE
    run(["cargo", "run", "--locked", "-q", "-p", "xtask", "--", route, "--no-game", str(game), str(folder)], cwd=ROOT)
    if identity["side"] == "client" and identity["target"].startswith("x86_64-pc-windows"):
        run([sys.executable, str(ROOT / "tools/ci/fetch_dxc.py"), str(folder)], cwd=ROOT)
    return folder


def tool(target: str) -> Path:
    """The folder holding `cargo-sunrise-mod` built from this source for `target`, which must be this computer."""
    if (CROSS_TARGET or rustc_host()) != target:
        sys.exit(f"the tool for {target} is built on that computer, not on {rustc_host()}")
    # `--version` names the commit; a build that names none says `dev`.
    environment = {**os.environ, "SUNRISE_TOOL_COMMIT": os.environ.get("SUNRISE_TOOL_COMMIT") or head()}
    run(["cargo", "build", "--locked", "--release", *(["--target", CROSS_TARGET] if CROSS_TARGET else []), "-p", TOOL, "--bin", TOOL],
        cwd=ROOT, env=environment)
    folder = lab.cargo_target().resolve() / (CROSS_TARGET or ".") / "release"
    if not (folder / TOOL_PROGRAM).is_file():
        sys.exit(f"cargo built no {folder / TOOL_PROGRAM}")
    # Only a debug-assertions build reads SUNRISE_SDK_TRUSTED_KEY (sunrise-release-sign's sdk_keys_for), and the
    # variable's name is in the binary exactly when it does: a tool that ships never trusts a key from the
    # environment, whatever a profile, a per-package override or the environment turned on.
    if b"SUNRISE_SDK_TRUSTED_KEY" in (folder / TOOL_PROGRAM).read_bytes():
        sys.exit(f"{folder / TOOL_PROGRAM} was built with debug assertions: it would trust an SDK key from "
                 "SUNRISE_SDK_TRUSTED_KEY")
    return folder


def pack_tool(dist: Path) -> None:
    """DIST/tool-<target>.tar.xz for this computer, the archive `pack` puts in an SDK: built and packed the same way."""
    target = rustc_host()
    dist.mkdir(parents=True, exist_ok=True)
    tar_xz(tool(target), [".", TOOL_PROGRAM], dist / f"tool-{target}.tar.xz")
    print(f"tool-{target}.tar.xz: {(dist / f'tool-{target}.tar.xz').stat().st_size} bytes")


def members(archive: Path, names: tuple[str, ...]) -> dict[str, str]:
    """The SHA-256 of each of `names` that the `tar.xz` `archive` holds at its root, by name."""
    found = {}
    with tarfile.open(archive, "r:xz") as tar:
        for member in tar.getmembers():
            name = member.name.removeprefix("./")
            if name in names and member.isfile():
                found[name] = hashlib.sha256(tar.extractfile(member).read()).hexdigest()
    return found


def gather(legs: Path, dist: Path) -> None:
    """The release folder DIST from the packing legs' folders under LEGS (one per OS and side, each `pack`'s DIST):
    every slice archive once, and one `build-data.tar.xz` and `surface.json`, after checking that every copy of the
    build data is the same: each leg's `build-data.tar.xz`, and the copy inside each leg's `api` slice. A leg whose
    surface or build data differs, or two legs with the same archive, stop it before anything is signed."""
    folders = sorted(path for path in legs.iterdir() if path.is_dir())
    if not folders:
        sys.exit(f"{legs} holds no packing leg")
    dist.mkdir(parents=True, exist_ok=True)
    reference = None
    for folder in folders:
        data = folder / BUILD_DATA_ARCHIVE
        surface = folder / SURFACE
        if not data.is_file() or not surface.is_file():
            sys.exit(f"{folder.name} has no {BUILD_DATA_ARCHIVE} or {SURFACE}")
        copies = {BUILD_DATA_ARCHIVE: members(data, BUILD_DATA)}
        if set(copies[BUILD_DATA_ARCHIVE]) != set(BUILD_DATA):
            sys.exit(f"{folder.name}'s {BUILD_DATA_ARCHIVE} does not hold {', '.join(BUILD_DATA)}")
        for archive in sorted(folder.glob("api-*.tar.xz")):
            copies[archive.name] = members(archive, API_BUILD_DATA)
        seen = (hashlib.sha256(surface.read_bytes()).hexdigest(), copies[BUILD_DATA_ARCHIVE])
        if reference is None:
            reference = (folder.name, *seen)
            shutil.copy2(data, dist / BUILD_DATA_ARCHIVE)
            shutil.copy2(surface, dist / SURFACE)
        if seen[0] != reference[1]:
            sys.exit(f"{folder.name}'s {SURFACE} differs from {reference[0]}'s")
        for name, hashes in copies.items():
            differ = sorted(file for file in (BUILD_DATA if name == BUILD_DATA_ARCHIVE else API_BUILD_DATA)
                            if hashes.get(file) != reference[2][file])
            if differ:
                sys.exit(f"{folder.name}/{name}: {', '.join(differ)} differ from {reference[0]}'s build data")
        for archive in sorted(folder.glob("*.tar.xz")):
            if archive.name == BUILD_DATA_ARCHIVE:
                continue
            if (dist / archive.name).exists():
                sys.exit(f"{archive.name} comes from two legs")
            shutil.copy2(archive, dist / archive.name)
    print(f"gather: {len(folders)} legs, one build data ({', '.join(f'{name} {digest[:12]}' for name, digest in sorted(reference[2].items()))})")


def index(dist: Path, nightly: str | None, previous: tuple[str, Path] | None = None) -> None:
    """DIST/sdk.json over every asset in DIST, written and checked by the one implementation (`sunrise_mod_package::sdk`).
    `previous` is the SDK published before this one and the `sunrise-stage surface-diff --json` of its surface
    against DIST's, which fill the index's surface comparison."""
    commit = head()
    date = run(["git", "log", "-1", "--format=%cd", "--date=format-local:%Y-%m-%d"], cwd=ROOT,
               env={**os.environ, "TZ": "UTC"}).strip()
    run(["cargo", "run", "--locked", "-q", "-p", "xtask", "--", "sdk-index", str(dist), commit, date,
         *(["--nightly", nightly] if nightly else []),
         *(["--previous", previous[0], str(previous[1])] if previous else [])], cwd=ROOT)


# Windows refuses a DLL past 65,535 exported names at link time. `exports` warns past 80% of that and fails past 90%, so
# a growing export list stops a package build with room left to cut it.
EXPORT_LIMIT = 65_535
EXPORT_WARNING = EXPORT_LIMIT * 8 // 10
EXPORT_FAILURE = EXPORT_LIMIT * 9 // 10


def export_count(library: Path) -> int:
    """Windows' exported-name count from a DLL's PE export directory, readable on any OS."""
    data = library.read_bytes()
    pe = int.from_bytes(data[0x3C:0x40], "little")
    optional = pe + 24
    directories = optional + (112 if int.from_bytes(data[optional:optional + 2], "little") == 0x20B else 96)
    export_rva = int.from_bytes(data[directories:directories + 4], "little")
    sections = optional + int.from_bytes(data[pe + 20:pe + 22], "little")
    for i in range(int.from_bytes(data[pe + 6:pe + 8], "little")):
        at = sections + 40 * i
        address, raw_size, raw = (int.from_bytes(data[at + o:at + o + 4], "little") for o in (12, 16, 20))
        if address <= export_rva < address + raw_size:
            directory = raw + export_rva - address
            names = int.from_bytes(data[directory + 24:directory + 28], "little")
            return names
    sys.exit(f"{library.name} has no export directory")


def exports(out: Path) -> None:
    """Report every DLL's Windows export count (#589); warn on one past 80% of
    the limit and fail, after reporting them all, on any past 90%."""
    over = []
    for library in sorted((out / "deps").glob("*.dll")):
        names = export_count(library)
        print(f"{library.name}: {names} exported names (Windows' limit is {EXPORT_LIMIT:,})")
        if names > EXPORT_WARNING:
            warning = (f"{library.name} exports {names:,} names, {EXPORT_LIMIT - names:,} short of Windows' "
                       f"{EXPORT_LIMIT:,}: cut exported names before exposing more internals (#589)")
            print(f"::warning::{warning}")
            if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
                with open(summary, "a", encoding="utf-8") as file:
                    file.write(f"- {warning}\n")
        if names > EXPORT_FAILURE:
            over.append(f"{library.name} ({names:,})")
    if over:
        sys.exit(f"past 90% of Windows' {EXPORT_LIMIT:,} exported names: {', '.join(over)}; cut exported names (#589)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build")
    build_parser.add_argument("out", type=Path)
    build_parser.add_argument("--server", action="store_true", help="the dedicated server's features")
    smoke_parser = commands.add_parser("smoke")
    smoke_parser.add_argument("out", type=Path)
    start_parser = commands.add_parser("start")
    start_parser.add_argument("out", type=Path)
    exports_parser = commands.add_parser("exports")
    exports_parser.add_argument("out", type=Path)
    pack_parser = commands.add_parser("pack")
    pack_parser.add_argument("out", type=Path)
    pack_parser.add_argument("dist", type=Path)
    # `build` compiles sunrise-stage from the commit's own source before staging it (target/prebuilt/tools); the
    # surface export reuses that binary instead of compiling it again in another target folder.
    pack_parser.add_argument("--stage-tool", type=Path, help="the sunrise-stage binary to export the surface with")
    pack_parser.add_argument("--with-tool", action="store_true",
                             help="also pack the target's tool on a server leg (a server-only SDK no client leg completes)")
    pack_parser.add_argument("--programs", action="store_true",
                             help="also pack programs-<target>[-server].tar.xz: the player folder without the game "
                                  "program (needs the checkout's exporters/)")
    tool_parser = commands.add_parser("tool")
    tool_parser.add_argument("dist", type=Path)
    gather_parser = commands.add_parser("gather")
    gather_parser.add_argument("legs", type=Path)
    gather_parser.add_argument("dist", type=Path)
    index_parser = commands.add_parser("index")
    index_parser.add_argument("dist", type=Path)
    index_parser.add_argument("--nightly", help="the Nightly name that ships this build")
    index_parser.add_argument("--previous", nargs=2, metavar=("BUILD", "DIFF"),
                              help="the SDK published before this one, and the surface diff against it")
    args = parser.parse_args()
    if args.command == "build":
        build(args.out.resolve(), args.server)
    elif args.command == "exports":
        exports(args.out.resolve())
    elif args.command == "start":
        start(args.out.resolve())
    elif args.command == "pack":
        pack(args.out.resolve(), args.dist.resolve(), args.stage_tool, args.with_tool, args.programs)
    elif args.command == "tool":
        pack_tool(args.dist.resolve())
    elif args.command == "gather":
        gather(args.legs.resolve(), args.dist.resolve())
    elif args.command == "index":
        index(args.dist.resolve(), args.nightly,
              (args.previous[0], Path(args.previous[1]).resolve()) if args.previous else None)
    else:
        smoke(args.out.resolve())


if __name__ == "__main__":
    main()
