#!/usr/bin/env python3
"""One-shot: split tools/ci/ignored-tests-known-failures.txt into tools/ci/ignored-tests/<topic>.txt.

Many independent pull requests inserted rows into the one list and conflicted on adjacent lines. Each row goes
to a file by the old file's section heading (and, in the section of harness inputs, by its variable):

  Parity harness steps.                         -> parity-harness.txt
  Tests that read an input only ... (needs)     -> wasm-fixture.txt for HORIZON_WASM_FIXTURE, else harness-inputs.txt
  Disabled by the owner's decision ...          -> old-world.txt
  Tests that need a graphics adapter ...        -> gpu.txt
  Known failures on main.                       -> flaky.txt (every row there is `flaky`)
  Client failures exposed ... (#2137).          -> issue-2177.txt (every row there references #2177)

Rows are copied byte for byte, in their old order; comment lines inside a section go with the row after them. The
old file's top block (the row format) becomes the folder's README.md, and the old file is deleted. Proof: the rows
of all new files, sorted, equal the old file's rows, sorted; both SHA-256 hashes are printed and must match.
"""

import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OLD = ROOT / "tools/ci/ignored-tests-known-failures.txt"
FOLDER = ROOT / "tools/ci/ignored-tests"

README = """\
# Ignored-test policy rows

`tools/ci/ignored_tests.py` reads every `*.txt` file here, in file-name order. Each file holds one topic: a
policy (`parity-harness.txt`, `wasm-fixture.txt`, `gpu.txt`, `flaky.txt`, ...) or one tracked failure, named for
its issue (`issue-2177.txt`). A lane that adds a known failure writes its own file; fixing it deletes that file.
A test may be listed in one file only: a second listing fails naming both files.

One row per line, `#` starts a comment line:

    PACKAGE TEST REFERENCE [flaky|harness|gpu|needs] [# note]

REFERENCE is the issue or PR that tracks it (for `needs`, the environment variable). A listed test that passes
fails the job until its row is removed, so a file only shrinks; `flaky` lets one pass. `harness` marks a parity
harness's own step: its harness runs it with the environment it needs, and the job skips it. `gpu` marks a test
that needs a graphics adapter: it runs, and must pass, only where the runner has a Vulkan driver. `needs` marks a
test that reads an input only its harness provides: refused by name when the variable is unset, run and required
to pass when it is set. `ignored_tests.py --needs VARIABLE --one-process-each` runs one variable's tests, each
alone in its own process (`wasm-mods.yml` does this for `HORIZON_WASM_FIXTURE`).
"""

HEADERS = {
    "parity-harness": "# Parity harness steps: each harness runs its test with the environment it sets (REFERENCE names\n"
                      "# the harness or the variable); the ignored-tests job skips them.\n",
    "harness-inputs": "# Tests that read an input only their parity harness provides (REFERENCE is the variable): refused by\n"
                      "# name at the start of the shard when the variable is unset; with it set they run and must pass.\n"
                      "# Not known failures.\n",
    "wasm-fixture": "# Tests that load the compiled wasm fixture mod (REFERENCE is the variable). The slow shards refuse\n"
                    "# them by name; wasm-mods.yml builds the fixture and runs each alone in its own process with\n"
                    "# `ignored_tests.py --needs HORIZON_WASM_FIXTURE --one-process-each`.\n",
    "old-world": "# Disabled by the owner's decision until H077_OLD_WORLD is wired into CI.\n"
                 "# needs the private old-world fixture: see followups\n",
    "gpu": "# Tests that need a graphics adapter (REFERENCE is the ignore reason's word).\n",
    "flaky": "# Known failures on main that may pass or fail.\n",
    "issue-2177": "# Client failures exposed after enforcing target completion (#2137).\n",
}


def topic(heading, row):
    """The file a row of the section starting `heading` goes to."""
    package, test, reference, *mark = row.split(" # ", 1)[0].split()
    if heading.startswith("# Parity harness steps."):
        return "parity-harness"
    if heading.startswith("# Tests that read an input only their parity harness provides"):
        assert mark == ["needs"], row
        return "wasm-fixture" if reference == "HORIZON_WASM_FIXTURE" else "harness-inputs"
    if heading.startswith("# Disabled by the owner's decision until H077_OLD_WORLD"):
        return "old-world"
    if heading.startswith("# Tests that need a graphics adapter"):
        assert mark == ["gpu"], row
        return "gpu"
    if heading.startswith("# Known failures on main."):
        assert mark == ["flaky"], row
        return "flaky"
    if heading.startswith("# Client failures exposed after enforcing target completion"):
        assert reference == "#2177", row
        return "issue-2177"
    sys.exit(f"no topic for the section starting {heading!r}")


def digest(rows):
    return hashlib.sha256("".join(sorted(line + "\n" for line in rows)).encode()).hexdigest()


def is_row(line):
    return bool(line.strip()) and not line.lstrip().startswith("#")


def main():
    if FOLDER.exists():
        sys.exit(f"{FOLDER.relative_to(ROOT)} already exists; this split is one-shot")
    lines = OLD.read_text().splitlines()
    top = lines.index("")  # The format block, before the first blank line, becomes README.md.
    files = {name: [header] for name, header in HEADERS.items()}
    # A section is a blank line, its heading block of comment lines, then rows; the heading block is replaced
    # by the topic's header, and a comment line after a row goes with the row after it.
    heading, pending, previous = None, [], "blank"
    for line in lines[top:]:
        if not line.strip():
            assert not pending, f"a comment with no row after it: {pending}"
            previous = "blank"
        elif is_row(line):
            name = topic(heading, line)
            files[name].extend(pending)
            files[name].append(line)
            pending, previous = [], "row"
        elif previous == "blank":
            heading, previous = line, "heading"
        elif previous != "heading":
            pending.append(line)
    assert not pending, f"a comment with no row after it: {pending}"
    FOLDER.mkdir()
    (FOLDER / "README.md").write_text(README)
    for name, content in files.items():
        (FOLDER / f"{name}.txt").write_text(content[0] + "".join(line + "\n" for line in content[1:]))
    OLD.unlink()
    old = [line for line in lines if is_row(line)]
    new = [line for path in sorted(FOLDER.glob("*.txt")) for line in path.read_text().splitlines() if is_row(line)]
    print(f"old rows: {len(old)} sha256 {digest(old)}")
    print(f"new rows: {len(new)} sha256 {digest(new)}")
    if digest(old) != digest(new):
        sys.exit("the new files' rows differ from the old file's")


if __name__ == "__main__":
    main()
