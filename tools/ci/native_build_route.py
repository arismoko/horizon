#!/usr/bin/env python3
"""Build one native mod through the shipped route: `cargo sunrise-mod native-build` against a prebuilt SDK folder.

The mod is the native starter plus one declared game internal, so it takes the engine route (the facade rlib and the
mod library are compiled against the prebuilt game). The SDK is laid out as an SDK release's slices with
`native-layout`, as the SDK a leg has just built has no release to fetch.

    native_build_route.py --sdk prebuilt --tool target/release/cargo-sunrise-mod --work target/native-route
"""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools import prebuilt_game as sdk_tools

TEMPLATE = sdk_tools.ROOT / "crates/cargo-sunrise-mod/template/native"
MIXINS_REV = re.search(r'REV: &str = "(\w+)"', (sdk_tools.ROOT / "crates/sunrise-mod-package/src/mixins_tool.rs").read_text())[1]


def tool(command, plan, extra_input=""):
    """One plan line in, the tool's standard output and exit code."""
    done = subprocess.run([*command], input=json.dumps(plan) + "\n" + extra_input, capture_output=True, text=True)
    sys.stderr.write(done.stderr)
    return done.stdout, done.returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path, help="the folder `prebuilt_game.py build` made")
    parser.add_argument("--tool", required=True, type=Path, help="the built cargo-sunrise-mod")
    parser.add_argument("--work", required=True, type=Path)
    args = parser.parse_args()
    sdk, work = args.sdk.resolve(), args.work.resolve()
    identity = json.loads((sdk / sdk_tools.IDENTITY).read_text())
    shutil.rmtree(work, ignore_errors=True)
    mods = work / "mods"
    mod = mods / "route_probe"
    (mod / "src").mkdir(parents=True)
    fill = lambda text: text.replace("{id}", "route_probe").replace("{name}", "Route probe").replace("{metadata}", "")
    (mod / "Cargo.toml").write_text(fill((TEMPLATE / "api.toml").read_text()))
    (mod / "src/lib.rs").write_text(fill((TEMPLATE / "lib.rs").read_text()))
    (mod / "sunrise-mod.toml").write_text(
        fill((TEMPLATE / "sunrise-mod.toml").read_text())
        + f'\n[native]\ngame = "26.3"\nsunrise = "{identity["commit"]}"\nmixins = "{MIXINS_REV}"\n'
        + '\n[[internals]]\npath = "sunrise_gameplay_common::game::CommonGame"\n')

    command = [str(args.tool), "sunrise-mod"]
    reply, code = tool([*command, "native-layout"], {
        "contract": 2, "prebuilt": str(sdk), "build": identity["commit"], "host": identity["target"],
        "target": identity["side"], "out": str(work / "package")})
    print(reply)
    if code != 0:
        sys.exit(f"native-layout exited {code}")
    package = json.loads(reply)["package"]

    sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
    events, code = tool([*command, "native-build"], {
        "contract": 2, "work": str(work / "build"), "toolchain": {
            "dir": sysroot, "target": identity["target"], "cargo_home": str(Path.home() / ".cargo")},
        "package": package, "mods": str(mods), "target": identity["side"], "registry": False})
    print(events)
    log = work / "build/build.log"
    if code != 0:
        print(log.read_text() if log.is_file() else "no build.log")
        sys.exit(f"native-build exited {code}")
    last = json.loads(events.strip().splitlines()[-1])
    if last["event"] != "built" or last["route"] != "prebuilt":
        sys.exit(f"expected a built event on the engine route, got {last}")
    missing = [library for library in last["libraries"] if not Path(library).is_file()]
    if missing:
        sys.exit(f"built libraries missing: {missing}")
    print(f"PASS: {len(last['libraries'])} libraries through the engine route")


if __name__ == "__main__":
    main()
