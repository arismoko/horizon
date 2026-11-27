# Gameplay function target map

`gameplay-crates-targets.tsv` maps the gameplay `functions.json` keys of the one-crate source at `ff4fa97a0dc` to the split source at `2efaebd6198`. Match the full old key, including its owner.
`old_target` and `new_target` are the old and current published keys. An empty end means the function has no selectable key in that build. Installers with no selectable key on either end are outside this map.
`status` is `moved` for a module change, `renamed` for a key change within the same module, `unchanged` for identical keys, `removed` for a lost target, or `new` for a newly selectable target.
`note` flags a changed Rust body or gives the refusal reason and an alternative boundary. Use `loops.json` to check the published generic loop body, which lacks a standalone installer.
For a later build, take both SDKs' `build-data/functions.json` files or run `sunrise-stage ROOT OUT/internals.json COMMIT` on disposable copies of both source trees.
Join their `functions` entries by source move, defining owner, and signature. Preserve exact index keys. Record entries with only one valid endpoint as `removed` or `new`.
For other internals and bridge owners, check the build's `internals.json`. After editing the native mod, run `cargo sunrise-mod internals sync` to refresh its declarations.
