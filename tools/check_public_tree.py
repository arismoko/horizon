#!/usr/bin/env python3
"""Reject local game artifacts and removed commercial features even when somebody uses git add -f."""

import argparse
import collections
from pathlib import PurePosixPath
import subprocess
import sys

PRIVATE_ROOTS = {".local", "game-files", "decompiled", "minecraft", "assets", "run", "logs"}
PRIVATE_EXTENSIONS = {".jar", ".class", ".jfr", ".hprof", ".java"}
ORIGINAL_JDK_TOOLS = {
    # Original observer of public Paper API events, with no server implementation code.
    "tools/fixtures/paper-trace-plugin/PaperTrace.java",
    "tools/JfrSummary.java",
    # Original generator of the Java character tables: calls only the public Character API and prints ranges; no JDK implementation source.
    "tools/JavaCharacterTables.java",
    "tests/java/JfrFixture.java",
    # Original differential-fuzz driver for java.lang.Math: calls only the public Math API on generated inputs (its
    # "mthtable" case applies Mth's one-line sine-table formula, as crates/sunrise-core/src/geometry/trig.rs does).
    "tools/jmath_fuzz/JmathFuzz.java",
    # Original differential-fuzz driver for vanilla's math helpers: no game source; it calls the 26.3 jar's methods at
    # run time (public ones, and Entity's private getInputVector by reflection) from the player's own prepared jars.
    "tools/jmath_fuzz/VanillaMathFuzz.java",
    # HISTORY-ONLY: the modified UTF-8 speed driver moved to mods/sunrise/src/test/java/dev/sunriselab/parity/.
    # Keep its old path allowed so --history/--pre-push accept its legitimate historical addition.
    "tools/modified_utf8_kit/java/Mutf8Bench.java",
    # HISTORY-ONLY: this original comparison driver is no longer in the current tree.
    # Keep its old path allowed so --history/--pre-push accept its legitimate historical addition.
    "tools/refactors/api-pr3/JsonNbtOracle.java",
}
# The prepared folder's files in one file (crates/sunrise-core/src/prepared_files.rs): Mojang's data.
PRIVATE_NAMES = {"sunrise-data.bin"}
# Vanilla's 1.13 data fixer tables, copied from its code until 2026-11; sunrise-prepare now exports them from the
# player's jar (datafix-tables.json). Their old paths stay in history, so only the current tree is checked.
RETIRED_TABLES = {"crates/sunrise-world/src/datafix/flat_tables.rs",
                  "crates/sunrise-world/src/datafix/block_state_data.txt"}
# A file with this many entries like those tables' holds a copy of them, whatever its name or format:
# ItemStackTheFlatteningFix's `id.damage` keys, and BlockStateData's `id<TAB>name<TAB>` rows (raw or JSON-escaped).
TABLE_FINGERPRINTS = [r"minecraft:[a-z_]+\.[0-9]+", "[0-9]+(\t|\\\\t)minecraft:[a-z_]+(\t|\\\\t)"]
TABLE_ENTRIES = 50

# Commercial features #1473 removed from this fork. A sync from a tree that still
# has them must not bring the paths back. Checked on the current tree only, not
# on --history/--range/--pre-push: the files remain in git history from before
# the removal, and main's history scan would otherwise fail.
REMOVED_COMMERCIAL_PREFIXES = (
    "crates/sunrise-cosmetics",
    # Upstream's in-game side of the cosmetics (emotes, the emote wheel, their effects).
    "crates/sunrise-client/src/game/cosmetics",
    "crates/sunrise-client/src/game/interface/cosmetics",
    "crates/sunrise-client/src/cut_layout/game/cosmetics",
    "crates/sunrise-client/src/cut_layout/game/interface/cosmetics",
    "crates/sunrise-gameplay-common/src/game/cosmetics",
    "crates/sunrise-gameplay-common/src/game/interface/cosmetics",
    "crates/sunrise-gameplay/src/game/cosmetics",
    "crates/sunrise-gameplay/src/game/interface/cosmetics",
    "crates/sunrise-client-gameplay/src/game/cosmetics",
    "crates/sunrise-client-gameplay/src/game/interface/cosmetics",
    "crates/sunrise-official-servers",
)
# The launcher lives in sunrise-org/sunrise-launcher (launcher split D1); its removed commercial pages and sculk art
# went with the tree, so nothing under launcher/ comes back.
REMOVED_TREE = "launcher/"


def commercial(path):
    """True when `path` is a removed commercial crate or game module, or anything under the removed launcher/ tree."""
    if path.startswith(REMOVED_TREE):
        return True
    parsed = PurePosixPath(path)
    for prefix in REMOVED_COMMERCIAL_PREFIXES:
        if path == prefix or path.startswith(prefix + "/"):
            return True
        parent = PurePosixPath(prefix)
        # partner.rs besides a partner/ folder, and the same for cosmetics, shop, wardrobe, wardrobe_page.
        if parsed.parent == parent.parent and parsed.stem == parent.name:
            return True
    return False


def prohibited(path):
    parsed = PurePosixPath(path)
    # Original JDK-only tools are an exact allowlist; recovered game Java stays private.
    if path in ORIGINAL_JDK_TOOLS:
        return False
    # Original Sunrise mod sources may also be tracked.
    # History keeps the paths from before the Sunrise rename (mods/horizon, dev.horizonlab).
    if parsed.suffix == ".java" and path.startswith(("mods/sunrise/src/", "mods/sunrise-lab/src/",
                                                     "mods/horizon/src/", "mods/horizon-lab/src/")):
        return not ("/dev/sunriselab/" in path or "/dev/horizonlab/" in path)
    return (
        bool(parsed.parts and parsed.parts[0] in PRIVATE_ROOTS)
        or parsed.suffix.lower() in PRIVATE_EXTENSIONS
        or parsed.name in PRIVATE_NAMES
    )


def copied_tables():
    """Tracked files (in the index) that hold vanilla's retired data fixer tables."""
    found = {path for path in paths_in_repository() if path in RETIRED_TABLES}
    for pattern in TABLE_FINGERPRINTS:
        # Every match, not matching lines: a minified JSON file is one line.
        result = subprocess.run(["git", "grep", "--cached", "-I", "-z", "-o", "-E", "-e", pattern],
                                capture_output=True, text=True)
        if result.returncode not in (0, 1):
            raise RuntimeError(f"git grep failed: {result.stderr.strip()}")
        counts = collections.Counter(line.split("\0", 1)[0] for line in result.stdout.splitlines())
        found.update(path for path, count in counts.items() if count >= TABLE_ENTRIES)
    return found


def paths_in_repository(history=False, revisions=None):
    """The set of paths in the tree, or in the history of `revisions` (every ref when None)."""
    if not history:
        result = subprocess.check_output(["git", "ls-files", "-z"])
        return set(result.decode("utf-8").split("\0"))
    # An empty push (including deletions only) must not fall back to HEAD or --all.
    if revisions is not None and not revisions:
        return set()
    roots = ["--all"] if revisions is None else revisions
    # Include diffs against merge parents so merge-resolution-only artifacts are caught.
    # --no-renames: a rename is listed as a deletion and an addition, so both paths still show, and git skips its
    # rename search on every large merge diff (53 s -> 8 s on main's whole history).
    # Git pushes the stored objects, not the history presented by local replace refs.
    # Millions of entries in a long history repeat a few thousand paths: read the log as a stream into a set, so
    # memory follows the distinct paths, not the length of the history (main's whole log held at once took 1.9 GB).
    paths, pending = set(), b""
    with subprocess.Popen(["git", "--no-replace-objects", "log", "--format=", "--name-only",
                           "-z", "-m", "--no-renames", *roots, "--"], stdout=subprocess.PIPE) as log:
        while chunk := log.stdout.read(1 << 20):
            *entries, pending = (pending + chunk).split(b"\0")
            paths.update(entries)
    if log.returncode:
        raise subprocess.CalledProcessError(log.returncode, log.args)
    paths.add(pending)
    return {path.decode("utf-8").lstrip("\n") for path in paths}


def pushed_revisions(stream):
    """Read Git's pre-push ref updates and retain the non-deleted local object IDs."""
    revisions = []
    for line in stream:
        _local_ref, local_oid, _remote_ref, _remote_oid = line.split()
        if local_oid.strip("0"):
            revisions.append(local_oid)
    return revisions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--history", action="store_true", help="Inspect paths in the history reachable from HEAD")
    mode.add_argument("--pre-push", action="store_true", help="Inspect history reachable from Git's ref updates on stdin")
    # CI's pull requests: only the commits a merge would add, not every ref's whole history.
    mode.add_argument("--range", metavar="BASE..HEAD", help="Inspect the commits of a revision range")
    args = parser.parse_args()
    # CI checks out every branch, so `--all` would fail main on any other branch's file: scan HEAD's ancestry.
    revisions = (pushed_revisions(sys.stdin) if args.pre_push else [args.range] if args.range
                 else ["HEAD"] if args.history else None)
    history = args.history or args.pre_push or bool(args.range)
    # Each path is judged once.
    tracked = paths_in_repository(history, revisions)
    rejected = sorted(path for path in tracked if path and prohibited(path))
    tables = [] if history else sorted(copied_tables())
    # The current tree only: history still contains the pre-removal paths.
    brought_back = [] if history else sorted(path for path in tracked if path and commercial(path))
    if tables:
        print("Vanilla's data fixer tables come from the player's jar (sunrise-prepare), not from Git:",
              file=sys.stderr)
        for path in tables:
            print(f"  {path}", file=sys.stderr)
        return 1
    if brought_back:
        print("Removed commercial features must not come back:", file=sys.stderr)
        for path in brought_back:
            print(f"  {path}", file=sys.stderr)
        return 1
    if rejected:
        print("Private game/profiling artifacts must stay outside Git:", file=sys.stderr)
        for path in rejected:
            print(f"  {path}", file=sys.stderr)
        return 1
    print("Public tree check passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
