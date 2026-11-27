# Advisory: gameplay side modules (`client_side` and `server_side`)

Native engine catalog entries under one-sided gameplay modules and function-level side mirrors use canonical paths under `client_side` or `server_side`. Unresolved manifest declarations fail `cargo sunrise-mod check` and `cargo sunrise-mod build`.

For the modding-book guide, see [Repinning native targets for game side modules](https://playsunrise.net/docs/modding/how-to/native-mods/repinning-side-module-targets) ([`docs/modding/src/how-to/native-mods/16-repinning-side-module-targets.md`](../../../docs/modding/src/how-to/native-mods/16-repinning-side-module-targets.md)).

## Path rules

- **One-sided modules (`tools/game_module_sides.tsv`):** any module whose `side` column is `client` or `server` carries that side segment in its canonical path while its source file stays in place:
  - `sunrise_client::game::<module>::...` → `sunrise_client::game::client_side::<module>::...` (`client`)
  - `sunrise_client::game::<module>::...` → `sunrise_client::game::server_side::<module>::...` (`server`)
  - Modules with `side = game` keep `sunrise_client::game::<module>::...`.
- **Function-level mirror modules (`tools/game_fn_sides.tsv`):** one-sided methods and free functions moved out of `game.rs` or a shared (`side = game`) module live in a side mirror module declared under `client_side` or `server_side`:
  - From `game.rs`: `sunrise_client::game::...` → `sunrise_client::game::client_side::game_client::...` (`crates/sunrise-client/src/game/game_client.rs`) or `sunrise_client::game::server_side::game_server::...` (`crates/sunrise-client/src/game/game_server.rs`)
  - From a shared module `<module>`: `sunrise_client::game::<module>::...` → `sunrise_client::game::client_side::<module>_client::...` (`<stem>_client.rs` or `mod_client.rs`) or `sunrise_client::game::server_side::<module>_server::...` (`<stem>_server.rs` or `mod_server.rs`)
- **State-module renames:** `client_side` and `server_side` replace `client_state` / `server_state` and rename their source files:
  - `sunrise_client::game::client_state::...` → `sunrise_client::game::client_side::...` (`crates/sunrise-client/src/game/client_state.rs` → `crates/sunrise-client/src/game/client_side.rs`)
  - `sunrise_client::game::server_state::...` → `sunrise_client::game::server_side::...` (`crates/sunrise-client/src/game/server_state.rs` → `crates/sunrise-client/src/game/server_side.rs`)

`game.rs` re-exports moved modules and aliases `client_state` / `server_state` for Rust helper code. Manifest target strings and SDK catalogs read the declaring module and require the canonical path.

## Affected manifest declarations

- **`[[native.mixin]]` targets:** methods, free functions, and trait methods declared in moved modules, mirror modules, or state modules. The rename maps emit normalized SDK keys; in manifests that spell lifetimes (`Game<'_>`) or trait-qualified targets (`<Type as Trait>::method`), apply the same `client_side` / `server_side` segment insertion or mirror-module rename before the item name.
- **`[[native.bridge]]` targets:** field bridges targeting `ClientState` or `ServerState`, field bridges targeting types declared in a moved module (for example `sunrise_client::game::client_side::native_panel::Panel`), and `INVOKE` bridges naming methods in moved or mirror modules.
- **`[[native.module]]` file paths:** ordinary moved modules keep their source file path; support modules targeting `crates/sunrise-client/src/game/client_state.rs` or `crates/sunrise-client/src/game/server_state.rs` update `file` to `client_side.rs` or `server_side.rs`; support modules targeting items moved into a mirror module point `file` at the corresponding `game_{client,server}.rs`, `<stem>_{client,server}.rs`, or `mod_{client,server}.rs` file.
- **Explicit `[[internals]]` entries:** declared engine items under moved modules, mirror modules, or `ClientState` / `ServerState` carry the canonical `client_side` / `server_side` path. Confirm keys against the pinned build's `internals.json` (`.sunrise/cache/<build>/build-data/internals.json`), since an item's canonical path follows the module holding its declaration or `impl`.

## How to repin a native mod

1. Pin the target build with `cargo sunrise-mod pin <build>` (`pin` refreshes hashes for declarations that resolve on the target build and reports moved declarations as `missing` with their old hashes untouched).
2. Generate the selectable rename maps by running the module-side and function-side rename tools on the prior and target builds' `functions.json` indexes (see `--help` on each script for options), then update affected `[[native.mixin]]`, `[[native.bridge]]`, `[[native.module]]`, and explicit `[[internals]]` paths:

```sh
python3 tools/game_module_sides.py --renames <before-functions.json> <after-functions.json> --out moved-module-targets.tsv
python3 tools/game_fn_sides.py --renames <before-functions.json> <after-functions.json> --out moved-fn-targets.tsv
```

3. Regenerate internal hashes for every rewritten path:
   - For mods using discovered internals, run `cargo sunrise-mod internals sync` to replace `internals` with compiler-discovered paths, kinds, and hashes for the pinned build.
   - For explicit hashed `[[internals]]` entries (when skipping `internals sync`), editing `path` leaves the old `hash` in place even when the item's catalog hash changed; refresh each moved entry's hash with `cargo sunrise-mod internals add <new-path>…` (pass `--kind <kind>` if a path is ambiguous), or re-run `cargo sunrise-mod pin <build>` after updating the paths (for mods without `[[native.module]]`).
4. Run `cargo sunrise-mod check` to verify the manifest and refreshed internal hashes, then run `cargo sunrise-mod build` to compile the package and regenerate `uses.json` with the target build's `made_for` id, internal hashes, and split function digests (mods with `[[native.module]]` or `[[native.bridge]]` target one build only and pack no `uses.json`). If a pinned mixin `fingerprint` fails with `E_FINGERPRINT`, inspect the target function on the target build and update the fingerprint to the reported `got` value once verified.
