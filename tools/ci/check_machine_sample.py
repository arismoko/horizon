#!/usr/bin/env python3
"""Compile the machine sample at PIN against this checkout's API (the daily leg machine-sample.yml).

A pull request that changes the API the sample uses updates PIN to a sample commit that builds against it.
"""
import json
import os
from pathlib import Path
import subprocess
import time
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PIN = "6afa900e312922c4d5fca59e6f9d5dedb53176d2"


def main():
    revision = PIN
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="machine-sample-", dir=ROOT.parent) as directory:
        check(Path(directory), revision)
    elapsed = time.monotonic() - started
    summary = f"Machine sample `{revision}`: {elapsed:.1f} seconds ({elapsed / 60:.2f} runner minutes).\n"
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as output:
            output.write(summary)


def check(directory, revision):
    sample = directory / "sample"
    subprocess.run(["git", "clone", "--no-checkout", "https://github.com/arismoko/horizon-machine-sample", str(sample)], check=True)
    subprocess.run(["git", "fetch", "origin", revision], cwd=sample, check=True)
    subprocess.run(["git", "checkout", "--detach", revision], cwd=sample, check=True)
    # Cargo source patches replace the pinned game dependency with this PR's API.
    patch = sample / "api-patch.toml"
    patch.write_text('[patch."https://github.com/arismoko/horizon"]\n'
                     + 'horizon-mod-api = { path = ' + json.dumps(str(ROOT / "crates/horizon-mod-api")) + ' }\n')
    subprocess.run(["cargo", "check", "--all-targets", "--config", str(patch)], cwd=sample, check=True)


if __name__ == "__main__":
    main()
