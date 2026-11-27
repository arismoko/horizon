#!/usr/bin/env python3
"""Compare ridden NoAI donkey travel and rotation with vanilla 26.3 on the JVM.

Compiles and runs VanillaNoAiRiddenTravelOracle on a real headless flat 26.3 ServerLevel,
writing the witness TSV and meta.json into .local/witness/<version>/noai_ridden_travel/,
then runs Sunrise's bit-exact witness test against it via SUNRISE_NOAI_RIDDEN_TRAVEL_WITNESS.
Use --record (or --record-only) to record the witness into .local without running Cargo.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness as harness, test_rust_parity as base

ORACLE = lab.ROOT / "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaNoAiRiddenTravelOracle.java"
TEST = "mobs::ai::noai_ridden_tests::a_ridden_noai_donkey_travels_and_turns_with_its_rider"
WITNESS_ENV = "SUNRISE_NOAI_RIDDEN_TRAVEL_WITNESS"


def java_version_string():
    result = subprocess.run(["java", "-version"], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def record(build, lock, jars, libraries):
    version = lock["minecraft"]["version"]
    witness_dir = lab.LOCAL / "witness" / version / "noai_ridden_travel"
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / "noai_ridden_travel.tsv"
    classes = build / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    cp = os.pathsep.join(map(str, [jars["server"], *libraries]))
    harness.run_logged(
        ["javac", "--release", str(lock["minecraft"]["java_major"]), "-proc:none",
         "-cp", cp, "-d", classes, ORACLE],
        build,
        "compile-java",
    )
    # Omit the optional world-dir argument so VanillaNoAiRiddenTravelOracle uses and removes its
    # own temp directory, leaving witness as the sole tracked output for tools/parity_shim.py.
    oracle_cmd = [
        "java", "-Xmx2g", "-Djava.awt.headless=true", "-cp", str(classes) + os.pathsep + cp,
        "dev.sunriselab.parity.VanillaNoAiRiddenTravelOracle", witness,
    ]
    harness.run_logged(oracle_cmd, build, "vanilla")
    if not witness.is_file():
        raise SystemExit(f"VanillaNoAiRiddenTravelOracle did not write {witness}")
    data_rows = [
        line for line in witness.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    ]
    if not data_rows:
        raise SystemExit(f"VanillaNoAiRiddenTravelOracle wrote no rows to {witness}")
    lab.write_json(
        witness_dir / "meta.json",
        dict(
            game_version=version,
            java_major=lock["minecraft"]["java_major"],
            java_version=java_version_string(),
            harness="tools/test_noai_ridden_travel_parity.py",
            oracle=ORACLE.relative_to(lab.ROOT).as_posix(),
            witness_env=WITNESS_ENV,
            command=[str(part) for part in oracle_cmd],
            input_hashes={
                "research-lock.json": lab.digest(lab.LOCK, "sha256"),
                "server.jar": lab.digest(jars["server"], "sha256"),
                ORACLE.relative_to(lab.ROOT).as_posix(): lab.digest(ORACLE, "sha256"),
            },
            witness={
                "file": witness.name,
                "sha256": lab.digest(witness, "sha256"),
                "bytes": witness.stat().st_size,
                "rows": len(data_rows),
            },
        ),
    )
    return witness, len(data_rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    parser.add_argument(
        "--record", "--record-only", dest="record", action="store_true",
        help="record the 26.3 witness file into .local and exit",
    )
    parser.add_argument(
        "--java-only", action="store_true",
        help="run the vanilla oracle to record the witness into .local and exit",
    )
    parser.add_argument(
        "--resources", type=Path,
        default=Path(os.environ.get("SUNRISE_TEST_RESOURCES", lab.RESOURCES)),
    )
    args = parser.parse_args()
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / "noai-ridden-travel-parity"
    build.mkdir(parents=True, exist_ok=True)
    report = build / "report.json"
    report.unlink(missing_ok=True)
    sources = [
        Path(__file__).resolve(),
        ORACLE,
        lab.ROOT / "tools/lab.py",
        lab.ROOT / "tools/oracle_harness.py",
        lab.ROOT / "tools/test_rust_parity.py",
        lab.ROOT / "Cargo.toml",
        lab.ROOT / "Cargo.lock",
        lab.ROOT / "crates/sunrise-sim/src/mobs/ai.rs",
        lab.ROOT / "crates/sunrise-sim/src/mobs/equine.rs",
        lab.ROOT / "crates/sunrise-sim/src/mobs/living.rs",
        lab.ROOT / "crates/sunrise-sim/src/mobs/living/travel.rs",
    ]
    hashes = base.source_hashes(sources)
    witness, row_count = record(build, lock, jars, libraries)
    if args.record or args.java_only:
        base.ensure_sources_unchanged(sources, hashes)
        print(f"Recorded {witness.relative_to(lab.ROOT)} ({row_count} data rows)")
        return
    resources = args.resources.resolve()
    if not (resources / "entity-types.json").is_file():
        raise SystemExit("prepared resources required; pass --resources PATH")
    with tempfile.TemporaryDirectory(prefix="sunrise-noai-ridden-") as scratch:
        env = lab.game_env(
            SUNRISE_HOME=scratch,
            SUNRISE_TEST_RESOURCES=str(resources),
            **{WITNESS_ENV: str(witness.resolve())},
        )
        env.setdefault("CARGO_BUILD_JOBS", "1")
        env.setdefault("RUST_TEST_THREADS", "2")
        harness.run_logged(
            ["cargo", "test", "--locked", "-p", "sunrise-client-lib", "--lib", TEST, "--", "--exact", "--ignored"],
            build,
            "rust",
            env=env,
        )
    rust_log = build / "rust.log"
    if "1 passed; 0 failed" not in rust_log.read_text(encoding="utf-8", errors="replace"):
        raise SystemExit(f"expected 1 passed; 0 failed for {TEST} in {rust_log}")
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(
        report,
        dict(
            schema=1,
            minecraft=lock["minecraft"]["version"],
            sources_sha256=hashes,
            server_jar_sha256=lab.digest(jars["server"], "sha256"),
            witness_sha256=lab.digest(witness, "sha256"),
            witness_rows=row_count,
            scope=__doc__,
        ),
    )
    print(f"Ridden NoAI donkey travel parity passed ({row_count} witness rows against {witness}): {report}", flush=True)


if __name__ == "__main__":
    main()
