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
