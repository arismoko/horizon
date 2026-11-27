#!/usr/bin/env python3
"""Reuse Cargo's cached workspace artifacts without hiding changed checkout inputs.

Run after restoring target and before Cargo. Preserve an input's old mtime only
when its Git object (including its mode) is identical. Tree objects cover directory
inputs to build scripts, including additions and deletions. Commit dates alone are
not safe: a newly merged change can have an older author or committer date.

Every job that builds in a shared persistent target must run this first, or its
artifacts outlive a snapshot that no longer describes them. The snapshot's name
carries a version: renaming it discards every runner's old snapshot once, so
targets left with another tree's artifacts are judged afresh. Missing or damaged
snapshots refresh all tracked mtimes. Replace a snapshot only after writing its
new contents in full.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def restore_and_snapshot(root, snapshot):
    # The object IDs describe the checkout only while its tracked inputs are clean.
    subprocess.run(["git", "diff", "--exit-code", "HEAD", "--"], cwd=root,
                   check=True, stdout=subprocess.DEVNULL)
    restored, total = restore_tree_mtimes(root, "HEAD", root, snapshot)
    print(f"Restored mtimes for {restored} unchanged Git files and trees out of {total}")


def restore_tree_mtimes(repo: Path, tree: str, root: Path, snapshot: Path) -> tuple[int, int]:
    """Reconcile a verified checkout or a fresh full archive of `tree` from `repo`.

    For an archive, the caller must hold the artifact cache lock from extraction
    through the build and pass the same resolved tree ID used for extraction.
    """
    entries = subprocess.check_output(
        ["git", "ls-tree", "-r", "-t", "-z", tree], cwd=repo,
    )
    try:
        previous = json.loads(snapshot.read_text(encoding="utf-8"))
    except (FileNotFoundError, ValueError, RecursionError):
        previous = {}
    # Treat a malformed record as a lost snapshot, not a partially trusted cache.
    # Fresh mtimes come from time.time_ns(), a signed 64-bit clock in CPython.
    if not isinstance(previous, dict) or any(
        not isinstance(record, list) or len(record) != 2
        or not isinstance(record[0], str) or type(record[1]) is not int
        or not -(2**63) <= record[1] < 2**63
        for record in previous.values()
    ):
        previous = {}
    current = {}
    restored = 0
    for entry in entries.split(b"\0"):
        if not entry:
            continue
        metadata, name = entry.decode("utf-8").split("\t", 1)
        mode, kind, oid = metadata.split()
        # Do not follow symlinks or enter submodules. Cargo's source files and
        # directory watches live in the checkout, not wherever a link points.
        if kind == "commit" or mode == "120000":
            continue
        path = root / name
        identity = f"{mode} {oid}"
        stat = path.stat()
        old = previous.get(name)
        if old is not None and old[0] == identity:
            mtime = old[1]
            restored += 1
        else:
            # The cache may have been saved after this job's checkout started.
            # Changed inputs must be newer than its artifacts, not merely new
            # relative to this checkout's start time.
            mtime = time.time_ns()
        os.utime(path, ns=(stat.st_atime_ns, mtime))
        current[name] = [identity, path.stat().st_mtime_ns]
    snapshot.parent.mkdir(parents=True, exist_ok=True)
    pending = snapshot.with_name(snapshot.name + ".pending")
    pending.write_text(json.dumps(current, sort_keys=True) + "\n", encoding="utf-8")
    pending.replace(snapshot)
    return restored, len(current)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", type=Path, help="snapshot inside the restored target directory")
    args = parser.parse_args()
    restore_and_snapshot(Path.cwd(), args.snapshot)
