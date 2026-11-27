#!/usr/bin/env python3
"""Compare body submission order after admission, removal, riding and camera changes.

The vanilla side runs client level storage, camera filtering and submission;
the Sunrise probe sends packets through the client and observes nonempty body
submissions. This compares order, not frustum culling, fire, pixels or GPU draw order.
"""
import json
import os

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness


def snapshots(path):
    prefix = "entity-draw-order: "
    # Libtest may print the running test name on the same line as its first snapshot.
    return [json.loads(line.split(prefix, 1)[1])
            for line in path.read_text().splitlines() if prefix in line]


def main():
    lock, directory = lab.config()
    manifest, client = lab.import_client(lock, directory)
    libraries = [lab.import_library(artifact, directory / "libraries/client")
                 for artifact in lab.client_library_artifacts(manifest)
                 if "/natives-" not in artifact["path"]]
    build = lab.LOCAL / "entity-draw-order"
    build.mkdir(parents=True, exist_ok=True)
    oracle = "VanillaEntityDrawOrderOracle"
    classes, dependencies, _ = oracle_harness.compile_oracle(
        build, oracle, {"client": client}, libraries, lock["minecraft"]["java_major"])
    scene = lab.ROOT / "crates/sunrise-client-gameplay/src/game/modding/entity-draw-order.txt"
    oracle_harness.java_run(build, classes, dependencies, oracle, [scene], "vanilla")
    env = lab.game_env(
        SUNRISE_TEST_RESOURCES=os.environ.get("SUNRISE_TEST_RESOURCES", lab.RESOURCES),
        CARGO_TARGET_DIR=lab.LOCAL / "parity-target",
    )
    oracle_harness.run_logged(
        [lab.parity_fixtures(),
         "entity_draw_order_after_client_packets", "--ignored", "--nocapture"],
        build, "rust", env=env, timeout=2400)
    expected, actual = snapshots(build / "vanilla.log"), snapshots(build / "rust.log")
    frames = scene.read_text().splitlines().count("frame")
    if not frames or len(expected) != frames or actual != expected:
        raise AssertionError("Body submission order differs; see the local entity-draw-order logs")
    print(f"{frames} mixed-body submission snapshots match the pinned vanilla client")


if __name__ == "__main__":
    main()
