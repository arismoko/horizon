<!-- chapter: Runtime packages and identity -->
<!-- separator: 1 -->

## Runtime packages and identity

The launcher generates the game's embedded native inventory from the selected `horizon-mod.toml`
manifests: mod id, version and side. Cargo package metadata and hook factories do not supply identity.
The game validates each compiled mod's package at
`<executable directory>/horizon-native-mods/<id>/` before installing hooks or starting a world.
Missing, unreadable, content-only or identity-mismatched packages refuse startup with the affected
mods and paths; restore the packages exported with that executable or rebuild it. There is no
build-record fallback and no discovery of native runtime packages from the working directory.
Content-only mods still follow the working directory, player toggles and explicit `--dev-mod` paths;
a content package cannot shadow a compiled mod's id.

Move the executable, prepared resources and canonical package directory together. The copied
projects retain their flat manifest, crate, assets and data. The game's mod list and crash details
use its embedded inventory, including code-only mods. Display names and descriptions come from the
validated manifests; Controls bindings are declared through `Registrar::register_key_mapping`, not a separate `NativeMod` metadata hook.

`native-mods.json` is provenance and rebuild status, not game identity, authentication, announcements
or startup input. The launcher retains it for build/status displays. The separate `horizon-server`
supervisor may report the adjacent record as a provenance snapshot, not a query of the running engine.
Its SHA-256 values and revision describe the built binary's inputs, not the running mod set.
The launcher prepares jar data from canonical manifests as non-authoritative candidates; the engine
validates identity against its embedded inventory. Original-jar pack order and
whole-package refusal rules are in [Original Java jar layers](../../reference/manifest.md#original-java-jar-layers).

### Resolve an entity by UUID

In a server hook, wrap a saved `u128` in `EntityUuid` and pass it to the host.
The lookup returns `None` if the entity is unloaded or belongs to a different
level than the hook. Try again from a hook running in the entity's level.

```rust
use mod_api::{EntityRef, EntityUuid, ServerHost};

fn find_saved_entity(host: &dyn ServerHost, saved_uuid: u128) -> Option<EntityRef> {
    host.entity_by_uuid(EntityUuid(saved_uuid))
}

fn entity_identity(host: &mut dyn ServerHost, entity: EntityRef) -> Option<EntityUuid> {
    entity.uuid(host).map(EntityUuid)
}
```

`EntityRef::uuid` returns the raw `u128`. Use `EntityUuid(raw_uuid)` to pass
that value back into the lookup.
