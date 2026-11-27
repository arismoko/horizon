# Drop-order side-owned `Game` fields wrapped in transparent side wrappers

Single-side fields of `sunrise_client::game::Game<'a>` that stay directly on `Game` to preserve drop order (rows in `tools/game_field_owners.tsv` whose `owner` is `game` and whose `wrapper` column is not `-`) are wrapped in `#[repr(transparent)]` single-field tuple structs defined in `sunrise_client::game::client_side` and `sunrise_client::game::server_side`:

- `ClientOnly<T>` / `ServerOnly<T>`: inner tuple field `.0` is visible only inside `game::client_side` / `game::server_side`.
- `ClientShared<T>` / `ServerShared<T>`: inner tuple field `.0` is visible across `game`, marking side ownership while allowing shared `game` modules to access `.0`.

The authoritative table is `tools/game_field_owners.tsv` (`wrapper` column).

Field declaration order, layout, and drop order on `Game<'a>` are unchanged. The stable `mod_api` surface, published loops, and selectable `functions.json` replacement targets are unchanged; wrapper types and field projections appear in `internals.json` and in affected functions' body digests.

## What changes for native mods

- **`[[internals]]` entries on `Game<'_>`:** every wrapped field listed in `tools/game_field_owners.tsv` has field type `client_side::ClientOnly<T>`, `client_side::ClientShared<T>`, `server_side::ServerOnly<T>`, or `server_side::ServerShared<T>` instead of bare `T`, and the four wrapper structs appear under `sunrise_client::game::client_side` and `sunrise_client::game::server_side`.
- **Direct field access in native helpers / mixins:** reaching the inner value through a `Game` handle uses field projection (`game.<field>.0`), preserving disjoint borrows across `Game` fields.
- **Function body digests:** functions whose source accesses a wrapped field through `.0` have updated body digests in `functions.json`.

## How to repin

1. Update direct accesses to wrapped `Game` fields in native mod helpers from `game.<field>` to `game.<field>.0` according to the `wrapper` column in `tools/game_field_owners.tsv` (`tools/game_field_owners.py` supports `--list`, `--check`, and `--fix`, failing closed outside `crates/sunrise-client/src/game*` with candidate sites on stderr).
2. Update any `[[native.bridge]]` or helper signature that names a wrapped `Game` field's exact type.
3. Update the mod's pinned Sunrise revision with `cargo sunrise-mod pin <build>`, refresh declared internals with `cargo sunrise-mod internals sync` (or `cargo sunrise-mod internals add` for explicit entries), and run `cargo sunrise-mod check` and `cargo sunrise-mod build`. If a pinned mixin `fingerprint` fails with `E_FINGERPRINT`, inspect the target function on the target build and update the fingerprint to the reported `got` value once verified.
