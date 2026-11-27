#!/usr/bin/env python3
"""Compare WritableBookContent CODEC/equality and BookEditScreen Done with vanilla 26.3.

VanillaWritableBookOracle runs the real 26.3 WritableBookContent.CODEC,
ItemStack.OPTIONAL_STREAM_CODEC, ItemStack.isSameItemSameComponents, and
BookEditScreen's Done button (saveChanges) on the pinned client jar, writing
writable_book.json and meta.json into .local/witness/<version>/writable_book/.
The sunrise-items and sunrise-client parity tests read that witness through
SUNRISE_WRITABLE_BOOK_WITNESS.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking

ORACLE = "VanillaWritableBookOracle"
ITEMS_TEST = "item_data::ingredient_components::tests::writable_book_pages_keep_their_filtered_text"
CLIENT_TEST = "game::client_side::native_client_screens::client_screen_tests::done_on_an_unedited_book_sends_it_and_drops_its_filtered_text"


def java_version_string():
    result = subprocess.run(["java", "-version"], capture_output=True, text=True, check=True)
    return (result.stderr or result.stdout).strip().splitlines()[0]


def record_witness(build):
    lock, jars, _ = base.verify_artifacts()
    version = lock["minecraft"]["version"]
    witness_dir = lab.LOCAL / "witness" / version / "writable_book"
    witness_dir.mkdir(parents=True, exist_ok=True)
    witness = witness_dir / "writable_book.json"
    classes, dependencies, oracle_path = oracle_harness.compile_oracle(
        build, ORACLE, jars, baking.client_libraries(), lock["minecraft"]["java_major"])
    oracle_harness.java_run(
        build, classes, dependencies, ORACLE, [witness], "vanilla")
    data = json.loads(witness.read_text(encoding="utf-8"))
    if data.get("minecraft") != version:
        raise AssertionError(f"unexpected witness minecraft version: {data.get('minecraft')!r} != {version!r}")
    lab.write_json(witness_dir / "meta.json", {
        "minecraft": version,
        "java_major": lock["minecraft"]["java_major"],
        "java_version": java_version_string(),
        "harness": "tools/test_writable_book_parity.py",
        "oracle": oracle_path.relative_to(lab.ROOT).as_posix(),
        "command": "python3 tools/test_writable_book_parity.py",
        "witness_env": "SUNRISE_WRITABLE_BOOK_WITNESS",
        "input_hashes": {
            "research-lock.json": lab.digest(lab.LOCK, "sha256"),
            "client.jar": lab.digest(jars["client"], "sha256"),
            oracle_path.relative_to(lab.ROOT).as_posix(): lab.digest(oracle_path, "sha256"),
        },
        "output": {
            "file": witness.name,
            "sha256": lab.digest(witness, "sha256"),
            "bytes": witness.stat().st_size,
        },
    })
    return witness


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", "--record-only", action="store_true",
                        help="Run only the vanilla 26.3 JVM oracle and write the witness under .local/witness/.")
    parser.add_argument("--resources", type=Path,
                        default=Path(os.environ.get("SUNRISE_TEST_RESOURCES", lab.RESOURCES)))
    args = parser.parse_args()
    build = Path(os.environ.get("SUNRISE_WRITABLE_BOOK_PARITY_DIR", lab.LOCAL / "writable-book-parity"))
    build.mkdir(parents=True, exist_ok=True)
    witness = record_witness(build)
    if args.record:
        print(f"Recorded vanilla writable book witness: {witness}")
        return
    oracle_harness.run_logged(
        ["cargo", "test", "--locked", "-p", "sunrise-items", "--lib", "--", "--ignored", "--exact", ITEMS_TEST],
        build, "rust-items",
        env=lab.game_env(SUNRISE_WRITABLE_BOOK_WITNESS=witness.resolve()),
    )
    if f"test {ITEMS_TEST} ... ok" not in (build / "rust-items.log").read_text(encoding="utf-8"):
        raise AssertionError(f"Rust witness replay did not run: {ITEMS_TEST}")
    oracle_harness.run_logged(
        [lab.parity_fixtures(), CLIENT_TEST, "--ignored", "--exact"],
        build, "rust-client",
        env=lab.game_env(
            SUNRISE_TEST_RESOURCES=args.resources.resolve(),
            SUNRISE_WRITABLE_BOOK_WITNESS=witness.resolve(),
        ),
    )
    if f"test {CLIENT_TEST} ... ok" not in (build / "rust-client.log").read_text(encoding="utf-8"):
        raise AssertionError(f"Rust witness replay did not run: {CLIENT_TEST}")
    print(f"Vanilla writable book codec and BookEditScreen parity passed (against {witness})")


if __name__ == "__main__":
    main()
