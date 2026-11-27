#!/usr/bin/env python3
"""Compare the two live-level spawn gates independently with the pinned server."""
import argparse
import os
from pathlib import Path
import subprocess

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base

PARITY_MANUAL = "python3 tools/test_spawn_obstruction_parity.py --test-binary GAMEPLAY_TEST_BINARY"
FIXTURE = "game::server_side::natural_spawning::spawn_obstruction_fixture::spawn_entity_obstruction"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path)
    parser.add_argument("--reference-only", action="store_true")
    parser.add_argument("--resources", type=Path, default=lab.RESOURCES)
    parser.add_argument("--recording", type=Path, default=lab.LOCAL / "spawn-obstruction-parity")
    args = parser.parse_args()
    if not args.test_binary and not args.reference_only:
        parser.error("pass the focused sunrise-client libtest binary or --reference-only")

    work = args.recording.resolve()
    classes = work / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    for name in ("vanilla.txt", "rust.txt", "report.json"):
        (work / name).unlink(missing_ok=True)
    source = lab.ROOT / "tests/fixtures/spawn_obstruction.tsv"
    cases = [line.split() for line in source.read_text(encoding="utf-8").splitlines() if not line.startswith("#")]
    inputs = work / "inputs.txt"
    with inputs.open("w", encoding="utf-8") as stream:
        for i, (name, installation, kind, dx, nbt, _, _) in enumerate(cases):
            z = 8.5 + 16 * i
            if installation.startswith("admission-"):
                mode = "obstruction_admission"
                nbt = nbt.replace('id:"spawn_fixture:relay"', 'id:"minecraft:item_display"')
            else:
                mode = "obstruction_batch" if installation == "batch" else "obstruction"
            player = f" p0=4.5,64,{z}" if mode != "obstruction" else ""
            stream.write(f"{name} mode={mode} installation={installation} type={kind} dx={dx} "
                         f"entity_data={nbt} z={z}{player}\n")
    names_and_modes = [(name, "admission" if installation.startswith("admission-") else
                        "batch" if installation == "batch" else "obstruction")
                       for name, installation, *_ in cases]
    lock, jars, libraries = base.verify_artifacts()
    sourcepath = lab.ROOT / "mods/sunrise/src/test/java"
    oracle = sourcepath / "dev/sunriselab/parity/VanillaNaturalSpawnOracle.java"
    deps = os.pathsep.join(map(str, [jars["server"], *libraries]))

    def run(log_name, command, env=None):
        with (work / f"{log_name}.log").open("w", encoding="utf-8") as log:
            return subprocess.run(list(map(str, command)), cwd=work, env=env, stdout=log,
                                  stderr=subprocess.STDOUT, timeout=600).returncode

    if run("javac", ["javac", "--release", lock["minecraft"]["java_major"], "-proc:none",
                     "-sourcepath", sourcepath, "-cp", deps, "-d", classes, oracle]):
        raise SystemExit("oracle compilation failed (see the private javac log)")
    if run("java", ["java", "-Xmx1G", "-cp", str(classes) + os.pathsep + deps,
                    "dev.sunriselab.parity.VanillaNaturalSpawnOracle", inputs, work / "vanilla.txt"]):
        raise SystemExit("oracle failed (see the private Java log)")

    def read_recording(file):
        if not file.is_file():
            raise SystemExit(f"missing recording {file.name}; see the private logs")
        rows = [line.split() for line in file.read_text(encoding="utf-8").splitlines()]
        if [tuple(row[:2]) for row in rows] != names_and_modes or any(
            len(row) != 4 for row in rows
        ):
            raise SystemExit(f"incomplete or invalid recording {file.name}; see the private logs")
        return [row[2:] for row in rows]

    vanilla = read_recording(work / "vanilla.txt")
    expected = [case[-2:] for case in cases]
    if vanilla != expected:
        labels = [name for (name, _), actual, spec in zip(names_and_modes, vanilla, expected) if actual != spec]
        raise SystemExit(f"fixture assumptions disagree with the oracle: {', '.join(labels)}")
    version = lock["minecraft"]["version"]
    if args.reference_only:
        print(f"{version}: reference fixture passed")
        return

    scratch = {key: work / key for key in ("SUNRISE_HOME", "SUNRISE_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME")}
    for path in scratch.values():
        path.mkdir(exist_ok=True)
    env = lab.game_env(**scratch, SUNRISE_TEST_RESOURCES=args.resources.resolve(),
                       SUNRISE_SPAWN_OBSTRUCTION_RECORDING=work / "rust.txt")
    status = run("rust", [args.test_binary.resolve(), FIXTURE, "--exact", "--ignored"], env)
    rust = read_recording(work / "rust.txt")
    columns = {
        "batch": ("accepted", "yaw"),
        "obstruction": ("collision", "obstruction"),
        "admission": ("tick_and_residents", "preview_and_installed_queries"),
    }
    differences = {metric: [] for pair in columns.values() for metric in pair}
    for (name, mode), ref, got in zip(names_and_modes, vanilla, rust):
        for metric, want, actual in zip(columns[mode], ref, got):
            if want != actual:
                differences[metric].append(name)
    lab.write_json(work / "report.json", dict(minecraft=version, cases=len(cases), **differences, test_exit=status))
    if any(differences.values()) or status:
        summary = "; ".join(f"{metric} failures: {', '.join(names) or 'none'}" for metric, names in differences.items())
        raise SystemExit(f"{version}: {summary}; test exit {status}")
    print(f"{version}: spawn obstruction passed ({len(cases)} cases)")


if __name__ == "__main__":
    main()
