<!-- chapter: Dimension shape -->
<!-- separator: 1 -->

### Dimension keys

`LevelRef::dimension` returns a `DimensionKey`, a small `Copy` value that names the dimension
for as long as the game runs, custom dimensions included. Compare it with another key or with
`DimensionKey::OVERWORLD`, `NETHER` and `END`; `level.dimension().id(host)` gives the name
(`minecraft:the_nether`). On a client, `host.level(name)` gives the level it shows under the
name the server calls it, whether or not that is a vanilla dimension.

### Dimension shape

`LevelRef::dimension_shape` reads the loaded server level's build range, logical height,
skylight and ceiling from its dimension shape. It accepts `&dyn Host`, including when the
level is parked, and returns `None` for a stale or unavailable level or on a client. It does
not load a level or allocate.

### Registering a dimension

`register_dimension` gives every world a level of the mod's own. Call it once, at startup, in
`Hooks::register`:

```rust,ignore
#[mod_api::horizon_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_dimension(
            "pocket",
            DimensionSettings { min_y: -128, height: 448, logical_height: 448, ..Default::default() },
        );
    }
}
```

The level is `mymod:pocket`. `DimensionSettings` sets the build height, `has_skylight`,
`has_ceiling`, the `generator` (`minecraft:noise` by default), its `noise_settings` and the level's
one `biome`; the default is the Overworld's shape and terrain over plains, and the parts of a
dimension type not named here are the Overworld's. The level opens, ticks, saves and closes as the
Nether and the End do, in `dimensions/mymod/pocket/` of the world, and clients receive its
dimension type when they join.

A dimension in another mod's namespace, an empty name, a vanilla level's id, an id registered twice,
a build height the dimension type codec refuses (`min_y` and `height` are multiples of 16, and the
level stays between -2032 and 2031) and a generator nobody registered stop the game at startup with
a message naming the mod. The generator may come from a mod that loads later.

A registered dimension is the level stem and dimension type a data pack would give, below every
data pack: a pack with `data/mymod/dimension/pocket.json` or `data/mymod/dimension_type/pocket.json`
replaces that half of it, in the worlds that have the pack.

Registration is native only for now: the method isn't there for a mod built to `mod.wasm`.

### Sending a player to a dimension

There is no portal to a registered level, and no travel call: a mod sends a player there with the
command vanilla has for it, from a server hook.

```rust,ignore
fn run_mod_command(&self, host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
    let CommandSource::Player { player, .. } = event.source else {
        return Flow::Continue;
    };
    let Some(name) = player.name(&*host) else {
        return Flow::Continue;
    };
    let line = format!("execute in mymod:pocket run tp {name} 8 100 8");
    host.commands().run_command(&line);
    Flow::Cancel
}
```

The teleport happens at the next tick's start, and the level opens then if it isn't open. Name the
player, as the line runs as the console. `run_command` answers whether the command
ran, not whether the player arrived: a level that won't open, such as one naming a generator
nobody registered, logs an error and leaves the player where they were. Nothing builds a platform
at the destination, so look for the ground first, or place one.

### Chunk generators

A chunk generator is a mod's code for a level's terrain: vanilla's `ChunkGenerator`, and
`build_terrain` its `buildTerrain`. Register it once, at startup, and name it from a dimension,
or from a data pack's level stem as `"generator": {"type": "mymod:void"}`:

```rust,ignore
struct Floor;

impl ChunkGenerator for Floor {
    fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
        let stone = chunk.state("stone");
        let [x, z] = [chunk.pos().x * 16, chunk.pos().z * 16];
        for (dx, dz) in (0..16).flat_map(|dx| (0..16).map(move |dz| (dx, dz))) {
            chunk.set_block([x + dx, chunk.min_y(), z + dz], stone);
        }
    }
}

#[mod_api::horizon_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_chunk_generator("void", Floor);
        r.register_dimension(
            "void",
            DimensionSettings { generator: "mymod:void".into(), ..Default::default() },
        );
    }
}
```

There is no `Host` in world generation: chunks fill on the worker threads, so a generator sees only
the `ChunkAccess` of the chunk it fills, is `Send + Sync`, and keeps no mutable state, because two
workers can fill neighbouring chunks at once. The same seed and chunk give the same blocks.
`ChunkAccess` has the chunk's `pos`, the world's `seed`, the level's `min_y` and `height`, `state`
and `state_with` to resolve a block by name, and `set_block`, which skips a position outside the
chunk or the build height and answers whether it wrote. The chunk starts as air.

The generator gives terrain only. The level has no surface, carvers or features, and every chunk
has the stem's one biome. `noise_settings` means nothing to it.

A generator in another mod's namespace, an empty name, an id registered twice and `minecraft:noise`
stop the game at startup with a message naming the mod. It is native only for now, as
`register_dimension` is.

A level stem naming a generator nobody registered, say after its mod was removed, is an error for
whoever opens that level, not for the world: a teleport there fails and is logged, and a player
saved there joins a server in the Overworld. Single player refuses to open a world whose player is
saved in such a level.
