#!/usr/bin/env python3
"""Slice experiment (sdk-lead-slice): an API-only native mod built from the API slice of the prebuilt
package and loaded through a replaceable `horizon_native_mods` library, on Linux, Windows (gnu) and macOS.

    python3 slice.py SDK WORK [--resources DIR] [--seconds N]

SDK is an unpacked prebuilt package (v5). Steps:
  1. SDK side (what CI would do once): the stub library and the executable, with all the package's metadata.
  2. The API slice: the closure of mod_api and mod_points (api_slice.txt) copied out of deps/ and host/.
  3. Player side: the mods and the real `horizon_native_mods` as rlibs with ONLY the slice on the search
     path, then the C linker makes the shared library against the two game libraries and std.
  4. Checks: both libraries export the same symbols; the executable starts with the replacement beside it,
     with no data (stops at the missing resources) and, with --resources, as a dedicated server.
Every step prints `slice-step:` with its time; every mod prints `slice:` lines.
"""
import argparse
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
OS = platform.system()  # Linux, Windows, Darwin
DLL = {"Linux": ("lib", ".so"), "Windows": ("", ".dll"), "Darwin": ("lib", ".dylib")}[OS]
EXE = ".exe" if OS == "Windows" else ""
MODS = ["slice_smoke", "slice_generic", "slice_inject"]


def dylib(name: str) -> str:
    return f"{DLL[0]}{name}{DLL[1]}"


def run(cmd, step=None, **kw):
    cmd = [str(c) for c in cmd]
    print("+", " ".join(cmd), flush=True)
    t = time.monotonic()
    out = subprocess.run(cmd, check=True, text=True, capture_output=True, **kw)
    if out.stdout.strip():
        print(out.stdout.rstrip())
    if out.stderr.strip():
        print(out.stderr.rstrip())
    if step:
        print(f"slice-step: {step}: {time.monotonic() - t:.2f} s", flush=True)
    return out.stdout


def rmeta(directory: Path, crate: str) -> Path:
    found = sorted(directory.glob(f"lib{crate}-*.rmeta"))
    assert len(found) == 1, (crate, found)
    return found[0]


def size(paths) -> int:
    return sum(p.stat().st_size for p in paths)


def std_dylib() -> Path:
    libdir = Path(run(["rustc", "--print", "target-libdir"]).strip())
    if OS == "Windows":
        # rustup's gnu toolchain keeps std's DLL in the sysroot's bin and lib dirs alike.
        found = sorted(libdir.glob("std-*.dll"))
    else:
        found = sorted(libdir.glob(f"libstd-*{DLL[1]}"))
    assert found, libdir
    return found[0]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("sdk", type=Path)
    parser.add_argument("work", type=Path)
    parser.add_argument("--resources", type=Path)
    parser.add_argument("--seconds", type=int, default=60)
    args = parser.parse_args()
    sdk, work = args.sdk.resolve(), args.work.resolve()
    shutil.rmtree(work, ignore_errors=True)
    full, slim, out, rundir = (work / d for d in ("full", "slim", "out", "run"))
    for d in (full, slim / "deps", slim / "host", out, rundir):
        d.mkdir(parents=True)
    identity = json.loads((sdk / "horizon-prebuilt.json").read_text())
    print("slice-sdk:", identity["commit"], identity.get("target"), identity.get("side"))
    link_args = identity.get("link_args", [])
    deps = sdk / "deps"
    common = ["--edition", "2024", "-C", "prefer-dynamic", "-C", "opt-level=3"]

    # 1. SDK side: stub and executable against the whole package.
    host_lib = deps / dylib("horizon_mod_host")
    wasm_lib = deps / dylib("horizon_mod_wasm")
    sdk_flags = [*common, "-L", f"dependency={deps}", "-L", f"dependency={sdk / 'host'}", "-L", f"native={sdk / 'native'}"]
    run(["rustc", *sdk_flags, "--crate-type", "dylib", "--crate-name", "horizon_native_mods", "-C", "metadata=nm",
         "--extern", f"mod_api={rmeta(deps, 'mod_api')}", "--extern", f"horizon_mod_host={host_lib}",
         "--out-dir", full, HERE / "stub.rs"], "stub library (SDK side)")
    origin = {"Linux": ["-Wl,-rpath,$ORIGIN"], "Darwin": ["-Wl,-rpath,@executable_path"], "Windows": []}[OS]
    run(["rustc", *sdk_flags, "--crate-type", "bin", "--crate-name", "horizon_native_game",
         *(f for a in origin + link_args for f in ("-C", f"link-arg={a}")),
         "--extern", f"horizon_client={rmeta(deps, 'horizon_client')}", "--extern", f"mod_api={rmeta(deps, 'mod_api')}",
         "--extern", f"horizon_mod_package={rmeta(deps, 'horizon_mod_package')}",
         "--extern", f"horizon_mod_host={host_lib}",
         "--extern", f"horizon_native_mods={full / dylib('horizon_native_mods')}",
         "-o", full / f"horizon-client{EXE}", HERE / "main.rs"], "executable against the stub (SDK side)")

    # 2. The API slice.
    names = set((HERE / "api_slice.txt").read_text().split())
    picked = []
    for sub in ("deps", "host"):
        for f in sorted((sdk / sub).iterdir()):
            m = re.match(r"(?:lib)?(.+?)-[0-9a-f]{16}\.(rmeta|so|dll|dylib)$", f.name)
            if m and m.group(1) in names:
                shutil.copy2(f, slim / sub / f.name)
                picked.append(slim / sub / f.name)
    print(f"slice-measure: API slice {len(picked)} files, {size(picked) / 1e6:.1f} MB raw:",
          " ".join(p.parent.name + "/" + p.name for p in picked))
    whole = [p for p in sdk.rglob("*") if p.is_file()]
    print(f"slice-measure: whole package {len(whole)} files, {size(whole) / 1e6:.1f} MB raw")

    # 3. Player side: only the slice on the search path.
    player = [*common, "-L", f"dependency={slim / 'deps'}", "-L", f"dependency={slim / 'host'}", "-L", f"dependency={out}"]
    api = ["--extern", f"mod_api={rmeta(slim / 'deps', 'mod_api')}"]
    points_extern = ["--extern", f"mod_points={rmeta(slim / 'deps', 'mod_points')}"]
    catalog = json.loads((sdk / "points.json").read_text())["points"]
    entries, slots, built, rlibs = [], [], [], []
    for i, mod in enumerate(MODS):
        src = HERE / "mods" / mod
        manifest = (src / "horizon-mod.toml").read_text()
        run(["rustc", *player, "--crate-type", "rlib", "--crate-name", f"mod_{i}", "-C", f"metadata={mod}",
             *api, *(points_extern if "[[inject]]" in manifest else []), "--out-dir", out, src / "src/lib.rs"],
            f"compile {mod} (API slice only)")
        rlibs.append(out / f"libmod_{i}.rlib")
        entries.append(f'mod_api::ModEntry::new("{mod}", mod_{i}::hooks())')
        built.append(f'("{mod}", "0.1.0", "Both")')
        # [[inject]] rows: the launcher's emitter (horizon_mod_point_emit::api) writes
        # `mod_points::listeners::<place>(id, priority, inject, listener)` for a head listener.
        for block in manifest.split("[[inject]]")[1:]:
            row = dict(re.findall(r'^(\w+) = "(.*)"$', block, re.M))
            plain = lambda s: re.sub(r"<[^>]*>", "", s).replace(" ", "")
            point = next(p for p in catalog if plain(p["target"]) == plain(row["at"]))
            print(f"slice-point: {row['at']} -> {point['name']} returns={point.get('returns')} host={point.get('host')}")
            slots.append(f'mod_points::listeners::{point["name"]}("{mod}", 0, "{row["id"]}", mod_{i}::{row["with"]})')
        pkg = rundir / "horizon-native-mods" / mod
        pkg.mkdir(parents=True)
        (pkg / "horizon-mod.toml").write_text(manifest.replace('horizon = "HEAD"', f'horizon = "{identity["commit"]}"'))
    real = out / "real.rs"
    real.write_text(
        "//! Generated by the launcher: the instance's API-only mods.\n"
        f"pub fn entries() -> Vec<mod_api::ModEntry> {{ vec![{', '.join(entries)}] }}\n"
        f"pub fn slots() -> Vec<mod_api::__private::points::SlotListener> {{ vec![{', '.join(slots)}] }}\n"
        f"pub fn built() -> Vec<(&'static str, &'static str, &'static str)> {{ vec![{', '.join(built)}] }}\n")
    print(real.read_text())
    run(["rustc", *player, "--crate-type", "rlib", "--crate-name", "horizon_native_mods", "-C", "metadata=nm",
         *api, *points_extern, *(f for i in range(len(MODS)) for f in ("--extern", f"mod_{i}={out / f'libmod_{i}.rlib'}")),
         "--out-dir", out, real], "compile the real horizon_native_mods (API slice only)")
    rlibs.insert(0, out / "libhorizon_native_mods.rlib")
    # Pass the object members only: an rlib also holds lib.rmeta, which the linkers warn about or refuse.
    objs = out / "objs"
    objs.mkdir()
    members = []
    for n, rlib in enumerate(rlibs):
        d = objs / str(n)
        d.mkdir()
        run(["ar", "x", rlib], cwd=d)
        members += sorted(p for p in d.iterdir() if p.suffix == ".o")
    std = std_dylib()
    lib = out / dylib("horizon_native_mods")
    if OS == "Linux":
        link = ["cc", "-shared", "-o", lib, *members, "-Wl,--no-undefined", "-L", deps,
                "-lhorizon_mod_host", "-lhorizon_mod_wasm", std, "-lgcc_s", "-lc"]
    elif OS == "Darwin":
        link = ["cc", "-dynamiclib", "-o", lib, *members, "-install_name", f"@rpath/{dylib('horizon_native_mods')}",
                host_lib, wasm_lib, std]
    else:
        # MinGW's ld links straight against the DLLs (or their .dll.a); export only the three functions.
        link = ["gcc", "-shared", "-o", lib, *members, host_lib, wasm_lib, std, "-Wl,--no-undefined"]
    run(link, "C link of the mods library")
    print(f"slice-measure: mods library {lib.name} {lib.stat().st_size} bytes; rlibs {size(rlibs)} bytes")

    # 4. Checks.
    def exports(path: Path) -> list[str]:
        if OS == "Windows":
            text = run(["objdump", "-p", path])
            table = text.split("[Ordinal/Name Pointer] Table", 1)[-1]
            return sorted(set(re.findall(r"\]\s+(_R\S*horizon_native_mods\S*)", table)))
        text = run(["nm", "-gU" if OS == "Darwin" else "-D", *([] if OS == "Darwin" else ["--defined-only"]), path])
        return sorted({l.split()[-1] for l in text.splitlines() if "19horizon_native_mods" in l})

    stub_syms, real_syms = exports(full / dylib("horizon_native_mods")), exports(lib)
    print("slice-check: stub exports", stub_syms)
    print("slice-check: replacement exports", real_syms)
    imported = [s for s in stub_syms if s.split("_")[-1] in ("entries", "slots", "built") or s.endswith(("7entries", "5slots", "5built"))]
    missing = [s for s in imported if s not in real_syms]
    print("slice-check: stub symbols missing from the replacement:", missing or "none")
    if OS == "Windows":
        dump = run(["objdump", "-p", lib])
        print("slice-check: replacement imports DLLs:", sorted(set(re.findall(r"DLL Name: (\S+)", dump))))
        print("slice-check: replacement export count:", len(re.findall(r"^\s+\[\s*\d+\]", dump.split("[Ordinal/Name Pointer] Table", 1)[-1], re.M)))
        exe_dump = run(["objdump", "-p", full / f"horizon-client{EXE}"])
        print("slice-check: executable imports DLLs:", sorted(set(re.findall(r"DLL Name: (\S+)", exe_dump))))
    elif OS == "Darwin":
        run(["otool", "-L", full / f"horizon-client{EXE}"])
        run(["otool", "-L", lib])
        run(["otool", "-D", lib])
        run(["codesign", "-dv", lib])
    else:
        run(["readelf", "-d", lib])

    shutil.copy2(full / f"horizon-client{EXE}", rundir)
    shutil.copy2(lib, rundir)
    for f in (host_lib, wasm_lib, std):
        shutil.copy2(f, rundir)
    if OS == "Windows":
        gcc_bin = Path(shutil.which("gcc")).parent
        for name in ("libgcc_s_seh-1.dll", "libwinpthread-1.dll"):
            shutil.copy2(gcc_bin / name, rundir)
    if OS == "Linux":
        print(run(["ldd", rundir / "horizon-client"], env={**os.environ, "LD_LIBRARY_PATH": str(rundir)}))

    home = Path(tempfile.mkdtemp(prefix="slice-home-"))
    env = {**os.environ, "HORIZON_HOME": str(home / "home"), "XDG_DATA_HOME": str(home / "data"),
           "XDG_CONFIG_HOME": str(home / "config"), "HORIZON_LAUNCHER_HOME": str(home / "launcher"),
           "HORIZON_LAUNCHER_NO_SELF_UPDATE": "1", "HORIZON_NO_AUDIO": "1", "RUST_BACKTRACE": "1"}
    if OS == "Linux":
        env["LD_LIBRARY_PATH"] = str(rundir)
    game = rundir / "game"
    game.mkdir()
    (game / "eula.txt").write_text("eula=true\n")
    cmd = [str(rundir / f"horizon-client{EXE}"), "--dedicated", "--nogui", "--game-dir", str(game)]
    if args.resources:
        cmd += ["--resources", str(args.resources.resolve())]
    else:
        cmd += ["--resources", str(home / "no-resources")]
    print("+", " ".join(cmd), flush=True)
    t = time.monotonic()
    proc = subprocess.Popen(cmd, cwd=game, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True)
    lines = []
    deadline = time.monotonic() + args.seconds
    want = {"slice: slice_smoke hook server_tick fired", "slice: slice_inject listener at Game::tick head called"}
    import threading
    def reader():
        for line in proc.stdout:
            lines.append(line.rstrip())
            print("game|", line.rstrip(), flush=True)
    th = threading.Thread(target=reader, daemon=True)
    th.start()
    while proc.poll() is None and time.monotonic() < deadline:
        if args.resources and all(any(w in l for l in lines) for w in want):
            break
        time.sleep(0.5)
    if proc.poll() is None:
        try:
            proc.stdin.write("stop\n")
            proc.stdin.flush()
            proc.wait(30)
        except Exception:
            proc.kill()
    th.join(5)
    print(f"slice-step: game run: {time.monotonic() - t:.2f} s, exit {proc.returncode}")
    for log in sorted(home.rglob("*.log")) + sorted(game.rglob("*.log")):
        for line in log.read_text(errors="replace").splitlines():
            if "slice" in line or "refus" in line.lower() or "error" in line.lower():
                print(f"log|{log.name}| {line}")
    print("slice-result:", sorted({l for l in lines if l.startswith("slice:")}))


if __name__ == "__main__":
    main()
