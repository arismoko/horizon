# Native mods

Native mod docs live in the modding book: [native mods](modding/src/how-to/native-mods.md), [trait mods, Parts and attachments](modding/src/concepts/trait-mods.md), [injections](modding/src/injections.md) and the [hook notes](https://arismoko.github.io/horizon/reference/hook-notes.html).

## Prebuilt SDK layout

The v4 prebuilt archive separates target artifacts (`deps/`) from host proc-macro
libraries (`host/`). Cargo builds with an explicit `--target <rustc host triple>`,
so host build dependencies cannot introduce a second metadata variant into the
target SDK. Only host proc-macro dynamic libraries are shipped, not their host
dependency metadata. Consumers pass both folders as `-L dependency` paths; named
extern and facade lookups use only `deps/`. `native/` still holds native linker
inputs. The identity manifest hashes files in all three folders.

Target compilation retains `-C prefer-dynamic` (and `-C rpath` on macOS), while
Cargo's explicit target keeps those flags off host proc-macros. Multiple versions
of a target dependency are legitimate: uniqueness is required only when selecting
a named extern or publishing an internals facade, never across the whole SDK.

To rebuild and prove the archive with the launcher's pinned Rust toolchain active:

```sh
python3 tools/prebuilt_game.py build prebuilt
python3 tools/prebuilt_game.py smoke prebuilt
python3 tools/prebuilt_game.py exports prebuilt
python3 tools/native_dylib_probe.py --sdk prebuilt --output target/native-dylib-probe
```

For a server archive, add `--server` to the build command. Use a fresh
`CARGO_TARGET_DIR` when collecting a clean-build proof; all commands must use the
same compiler as `launcher/src/rust_toolchain.rs`.

## Upstream service contracts carried by this sync

The upstream additions below describe behavior to preserve in the typed `Host` and
handle API, not additional `native_*` entry points or a bundle API counter. Use the
book and generated API reference for the available signatures; pending hook fragments
must be converted to the fork's hook catalog before being advertised as available.

- Tamed-mob death messages are offered after construction and only when an online
  owner would receive them with `show_death_messages` enabled. Listeners may replace
  or suppress the message in dispatch order. The server-only named-mob log is not
  a second player-message hook.
- Dedicated-server settings describe the live listener: account authentication,
  bound port and whether Velocity forwarding is active. They never expose the
  forwarding secret, and are unavailable before listening or on integrated/LAN
  servers. Secure-profile enforcement and hardcore are not part of this read.
- Vehicle and passenger reads use entity handles (`entity_vehicle` and
  `entity_passengers` on `Host`), not parallel numeric-ID helpers. Passenger order
  follows vanilla, with the controlling passenger first. These are read-only.
- UUID lookup from a read-only context must not swap levels. A mutable lookup can
  reach other loaded levels; entity NBT reads there must refuse unloaded levels,
  missing entities and client-side use rather than load a level implicitly.
- Map reads return the 128 × 128 row-major color bytes and scale, dimension and
  locked metadata. Reading saved data must not install a loaded map as a side
  effect; unknown maps and client-side world reads return no value.
- Living-entity sight uses eye-to-eye COLLIDER shapes, no fluids, the source's
  collision context and vanilla's 128-block limit. Unknown entities, different
  levels and unavailable server levels must not report visibility. The typed
  `living_has_line_of_sight` and `level_line_of_sight` reads replace raw-ID helper
  proposals; the point-to-point form has no entity collision context.

These are contract notes, not runtime verification of the converted services.
