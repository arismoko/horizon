# Ignored-test policy rows

`tools/ci/ignored_tests.py` reads every `*.txt` file here, in file-name order. Each file holds one topic: a
policy (`parity-harness.txt`, `wasm-fixture.txt`, `gpu.txt`, `flaky.txt`, ...) or one tracked failure, named for
its issue (`issue-2177.txt`). A lane that adds a known failure writes its own file; fixing it deletes that file.
A test may be listed in one file only: a second listing fails naming both files.

One row per line, `#` starts a comment line:

    PACKAGE TEST REFERENCE [flaky|harness|gpu|needs|needs-plain] [# note]
    PACKAGE TEST TARGET daily REASON [# note]

REFERENCE is the issue or PR that tracks it (for `needs`, the environment variable). A listed test that passes
fails the job until its row is removed, so a file only shrinks; `flaky` lets one pass. `harness` marks a parity
harness's own step: its harness runs it with the environment it needs, and the job skips it. `gpu` marks a test
that needs a graphics adapter: it runs, and must pass, only where the runner has a Vulkan driver. `needs` marks a
test that reads an input only its harness provides: refused by name when the variable is unset, run and required
to pass when it is set. `ignored_tests.py --needs VARIABLE --one-process-each` runs one variable's tests, each
alone in its own process (`wasm-mods.yml` does this for `HORIZON_WASM_FIXTURE`).

`needs-plain` has the same variable and refusal policy as `needs`, but is not an ignored test or a known
failure. The one-process runner selects it with `--exact` without `--ignored`; ordinary data shards skip it
when the variable is absent and require it to pass when present.

`daily` marks a test moved out of `verify` into the daily run (CONTRIBUTING.md "CI tiers"): it is
`#[ignore = "daily"]` in its source, and its row, in `daily-<package>.txt` with the rows sorted, names its cargo target
(`lib`, `bin:NAME` or `test:NAME`) and why it is a daily test (`slow`, `game-boot`, `resources`, `child-process`,
`network`, `wasm-fixture`). It is not a known failure: it runs every day and must pass, and a row whose test no
longer runs fails. Its package's shard runs it when that shard builds its target, and the `daily-rows` leg runs
every other one; verify-static fails a row no leg runs. `python3 tools/ci/move_to_daily.py apply LIST` writes the
attribute and the row (its docstring has the list format). The attribute is the source's mark and the row is the
daily run's list of what must run: an attribute without a row in a package no shard covers would run nowhere.
