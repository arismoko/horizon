# Conflict hotspots

A file that many independent PRs keep editing (a hand-kept list, a registry, a giant module) becomes a generated
list or one file per item, so independent PRs stop conflicting. These scripts find such files, keep new long lists
out, and carry old-style PRs across a switch. Standard library Python and git only.

| Script | For |
|---|---|
| `measure.py` | Ranks files by distinct PRs that changed them and by merges git could not replay (`git merge-tree`, git 2.38 or newer), on rollups, syncs and PR branches. `--days N` or `--since DATE` for a window; the fork's start comes from `--bound` or `--upstream`. `--markdown` (default), `--json`, `--report`. Its docstring defines every count. |
| `open_prs.py` | Lists the open PRs whose own diff touches a path. Run it before a switch PR; the list goes in that PR's body. |
| `carry.py` | Writes an old-style PR's addition to a switched list in the new form, into the working tree. |
| `check_lists.py` | `verify-static`: a `pub use` brace list under `crates/horizon-mod-vocab/src/exports/` names at most 12 items; the longer ones in its table may not grow past their entry (shortening needs no edit). |

The nightly schedule of `dyna-checks` runs `measure.py --days 3 --report` and rewrites the issue
"Conflict hotspots: churn report" (files with 3 or more rollup conflicts or 15 or more PRs in 3 days). It is never a gate.

## Measuring

```sh
git fetch origin '+refs/pull/*/head:refs/pull/*/head'      # PR numbers for rollup members
git fetch https://github.com/horizon-rust/horizon.git main:refs/remotes/upstream/main
python3 tools/hotspots/measure.py --days 3                  # last 3 days
python3 tools/hotspots/measure.py --open-rollup origin/fork-review-rollup-68 --json > raw.json   # since the fork began
python3 tools/hotspots/open_prs.py --from-gh crates/horizon-mod-vocab/src/exports/part_table.rs
```

## In a rollup, after a switch

A member PR written against a switched list conflicts on the old file. Take main's side of that file, then:

```sh
git fetch origin pull/<N>/head:refs/pull/<N>/head
python3 tools/hotspots/carry.py ignored-test <N>             # or --topic <name>; --head pr/<N> for another ref
git add <the files it names> && git commit -m "Carry for #<N>: <what moved>"
```

It writes nothing and prints the hunk when it cannot place one: carry that hunk by hand. `part-table` and
`dispatch-stub` answer that they are not available yet; the switch PRs that make those forms add them.
