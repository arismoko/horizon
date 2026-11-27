# Repinning native targets for gameplay crates

Gameplay code lives in three crates so each side's private state stays behind Rust's crate boundary: `sunrise_gameplay_common` (shared gameplay), `sunrise_gameplay` (server gameplay), and `sunrise_client_gameplay` (client gameplay). Manifest target strings and SDK catalogs resolve against the declaring crate, module, and `impl` receiver type, and require the canonical path. For a full step-by-step migration walkthrough with before/after rewrites, target-map lookups, and removed slots, see [Moving a native mod to the gameplay crates](17-moving-to-the-gameplay-crates.md).

## Canonical paths

Each gameplay crate mounts its modules from `src/lib.rs` through `pub mod game;`:

- **Shared gameplay (`crates/sunrise-gameplay-common/src/game/`):** `sunrise_gameplay_common::game::<module>::…`, with game type `CommonGame<'_>` and shared extension traits bounded by `SideGame<'_>`.
- **Server gameplay (`crates/sunrise-gameplay/src/game/`):** `sunrise_gameplay::game::server_side::<module>::…` (and `sunrise_gameplay::game::native_hooks::…` for server hook adapters), with game type `ServerGame<'_>` (`{ common, server }`) and state module `sunrise_gameplay::game::server_side` (`crates/sunrise-gameplay/src/game/server_side.rs`, `ServerState`).
- **Client gameplay (`crates/sunrise-client-gameplay/src/game/`):** `sunrise_client_gameplay::game::client_side::<module>::…` (and `sunrise_client_gameplay::game::native_hooks::…` for client hook adapters), with game type `ClientGame<'_>` (`{ common, client }`) and state module `sunrise_client_gameplay::game::client_side` (`crates/sunrise-client-gameplay/src/game/client_side.rs`, `ClientState`).
- **Split files:** modules that hold code for more than one crate split by file stem (`<stem>_common.rs`, `<stem>_server.rs`, `<stem>_client.rs`, or `mod_{common,server,client}.rs`). Follow the `mod` and `#[path]` declarations in that crate's `src/game.rs`, `server_side.rs`, or `client_side.rs` (a `pub use` re-export does not change the declaring module path).
- **Inherent and trait receivers:** inherent methods use the exact receiver type spelled on the `impl` block (for example `ServerGame<'_>`, `crate::game::ServerGame<'_>`, `ClientGame<'_>`, or `CommonGame<'_>`). Trait implementations use `<Type as Trait>::method`.

## Declarations to update in `sunrise-mod.toml`

- **[Mixins (`[[native.mixin]]`)](../../concepts/native-route/2-mixins.md#naming-the-target):** update `target` to the declaring crate, module, and receiver spelling in the target build's `functions.json` (`.sunrise/cache/<build>/build-data/functions.json`). Update helper parameter types and `bindings` `ty` strings to match the method's receiver (`&mut crate::game::ServerGame<'_>`, `&mut ServerGame<'_>`, `&mut ClientGame<'_>`, or `&mut CommonGame<'_>`), and read side state through `game.server` or `game.client` (and shared state through `game.common` or `Deref`).
- **[Bridges (`[[native.bridge]]`)](../../concepts/native-route/4-bridges.md):** update `target` to the declaring crate and module for field bridges (`sunrise_gameplay::game::server_side::ServerState`, `sunrise_client_gameplay::game::client_side::ClientState`, `sunrise_gameplay_common::game::CommonGame`, `sunrise_gameplay::game::ServerGame`, `sunrise_client_gameplay::game::ClientGame`, or a struct in a side module) and `INVOKE` bridges.
- **[Support modules (`[[native.module]]`)](../../concepts/native-route/5-support-modules.md):** set `file` to the source file in its owning crate (`crates/sunrise-gameplay-common/src/…`, `crates/sunrise-gameplay/src/…`, or `crates/sunrise-client-gameplay/src/…`).
- **Explicit `[[internals]]` entries:** update canonical paths to the declaring crate and module. Confirm each key against the pinned build's `internals.json` (`.sunrise/cache/<build>/build-data/internals.json`), since an item's path follows the crate and module holding its declaration or `impl`.

## Steps to repin a native mod

1. **Pin the target build:** run `cargo sunrise-mod pin <build>`. `pin` updates `[native] sunrise` and refreshes hashes for declarations that resolve on the target build, reporting unresolvable paths as `missing` with their old hashes untouched.
2. **Locate moved targets and update declarations:** look up each unresolvable target in the pinned build's `functions.json` and `internals.json` under `.sunrise/cache/<build>/build-data/`, matching the crate (`sunrise_gameplay_common`, `sunrise_gameplay`, or `sunrise_client_gameplay`), declaring module, and `impl` receiver type. Update affected `[[native.mixin]]`, `[[native.bridge]]`, `[[native.module]]`, and explicit `[[internals]]` entries in `sunrise-mod.toml`, along with any helper signatures and `game.common` / `game.server` / `game.client` field accesses under `src/`.
3. **Regenerate internal hashes:**
   - If your mod uses discovered internals, run `cargo sunrise-mod internals sync` to replace `internals` with compiler-discovered paths, kinds, and hashes for the pinned build.
   - If your mod declares explicit hashed `[[internals]]` entries without `internals sync`, refresh every rewritten entry's hash with `cargo sunrise-mod internals add <new-path>…` (pass `--kind <kind>` if a path is ambiguous), or re-run `cargo sunrise-mod pin <build>` after updating the paths (when the mod has no `[[native.module]]` rows).
4. **Verify and rebuild:** run `cargo sunrise-mod check` to validate declarations and refreshed internal hashes, then run `cargo sunrise-mod build` to compile the package and regenerate `uses.json` with the target build's `made_for` id, internal hashes, and split function digests (see [Fitting another build](../../reference/sdk/04-fit.md); mods with `[[native.module]]` or `[[native.bridge]]` target one build only and pack no `uses.json`). Follow [`E_FINGERPRINT`](../../concepts/native-route/2-mixins.md#fields) for any fingerprinted mixin whose digest differs on the target build.

## Crate boundaries and side separation

`sunrise-gameplay` (server) and `sunrise-client-gameplay` (client) both depend on `sunrise-gameplay-common`, and neither side crate depends on the other. The entry crate `sunrise-client` wires them together (with `sunrise-client-gameplay` optional so dedicated server builds omit client code completely).

- Because each gameplay crate compiles its own mirror when building function-replacement patches, a `[[native.module]]` or `[[native.bridge]]` belongs to the gameplay crate whose file or type it targets, and helpers see items in that crate (plus `sunrise_gameplay_common` from a server or client crate).
- A client-only target in `sunrise_client_gameplay` exists only in desktop builds, not in dedicated server builds (`--no-default-features`).
