# Conflict hotspots

A file that many independent PRs keep editing (a hand-kept list, a registry, a giant module) becomes a generated
list or one file per item, so independent PRs stop conflicting. These scripts find such files, keep new long lists
out, and carry old-style PRs across a switch. Standard library Python and git only.

| Script | For |
|---|---|
| `measure.py` | Ranks files by distinct PRs that changed them and by merges git could not replay (`git merge-tree`, git 2.38 or newer), on rollups, syncs and PR branches. `--days N` or `--since DATE` for a window; the fork's start comes from `--bound` or `--upstream`. `--markdown` (default), `--json`, `--report`. Its docstring defines every count. |
| `open_prs.py` | Lists the open PRs whose own diff touches a path. Run it before a switch PR; the list goes in that PR's body. |
| `carry.py` | Writes an old-style PR's addition to a switched list in the new form, into the working tree. |
| `check_lists.py` | `verify-static`: a `pub use` brace list under `crates/horizon-mod-vocab/src/exports/` names at most 12 items; the longer ones in its table may not grow past their entry (shortening needs no edit). A fragment a switch removed (`part_table.rs`) may not come back. |
| `check_stand_ins.py` | `verify-static`: the dispatch tests' stand-ins are one file each (`fixtures/stubs/<Name>.rs`, `fixtures/api/<Name>.rs`, a name in one folder only, none back in `shared.rs`); every type a hook signature names has one; the wire loopback's hand-written impls of vocabulary traits have every required method, and of `CapabilityProviders` every member trait. Its docstring lists what it cannot see (names the generator adds, derives, stub methods). |

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

A member PR written against a switched list conflicts on the old file.

The vocabulary's type lists in `crates/horizon-mod-vocab/src/exports/domains.rs` are one glob per domain
(`pub use <domain>::types::*;`): every `pub` item of a `<domain>/types.rs` is API by its keyword. A member PR that
added a name to one of the removed lists conflicts in that list: take main's side of the list after checking that the
PR's diff defines the name as a `pub` item in that `types.rs`. No carry script is needed. Any other hunk of the PR in
`domains.rs` (a `mod` line, a `backend` or `runtime` re-export) is outside the lists and merges as before.

For the other switched lists, take main's side of the old file, then:

```sh
git fetch origin pull/<N>/head:refs/pull/<N>/head
python3 tools/hotspots/carry.py ignored-test <N>             # or --topic <name>; --head pr/<N> for another ref
python3 tools/hotspots/carry.py dispatch-stub <N>            # a stand-in added to the old shared.rs stubs/api modules
python3 tools/hotspots/carry.py part-table <N>               # a name added to the old exports/part_table.rs
git add <the files it names> && git commit -m "Carry for #<N>: <what moved>"
```

It writes nothing and prints the hunk when it cannot place one: carry that hunk by hand. `shared.rs` itself stays
for `dispatch-stub`: take main's side only of its hunks inside the old `mod stubs` and `mod api`. `part-table`
writes each name the PR added to the old `crates/horizon-mod-vocab/src/exports/part_table.rs` into
`exports/parts_<file>.rs`, the fragment of the parts file that defines it on the PR's head, in the same scope and
`#[cfg]`; run `rustfmt --edition 2024` on the files it names. Taking main's side alone would drop the export, so a
forgotten carry fails to compile.

## `game.rs` sync remnants

After resolving a sync, run `python3 tools/hotspots/game_rs.py <sync-branch> <upstream-commit>`.
`--repo` defaults to this checkout. Use the upstream commit actually being synced, not its later head.
This is intentionally not wired into CI: CI need not have that ref.

The report separates genuine remnants and classified name-only matches, with function and line counts,
plus unchanged, changed-in-place, fork-only and missing-from-fork sections. Exit 1 means genuine remnants
or stale classifications; exit 0 means neither. No automatic rename or move is performed.

The parser records impl target types (`impl Trait for Type` is owned by `Type`), or `free` for free
functions, by brace depth. It is approximate: comments, strings, macros, cfg branches and complex or
multiline impl headers are not Rust-parsed. Counts are a sync aid, not a semantic equivalence proof.

`game_rs.txt` is a whitespace table: fork owner, name, upstream owner, upstream path, then a reason.
Each row pins both sides of a verified name-only collision. A same-owner destination method always
counts as genuine, even if a row matches an unrelated destination. Free functions have no type owner:
for them a destination inside `game/` overrides a row, while an explicitly pinned helper outside
`game/` may be classified. Unclassified collisions conservatively count as genuine. Thus adding a
real `Game` method move with any classified name cannot be hidden by that classification. Rows that
match nothing (including rows superseded by genuine moves) fail with a request to delete them.
This table only shrinks or stays true; new remnants must not be classified away.
