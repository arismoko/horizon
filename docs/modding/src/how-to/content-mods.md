# Content mods

Add blocks, items, fluids, entities and world-generation data with a content package.
Create a project, edit its JSON files, then build and run it.

## Create and package your mod

Install the tool first ([Install the mod tool](../start/install-tool.md)).

```sh
cargo sunrise-mod new my-mod        # a content mod: manifest, one block with its model, loot and recipe
cd my-mod
cargo sunrise-mod run               # build my-mod.smod, then start the game with it turned on
cargo sunrise-mod build             # only build: target/sunrise-mod/my_mod.smod
cargo sunrise-mod check             # validate the project before release
```

- `new <path> [--name "Name"]` writes the content template. `--template native` writes
  a flat native mod instead: `sunrise-mod.toml`, root `Cargo.toml`
  and `src/lib.rs`, and empty `assets/` and `data/` directories. The mod is pinned to the build of the current
  Nightly (players have it), or to `--sunrise`'s: a game release's tag (`nightly-<YYYY-MM-DD>`, a Stable tag such as
  `26.3-0.1.0`, or `dev-<YYYY-MM-DD>-<short sha>`), or a commit sha (7 hex digits are enough when they name one build). The native mod pins it in `[native] sunrise`. `Cargo.toml` names the API by path,
  `sunrise-mod-api = { path = ".sunrise/sdk/api/crates/sunrise-mod-api" }`, the only form a build
  takes. Every other form is refused under `E_SDK_SOURCE`, naming why: the old git form, a version alone (Cargo
  would take the crate from crates.io, where anyone can publish its name), another source, or another path. `new` fetches that build's SDK into `.sunrise/sdk/`,
  and every command that compiles the mod fetches it again when it is missing or another build's, so a fresh clone
  works after one command; your editor needs that first command too. `.sunrise/` is in the template's `.gitignore`
  and is never packed. The fetch checks Sunrise's signature on the release and every file against its `sdk.json`;
  `SUNRISE_SDK_BASE` (a URL or a folder) reads the releases from a mirror instead. When that Nightly has no SDK,
  `new` asks for `--sunrise`. `pin [<build>]` moves a native mod to another build (the current Nightly by default)
  ([the SDK in a mod project](../reference/sdk/03-in-a-project.md)). Its root `impl Hooks` uses
  `#[mod_api::sunrise_mod]`. Compile it with `cargo sunrise-mod build`
  ([writing hooks](../start/writing-hooks.md)). Any other `--template` is refused.
- `build` packs the checked project into `target/sunrise-mod/<id>.smod`: `sunrise-mod.toml`, `assets/`,
  `data/`, optional `icon.png` and, for code, the root Cargo project. It reads content with the game's own package
  rules and runs the reference checks of `check` below. If `target/` or `target/sunrise-mod/` is a symlink, the build
  refuses it without touching the previous package. Otherwise, once the build reaches output cleanup, it removes the
  previous `<id>.smod` before running the reference checks. Earlier failures, such as a manifest or SDK error,
  can leave the previous package in place. A content build compiles nothing; a native build compiles against the pinned SDK. Native code requires
  both `Cargo.toml` and `[native]`; inconsistent declarations and a manifest without capability `registries` are refused.
- For content mods, `run` builds, then starts `sunrise-client` (on `PATH`, or `--game PATH`, or `SUNRISE_CLIENT`) with
  `--dev-mod target/sunrise-mod/<id>.smod`, always on. Native projects use the pinned SDK's game. Use `--jars <dir>`
  to choose a folder of [original Java jars](#java-jars-as-external-assets). Arguments after `--` go to the game.
  The game reads content once at start, so run again after an edit.
- `check` in a project folder runs the package rules, the render pipeline shader checks and offline reference checks.
  It checks references to IDs in namespaces the manifest owns, not to vanilla or other mods. It checks these forms:
  - Models in blockstate `variants` and `multipart`, model `parent` and `textures` fields, and model references in
    item definitions. The game's model decoder reads one model and ignores trailing input.
  - Ingredients and results in built-in shaped, shapeless, cooking, stonecutting, smithing and transmute recipes.
    Recipe `#tag` ingredients, custom recipe types, components and other opaque data are not checked.
  - Tag `values` are checked for direct IDs under `tags/block`, `tags/item`, `tags/fluid` (including
    `flowing_` forms) and `tags/entity_type`. Required nested `#tag` references are checked in those folders
    plus `tags/worldgen/biome` and `tags/worldgen/structure`. Tag files can be in any namespace, including
    `minecraft`; other tag folders, optional entries and removals are not checked.
  - Loot item entries' `name` and block-state conditions' `block` fields. Recipes, tags, loot tables and loot pools
    that `neoforge:conditions` or `fabric:load_conditions` may disable are left alone.

  A missing reference is an **error** (exit 1) only when the tool has enough information to confirm it. When a
  manifest lists `[[jar]]`, `check` and `build` read each jar from the local jar store (fetching any missing jar
  from Modrinth) and resolve references, blockstates, loot tables, item definitions and English names against the
  mod and its jars together. If a jar cannot be fetched (for example offline), the tool prints a note and skips
  the checks that jar could satisfy; `build` still succeeds offline. Otherwise, it can **warn** without failing: a
  dependency's jar or atlas source may supply the resource, an inline composite child may override a texture, or a
  local recipe filter may remove the recipe or swap the reference.
  The tool does not fetch dependencies or check registry entries declared in code.
  It also checks for blockstates on blocks with `render_shape` set to `MODEL` and warns about missing local loot
  tables, item definitions or English names, or registry content on a client-only package. Reference diagnostics name
  the source file, the relevant field or member index, and the ID.
- `check <file>` reads `.smod`, `.spack` and resource pack `.zip` files. A `.smod` also
  needs its [release attestation](../reference/sdk/05-commands.md#check-a-released-package).
- To release a mod to the registry, follow [Publishing your mod](publish.md).
- `use-jar <path | modrinth-url | <project> <version>>` records and caches an original Java jar as an external
  asset/data source, without copying it into the mod ([Java jars as external assets](#java-jars-as-external-assets)).

## Vegetation behaviour is independent of the template

Set `properties.vegetation` to `true` in block JSON or in a code declaration's
`BlockProperties` to give a block `VegetationBlock` survival, even on a glass or
stairs template. Placement refuses unsupported ground and a neighbour update
removes an unsupported plant. The default support is `minecraft:supports_vegetation`
on both sides. Templates that already inherit `VegetationBlock` keep their own
checks, such as crop light requirements.

On the server, the soil's `can_sustain_plant` answer decides first. If it returns
`None`, the plant's `may_place_on` Part can override the ground rule. Neither
registering a Part for a block nor registering one on a tag changes what kind
of block it is. The client predicts using the default rule without running these
server Parts.

## Declare content in code

For a complete example with Rust and resource files, see
[Register a block with a model](world/register-a-block.md).

Code mods declare content through `BlocksEvent` and `ItemsEvent`, using API-owned
`mod_api::Block` and `mod_api::Item` records. Start with `Block::new(id, template)` or
`Item::new(id, template)`, passing identifier strings, then set the overrides and call
`event.register(declaration)`. Invalid IDs return a typed `DeclarationError`; the remaining checks
run before resources load. These are load-time declarations, not live registry entries. The game
admits them with the content-file checks, atomically for each owner.
A block's `item` defaults to true, in code as in a file: `Block::with_item(false)` gives it none.
Code registry declarations keep a valid explicit namespace, regardless of who supplies files
in it. Bare names use the submitting mod ID. Exact duplicates and reserved built-in keys are
still refused according to the registry, and diagnostics identify the submitting mod. A host
cannot re-declare a key already declared by a content package it serves. Files and assets keep
manifest namespace checks and `served_by` delegation requires `[native] serves_content = true`
(see [Manifest reference](../reference/manifest.md#sunrise-modtoml)). A host runtime should use
`context.mods()` to identify the delegating packages it recognizes. Attaching a Part to an
existing vanilla or foreign block or item is a reference, not a new declaration.

Code declarations carry component patches as `Vec<(String, Option<Tag>)>`:
`Some(tag)` sets the named component and `None` removes it (without a `!` prefix).
Repeated component IDs, including a set and removal of the same ID, are refused; `lore` and `minecraft:lore`
count as the same ID, and an ID may not start with the removal prefix `!`.
The game converts this opaque NBT data at load; file declarations below use JSON.
No declaration conversion runs per tick.

### Choosing what remains after removal

Implement `BlockPart::remove_block` for a block that leaves a support or another
state behind when removed. This server-only replacement receives the dimension,
position, removed state, `moved_by_piston` and `replacement`: the exact state
written if the listener leaves `answer` unanswered, the fluid's legacy block
(air when dry, water for a waterlogged block). Answer `Some(state)` to choose a
different state, or `None` to keep `replacement`. Only the removed block's own Part is called, not the
replacement block's Part.

The chosen state goes through the normal `setBlock` effects with flags 3, plus
64 for piston movement. An unknown or unregistered state is refused with a
reason and the vanilla replacement is written. This covers `Level.removeBlock`,
not every direct `setBlock` or the separate `destroyBlock` path. The level view
is read-only while answering.

## The manifest

```toml
id = "my_mod"                # [a-z0-9_]{1,64}: settings key, log name
name = "My mod"
version = "0.1.0"
description = "..."
authors = ["..."]
license = "MIT"              # optional: the Mods screen shows it
license_url = "https://..."  # optional: makes the license a link
credits = "..."              # optional: thanks, shown under the authors
homepage = "https://..."     # optional: the Mods screen's Homepage button
issues = "https://..."       # optional: the Mods screen's Issues button
side = "both"
namespaces = []              # optional: more namespaces the mod's files use (below)
served_by = "my_runtime"     # optional (content only): native runtime mod with serves_content = true

[properties]                 # optional: values other mods read (reference/manifest.md)
tier = 3
```

The optional fields fill the game's Mods screen, which lists every mod with its icon, name, version, id, license,
authors, credits and description, as NeoForge's mod list does. A link must be `http` or `https` and at most 2048
characters; the others are plain text. The screen has no switch: a mod runs or not by what
`sunrise-mods.txt` says ([installing and using mods](../players/installing-mods.md)).

Unknown keys are refused, so a typo is an error that names the nearest valid key. `side` is `"client"` (the default), `"both"` or `"server"`.
`api` and `capabilities` are not written: `cargo sunrise-mod build` puts `api = "0.2"` and `capabilities = ["registries"]` into the package. A manifest that does write them must write exactly those values, and is refused with both values otherwise.
Native code mods add `[native]` for exact build pins. Their id and version come only from this manifest,
not Cargo metadata ([Native mods](native-mods/02-the-package.md)). Both content and code mods support `[dependencies]`: use `base = "^1.2"` for a required dependency or
`extra = { version = "^2", optional = true }` for an optional one. Missing required dependencies, incompatible installed versions and cycles refuse startup. Content layers load after their dependencies; an absent optional dependency is ignored. Both kinds may include a fixed-name `icon.png` at the top level, with no
manifest key: a valid square PNG, at most 512 × 512 pixels and 256 KiB encoded ([Mod files](../reference/manifest.md)).

### Java jars as external assets

From a content or native mod's project folder:

```sh
cargo sunrise-mod use-jar /path/to/original-mod.jar
cargo sunrise-mod use-jar https://modrinth.com/mod/<project>/version/<version>
cargo sunrise-mod use-jar <project> <version>
```

When given a Modrinth version page URL or a `<project> <version>` pair, `use-jar` downloads the verified jar from
Modrinth's CDN into the local jar store before inspecting it. The command reads NeoForge or Fabric metadata and
appends a `[[jar]]` table to `sunrise-mod.toml` with the original `id`, `version` and the exact file's `sha512`
(128 lowercase hexadecimal characters). It does not infer identity from the filename, rename the Java mod to the
Sunrise mod's namespace, or run Java code. An unknown jar, invalid identity, unresolved version or file larger than
the shared 512 MiB hash limit is refused without changing the manifest.

The original manifest text, comments and existing list order are preserved. Listing the same hash and identity again
leaves the file byte-for-byte unchanged; a hash already listed with another ID or version is an error to correct,
not another entry. Use `[[jar]]` tables rather than an inline `jar = [...]` array when adding further jars with the command.
The list is ordered: new references are appended, not sorted.

`use-jar` stores the exact file as `<sha512>.jar` in the usual game home's `jars/` directory (`SUNRISE_HOME` if set,
otherwise `sunrise` in your platform's data folder). `check` and `build` read cached jars from that store (fetching
missing ones from Modrinth when online) so resource references resolve against the mod and its jars together. Run
`cargo sunrise-mod run` after adding the jar; `run` verifies
and stages only the jars in your manifest in `target/sunrise-mod/run/home/jars/` before building. The first run copies
each original while checking its SHA-512, using space for one extra copy per jar. Later runs still verify the original.
On Unix, `run` reuses a matching staged copy only after checking its link count to confirm it is independent.
On other platforms, it writes a fresh copy. Editing or replacing the original cannot change the staged bytes.
Leave the game's `--home` unset to use this development home; the staging path must not go through directory links.
To read originals from a different folder, supply its `<sha512>.jar` files with `cargo sunrise-mod run --jars <dir>`.

Jar bytes stay external. The original's `assets/` and `data/` are passive pack layers below the Sunrise mod's own
files. Do not copy a jar or its extracted files into the mod's source or `.smod`. A jar reference supplies assets
and data, not the original Java behavior: port that behavior through the mod API.

### Content mods (API 0.2)

A content mod has capability `registries` and carries `assets/` and `data/` in the `.smod`. It adds blocks and items
by declaring them in `data/<id>/sunrise/block/*.json` and `data/<id>/sunrise/item/*.json`, each copying a vanilla
template ([modpacks.md](https://github.com/sunrise-org/sunrise/blob/main/docs/modpacks.md#registries-02-data-driven-blocks-items-and-mob-effects)), and creative tabs, mob
effects, entity types and menu types (each below) in `creative_tab/`, `mob_effect/`, `entity_type/` and `menu/` beside
them. `fluid/` gives a fluid a client look (`sprite`, an atlas sprite id, and `tint`, `0xAARRGGBB`, both optional), as
NeoForge's `IClientFluidTypeExtensions`; the fluid needs no block, and a recipe page draws its well from it. Its `world`
object makes it exist in the world: `{"world": {}}` adds the block `<id>:<path>`, a copy of `minecraft:water` that flows
with vanilla's fluid simulation, and the bucket item `<id>:<path>_bucket`, which needs its model as any item does. `world`
takes `tick_delay` (1 or more, default 5), `drop_off` (1 to 7, default 1), `slope_find_distance` (1 to 4, default 4),
`source_conversion` (default false), `bucket` (default true), `flowing`, the flowing form's id in the fluid's
namespace and not the fluid's own id (default `<id>:flowing_<path>`), and `block`, the liquid block's id in the fluid's
namespace and not the flowing form's (NeoForge's `BaseFlowingFluid.Properties.block`; default the fluid's own id), which
commands, data packs and structure files then name. Code passes the same through `WorldFluid::block`. Beside `world`, `bucket_item` names the item that
holds a bucket of the fluid (`Fluid.getBucket`): the generated bucket's id, in the fluid's namespace, or, for a tank-only
fluid or one with `"bucket": false`, an item the mod declares itself, which keeps the fluid through its own parts. An
empty bucket fills to that item from any fluid handler, with the bucket's fill sound, and a generated bucket empties
back into one. An item no item has, or the game's bucket of another fluid such as `minecraft:water_bucket`, is no
bucket of it. A fluid file may also state NeoForge's
`FluidType` numbers, which other mods read through `FluidTypeProperties`: `temperature` and `density` (any signed
32-bit integer, defaults 300 and 1000), `viscosity` (0 or more, default 1000) and `light_level` (0 to 15, default 0,
whether or not the fluid has a block). A file without `world` declares no
fluid by itself; its numbers describe the tank-only fluid a native mod declares in code with `declare_fluid`. The fluid is in no vanilla fluid tag, so entities don't
swim in it or get pushed by it. In the world it draws its `sprite`, a block atlas texture such as `mymod:block/oil`, times
its `tint`; with neither it looks like water, and a `tint` alone colours water's textures. A client mod's
`register_fluid_models` hook accepts `mod_api::FluidModel` values with separate flowing and overlay sprites via
`event.register(model)`. The hook runs again before each resource bake, so a resource reload can change model
identifiers as well as textures. Far terrain (LOD) omits mod fluids. LAN and dedicated server players play with
them when they run the same mods: a client whose mods add different content is refused when it joins, with
the reason.

#### Join compatibility and package sides

Every Sunrise server checks loaded mods in the configuration phase (`sunrise_protocol::mod_list`, channel
`sunrise:mod_list`), including when its own list is empty. Only the ids and versions of `side = "both"` mods cross
this channel. Missing mods, extra both-side mods and version differences refuse the join, naming every id and
version in a translated, wrapped disconnect. Client-only mods stay private and never block a join. A manifest
with no `side` line is `client`, so a mod meant for both halves must say `side = "both"`. `side = "server"` is a mod
only the server needs: it installs and runs like any mod (in single player and on a LAN host it runs in the integrated
server), but it is never compared at join, so players joining need not have it. A server mod is refused by name at
load when it carries what every joining client must know: registry declarations (`data/<id>/sunrise/`, or blocks,
items, block entity types and fluids its code declares), a listener on a client-side hook, a part that listens to one,
a required payload the server sends to clients, or a synced attachment type it declares.
For attachments, “carries” means **declares**: a server package may look up and use another loaded package A's synced
attachment, because clients compare A and decode the value through A. If A is itself a server package, A is the
declarer refused by name. Attachment load checks registration refusals (invalid descriptors, duplicate IDs, and stale
part-state keys) first, then server-package synced declarations (matched by the declaring package's owner ID
regardless of attachment namespace); no content is admitted after a refusal. See [attachments](../concepts/attachments.md)
for the declaration API.
Assets, lang and data vanilla syncs itself (recipes, loot, tags) are allowed. Where a hook runs is still the hook's own side: the manifest says what a client must have, not where a
hook runs. A vanilla or other
non-Sunrise client is refused when the server needs both-side mods, with their names and versions and an English
fallback for clients without Sunrise translations. The content registry identity check remains separate; the
mod list does not hash package bytes. Native build inventory validation is a separate server-export concern.

A payload that breaks the format (an oversized list, a duplicate id, trailing bytes) is refused the same way a
mismatch is: the join ends in a translated disconnect naming the decode error, never in play, and never as a
silently empty list.

Sunrise peers identify themselves before sharing mod lists. After receiving the server's `sunrise` brand, the
client announces its own brand and list before answering known packs; that reply orders the compatibility
check without an extra ping. The server sends its list only to an identified Sunrise client, including when
either list is empty. Vanilla clients receive no mod-list payload, and clients do not disclose their mod
identities to servers that have not identified themselves as Sunrise. Nothing here repeats per tick.

#### Blocks

A block names its template and what differs from it:

```json
{ "template": "minecraft:iron_block",
  "properties": { "hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block" },
  "item": true }
```

A fence gate needs no kind of its own: a block whose template is a vanilla `minecraft:*_fence_gate` is a
`FenceGateBlock`, with the template wood's open and close sounds. `"open_sound"` and `"close_sound"` accept a sound
event id, such as `"minecraft:block.chain.break"` or a sound of the mod's own `sounds.json`, to replace that sound at
all three sites. They also accept an object, for example `"open_sound": {"use": "rope:open", "redstone": "rope:powered"}`:
`use` is a player's interaction (including client prediction), `explosion` is an unpowered gate triggered by an
explosion, and `redstone` is a power change that changes the open state. A missing site keeps the **template wood's**
sound, not another site's override. Code declarations use `GateSound::All` or `GateSound::Sites` with
`on_use`, `on_explosion`, and `on_redstone`. Both keys need a `*_fence_gate` template; unknown sites and invalid
`namespace:path` sound ids fail the load naming the file and key. Leaving both keys out keeps the wood's sounds.
Pitch draws, volume, sound source, player exclusion and game events are unchanged.

A block gets a block item of its own id by default. `"item": false` gives it none: pick-block takes nothing and
`Block.asItem` is air (what a block drops comes from its loot table). `"block_item": "<item id>"` (with `"item": false`) makes the block
that other item instead, an item of the mod or a vanilla one, such as `"minecraft:diamond"`: pick-block and
`Block.asItem` both read it, and the load fails naming the block when no such item exists. `"none"` is not an item id;
write `"item": false`. A sign's wall form takes its sign's item through `wall_block`, not `block_item`.

A block's item is named by the block's key, `block.<namespace>.<path>`, as vanilla's own block items are
(`Item.Properties.useBlockDescriptionPrefix`). `"use_block_description_prefix": false` names it by its own key,
`item.<namespace>.<path>`, as a plain `new BlockItem(block, new Item.Properties())` is: write it when your language
file keys the item under `item.`. A `minecraft:item_name` in the block's `components` wins either way.

Pick-block takes the block's item by default. `"pick_item": "<item id>"` names a different one, with any `item` and
`block_item`: vanilla's `getCloneItemStack`, which lets a mature crop pick its seed while `Block.asItem` stays what it
is (air for `"item": false`). It is an item of the mod or a vanilla one, and the load fails naming the block when no
such item exists. A sign's wall form picks its sign's item through `wall_block`, so it declares no `pick_item`.

`"suspicious_stew_effects": [{"id": "minecraft:night_vision", "duration": 100}]` makes the block a suspicious stew
source. Entries are applied in order; duration is in ticks and defaults to 160 when omitted. An unknown mob effect is
reported when content loads. Native `Block` declarations give the same list as `(effect ID, duration in ticks)` pairs.
Food & Drinks and Search show one stew for each distinct list, and feeding the block's item to a brown mooshroom
lets you milk it into that stew. The item must place the block: neither `block_item` nor `pick_item` makes an item
place it. This does not add a crafting recipe. To craft the stew, supply a `minecraft:crafting_shapeless` recipe with
the block's item, a bowl and two `#minecraft:mushrooms`, and set `minecraft:suspicious_stew_effects` on the result.

`push_reaction` sets vanilla's `BlockBehaviour.Properties.pushReaction` to `"PUSH_PULL"`, `"PUSH"`, `"POPPED"`, `"IMMOVEABLE"`, or `"IGNORE_ENTITY"`.

`random_ticks` makes a block tick randomly whatever its template does, as vanilla's
`isRandomlyTicking(state)` does for its own blocks, so a crop, sapling or soil on a template that never
ticks grows: `"random_ticks": true` for every state (or `false`, to stop a template that ticks), or
`{ "property": "grown", "values": ["false"] }` for the states whose own state property (declared in
`state_properties`) is one of the values. The block's native `random_tick` hook then fires as it does
for a vanilla block that ticks, and its sections count it.

`render_shape` says how a block is drawn whatever its template does, as vanilla's `getRenderShape`:
`"MODEL"` puts its block model in the chunk mesh, `"ENTITYBLOCK_ANIMATED"` leaves the model out for a block whose
moving parts its block entity draws every frame (a turning shaft, a machine's arm), and `"INVISIBLE"` draws nothing.
Its outline, collision, light and occlusion stay as declared: on a template that occludes (stone), the faces it
covers stay hidden unless it also sets `occludes`.

`occludes: false` makes a block occlude nothing whatever its template does, as vanilla's `noOcclusion()`: the faces
of the blocks beside it are drawn, so a thin panel or a block drawn by its block entity on a full-cube template
doesn't leave a hole in the floor under it. It is not a solid render either, so light passes it as it passes
leaves: dampened by 1, or not at all if skylight passes down; special template light rules remain inherited.
`occludes: true` occludes whatever the template does, using the declared outline when supplied, otherwise the
template's published occlusion shape. A full shape is a solid render with light dampening 15; a partial or empty
shape is not. Unset preserves the template's ability to occlude. Skylight-down and special template light rules
remain inherited. Partial declared outlines retain the existing directional shape-light occlusion rule.

`offset` takes `NONE`, `XZ` or `XYZ` (`OffsetType` in code drafts).
`NONE` disables an inherited offset. `XZ` jitters horizontally; `XYZ` also moves downward.
An explicit mode defaults to maximum horizontal displacement 0.25 and, for `XYZ`, maximum downward
displacement 0.2. Optional `max_horizontal_offset` (finite 0 to 0.25) and `max_vertical_offset`
(finite 0 to 0.2) set these limits; `XZ` ignores the vertical limit. Without `offset`, the template's
mode and default limits are inherited, and supplied maxima adjust that mode without enabling one.

The model and **declared** outline, collision and interaction shapes follow the same column seed
(`BlockState.getOffset`, x and z only), including declared entity-context variants. Shape caches stay
block-local; the position is applied at query time. The occlusion cache uses the outline after applying
its column offset at `(0, 0, 0)`, as vanilla's state cache does.
This works on both sides and does not require a hook.

`instrument` takes any vanilla `NoteBlockInstrument` constant name (for example `HARP` or `FLUTE`);
code declarations use `NoteInstrument`. A note block reads ordinary instruments from the block
below it and head instruments (`ZOMBIE` through `CUSTOM_HEAD`) from the block above it. `CUSTOM_HEAD`
is silent without a skull block entity above the note block with a `note_block_sound`.
`ignited_by_lava` sets lava's ignition flag; a subscribed per-position `ignited_by_lava` hook still answers
first. `replaceable` controls replacement by block placement and by features that check replaceability;
the usual refusal when the hand holds the block's own item still applies. Unset values inherit the
template. These are load-time properties on both sides, with no new hook.

`skip_rendering`, `hides_neighbor_face` and `supports_external_face_hiding` control culled neighbor
faces as compiled block data (`BlockBehaviour.skipRendering`, NeoForge
`IBlockExtension.hidesNeighborFace` and `supportsExternalFaceHiding`), without running mod code on
meshing workers:
- `skip_rendering` is a list of face rules that replaces the template's skip rule (`[]` disables it;
  omitted inherits the template). `[{"neighbor": "self"}]` skips faces against any state of this
  same block, as glass does.
- `hides_neighbor_face` is a list of face rules whose matching neighbor faces pointing toward this
  block are hidden when the neighbor supports external face hiding.
- `supports_external_face_hiding: false` prevents any neighbor's `hides_neighbor_face` rules from
  hiding this block's faces (default `true`).

Each rule object accepts optional `neighbor` (a block id, `#tag`, `{"block"|"tag", "state"}`,
`"self"`, or omitted for any neighbor), `state` (required property values on this block), `same`
(property names that must match on both states), `faces` (`"all"`, `"horizontal"`, `"vertical"` or
a list of direction names) and `facts` (a closed set of `BlockBehaviour.BlockStateBase` booleans:
`is_air`, `can_occlude`, `is_solid_render`, `liquid`). Tag rules resolve against the client's
synced block tags and remesh loaded sections on tag sync.

`spawn_rule` sets which mobs spawn on the block (`BlockBehaviour.Properties.isValidSpawn`): `default` is vanilla's
(a sturdy top face and light emission below 14), `always` and `never` answer for every mob, and `ocelot_or_parrot`
only for those two. It is read once at load into the block's spawn tables, so it is one answer per state; `default`
uses the template's top face and the declared `light` or `state_light` (the template's when none is declared), not a
`dynamic_light` cell's (sunrise-org/sunrise#5518: a declared shape doesn't change the top face yet). Unset keeps
the template's rule. In code, `BlockProperties::spawn_rule` takes the same rules as `SpawnRule`.

`destroy_overlay: false` hides the breaking crack (`destroy_stage_0` to `9`) on the block, for one broken by use
(a feast or a cake taken apart bite by bite). It is read once at load into the block's facts, and only the overlay
is skipped: the mining progress, the server's progress and the destroy-progress packets other players see stay
vanilla's. Unset or `true`, the crack is drawn.

`dynamic_light: true` makes a block's light depend on where it is, not only on its state
(`IBlockExtension.hasDynamicLightEmission`): a lamp that glows only on a powered rail, a crystal brighter deeper
down. Its parts answer `BlockPart::light_emission` with the light a cell emits, and `light` (or `state_light`) is
what a cell emits without an answer. The game asks on the thread that owns the level, the server's and each
client's, as a chunk holding the block comes in and as an edit sets it, and keeps the answer until the cell's state
changes; the light engine never asks on its own. A part whose answer reads more than the state (the block below
it) calls `LevelRef::check_light(host, pos)` when that changes (`LevelLightEngine.checkBlock`): the server asks
again and relights, and clients ask their own parts again as its light update reaches them. The flag is per block,
as it sits on the block's row: every state of the block is asked, so a block with only some states that vary pays
the ask for all of them. Light that depends only on the state needs no part: use `light` or `state_light`.

A mod can [query a candidate state's source light](world/block-state-facts.md#contextual-light-emission)
with `LevelRef::get_light_emission`. It returns the raw Part answer or the declared light, with no
relight. This differs from `LevelRef::brightness`, which reads the light the engine has propagated.

##### Per-state light

Blocks in JSON and code can set `state_light`, which overrides `properties.light`.
Use `properties` for an ordered list of state property names, `values` for
comma-separated value tuples mapped to emission (0 to 15), and `default` for
unmatched states. Without a default, every state must match a tuple.
For example, `"state_light":{"properties":["lit"],"values":{"true":15},"default":0}`
makes a lamp dark when unlit and emit 15 when lit. Code uses `mod_api::StateLight`
with the same `properties`, `values` and `default` fields on `Block::state_light`.

A block that joins its neighbours in six directions (a cable, a pipe) copies
`minecraft:chorus_plant`, vanilla's `PipeBlock`, and names the block tag it
connects to besides itself:

```json
{ "template": "minecraft:chorus_plant", "connects_to": "#mymod:cable_connects" }
```

It keeps the chorus plant's six sides but none of its rules: it needs no
support, and each side is joined when the block there is the same block or in
the tag, on placement and whenever that neighbour changes. Its shapes are the
chorus plant's (10 pixels across) unless it sets `"thickness"`, 2 to 14
pixels: its outline and collision are then built as `PipeBlock` builds its
own, a core and an arm to each joined side.

A block copies its template's states. It can also declare state properties of
its own, which its template does not have — how a pipe says which sides it
connects to, or a machine whether it is running. Its states then become its
template's states times the combinations of the ones it declares, laid out as
vanilla lays out a block's states (`StateDefinition`): every property, the
template's and the declared ones, sorted by name with the first name varying
slowest, and each declared property's default value first. The order is fixed,
so ids do not move between loads, and a declared name may sort before one of
the template's (`bites` on a carved pumpkin, whose `facing` comes after it):

```json
{ "template": "minecraft:stone",
  "state_properties": {
    "facing": { "type": "enum", "values": ["north", "south", "east", "west"] },
    "lit": { "type": "boolean" } },
  "default_state": { "lit": "true" } }
```

A `boolean` is `false` and `true`, as vanilla spells them; an `enum` is a list
of values, and a name or value is lower case letters, digits and underscores.
A declared name may not be one the template already has, and `default_state`
takes one value per declared property too, so a block's default is exactly one
state of its own. Every combination has to fit in the states a world holds
(2^20), so a declaration that would not is refused when the mods load.

A declared **boolean** `waterlogged` also makes the block a `SimpleWaterloggedBlock`: buckets fill and drain it,
water can flow into it, and draining breaks it if it cannot survive dry. This applies even to a stone template.
The default placement takes `waterlogged` from source water at the target cell (flowing water leaves it dry); a shape update schedules the contained
water's tick. A block's `state_for_placement` or `update_shape` Part answer replaces the corresponding default.
A double plant keeps its template's class and companion-half behaviour, so this declares a two-high water plant:

```json
{ "template": "minecraft:tall_grass",
  "state_properties": { "waterlogged": { "type": "boolean" } },
  "default_state": { "waterlogged": "true" } }
```

The placed half takes source water at its cell; the companion half uses the water tag at its own cell, so an upper half above the water line is dry. An enum named `waterlogged`,
a template that already has that property, a standalone liquid block such as `minecraft:lava`, or an
always-water template (`seagrass`, `tall_seagrass`, `kelp`, `kelp_plant`, `bubble_column`) is refused.

A block with template `minecraft:farmland` may set `"base_block": "namespace:block"` to choose the block it becomes when it dries out, is trampled, or has a solid block placed above it. The ID must name a registered block; without the key, the template's dirt base is used.

A block whose template is one of vanilla's signs is a sign, whatever its name: it has vanilla's sign block
entity, so the text, the editor, dye, glow ink, wax and saving all work as they do on an oak sign, and the
client draws its text. A wooden sign is two blocks and a hanging sign two more, one form per template, and the
one with the item names its wall form:

```json
{ "template": "minecraft:oak_sign", "wood_type": "ruby:elm", "wall_block": "ruby:elm_wall_sign" }
```

```json
{ "template": "minecraft:oak_wall_sign", "wood_type": "ruby:elm", "item": false }
```

The same goes for `minecraft:oak_hanging_sign` and `minecraft:oak_wall_hanging_sign`. `wall_block` is what the
sign's item places on a wall, a block of the same mod, declared in its files when the sign is; without it the
item places the template's wall sign. The wall form has no item of its own, and its sign's item is what picking
it gives. `wood_type` names the editor's background:
`ruby:elm` is `assets/ruby/textures/gui/signs/elm.png` (24x26) and `assets/ruby/textures/gui/hanging_signs/elm.png`
(16x16), and a bare name is vanilla's, such as `cherry`; without it the template's wood is used. The board is
the block's model: `minecraft:block/template_sign_rot_0` to `_rot_3` and `template_wall_sign`, and for hanging
signs `template_hanging_sign_rot_0` to `_rot_3`, `template_attached_hanging_sign_rot_0` to `_rot_3` and
`template_wall_hanging_sign`, with `all` and `particle` textures. List your blocks in vanilla's tags as well:
`minecraft:standing_signs`, `minecraft:wall_signs`, `minecraft:ceiling_hanging_signs` and
`minecraft:wall_hanging_signs`, which vanilla's other sign tags and `minecraft:mineable/axe` include. Give the
wall form a loot table of its own, or name it `<x>_wall_sign` so that it drops what `<x>_sign`'s table does.

`sign_text_color` (a `DyeColor` name such as `"white"` or `"light_blue"`, black when unset) and `sign_text_glow`
(`true` or `false`) are the text a new sign starts with, as `SignBlockEntity.createDefaultSignText` gives it, for a
dark board whose black text would not read. They apply to the front and back text of a sign when it is placed, and
dye and glow ink work on it as usual; a saved sign loads as vanilla's does. A sign and its `wall_block` declare the
same values, so a sign does not change colour when it is hung on a wall.

A block can override its targeting outline with `shapes`. The declared outline also controls render
solidity, face occlusion and default lighting: an occluding full cube is solid and blocks light; a partial
shape uses its covered faces to stop light directionally. An empty or partial outline is not a solid
render. Fluid-bearing states, including intrinsic water in bubble-column and kelp templates, retain their
fluid attenuation and downward-skylight behavior when the declared geometry is unchanged. Special
skylight and light-dampening rules remain inherited from the template: fluid-dependent skylight rules
(such as vegetation) use the resolved state's fluid, including declared waterlogging, while glass keeps
its unconditional skylight rule and leaves keep their dampening. For example, a partial poppy-template
outline passes skylight without attenuation when dry, but attenuates it by 1 when waterlogged.
`occludes: false` disables face occlusion.

Movement collision is independently selectable with `collision_shapes`; it does not change the outline,
render solidity, occlusion or lighting. Both take the same table format. Boxes are
`[minX,minY,minZ,maxX,maxY,maxZ]` in block units (one block is 1), any number per shape, as in vanilla.
Coordinates must be finite and each minimum must not exceed its maximum. `[]` is an empty shape.

A declared shape also decides the block's legacy solid bit (`BlockState.isSolid`, which attachment
survival such as wall signs and torches reads): it is calculated from `collision_shapes`, else from
`shapes`, as vanilla does for any block (bounding-box average size at least 0.7291666666666666, or at least a block tall). A
template that forces solidity on or off keeps its answer, and a block that declares no shape keeps the
template's. This is not render solidity and not sturdy-face support.

```json
{
  "template": "minecraft:stone",
  "state_properties": {
    "facing": { "type": "enum", "values": ["north", "south"] },
    "count": { "type": "enum", "values": ["1", "2"] }
  },
  "shapes": {
    "properties": ["facing", "count"],
    "values": { "north,2": [[0,0,0,1,0.5,1]] },
    "default": [[0,0,0,1,1,1]]
  },
  "collision_shapes": { "default": [] }
}
```

`properties` names an ordered tuple of template and/or declared state properties; each `values` key
lists their values in that order, separated by commas without spaces. A nonmatching tuple uses
`default`. Without `default`, every tuple must be covered. A default-only table gives every state the
same shape. A single-property table uses `"properties":["axis"]`. Unknown properties, unreachable
value tuples, malformed boxes and duplicate property names reject the mod with a load error rather
than silently ignoring a typo. Omitted `shapes` preserves the template outline (or the connecting
block's `thickness` outline). Omitted `collision_shapes` preserves the existing behavior: a declared
outline also becomes collision, otherwise the template collision is retained. An explicit collision
table can be used without an outline override, and overrides `thickness` collision too.

##### Entity-context shape declarations

Native declarations use the same fields on `mod_api::Block`, with API-owned
`ShapeEntityKind::{Item, Player, Living, Other}` keys and `BlockShapes` values.
`interaction_shapes` is an optional `BlockShapes`. The game validates these drafts
at content admission, just like their JSON counterparts.

`outline_context_shapes` and `collision_context_shapes` independently override
`shapes` and `collision_shapes` for a captured entity class. Their keys are
`item`, `player`, `living` (a non-player living entity) and `other`; each value
is a shape table with the same properties, tuples, defaults and box validation
as `shapes`. There is no inheritance between classes or channels. An omitted
class keeps that channel's ordinary shape. Empty and position-only queries
always keep the ordinary shape; a selector is not inferred from a runtime entity ID.

```json
{
  "template": "minecraft:stone",
  "shapes": { "default": [[0, 0, 0, 1, 0.125, 1]] },
  "outline_context_shapes": {
    "player": { "default": [[0, 0.125, 0, 1, 1, 1]] }
  },
  "collision_context_shapes": {
    "item": { "default": [[0, 0, 0, 1, 0.3125, 1]] }
  },
  "interaction_shapes": { "default": [[0, 0, 0, 1, 1, 1]] }
}
```

`interaction_shapes` uses the same state-table format. It supplies
`BlockGetter.clipWithInteractionOverride`: an earlier interaction-shape entry
changes the face of an existing outline hit while keeping its point. It cannot
create a hit through an empty outline or extend the ray's reach.

Tables compile once per resource snapshot. Collision scan emptiness and
out-of-cell metadata account for every collision variant. Dropped-item movement
and overlap checks carry `item`, player movement and placement carry `player`,
and player selection uses its own outline channel. Callers without a captured
entity class retain ordinary shapes. These declarations select state/class-only
geometry, not arbitrary world-dependent block behavior or native callbacks.

##### Scaffolding's context collision

A block that is stood on like scaffolding and dropped through while sneaking (a metal
scaffolding) sets `"context_collision": "scaffolding"` (`Block::context_collision =
Some(ContextCollision::Scaffolding)`), whatever its template. Entities then collide with it as
with `minecraft:scaffolding` (`ScaffoldingBlock.getCollisionShape`), which takes the same rule:
solid on top unless the entity is descending, nothing below the top, and in a `bottom` state
that is not supported (`distance` above 0) a floor 2 pixels high under an entity above the block
below. Placement checks see no collision. Its states need scaffolding's `distance` (0 to 7) and
`bottom` (`false`, `true`) properties, from a `minecraft:scaffolding` template or declared in
`state_properties`; loading refuses the block, by name, otherwise. Climbing is the
`minecraft:climbable` block tag, and what keeps `distance` and `bottom` up to date is the
template's behaviour: neither comes from this field. A `collision_context_shapes` entry for an
entity class stands over the rule for that class, as it does over the block's other shapes.

```json
{
  "template": "minecraft:scaffolding",
  "context_collision": "scaffolding"
}
```

##### Shapes a block entity publishes

A block whose geometry depends on what is inside it (a cable bus with attached parts, a table
showing its tools) sets `"shapes_from_block_entity": true` (`Block::shapes_from_block_entity`).
Loading refuses it, naming the block, when no block entity type owns the block. Its block entity
then publishes the block's shapes: `BlockEntityRef::set_shapes(host, Some(BlockEntityShapes {
outline, collision }))`, boxes in block units as in `shapes`, or `None` to clear them. The
declared `shapes` and `collision_shapes` apply while it holds none, which is also the case until
the first publish.

This differs from NeoForge, where `getShape` is asked for on every query: here a shape is
**published when it changes, not asked for per query**, and a port of a `getShape` override calls
`set_shapes` wherever that state changes (a part attached, a lid opened). Physics, selection,
ray casts (outline and collider, over a dynamic template's shape too), support, the floor a
mob's path stands on, its move control's jump, the height of an evoker's fangs, a panicking mob's
search for water and whether a summoned iron golem can stand there read the published value at
the position; nothing calls into the mod.
Face sturdiness and multiface attachment between neighbouring blocks (placement, survival,
shape updates, as for lichen and sculk veins) read the published collision shape at the position too.
The sturdy floor an evoker's fangs look for under them still reads the declared shape.
Fluid passage reads it as well: water and lava pass between two cells by their collision shapes at
their positions. A publish, a clear and an aborted publish (the old shapes put back) each notify the
block's neighbours, so a fluid beside it reads the new shape; publishing an unchanged value notifies nothing.
Shapes that depend on the querying entity are not published: a part answers them per query
(next section), and a published shape stands over the declared `outline_context_shapes` and
`collision_context_shapes`.

- **Checks**, refused by name with a `BlockEntityShapesError` and never clamped: every
  coordinate finite and each minimum at most its maximum; at most 64 boxes in each list.
  Vanilla has no box limit; 64 bounds what a block entity makes every client receive and
  keep, while a declared shape table (`shapes`, above) syncs nothing and has no box limit.
  There is no coordinate range. A box under 1e-7 thick on an axis is an empty box,
  as in vanilla.
- **Reach**: collision finds boxes only within one cell of the block, one axis at a time (a
  state that publishes is always scanned that way); a box beyond that is stored and sent but
  inert, as in vanilla. An outline can be hit past its cell once the ray enters the owning
  cell. A debug build warns once per block when a publish holds a box beyond one cell.
- **Timing**: the server applies a publish at once, to the queries that follow in the same
  tick. A client applies it when the block entity update arrives, which may be a tick later, as
  with any block change. An entity the new shape now overlaps is treated as vanilla treats a
  block that appears inside it: nothing pushes it out.
- **Lifetime**: the shapes are saved with the block entity, restored when its chunk loads before
  anything ticks, and sent with the block entity in the chunk and in its updates, so a part
  need not publish again after a load or a chunk receive: a publish is an update, never a
  repair. They are dropped when the block entity is removed or its block changes. The key they
  are kept under is the engine's: `replace_data` and `merge_data` leave them alone, and a
  payload that carries it has it ignored.

##### Shapes that depend on the querying entity

A block part can give a block another collision shape for the entity that asks, as vanilla's
`getCollisionShape(state, level, pos, context)` does: `BlockPart::collision_shape(context, event)`
sets `*event.shape = Some(shape)` for the state `event.state` at `event.pos`.
Build the shape with `VoxelShape::try_from_boxes(&boxes)` using block-local
`[min_x, min_y, min_z, max_x, max_y, max_z]`, or use `VoxelShape::empty()` to remove collision. This is the one place the engine calls a
mod in the middle of physics, for the blocks whose parts opt in; everything else, and every
published shape, is read without a call. It reverses the rule above for those blocks only.

`event.query` is a `CollisionQuery`, vanilla's `EntityCollisionContext` as it was when the query
was made: `entity()` (the entity's id, `None` for the empty, position and cache contexts),
`kind()`, `is_descending()`, `is_placement()`, `always_collide_with_fluid()`,
`is_above(shape, pos, default)`, `is_holding_item(item)` and `can_stand_on_fluid(above, fluid)`;
and `dimension()`, the dimension id of the level the query is in (the `level` vanilla's
`getCollisionShape` gets), `None` only for a query a mod built by hand.

- **Ordering.** `None`, the starting value, keeps whatever the block would otherwise have: its
  declared shape, a context-keyed one, or the one its block entity published. `Some` replaces
  that for this query only. The per-state shape cache stays the empty-context answer, so a block
  that is large there is still scanned in the cells around its own.
- **Who asks.** Entity movement (step-up retries included), placement checks and projectile
  paths, on the thread that owns the level: the server tick, and the client's own prediction. A
  guest's move is checked by the server with the same entity id. Fluid passage and face-support
  checks never ask. Another thread (the mesher, world generation) reads the normal shape.
- **Every query.** A part is asked for every query that meets an opted-in state, as vanilla asks
  `getCollisionShape` for every block of every sweep. A step-up retry or a second sweep asks
  again, and a change to the mod's own state inside a tick is seen by the next query.
- **Not the selection shape.** The outline a player targets is the declared shape or the one the
  block entity publishes; this part is never asked for it, so a block can be solid and
  unselectable, or passable and selectable.
- **No re-entry.** The query holds the entity's state from before the move, and a collision query
  made inside the callback gets the empty-context answer, never a nested call.
- **No `Host`.** A move holds the level's collision data and cannot lend a `Host`, as for
  `entity_move`. The part answers from the query and from state it keeps itself. A
  rule that needs more (a belt's set of carried entities; powder snow's fall distance) tracks that
  state in the mod, keyed by `query.entity()`. State kept by position (a block whose shape
  depends on data the mod stores for that spot) is keyed by `event.pos` and `query.dimension()`,
  since one position exists in every dimension. If a mod needs an entity read it cannot track, one
  captured field is added to `CollisionQuery`; the `Host` is not reopened.
- **The client must sync that state.** The client asks the same question for its own prediction,
  so a mod whose rule reads its own state must send it to the client. If the two disagree the
  server corrects the player's position (the usual "moved wrongly" correction), never a kick.
- **Bounds.** The constructor rejects non-finite or inverted bounds and unions it cannot
  represent as a voxel grid. These in-process answers have no box-count cap; this does not
  change the separate saved and networked block-entity shape format.
- **Faults.** If a Part panics, only that invocation's answer is discarded. The next Part sees
  the previous answer, or `None` if no earlier Part answered. A failed shape construction
  returns an error to its caller without changing the answer.
- **Cost.** Nothing for a block without such a part. An opted-in block costs one call per
  query that meets it.

#### Items

An item copies its template's data components, with `max_stack_size` and
`rarity` changed, and any other 26.3 component set or removed in
`components`, written as a data component patch is: `minecraft:<type>` sets
one, `!minecraft:<type>` with `{}` removes it. That is how a modded sword
gets its own damage and durability, or a tool its mining speed:

```json
{ "template": "minecraft:iron_sword",
  "components": {
    "minecraft:max_damage": 900,
    "minecraft:attribute_modifiers": [{ "type": "minecraft:attack_damage",
      "id": "minecraft:base_attack_damage", "amount": 7.0,
      "operation": "add_value", "slot": "mainhand" }],
    "!minecraft:repairable": {} } }
```

A held item's third-person arm pose follows its use animation, as vanilla's `AvatarRenderer.getArmPose` reads it: the
`minecraft:consumable` component's `animation` (`eat`, `drink`, `block`, `bow`, `trident`, `crossbow`, `spyglass`,
`toot_horn`, `brush`, `bundle`, `spear` or `none`), else `block` for an item with `minecraft:blocks_attacks`. A content
item with `"minecraft:consumable": { "animation": "toot_horn", "consume_seconds": 7 }` raises its arm like a goat horn
while it is used, for players and in other players' views.

`arm_pose` names the pose a humanoid's arm takes holding the item, in place of what vanilla gives it, for as long as
the item is in that hand (a used item's animation included): one of vanilla's `HumanoidModel.ArmPose` names, `empty`,
`item`, `block`, `bow_and_arrow`, `crossbow_hold`, `crossbow_charge`, `throw_trident`, `toot_horn`, `spyglass`, `spear`
or `brush`. A name that is no pose fails the mod's load, and an item without the key poses as before. It applies to
player models, in the first person's own third-person view and in other players'; `spear` poses an arm only for an item
with a kinetic weapon. A code mod that must decide by stack, hand or entity answers the `get_arm_pose` hook instead,
which wins over the declared key.

Code mods can also register a namespaced custom pose once in `Hooks::register` through
`Registrar::register_arm_pose("mymod:flip", uses_both_arms, affects_offhand)`. Keep its
`CustomArmPose` token and return `ArmPose::Custom(token)` from `get_arm_pose`. The two
flags independently control the humanoid's two-arm ordering and whether the other
hand's pose gives way. The token carries both flags. Any valid explicit `namespace:path` is
accepted with ownership attributed to the registering mod; duplicate and malformed ids refuse
registration.

In `arm_pose_transform`, match `event.pose` to that token and set `event.angles.right`
or `event.angles.left` to absolute `[pitch, yaw, roll]` in radians. `event.right_arm`
identifies the physical arm; `event.humanoid` supplies both stacks, the use state,
and the poses selected after `get_arm_pose` answers. Transformers follow vanilla's
physical-arm application order and do not run for a suppressed pose.
Each non-finite angle leaves the base model component unchanged. The transform runs
only for selected registered poses, on local players, remote players and humanoid mobs;
swing and mob-specific animations run afterward and can change those angles.

For a change to an intermediate vanilla pose (including `item`), listen to
`humanoid_setup_anim` instead. This client hook runs inside each actual humanoid
setup-animation invocation, after head, walk and riding setup but before arm
priority and posing. Its read-only `ClientHost` and `event.humanoid` provide both
held stacks, selected poses and `is_using_item`. `event.partial_tick` and optional
`event.game_time` are captured for that draw; game time is absent outside a level.
`event.parts` is a `&mut dyn HumanoidPose` borrowed for this invocation's callback, not an
owned or serializable snapshot. Use `event.parts.get(HumanoidPart::RightArm)` to read
`[position, rotation]`, then `set_rotation` or `set_position` on that same view. The head, hat,
body, arms and legs are editable if present; sleeves, jacket and pants are not. Positions are
local texels and rotations are radians. Writes apply all three components
immediately on any present editable part, matching `HumanoidModel.setupAnim`
with no finiteness gate, so later listeners see those edits. Scale and visibility
are not changed. Later ITEM, attack, crouch, bob and swim animation still apply.
Armor stands then replace rotations with their saved pose, preserving position
edits. Their armor models run the same setup independently. A model can animate
more than once per frame, so listeners must not treat this as a once-per-entity
frame notification or advance simulation from it. When nobody subscribes, there
is no callback or held-stack snapshot. To read
the completed animation, use a [layer's or Post hook's final model](client/model-drawing.md#read-the-final-body-model).

```json
{ "template": "minecraft:stick", "arm_pose": "toot_horn" }
```

An item can place any registered block by naming it with `block`, including a block another package declares:

```json
{ "template": "minecraft:wheat_seeds", "block": "ruby:crop" }
```

This uses the block's placement state and survival rules and consumes the item outside creative mode. Add seeds to
`minecraft:villager_plantable_seeds` for farmers to plant them. The mapping does not change what the block drops or
what Pick Block gives.

An item whose template is `minecraft:fishing_rod` (and no `block`) is a fishing rod, as a NeoForge mod's
`new FishingRodItem(...)` is: it casts, floats, bites, reels in and wears as vanilla's rod, its item model's
`minecraft:fishing_rod/cast` is true while its bobber is out, and `player_fish` hears it. List it in
`#minecraft:enchantable/fishing` for Lure and Luck of the Sea, as vanilla's rod is. Other behaviour that vanilla ties
to an item's class, such as a bow's draw or a shield's block, belongs to vanilla's item alone; components
(`minecraft:tool`, `minecraft:consumable`, `minecraft:equippable` and the others) carry over for every template.

```json
{ "template": "minecraft:fishing_rod", "components": { "minecraft:max_damage": 128 } }
```

Native mods can register component types before content loads. A content item
can set a registered type with a JSON value, such as `"sample:charge": {"charge": 42}`.
JSON values become NBT values. Values must fit the network NBT limits and
round-trip through saved NBT; negative zero is not supported. A key such as
`"!sample:charge": {}` removes the template's default. Unknown types fail when Sunrise builds the content root;
`cargo sunrise-mod check` cannot check native registrations.

`crafting_remainder` names what the item leaves in the crafting grid when a
recipe uses it, as a milk bucket leaves a bucket (vanilla's
`Item.Properties.craftRemainder`, not a data component). It can name any registered item except
`minecraft:air`; omit the field for no remainder. `block` and `crafting_remainder` in a file take
`namespace:path`, and Sunrise resolves both against the completed registries at startup. A missing
target refuses loading with the declaring item, field and missing ID. If your pack needs another package
to register that target, declare the package as required in `[dependencies]`; registry namespaces do not
identify packages or set their load order. This also applies to items and item-bearing blocks
registered by a serving host or a package it serves (`served_by`).

A crafting remainder works on the crafting table, the inventory grid, the crafter and as furnace
fuel (the last of the stack leaves it in the fuel slot, a stack with more left drops it), not in a
brewing stand. What the item leaves once eaten or drunk is the `minecraft:use_remainder` component
instead:

```json
{ "template": "minecraft:mushroom_stew", "crafting_remainder": "minecraft:bowl" }
```

A block's item takes the same `components`, over the components of its
template block's item; a block with `"item": false` can't have them. Block files
have no `max_stack_size` or `rarity` keys, so those two go in `components` here
(an integer 1 to 99, and one of `common`, `uncommon`, `rare`, `epic`):

```json
{ "template": "minecraft:iron_block",
  "components": {
    "minecraft:max_stack_size": 16,
    "minecraft:damage_resistant": { "types": "#minecraft:is_fire" } } }
```

Unknown component types fail content loading unless a code mod registers them.
Item-id-specific behaviour does not follow a template: a copy of the trident does not throw.

#### Creative tabs

A mod's creative inventory tabs are `data/<id>/sunrise/creative_tab/*.json`:
a title, the item on the tab's button, and its items in order (items the game
doesn't have are left out):

```json
{ "title": "Ruby", "icon": "ruby:ruby", "items": ["ruby:ruby", "ruby:ruby_block"] }
```

They come after vanilla's tabs, ten to a page; the
buttons above the inventory turn the pages. The title is a lang key or English
text: `"title": "itemGroup.ruby"` uses that key from the mod's lang files, and a
lang file's `itemGroup.<mod id>.<file name>` (`creative_tab/gems.json` →
`itemGroup.ruby.gems`) translates any title. A mod's items join the search tab
under their names.

A file only makes new tabs. To edit a registered tab's contents, listen to
`build_creative_mode_tab_contents` (NeoForge's `BuildCreativeModeTabContentsEvent`). It fires for every tab on the
logical side building its catalog. Match `event.tab` (for example, `minecraft:ingredients`)
and edit `event.output` directly; its methods do not take a Host. To append a count-one
`stack` to both lists, call
`event.output.insert(CreativeTabPlacement::Last, stack, CreativeTabVisibility::Both)`.
You can also insert first, before or after a matching stack, or remove a matching stack.
Matching ignores counts and uses exact item and components. Edits to both lists
change parent first; a search error does not undo the parent edit. Feature
filtering happens after listeners finish.

Read the current entries in order with
`event.output.entries(CreativeTabList::Parent, &mut |entry| { /* read entry */ })`.
Use `entry.item()`, `entry.count()` or `entry.damage()` without copying the stack;
call `entry.project()` to copy the full stack when you need its components or want to
keep it. `event.output.remove_if(visibility, &mut |entry| predicate)` visits each
entry of the selected lists once and keeps survivors in their original order.
Neither the editor nor a borrowed entry can outlive its callback. Refused edits
return a typed error and are logged. A panic disables just that mod's creative-build
callback for the session. Completed edits remain visible to later listeners, and
the action that requested the catalog can continue.

From either side of an open world, call
`host.creative_tabs(CreativeTabPermissions::Normal)` to read the catalog for
ordinary players, or request `Operator` explicitly to include operator
generator output. The choice does not grant permission. During setup and reload,
the read returns `CreativeTabReadError::Unavailable`. The returned snapshot includes
every registered tab, even empty or special tabs. Use
`tab.contains(host, &stack)` for exact membership in that tab's search list and
`snapshot.tabs_showing(host, &stack)` for category tabs that display it. Use the
originating world's Host so redundant component patches compare against its
item prototypes. The game
builds only on demand, keeps one complete snapshot per side, and does not
rebuild just because tags changed. A same-parameter read from a build listener
sees completed tabs and the previous contents of the remaining tabs;
a different-parameter read returns `CreativeTabReadError::Reentrant`.

With a shared `&dyn Host`, use `host.built_creative_tabs()` to read the last
completed snapshot without firing any build callbacks. It returns `Ok(None)`
before that side's first completed build and `CreativeTabReadError::Unavailable`
outside a live world. It does not change the snapshot's build permissions.
Each tab's `display_name` is a translatable component for its `itemGroup.*` key,
with the catalog's `title` as its text fallback.

#### Mob effects

A mod effect is declared in `data/<id>/sunrise/mob_effect/<name>.json` with what vanilla's `MobEffect` takes: its
category (`beneficial`, `harmful` or `neutral`) and its RGB colour (0 to `0xFFFFFF`), which its particles take:

```json
{ "category": "harmful", "color": 9154528 }
```

An optional `sort_order` (an integer) sets where the effect sits among the HUD's icons and the inventory's effect list,
as NeoForge's `IMobEffectExtension.getSortOrder` does: the lists compare it where vanilla compares the colour, and it
is the colour when left out. It is fixed per effect, where NeoForge may answer per instance.

An optional `attribute_modifiers` list is what the effect's constructor would add with
`MobEffect.addAttributeModifier(attribute, id, amount, operation)`:

```json
{
  "category": "harmful",
  "color": 16111426,
  "attribute_modifiers": [
    { "attribute": "minecraft:movement_speed", "id": "ruby:effect/stun_speed", "amount": -1.0, "operation": "add_multiplied_total" },
    { "attribute": "minecraft:jump_strength", "id": "ruby:effect/stun_jump", "amount": -1.0, "operation": "add_multiplied_total" }
  ]
}
```

`attribute` is vanilla's or a mod's attribute and `id` the modifier's id, both `namespace:path`; `operation` is
`add_value`, `add_multiplied_base` or `add_multiplied_total`. One entry per attribute, as vanilla keeps one per
attribute: a second for the same attribute, or an attribute the game does not have, refuses the mod. While the
effect is on an entity, player or mob, each modifier is on it with `amount × (amplifier + 1)`, replaced when the
amplifier changes and removed with the effect, however it ends (expiry, milk, `/effect clear`, death); an entity with
no such attribute gets none. The modifiers are saved and synced as the entity's others are, and a potion's "When
Applied:" lines list them, for the potion's own effects and its `custom_effects`.

It is the effect `<id>:<name>`: `/effect give @s ruby:glow 30` gives it, saves and mobs keep it, and it reaches LAN and
dedicated server players who run the same mods (its network id follows vanilla's). Its name is the translation
`effect.<id>.<name>` from the mod's `assets/<id>/lang/`, its icon `assets/<id>/textures/mob_effect/<name>.png` (18x18,
as vanilla's; without one it draws as the missing texture). The declaration supplies no tick behaviour. A world opened without the mod drops the effect from its players and mobs. Clients join a server
only with the same mods' effects, as with its blocks.

Code mods read a registered type's classification with `effect_key.category(host)`,
which returns `EffectCategory::Beneficial`, `Harmful` or `Neutral`. For an active
instance, use `active_effect.id.category(host)`. The read works on both sides and
returns `None` before the effect registry is complete; it never guesses a category.

#### Entity types

A mod entity type is declared in `data/<id>/sunrise/entity_type/<name>.json`. It copies a vanilla type:
`minecraft:snowball`, a thrown item projectile, or a living mob (below). A thrown type's `item` is one of the mod's own
items (one type per item), and `damage` (0 by default) is what a hit deals to anything it hits, as `thrown` damage from
the thrower (the snowball's 3 to blazes is not copied):

```json
{ "template": "minecraft:snowball", "item": "ruby:ember", "damage": 2.0 }
```

It is the entity type `<id>:<name>`, with its network id after vanilla's. Using its item throws it as a snowball is
thrown: same speed, spread, sound and cooldown, and one item used outside creative. It flies as a snowball, breaks on
what it hits with its item's crumbs, and is drawn as its item's model (`ThrownItemRenderer`), unless a client
mod registers a renderer for it ([entity renderers](client/entity-renderers.md)). Its name is the translation
`entity.<id>.<name>`. It works in single player, on LAN and on dedicated servers with the same mods. Clients join a server only with the same mods' entity types, as with its blocks. It is saved with its
chunk, `/summon <id>:<name>` makes one, and it has its template's row in `entity-types.json`, with `template` naming
it. A world opened without the
mod keeps the saved ones as they are, frozen in place. A dispenser does not throw it.

A code mod can spawn a thrown type at a position with a velocity of its own through
`LevelRef::spawn_entity` with `SpawnBody::Thrown` ([spawning from a level](world/spawning.md#spawning-from-a-level)), or launch one with
`host.spawn_projectile_from_rotation(owner, &entity_type, &stack, offset, power, inaccuracy)`.
This is the owner-based `Projectile.spawnProjectileFromRotation` operation, not a summon:
ownership is set before shooting and the existing launch hook runs after insertion.
It supports registered mod thrown types and vanilla snowballs, eggs, pearls, potions,
experience bottles and wind charges. Spread uses the projectile's own random in x/y/z
order, and known owner movement is inherited (vertical movement only when airborne).
It does not consume an item, award use statistics or apply cooldowns; invalid parameters
or unsupported types return `None` without spawning or drawing random values.
The owner must be in the current level; a guest in another dimension is refused.
The carried item count is one; the supplied stack is unchanged. Once the projectile is
in the level, the stack's enchantments run their `projectile_spawned` effects on it, as
vanilla's `Projectile.applyOnProjectileSpawned` does after a throw: in the stack's order,
each effect's requirements tested with the level random just before it applies, and its
own draws from the projectile's random. `ignite`, `apply_impulse`, `play_sound` and an
`all_of` of them apply; `apply_mob_effect` and `apply_exhaustion` do nothing to a
projectile, as in vanilla. A stack with any other `projectile_spawned` effect (blocks,
damage, explosions, particles, summons, functions, item damage) is refused before spawning, so this operation never silently discards one.
Owner launch also refuses unsupported requirement contexts (including entity flags),
and invalid clamped level-value ranges before insertion. Supported requirements
are entity type predicates, random chance, and their boolean combinations.

A living type may set optional `shadow_radius` and `shadow_strength` next to `template`:
`{ "template": "minecraft:zombie", "shadow_radius": 0.2, "shadow_strength": 0.6 }`.
Each omitted value inherits the template renderer's value. Values must be finite and nonnegative;
admission errors name the file. Radius 0 disables its shadow. Radius follows `minecraft:scale`,
baby age scaling and the template's extra baby factors; cube sizes, pufferfish and dragon retain
their renderer rules. The usual visibility, 32-block radius cap and distance fade still apply.

A living type can set `allowed_in_peaceful` to `false` to prevent summons and spawns
in Peaceful and to have existing mobs of that type despawn there. Its spawn egg
shows the disabled-in-Peaceful line. Set it to `true` to allow the type in Peaceful
without changing the template's other spawn rules or AI. If omitted, the type
inherits its template's setting. Thrown types cannot set this key.

A living type copies a mob, the way NeoForge's `EntityType.Builder.of(Zombie::new, ...)` makes a type of a vanilla
class: `minecraft:zombie`, `minecraft:skeleton`, `minecraft:drowned`, `minecraft:villager`,
`minecraft:wandering_trader`, or one of the tameable mobs
(`TamableAnimal`), `minecraft:wolf`, `minecraft:cat` and `minecraft:parrot`. A tameable type tames with its template's
item, then sits, follows and teleports to its owner, and fights for it, as its template does. Its owner, sitting state
and its template's own fields (a wolf's collar, a cat's or a parrot's variant) save and load under the type's id. A
tamed wolf type's hits are its owner's, as a wolf's are. A parrot type rides a player's shoulder and comes back off
as itself; drawing it there is its mod's (an [entity layer](client/entity-layers.md)), as vanilla draws only its own
parrot there. A skeleton type draws and shoots the bow it holds, keeps out of the sun and flees wolves, and one that holds none
fights in melee; a drowned type goes to water, swims, and throws a trident it holds or fights in melee, as a drowned
does. Every living type is tracked to clients as far, and as often, as its template. It has no `item` or
`damage`. `attributes`
sets default attribute values that differ from its template's (`EntityAttributeCreationEvent`), by attribute id, and
only for attributes the template has; one it lacks stops the content root's build:

```json
{ "template": "minecraft:zombie", "attributes": { "minecraft:max_health": 40.0, "minecraft:movement_speed": 0.3 } }
```

A living type can set `sounds.ambient`, `sounds.hurt`, `sounds.death` and `sounds.step`:

```json
{
  "template": "minecraft:wandering_trader",
  "sounds": {
    "ambient": "bird:entity.vulture.ambient",
    "hurt": "bird:hurt",
    "death": "bird:death",
    "step": "none"
  }
}
```

Each omitted slot inherits the template's existing selection, including runtime choices
such as a villager's trading voice and the supporting block's footstep sound. An explicit
full `namespace:path` id selects that sound event from the resource packs; `"none"` mutes
only that slot. Omission and `"none"` are different: a villager with no step override
plays `minecraft:block.stone.step` on stone. Invalid ids stop content admission,
with the entity and sound slot in the error. These four slots cover ordinary ambient,
hurt, death and footstep playback; additional events such as trading interactions or
projectile attacks keep their existing sounds.

`sounds.step` also takes an object, `{"sound": "<id>", "volume": v, "pitch": p}`: the mob plays its own step at that
volume and pitch, as a Java mob overriding `playStepSound` with `playSound(step, v, p)` does. A plain step id plays at
the block step's volume 0.15 and pitch 1.0. Three numbers take the place of what vanilla's sound methods return for the
template:

```json
{
  "template": "minecraft:zombie",
  "sounds": {
    "step": { "sound": "bird:step", "volume": 1.0, "pitch": 0.9 },
    "volume": 0.6,
    "voice_pitch": 0.8,
    "ambient_interval": 160
  }
}
```

- `volume` is `LivingEntity.getSoundVolume()`, the volume of the ambient, hurt and death sounds.
- `voice_pitch` is the base of `LivingEntity.getVoicePitch()` in place of 1.0: the random spread of ±0.2 still applies,
  and a baby's pitch is still raised by the template's factor (1.5 for most mobs), now times the base.
- `ambient_interval` is `Mob.getAmbientSoundInterval()`, in ticks: after each ambient or hurt sound, and after a
  merchant's trade, the ambient sound's timer starts again from minus this value. Any int is accepted, as vanilla
  accepts any; the arithmetic on it wraps as Java's does.

Each omitted key keeps the template's value. `volume`, `voice_pitch` and the step's `volume` and `pitch` must be
finite as a float; another number, or a step given in another form than an id, `"none"` or that object, stops
content admission, with the file and the key in the error.

A code mod can also use `Registrar::declare_entity_attributes(path, template, attributes)` during registration.
That declaration starts from the vanilla template's attribute row, replacing the content row's overrides.
Declared bases remain available to base-value reads, but computed values and spawn fields clamp them to each
attribute's vanilla range (for example, `minecraft:max_health` is clamped to 1–1024).

To give an existing type, vanilla or a mod's, an attribute it lacks, a code mod calls
`Registrar::modify_entity_attributes(entity, attributes)` (NeoForge's `EntityAttributeModificationEvent`) during
registration. Each attribute takes its base, or with `None` the attribute's own default. It changes the type's
default after every declared set, so entities created after load have it; a later addition of the same attribute
wins. A mod's type needs its declared set first: one that only a content file defines has no attribute row to add
to. An attribute the game does not know, or a type without an attribute row, stops the resource load:

```rust,ignore
registrar.modify_entity_attributes(
    "minecraft:zombie",
    vec![("minecraft:luck".to_string(), Some(2.5))],
)?;
```

It is a mob of its template's class under its own id, `<id>:<name>`: the template's size, model, texture, sounds and
behaviour, with its own attributes. It saves what its template's class saves under its own id (a zombie's `IsBaby`,
`CanBreakDoors` and conversion times) and loads back as itself. Its entity data is its template's class's. Its loot
table is its own, `<id>:entities/<name>` (vanilla's `EntityType.getDefaultLootTable`). It is not in vanilla's entity type
tags (`#minecraft:undead`, `#minecraft:zombies`...) unless the mod adds it. Its row in `entity-types.json` is its
template's, with `template` added and its attributes in place of the template's. To draw a custom model or texture,
a client mod registers a renderer for it ([entity renderers](client/entity-renderers.md)); the content declaration
inherits the template's goals.

Daylight burning follows the type's own id, not its template. To make a type burn in sunlight,
add its id to `data/minecraft/tags/entity_type/burn_in_daylight.json`. Leaving it out does not
make it immune to ordinary fire.

##### Passenger attachment points

An entity type can set `passenger_attachments` to seat offsets relative to its feet.
A list of exactly three plain numbers is **one** `[x, y, z]` point, not three heights.
For multiple seats use nested points, for example `[[1, 2, 3], [-1, 4, 5]]`.
Seats follow passenger order; extra passengers use the last point. Offsets scale with
the entity and rotate with its yaw.

#### Custom attributes

A code mod can also register an attribute type of its own with `Registrar::declare_attribute(path, default,
min, max, syncable, sentiment, tooltip)` (vanilla's `RangedAttribute` in the attribute registry). Only both-side
packages may declare attributes; client-only packages refuse with `WrongSide` so extra
client mods cannot shift packet IDs. `mod_api::Sentiment` is `Positive`, `Negative`, or `Neutral`.
A bare `path` uses the
mod's namespace while an explicit valid `namespace:path` keeps its namespace (built-in and duplicate keys refuse),
the bounds and default follow `RangedAttribute`'s own checks, `syncable` decides whether the
value syncs to clients, and `sentiment` (POSITIVE by default) colors the attribute's tooltip lines. The declared
attributes append after the vanilla ones in the attribute catalog — sorted by declaring mod id, then attribute
id, so the network ids are the same on every side running the same mods — before entity attribute rows resolve,
so a mod's entity set may name its own new attribute. Values sanitize through the definition (NaN maps to the
minimum, everything else clamps), and saves, item modifiers and the sync set carry the attribute by its string
id.

`tooltip` (`mod_api::AttributeTooltip`, `Default::default()` for vanilla's look) sets how the attribute's
modifier lines read on item tooltips, as NeoForge's `IAttributeExtension` does:

- `display: AttributeDisplay::Percentage` (NeoForge's `PercentageAttribute`) shows added values as percentages
  too, so `+0.05` reads `+5% Swim Speed`. `AttributeDisplay::Boolean` (`BooleanAttribute`) shows an added
  value above zero as enabled and any other as disabled (`sunrise.attribute.modifier.enabled` and
  `.disabled`). `Ranged`, the default, is vanilla's.
- `base_modifier: Some("mymod:base_mana")` (`getBaseId`) makes a modifier with that namespaced id read as the
  green total of the player's base value and its amount, as attack damage's base modifier does. The recipe
  search, which has no player, shows it as a plain line. A base modifier that is not a namespaced id refuses
  with `InvalidId`.
- With advanced tooltips (F3+H), a mod attribute's line is followed by its modifier id, as NeoForge's default
  `getDebugInfo` does. There is no per-attribute callback for it.

The display is fixed per attribute type: NeoForge's `toValueComponent` and `toComponent` callbacks, and its
merged attribute tooltips (`getMergedStyle`), have no counterpart.

A code mod can widen the bounds of an attribute that already exists, vanilla's or another mod's, with
`Registrar::widen_attribute_bounds(id, min, max)` (Java mods use a mixin or an access transformer on
`RangedAttribute.minValue` / `maxValue`): an RPG mod lets `minecraft:max_health` go past 1024 with
`widen_attribute_bounds("minecraft:max_health", 1.0, 1e6)`. Every mod's asks join the catalog once, at load, after
the declared attributes: the bounds become their union (the lowest `min`, the highest `max`, never narrower than the
attribute's own), whatever the load order, so an ask narrower than another mod's is a silent no-op. The default never
moves. Every clamp and check of the value reads the catalog, on both sides: the player's and mobs' attributes, the
LAN record check, the sync. Only both-side packages may widen bounds (`WrongSide` otherwise); a non-finite bound
(`NotFinite`) or `min > max` (`InvalidBounds`) refuses at registration, and an attribute the catalog does not have or
a range that leaves the default out refuses the load, naming the mod. A client needs nothing more than the mod-set
check it already passes at join: a client without the mod cannot join. A save keeps a base past vanilla's bound while
the mod is there; once it is gone, the load clamps the base into the bounds and logs it, once per attribute in each
world load.

#### Custom datapack registries

A code mod can declare a datapack registry of its own with `Registrar::declare_datapack_registry(path, sync,
fields)` (NeoForge's `DataPackRegistryEvent.NewRegistry`).
A bare `path` uses the mod's namespace (`spell` of `example` is `example:spell`), while a valid explicit key keeps its
namespace. Built-in and duplicate registry keys are refused. `fields` is the
entry's record schema. Its scalar kinds are `Bool`, `Int { min, max }`, `Float { min, max }`, `Text { max_len }`,
`Identifier` and `Reference { registry }`, and they nest, as vanilla's codecs do:

- `List { of, min_len, max_len }` is a JSON array (`Codec.list`, `sizeLimitedListOf`);
- `Map { key, value }` is a JSON object with `Text`, `Identifier` or `Reference` keys (`Codec.unboundedMap`).
  `as_map` returns a file's pairs in file order, and a default's in its declared order, on the server and on clients. Two keys naming the same id (`zombie` and
  `minecraft:zombie`) are a duplicate. A text map key with `max_len: 0` accepts only the empty key;
- `Record { fields }` is a nested object with the same field rules as the entry's own (a nested
  `RecordCodecBuilder`);
- `Dispatch { key, variants }` is an object whose `key` field (usually `type`) names one of the
  `DatapackVariant`s the schema lists, with that variant's fields beside it (`Codec.dispatch`). The id reads as
  vanilla's `byNameCodec` reads one (`growing` is `minecraft:growing`), and an id no variant declares fails the
  entry. `variant_id` names the variant, and `get` reads its fields. Only the declaring mod lists variants; there
  is no way for another mod to add one;
- Four vanilla codec kinds, decoded by the engine at load (and re-decoded on clients for synced registries) into
  small `Copy` handles you copy out of `level.datapack_read` and evaluate afterwards:
  - `IntProvider { min, max }` (`IntProviders.codec(min, max)`): `as_int_provider()` gives an `IntProviderValue`,
    evaluated with `value.sample(host, random)`;
  - `BlockStateProvider` (`BlockStateProvider.CODEC`): `as_block_state_provider()` gives a
    `BlockStateProviderValue`, evaluated with `value.state(host, level, random, pos)`;
  - `BlockPredicate` (`BlockPredicate.CODEC`): `as_block_predicate()` gives a `BlockPredicateValue`, evaluated
    with `value.test(host, level, pos)`;
  - `HolderSet { registry }` (`RegistryCodecs.homogeneousList`, `minecraft:block` only; any other registry is
    refused at registration): `as_holder_set()` gives a `HolderSetValue`, queried with `value.contains(host, block)`.
  Handles carry the load's generation number: after `/reload` (or a client snapshot swap), evaluating an older
  handle returns `Err(DatapackValueError::Stale)` instead of panicking.

Each field, at any depth, is required (`DatapackField::required`), given a default (`with_default`) or optional with
no default (`DatapackField::optional`, `optionalFieldOf`), which reads as `None` when left out. An optional kind
stands only for a whole field, never inside a list, a map or another optional. Defaults are checked against their
kinds at every depth, and an id or reference default is normalised wherever it sits (`stone` is `minecraft:stone`,
map keys included).
A list has no per-list count limit unless its `max_len` sets one. Local loading does not impose network byte or
collection limits; the declared schema and the JSON parser's nesting limit still apply. Read nested values with `as_list`, `as_map`, `get` and the scalar accessors (`as_int`, `as_text`...):

```rust,ignore
let int = |min, max| DatapackFieldKind::Int { min, max };
r.declare_datapack_registry("model", DatapackRegistrySync::Required, vec![
    DatapackField::required("entity", DatapackFieldKind::Identifier),
    DatapackField::required("drops", DatapackFieldKind::List {
        of: Box::new(DatapackFieldKind::Record { fields: vec![
            DatapackField::required("item", DatapackFieldKind::Identifier),
            DatapackField::with_default("count", int(1, 64), DatapackValue::Int(1)),
        ] }),
        min_len: 1,
        max_len: None,
    }),
    DatapackField::with_default("tiers", DatapackFieldKind::Map {
        key: DatapackKeyKind::Text { max_len: 16 },
        value: Box::new(DatapackFieldKind::Record { fields: vec![
            DatapackField::required("data_per_kill", int(0, 1000)),
        ] }),
    }, DatapackValue::Map(Vec::new())),
    DatapackField::optional("color", DatapackFieldKind::Text { max_len: 7 }),
])?;
// Later, from an entry's values: the count of its second drop, the data a superior kill gives.
let count = entry.get("drops").and_then(|d| d.as_list()?.get(1)?.get("count")?.as_int());
let data = entry.get("tiers").and_then(|t| t.get("superior")?.get("data_per_kill")?.as_int());
```

Entry `addon:fire` is `data/addon/example/spell/fire.json`
(`DatapackRegistryDeclaration::entry_file`), the registry's namespace being a folder as NeoForge's modded
registries have it. The game decodes entries against the schema without running mod code: an undeclared key, a missing required key, a wrong type, an out-of-range value or a list
of the wrong length fails the entry, at any depth, and a default never stands in for invalid data. The error names
the value's path (`drops[3].item`, `tiers.basic.data_per_kill`); a key with a character other than letters, digits,
`_`, `:`, `/` and `-` is quoted as the file spells it (`tiers["basic.data_per_kill"]`). A reference resolves against
the whole candidate set wherever it sits. A feature type's config (`register_feature_type`) uses the same scalar kinds
but stays a flat record: a nested or `Reference` config field is refused at registration.

The game selects the winning resource from mod content and enabled data packs before it reads the file or
evaluates [its conditions](items/conditional-data.md#registry-conditions). If two registry directories contain
that path, each interprets it relative to its own directory. A false condition omits the winner, without a
lower-pack fallback. An unreadable winner refuses the load and names its pack and file. After conditions pass,
the game decodes the entry against `fields`. When a resource has `neoforge:conditions`, write its schema object
inside `neoforge:value` or write its fields beside the conditions array. In the second form the outer
`neoforge:conditions` key is not a schema field. Tags resolve against admitted entries.

Client-only packages cannot declare datapack registries. A `side = "server"` package can declare
`DatapackRegistrySync::ServerOnly` (entries and tags are never sent) or `DatapackRegistrySync::Optional`
(entries and tags go only to clients that offer the same registry and schema). Neither declaration by itself
requires the client to install the mod; the package must still meet the join-content rule for everything else
it declares. Use `DatapackRegistrySync::Required` with `side = "both"` when joining clients must have the
registry. A server-only package declaring a `Required` registry is refused at load, naming the registry. A both-side
declarer using a server-only `served_by` namespace for a required registry gets `WrongSide` at registration.

A client's offer lists only synchronized registries and their schemas. It must fit one vanilla
serverbound custom payload (32,767 bytes); an oversized offer stops the join rather than acting as an empty
offer. The server encodes only the registries negotiated for that client. Each fragment, including its header,
fits vanilla's clientbound custom payload limit (1 MiB); the combined encoded snapshot can be up to 128 MiB
(NeoForge's default split-packet limit). There is no separate registry, entry, tag or tag-member count limit.
After a successful local reload commits, a connection that negotiated a snapshot receives it before the reload's
mod messages. If the snapshot cannot be sent or validated, only that connection ends. The client never activates
a partial snapshot, and the server keeps the new local generation.

Use `level.visit_datapack_entries(host, registry, &mut visit)` to borrow entry IDs in
lexical order. The callback takes an `&str` and returns `ControlFlow::Continue(())` to
keep visiting or `ControlFlow::Break(())` to stop. Copy an ID only if you need to keep it.
Use `level.datapack_read(host, registry, entry_id, &mut visit)` to borrow one published
entry through a callback. For example, keep only the `power` number:

```rust,ignore
let mut power = None;
level.datapack_read(host, "example:spell", "addon:fire", &mut |entry| {
    if let Some(mod_api::DatapackValue::Int(value)) = entry.get("power") {
        power = Some(*value);
    }
})?;
```

Read `entry.values` in schema order or use `entry.get(name)` by field name. The active registry
set stays pinned until the visitor returns, with no registry lock held and no disk access. Keep
IDs to look up again, or copy only the fields you need to retain as a snapshot.

The visitor runs once on success, never on error. On the logical server, both the active
dimension and any loaded parked server dimension are accepted (matching vanilla
`Level.registryAccess()`); stale epochs and unloaded dimensions are refused. Unknown or
unavailable registries, stale or unloaded levels, and a missing published set give
`UnknownRegistry`; a registry without the entry gives `MissingEntry`. A host without a backend
returns `Unavailable`. The ID visit returns `Ok(Break(()))` on early stop or `Ok(Continue(()))`
on completion, including for an empty registry. Once a client receives a valid snapshot at join
or reload, it reads those entries; any of its declared registries omitted from the snapshot have
no entries there. Resolve `host.level(event.dimension)` in `client_level_load` or
`client_level_unload` to read the available synchronized registries through the entering or
retiring level. Unload still sees its old configuration during a server reconfiguration; the
next Load sees the new set. A retiring `LevelRef` is stale after Unload.

#### Block entity containers

A code mod declares a block entity's `ModContainer` with `Registrar::block_container`.
Registration refuses more than 256 slots because the saved `Items` list uses an unsigned byte for
each slot index (vanilla's `ItemStackWithSlot.CODEC`). Content files can declare menu layouts, not
saved block container storage. The container's `faces` say which slots each side reaches; `inputs`
lists the slots insertion may use, and `outputs` the slots items may leave through a side.
Per-slot `insert_rules` provide static admission, with missing entries treated as `Any`.

Registration validates the declared shape: `slots` must be at most 256, every index in
`faces`, `inputs` and `outputs` must be strictly below `slots`, and `insert_rules.len()` must be at
most `slots`. Malformed declarations are refused at registration rather than clamped or filtered.
An optional `menu` opens only when the menu type is supported and its slot count matches `slots`;
an unsupported or mismatched menu is ignored without changing `slots`. Structure templates and chunk saves both load these entries against the placed
block's declared container shape.

For state-dependent admission, implement `BlockEntityPart::can_place_item`
(vanilla's `Container.canPlaceItem`). Its `CanPlaceItemEvent` gives the server part the block
entity's own `pos` and current block `state`, its saved data and slots, the queried slot index,
and the offered stack. Parts shared by several blocks can branch on `state.block()`. To filter
by a property without a Host in this query, resolve matching `BlockState` values in a hook with
block metadata and keep them for comparison here. Returning
`Some(true)` or `Some(false)` replaces the declared rule for that slot; `None` keeps it. Face
and input-slot rules and stack limits still apply. Hoppers and hopper minecarts, droppers,
default `ITEMS` and `STORAGE` insertion, and nonempty offers from `/loot insert` and
`/loot replace block` use this query. `/item replace block` keeps its direct slot-write behavior.
The default `STORAGE.space_for` does not ask the part: for subscribed slots it counts
physical room after face and input checks. An actual insert asks with its remaining offer
before applying the stack limit.

For state-dependent extraction, implement `BlockEntityPart::can_take_item_through_face`
(vanilla's `WorldlyContainer.canTakeItemThroughFace`). Its `CanTakeItemThroughFaceEvent` gives the
same entity, `pos` and `state`, the queried slot, the stack in it and the `face` it would leave
through. Returning `Some(true)` or `Some(false)` replaces whether `outputs` lists the slot; `None`
keeps it. The face's slots still bound which slots are asked. A hopper pulling from below and
`ITEMS` and `STORAGE` extracts through a side use this query; an extract with no side skips the
face rules, as NeoForge's unsided item handler does, and menus use `menu_slot_rules`.

`can_place_item` has no Host; `can_take_item_through_face` gets a read-only one. Either query
may run for a slot that is never written. Seed a filter from the
accepted write, not from the query. Default `ITEMS` and `STORAGE` writes to these slots emit
`contents_changed` once per backing block on commit. During a handled `menu_quick_move`, the notice
waits for the clicked source to settle and only runs if the original backing container is still there.
If the hook declines or its reply is refused, its tentative writes emit no notice. Ordinary click routing,
if it runs, can still emit notices for its own writes. A write to the same block from its active notice
still applies without a second notice. If your provider
writes directly to storage it owns, issue `set_changed` explicitly or seed the filter in the
provider. Menu clicks and quick-move still use the declared rules and `menu_slot_rules`
(vanilla's `Slot.mayPlace`); they do not ask the block entity part.

#### Menu types

A mod menu type (vanilla's `MenuType`) is declared in `data/<id>/sunrise/menu/<name>.json`: its layout's slot count,
the data slots its menu syncs (`ContainerData`, 0 by default), and the slots players only take from
(`results`, as a furnace's result slot). The layout and any appended player inventory can total up to
32,768 slots (indices 0 to 32,767 in vanilla's signed-short container packets). A menu type does not
allocate a block's saved item slots.

```json
{ "slots": 9, "data_slots": 2, "results": [7, 8], "inputs": [0, 1] }
```

A status or control screen that has no item slots at all can omit the player inventory group too:

```json
{ "slots": 0, "player_inventory": false, "data_slots": 2 }
```

`player_inventory` defaults to `true`. Result, crafting, linked-slot and recipe-book mappings name declared
container slots, never the optional player group. A mapping to an absent slot is refused at load.
The `data_slots` count is separate from `slots`. Larger declarations can be stored on the server,
but each sync packet retains its index range: 0 through 32,767 for a block-backed menu and
0 through 65,535 for the negotiated blockless payload.

For blockless opens, an optional `owned_slots` array declares [transient menu storage](../concepts/capabilities/menu-slots.md#menu-owned-slots).
It must contain exactly `slots` entries, each the maximum stack size (1–99) of that slot.
Types declaring `owned_slots` are blockless-only and cannot declare `crafting`; they may declare `results` for take-only output slots whose non-result inputs return to the player on close.
A block container cannot bind an owned-slot type as its menu.

The optional menu `inputs` list marks machine feeds, including fuel, as `Input`
in [`MenuRef::slot_kinds`](../concepts/capabilities/menu-slots.md). Indices start at zero and
must be unique and below `slots`; they cannot overlap `results`, the crafting grid,
or its result slot. Both linked and unlinked slots can be inputs. Omit the list (or
use `[]`) to leave ordinary slots as `Storage`. The role does not change which items
a slot accepts or how clicks and item transfers work.

It is the menu type `<id>:<name>`, with its network id after vanilla's. A mod block's container opens it when its
`menu` names it with as many slots ([native mods](native-mods.md), `r.block_container(&block, container)`). The menu holds
its slots, then the player's 36 if `player_inventory` is on. A shift-click from the inventory goes to the first slots with room, never a result
slot, and one on the container goes to the inventory, as a chest's does. `results` fence players only: hoppers follow the
block entity container's faces and insertion rules, not the menu's `inputs` list. A block-backed menu syncs the block entity's
`data` (16-bit on the network, as vanilla's). A [blockless menu](world/machine-menu-data.md#blockless-status-menus)
gets its initial data from its opener and syncs subsequent signed `i32` changes without reopening. A Sunrise client shows it through the
mod's [registered screen factory](world/machine-menu-data.md#the-menus-screen). If the factory is missing,
the client warns and keeps its current menu. `player.open_mod_menu` can open the same type without a block.
A recipe-book binding applies to either opening; the screen factory supplies its geometry and book.
Clients join a server only with the same mods' menu
types, as with its blocks, and a client that isn't Sunrise's is never sent one: the block opens nothing for it. Its
clicks run on the server's vanilla menu engine (`menu::Kind::Mod`), shift-clicks routed as `ChestMenu`'s, and buttons
reach the type's `MenuPart::menu_button` on both block-backed and blockless openings. A native mod can decide what a
slot takes in code with `menu_slot_rules` ([hook notes](https://github.com/sunrise-org/sunrise/blob/main/crates/sunrise-mod-vocab/src/hooks/menu_slot_rules.rs))
and whether the menu stays open, with or without a backing block, with [`menu_still_valid`](../concepts/capabilities/menu-slots.md#keeping-a-mod-menu-open).
The declaration specifies per-slot rules through `results`; clicks are handled by the server.

##### Lending ingredients and linking containers

For a mod crafting menu, a server `recipe_sources` listener can lend the chest
beside its machine to recipe-book placement and transfer checks:

```rust,ignore
#[mod_api::sunrise_mod]
impl mod_api::Hooks for Machine {
    fn recipe_sources(
        &self,
        _host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RecipeSourcesEvent<'_>,
    ) {
        if event.menu == "machine:crafting" {
            let [x, y, z] = event.pos;
            event.sources.push([x + 1, y, z]);
        }
    }
}
```

`player` is the requesting player's UUID; their state is not swapped in. The
engine uses the player's inventory first, then answered containers in mod/push
order. It silently skips the open machine itself, duplicate positions (first
wins), and missing or non-lendable containers. There is no container-count cap.
Chest-like block entities (including barrels, shulkers, hoppers,
dispensers and droppers) and mod containers' declared slots can lend. No answer
keeps vanilla placement. The engine owns ingredient removal, container
write-back and `container_changed` notifications, not the listener.

`menu_links` has the same input fields and an append-only `links` output for menu
types declaring linked slots. It silently skips `pos`, duplicate positions, and missing
or non-lendable containers, preserving first-accepted answer order with no container-count
cap. It resolves distinct live lendable storage and refreshes the named containers' displayed items
in the same server tick. `recipe_sources` is asked per player action, never per
tick; `menu_links` is asked per open linked mod menu each tick, only when a mod
subscribes. With no listener its tick path does no work or allocation. Both hooks
are public server collect hooks.

#### Namespaces and limits

The fence: files go in the mod's own namespaces (`assets/<id>/`,
`data/<id>/`, and those its manifest lists), plus additions to vanilla tags
(`data/minecraft/tags/`), to the convention tags NeoForge and Fabric mods
share (`data/c/tags/` and `data/neoforge/tags/`), and sources added to vanilla's atlases
(`assets/minecraft/atlases/`), which follow vanilla's own as a reload appends them.
Packages of every kind may also add to the declared [`sunrise:` extension tags](extension-tags.md),
by writing exactly the listed registry and tag path under `data/sunrise/tags/`.
Other `data/sunrise/` and all `assets/sunrise/` files remain refused; the namespace cannot be claimed.
Generated `sunrise:class/*` tags are refused by name: attach a part with `block_part_tag` instead.
The SDK surface's `extension_tags` lists these same declarations, and `sdk diff` treats a removed tag as breaking.
Templates and `sounds` name vanilla blocks; unknown keys are refused.

Some Java mods ship their files under other names than their mod id (Towns and Towers, mod id `t_and_t`, under
`kaisyn` and `towns_and_towers`). A content mod lists those in its manifest:

```toml
id = "t_and_t"
namespaces = ["kaisyn", "towns_and_towers"]
```

A listed namespace is the mod's as its id is: its `assets/` and `data/` files load, and the game reads its tags,
recipes, names and loot. Blocks, items, creative tabs, mob effects, entity types and menu types may be declared in
`data/<owned_namespace>/sunrise/`. This file rule does not restrict the IDs used for code-declared
blocks, items, and block entity types. For example, a package with ID `ported_farmersdelight` can
register a code-declared `farmersdelight:cooking_pot` without changing its saved-world ID;
its `farmersdelight:` asset and data files still require `namespaces = ["farmersdelight"]`.
`minecraft`, `c` and `sunrise` cannot be listed. When two packages list the same extra namespace, the first
admitted package owns it; the game leaves the later package's content out and logs the conflict. A package's
primary namespace is its ID. Listing another package's ID as an extra namespace is a load error, regardless of
load order.

Paths use vanilla's resource characters, at most 65,536 files, 16 MiB each and 512 MiB in all. A mod whose
content is refused is left out, with the reason in the log. The game reads this content into its block and item
tables (`sunrise_sim::mod_content`, [modpacks.md](https://github.com/sunrise-org/sunrise/blob/main/docs/modpacks.md)). A tag entry
naming an id nobody registers (a block a port left out, another mod's item) is dropped, so the rest of the tag
loads; vanilla would drop the whole tag. In a block or item tag, an entry naming a `#minecraft:…` tag that neither
vanilla nor an installed mod defines is also dropped. The warning names the entry and its file; an optional
entry (`"required": false`) is dropped silently. If the base has no tag folder for that registry, the reference is
left in place.

#### Tags and enchantments

Tags merge as a data pack's do. Every mod's file for one tag (vanilla's `data/minecraft/tags/`, the convention
`data/c/tags/` and `data/neoforge/tags/`, or another mod's) adds its values after vanilla's and the mods' before it, in load order. A file with
`"replace": true` first drops what came before it, vanilla's entries and the earlier mods', and the mods after it
add theirs, as a higher data pack does. Since the result depends on load order, the game says so in the log:
"mod towns replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json: vanilla's entries and those
of adds are left out, ...". Only tags and `assets/minecraft/atlases/` files may use the `minecraft` namespace; other content stays in owned namespaces.

**Connecting a block to fences, walls or panes**

To make a narrow post connect to oak and other wooden fences, add its id to both
`data/minecraft/tags/block/fences.json` and `data/minecraft/tags/block/wooden_fences.json`:

```json
{ "values": ["mymod:rope_post"] }
```

Use only `fences.json` to join nether brick fences. A fence checks that the neighbour is in
`minecraft:fences` and that both blocks agree on membership in `minecraft:wooden_fences`.
A sturdy face toward the fence can also connect, subject to vanilla's connection exceptions.
Fence gates use their own orientation rule.

Use `data/minecraft/tags/block/walls.json` for a narrow block that should join walls, iron bars
and glass panes. The tags tell vanilla neighbours to draw their arms. Give the custom block
its own model and shapes for any arms it needs to draw.

Enchantments use vanilla files at `data/<namespace>/enchantment/<name>.json` (subfolders keep their path in the id,
`<namespace>:<sub>/<name>`), with the mod's own tags in `data/<namespace>/tags/enchantment/` and its English names
(`enchantment.<namespace>.<name>`) in `assets/<namespace>/lang/en_us.json`. They join vanilla's in the enchanting
table, the anvil, loot, commands and the registry a server sends its players, whose data no client knows, so it is
always sent. An enchantment reaches the enchanting table through vanilla's tags (`data/minecraft/tags/enchantment/
in_enchanting_table.json` and the others), which a mod adds to as any tag. A world's data pack replaces a mod's
enchantment of the same id. A native mod can register a codec for a custom value or entity effect kind, then use
that kind in a vanilla effect component in the enchantment definition. See
[enchantment effect kinds](items.md#custom-enchantment-effect-kinds).

#### Worldgen additions, biome modifiers and custom biomes

A mod adds to structure sets, template pools and processor lists that are not its own (vanilla's villages, another
mod's houses) with a set, pool or processor list file in its own namespace holding `sunrise:add_to`, a key vanilla data
never has. Its entries are appended to each set, pool or list named, after every mod's own files, by
`sunrise:priority` (lower first, 1000 when it has none), then in mod and file order; the file is no set, pool or list
itself. A name nobody defines is skipped.

```json
{ "sunrise:add_to": "minecraft:villages",
  "structures": [ { "structure": "mymod:village_beach", "weight": 10 } ] }
```

A pool addition holds `"elements"` as a pool does, a processor list addition `"processors"`. This is what
Lithostitched's `add_structure_set_entries`, `add_template_pool_elements` and `add_processor_list_processors` modifiers
do on NeoForge and Fabric, and `sunrise-port-pack carry` writes the last two from those.

A mod's own configured and placed features (`data/<ns>/worldgen/feature/`, `placed_feature/`) generate in
biomes it does not own (vanilla's) through NeoForge's biome modifiers, files in `data/<ns>/neoforge/biome_modifier/`
(older Forge mods: `data/<ns>/forge/biome_modifier/`, `forge:` types). Pack precedence picks
one effective file; a code declaration of its registry key is refused, not applied a second time.
`add_features` appends its `features` to one
generation `step` of every biome in `biomes`; `remove_features` takes them out of its `steps` (every step when it lists
none); `none` does nothing. `biomes` and `features` are an id, a `#tag` or a list of ids; `biomes` may also be
NeoForge's `any`, `or`, `and` or `not`. Every addition comes before every removal, each in mod and file order, and a
feature already in the step is not added twice. Configured features use 26.3's `worldgen/feature/` directory
(registry `minecraft:worldgen/feature`).

Spawn modifiers also apply to those biomes: `add_spawns` appends `spawners` (one object or a list with `type`,
`weight`, `minCount` and `maxCount`) to each entity type's category, converting the count range to 26.3's `count`;
`remove_spawns` removes `entity_types` from every category. `add_spawn_costs` sets `spawn_costs` from `spawn_cost`
(`energy_budget` and `charge`) for its `entity_types`, and `remove_spawn_costs` removes them. Entity type selectors
accept an id, `#tag`, list of ids, or NeoForge's `any`, `or`, `and` or `not`.

Carver modifiers edit the biome's `carvers`: `add_carvers` appends `carvers` (an id or a list of ids) that it lacks, and
`remove_carvers` takes them out. They read the root's `worldgen/carvers.json` for the ids that exist, so a carver a
data pack adds is not known to them, and carver tags are not read. NeoForge's docs call these legacy carvers: noise
caves are not carvers.

NeoForge calls a modifier once in each of five phases (`BEFORE_EVERYTHING`, `ADD`, `REMOVE`, `MODIFY`,
`AFTER_EVERYTHING`, in that order; its public biome modifier documentation). Sunrise applies every addition, then every
removal, then every `MODIFY` modifier, in mod order within each. NeoForge's own modifier types only add and remove;
`MODIFY` is for the single values (climate, colours) that a mod's own modifier type changes, so Sunrise's is its own
type, `sunrise:modify_biome`, which sets any of `temperature`, `downfall`, `has_precipitation`, `temperature_modifier`
(`none` or `frozen`) and `effects` colours (`sky_color`, `fog_color`, `water_color`, `water_fog_color`,
`foliage_color`, `dry_foliage_color`, `grass_color`), with vanilla's biome JSON names, on its `biomes`:

```json
{
  "type": "sunrise:modify_biome",
  "biomes": "#minecraft:is_forest",
  "temperature": 0.3,
  "has_precipitation": true,
  "effects": { "sky_color": 12632319, "grass_color": "#80a0ff" }
}
```

What the file leaves out stays. A colour is an integer 0 to 0xFFFFFF or `#rrggbb`; the biome keeps `#rrggbb`, with the
sky, fog and water fog colours as its `minecraft:visual/...` attributes, where 26.3 keeps them. A value of the wrong
type, a colour out of range, an unknown colour name or a `temperature_modifier` other than `none` or `frozen` skips that
modifier with a warning naming the mod and file. `BEFORE_EVERYTHING` and `AFTER_EVERYTHING` have no modifier type
here, so the API names no phase for them.

A `null` for `foliage_color`, `dry_foliage_color` or `grass_color` removes that override from the biome's `effects`, so the
colormap tints it again; removing one the biome lacks changes nothing. The other colours are required or are attributes,
so a `null` there skips the modifier with a warning.

In code, these three optional colour overrides are `Option<BiomeColourChange>`:
`Some(BiomeColourChange::Set { rgb })` sets one, `Some(BiomeColourChange::Remove)`
removes one, and `None` preserves it.

A modifier also sets the biome's ambient and music attributes, in an `attributes` object with the vanilla keys and
values of a biome file (`warped_forest.json` has all of them). Each key present replaces the biome's whole value;
the keys it leaves out stay:

```json
{
  "type": "sunrise:modify_biome",
  "biomes": "minecraft:plains",
  "attributes": {
    "minecraft:visual/ambient_particles": [{ "particle": { "type": "minecraft:ash" }, "probability": 0.25 }],
    "minecraft:audio/ambient_sounds": {
      "loop": "minecraft:ambient.warped_forest.loop",
      "additions": { "sound": "minecraft:ambient.warped_forest.additions", "tick_chance": 0.0111 },
      "mood": { "sound": "minecraft:ambient.warped_forest.mood", "tick_delay": 6000, "block_search_extent": 8, "offset": 2.0 }
    },
    "minecraft:audio/background_music": { "default": { "sound": { "sound_id": "zinc:music.dunes" }, "min_delay": 12000, "max_delay": 24000 } },
    "minecraft:audio/music_volume": 0.5
  }
}
```

The particle list also accepts `{ "argument": [...] }`.

| Value | Validation |
|---|---|
| Particle | Namespaced ID |
| Sound | Registered vanilla sound ID string, or inline `{ "sound_id": "<ns>:<path>" }` (optional `"range"`) for any sound including custom mod sounds |
| Particle `probability` | 0 to 1 |
| Music volume | 0 to 1, or a float environment-attribute modifier object (e.g. `{ "modifier": "multiply", "argument": 0.5 }`) |
| Ambient additions | One object or a list; any finite `tick_chance` |
| Ambient mood | Requires sound, `tick_delay`, `block_search_extent` and `offset` |
| Music `default`, `creative`, `underwater` | Requires sound and nonnegative `min_delay` and `max_delay`; delays need not be ordered |

 Any other attribute, or a value that is not valid, skips the modifier with a warning naming the mod,
file and attribute. A client plays these from the biome data the server sends. In code, `ModifyBiome` has
`ambient_particles`, `ambient_loop`, `ambient_additions`, `ambient_mood`, `background_music` and `music_volume`; the
three ambient sound fields together make the one `ambient_sounds` value, and a value out of range refuses the mods at
startup.

**When they apply.** Modifiers are applied while the world's worldgen data is loaded, before any chunk is generated: the content build writes the modified biomes into the content root, and when a world opens, `sunrise_sim::vanillagen::biome_modifiers_over_packs` applies them again over the world's data packs (`shared_data_with` for the generator, `packed_biomes` for the game's biome table), so a pack that replaces a vanilla biome keeps the mods' changes and a pack's own biome a tag selects gets them. Modifiers a mod declares in code (`Registrar::declare_biome_modifier`) register before any world opens and are read at the same moment. The generator's data is loaded once per bundle and set of packs and shared by the world's dimensions, so a modifier never applies halfway through a world. The dedicated server is the same game run headless (`sunrise-client --dedicated`, [server.md](https://github.com/sunrise-org/sunrise/blob/main/docs/server.md)), so it takes this path too; `sunrise-server` only supervises it.

A biome holder set can also use Sunrise's own `sunrise:climate` type, alone or nested in those boolean sets; it is not
a NeoForge format. It filters each biome's base `temperature` and `downfall` (the names in
[Fabric's public biome weather API](https://maven.fabricmc.net/docs/yarn-1.21-rc1+build.1/net/minecraft/world/biome/Biome.Weather.html)),
not its positional temperature. Each range takes `min`, `max`, or both, inclusive; an omitted bound is unbounded. At
least one field and one bound per range are required. Unknown keys, non-numbers and inverted bounds stop the content
load. Climate sets apply only to `biomes`, not `features`.

```json
{ "type": "neoforge:and", "values": ["#minecraft:is_overworld",
  { "type": "sunrise:climate", "temperature": { "min": 0.8 }, "downfall": { "max": 0.5 } }] }
```

```json
{ "type": "neoforge:add_features", "biomes": "#minecraft:is_overworld",
  "features": "zinc:ore_zinc", "step": "underground_ores" }
```

A code mod declares the same modifiers without a file: `Registrar::declare_biome_modifier(path, modifier)` takes a
`BiomeModifier` (its `biomes`, and an operation: `AddFeatures`, `RemoveFeatures`, `AddSpawns`, `RemoveSpawns`,
`AddSpawnCosts`, `RemoveSpawnCosts`, `AddCarvers`, `RemoveCarvers` or `ModifyBiome`, whose `Option` fields are the
values above). `BiomeModifier::json` renders it as the file above, so a declared modifier and
a shipped one are applied by the same code, in the same order (additions first, then removals, then modifications, mods in dispatch
order, declarations in registration order). A bare path uses the declaring mod's namespace, while an explicit valid
`namespace:path` keeps its namespace. A malformed id, a repeated id, an empty list, a name that is not namespaced, an
unknown generation step, a spawner of weight 0 or with its smallest group above its largest, a spawn cost that is not
finite, a climate number that is not finite, a colour above 0xFFFFFF and a temperature modifier other than `none` or
`frozen` refuse the mods at startup, naming the mod.
Declared modifiers apply when the world loads, through the same content loader as file modifiers, to both the
generator and the game's biome table, including when no data pack is enabled.

A modifier changes a biome that already generates. To add a mod's own biomes to the Overworld or the Nether,
declare a [biome region](content-mods/biome-regions.md) instead.

The modifiers rewrite the biomes when the content root builds, and the generator orders every biome's features as
vanilla's `FeatureSorter` does, so an added feature takes its place in that order (its random seed follows from it). A
biome no modifier names keeps its text, and a world without mods keeps vanilla's biomes byte for byte. Built-in `c:`
and `neoforge:` convention tags resolve automatically beneath vanilla, pack, and mod tag files. A modifier
naming a biome, biome tag, feature, feature tag, entity type or entity type tag the game doesn't have (such as another mod's), or of an unsupported type (a mod's own serializer), is skipped with a warning
naming the mod and file; the mods load. A malformed modifier of a type
that is read (an unknown step, a missing field, `minCount > maxCount`) stops the build, and the game starts without
content mods ("content mods not loaded: ..." in the log). Feature selectors read vanilla feature tags, not a mod's `placed_feature` tags. Modifiers are reapplied after the world's data packs, so a pack
that replaces a biome keeps the mods' changes too.

**A biome of your own.** A mod's `data/<ns>/worldgen/biome/<path>.json` (any namespace but `minecraft`) adds a biome.
The game loads it into its biome table after vanilla's biomes, sorted by path and then namespace, so vanilla's biome ids
never move. The content build puts it in the content root's biome bundle as `<ns>:<path>`, after vanilla's biomes and
in namespace then path order; a file in the `minecraft` namespace is ignored, since mods cannot replace vanilla's
biomes. Biome tags keep the mod's ids, biome modifiers apply over vanilla's biomes and the mods' together (a modifier
reaches a mod biome through its tag and never drops it), and `/locate biome` and `/place` suggest it. A joining client receives it in the biome registry with its data even when it has the vanilla pack, builds
it from that data, and plays its colours, ambient particles, ambient sounds and music from it; a client with the same
mod already has it. Its network id is its place in the registry sorted by path, then namespace, as in vanilla.
Saved worlds name a column's biomes, not their ids: a world saved before the mod existed holds only vanilla names and
loads unchanged; a world saved with the mod and opened without it reads the mod's biomes as plains, and the next save
writes plains. A mod biome appears in the world only where a dimension's biome source or a worldgen file uses it.

A pool element may be one of Lithostitched's: `lithostitched:limited` (`delegate`, `limit`, `min_depth`) places its
`delegate` at most `limit` times in one structure start, and `lithostitched:guaranteed` (`delegate`, `count`,
`min_depth`) is tried before its pool's other elements at each jigsaw that reaches that pool (as target or fallback)
until `count` are placed. Neither goes below a piece shallower than `min_depth` (the start is at depth 0).

A structure (and any of its pool elements, in place of the structure's) may hold
`sunrise:enhanced_terrain_adaptation`: MoogsStructureLib's terrain kernel, which carves or buries terrain around the
pieces on top of vanilla's `terrain_adaptation`. `kernel_size` and `kernel_distance` shape the kernel; `top` and
`bottom` (`carve`, `bury` or `none`) act above and below a piece's floor; `bottom_offset`, `padding` (`x`, `top`,
`bottom`, `z`) and MoogsStructureLib's `band` (`bottom`, `top`, `piece_heights`) move the box it measures from. On a
structure, `rigid_junctions` adds rigid pieces' junctions as well and `inflate` grows the start's box by the kernel
radius. `kernel_size` 0 is none.

```json
{ "sunrise:enhanced_terrain_adaptation": { "kernel_size": 24, "kernel_distance": 24,
    "top": "carve", "bottom": "none", "rigid_junctions": true, "inflate": true } }
```

#### NeoForge model loaders and the root transform

Block and item model files read NeoForge's built-in loaders `neoforge:composite`, `neoforge:obj` and
`neoforge:empty`. `"loader": "neoforge:empty"` is a model with no quads: its other keys, `parent` and
`textures` included, are ignored, and a model inheriting from it has no geometry either.

A model's `transform` (NeoForge's root transform) moves its elements, its composite children and its OBJ faces
before the blockstate's rotation. It is `"identity"`, a 3x4 row-major matrix (bare or as `{"matrix": ...}`), or an
object of `translation`, `rotation` (or `left_rotation`), `scale` (one number or three), `right_rotation` (or
`post-rotation`) and `origin` (`"center"`, `"corner"`, `"opposing-corner"`, the default, or three numbers). A
rotation is one `{"x": degrees}` axis object, a list of them, three XYZ degrees or a quaternion. The model closest
to the leaf that names a `transform` wins, so a child's `"identity"` removes its parent's. A composite's transform
applies after each child's own. A registered loader's quads don't get it: the loader reads `transform` from
`input.config` itself.

```json
{ "parent": "mymod:block/pipe", "transform": { "scale": 0.5, "origin": "center" } }
```

#### Custom block-state definition types

Register a definition provider with `Hooks::register_block_state_models` before client resources
load (`event.register(id, provider)`). Set `neoforge:definition_type` to that id in the
blockstate JSON; the bound provider runs directly on every load and reload. Read the whole
file from `input.config`, the block identifier from `input.block`, and enumerate `input.states`
in registry order. Return `Ok(picks)` with a `BlockStateModelPick::Model` for each state you
choose (its index, model id, quarter turns and UV-lock setting), or `Err(reason)` to reject a
file. Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted, and a
later registration for the same id replaces an earlier one. See the generated hook notes for a
powered-block example.

A state not picked inherits the lower pack layer's selection when present;
otherwise it draws no model. Pack layers bind independently. An unknown type,
explicit refusal, invalid model, out-of-range state, duplicate state or answer
with no picks rejects the whole layer with a diagnostic, leaving lower layers
available. Vanilla `variants` and `multipart` remain available for ordinary files.
