//! Where blocks' textures are in the block sprite arrays, for a client mod drawing blocks with the player's own
//! textures (resource packs included) in its own pass: with `WorldDrawn::block_sprites`, the arrays themselves.
use crate::game::ClientGame;
use mod_api::{AtlasRegion, ColorResolver, Identifier, LevelReadError};
use sunrise_client::block_sprites::{BlockSprites, Lookup, SpriteRef};
use sunrise_client_render::assets::RenderAssets;
use sunrise_level::chunk::Chunk;
use sunrise_world::registry::BlockStateId;

impl ClientGame<'_> {
    /// For a client mod: what each block (`minecraft:grass_block`, `oak_log[axis=x]`; properties left out take the
    /// block's default) draws on each of its six sides, as the mesher bakes its model: each face's sprite (array and
    /// layer, as `WorldDrawn::block_sprites` cuts them), where it lies on the side, how its texture runs across it,
    /// its tint source and whether it is cut out, translucent or animated; its particle sprite; whether it is a plain
    /// cube. None for a block the game does not know. The model is the state's first random variant. API
    /// 380.
    pub(in crate::game) fn block_sprites(
        &self,
        blocks: &[sunrise_world::registry::BlockStateId],
    ) -> Vec<Option<BlockSprites>> {
        let lookup = self.native_sprite_lookup();
        blocks
            .iter()
            .map(|state| {
                self.client
                    .client_pack
                    .context
                    .chunk
                    .assets
                    .registry
                    .state(*state)?;
                Some(lookup.state(*state))
            })
            .collect()
    }
    pub(in crate::game) fn sprite_atlas_and_split(&self) -> (&RenderAssets, u32) {
        let atlas = self
            .client
            .stitching_block_atlas
            .0
            .as_ref()
            .unwrap_or(&self.client.client_pack.render);
        let split = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or(u32::MAX, |r| r.sprite_split());
        (atlas, split)
    }

    /// For a client mod: each texture's sprite by name, or none for one no model uses.
    pub(in crate::game) fn texture_sprites(&self, textures: &[&str]) -> Vec<Option<SpriteRef>> {
        let (atlas, split) = self.sprite_atlas_and_split();
        textures
            .iter()
            .map(|name| sunrise_client::block_sprites::texture_at(atlas, split, name))
            .collect()
    }
    /// `ClientLevel.getBlockTint(pos, resolver)`, without consulting block or fluid tint sources.
    pub(in crate::game) fn biome_tint(
        &self,
        pos: [i32; 3],
        resolver: ColorResolver,
    ) -> Result<u32, LevelReadError> {
        self.native_client_level()
            .ok_or(LevelReadError::Unavailable)?;
        Ok(sunrise_client::mesher::biome_tint(
            pos,
            resolver,
            &self.tint_region(pos),
            self.mesh_biomes(),
            self.client.streaming.biome_zoom,
            self.client.streaming.biome_blend,
        ))
    }

    /// For a client mod drawing a block's faces itself (a copycat drawing its material's, from
    /// `block_sprites`): the 0xRRGGBB colour a face of `state` with tint index `tint_index` is
    /// multiplied by at `pos`, as the chunk mesher tints it there (`MeshTables::block_tints`:
    /// `BlockColors`, biome blend included). None for an untinted face. A fluid's face
    /// (`block_sprites::fluid_faces`: water at tint index 0) takes the water colour the fluid
    /// mesher gives it (`MeshTables::water_tint`, the packs' water colormap included). Both read the
    /// mesher's current biome colours, including after `refresh_biome_colors`.
    pub(in crate::game) fn block_tint_color(
        &self,
        state: BlockStateId,
        tint_index: i32,
        pos: [i32; 3],
    ) -> Option<u32> {
        let region = self.tint_region(pos);
        let biomes = self.mesh_biomes();
        let block = self
            .client
            .client_pack
            .context
            .chunk
            .assets
            .registry
            .state(state)
            .map_or("", |s| s.name.as_str());
        if let Some((_, tint, _)) = sunrise_client::block_sprites::fluid_faces(block) {
            return (tint.is_some() && tint_index == 0).then(|| {
                self.client.client_pack.context.mesh.tables.water_tint(
                    pos,
                    &region,
                    biomes,
                    self.client.streaming.biome_zoom,
                    self.client.streaming.biome_blend,
                )
            });
        }
        let tints = self.client.client_pack.context.mesh.tables.block_tints(
            state,
            pos,
            &region,
            biomes,
            self.client.streaming.biome_zoom,
            self.client.streaming.biome_blend,
            self.client.streaming.nether_shading,
        );
        tints(tint_index)
    }

    /// The mesher's 3×3 chunk region centred on the chunk that contains `pos`.
    fn tint_region(&self, [x, _, z]: [i32; 3]) -> [[Option<&Chunk>; 3]; 3] {
        let (cx, cz) = (x >> 4, z >> 4);
        std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                self.common
                    .world
                    .chunks
                    .get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                    .map(AsRef::as_ref)
            })
        })
    }

    /// For a client mod: changes each time the resources load (a resource pack turned on or off can move every
    /// sprite): ask `block_sprites` again then. API 380.
    pub(in crate::game) fn block_sprites_version(&self) -> u64 {
        self.client
            .stitching_block_atlas
            .0
            .as_ref()
            .unwrap_or(&self.client.client_pack.render)
            .generation
    }
    /// For a client mod: where GUI sprite `sprite` lies in the GUI atlas and its size, none for one the atlas lacks
    /// or whose entry its definition removed (a server's metrics-only atlas lacks them all). The atlas names its
    /// sprites by GUI id: `container/slot` or `minecraft:container/slot`, a mod's `mymod:panel`.
    pub(in crate::game) fn gui_sprite(&self, sprite: &Identifier) -> Option<AtlasRegion> {
        let found = if sprite.namespace() == "minecraft" {
            self.client.gui.atlas.gui_lookup(sprite.path())
        } else {
            self.client.gui.atlas.gui_lookup(&sprite.to_string())
        };
        let Some(sunrise_client::hud::GuiSprite::Present { rect: r, .. }) = found else {
            return None;
        };
        Some(AtlasRegion::new([r.u0, r.v0, r.u1, r.v1], [r.w, r.h]))
    }
    fn native_sprite_lookup(&self) -> Lookup<'_> {
        // (no renderer, no arrays: every layer counts as array 0's)
        let split = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or(u32::MAX, |r| r.sprite_split());
        Lookup::new(
            &self.client.client_pack.context,
            split,
            &self.client.client_pack.layer_names,
        )
    }
}

#[cfg(test)]
mod texture_atlas_stitched_tests {
    use mod_api::{
        AtlasRegion, ClientHost, Hooks, Identifier, ModEntry, SpriteRef, TextureAtlas,
        TextureAtlasStitchedEvent,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// What the mod kept: each atlas it heard, and one sprite of each, looked up once there.
    #[derive(Default)]
    struct Kept {
        atlases: Vec<TextureAtlas>,
        stone: Option<SpriteRef>,
        slot: Option<AtlasRegion>,
    }
    struct Keeper(Arc<Mutex<Kept>>);
    #[mod_api::__subscriptions]
    impl Hooks for Keeper {
        fn texture_atlas_stitched(
            &self,
            host: &mut dyn ClientHost,
            event: &mut TextureAtlasStitchedEvent,
        ) {
            let mut kept = self.0.lock().unwrap();
            kept.atlases.push(event.atlas);
            match event.atlas {
                TextureAtlas::Blocks => {
                    kept.stone = host.texture_sprites(&["minecraft:block/stone"])[0];
                }
                TextureAtlas::Gui => {
                    kept.slot = host.gui_sprite(&id("container/slot"));
                }
                _ => {}
            }
        }
    }

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    /// A mod's sprite source: a 2x2 block sprite no model names, and a GUI sprite one pixel
    /// wider each time the GUI atlas is asked, so a reload's answer differs from the first load's.
    #[derive(Default)]
    struct Marker {
        gui_asks: u32,
        block: Option<SpriteRef>,
        gui: Option<AtlasRegion>,
    }
    struct Source(Arc<Mutex<Marker>>);
    #[mod_api::__subscriptions]
    impl Hooks for Source {
        fn register_sprite_sources(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterSpriteSourcesEvent<'_>,
        ) {
            let marker = Arc::clone(&self.0);
            event.register(id("marker:own"), move |input| {
                let mut marker = marker.lock().unwrap();
                let mut output = Vec::new();
                match input.atlas.to_string().as_str() {
                    "minecraft:blocks" => output.push(mod_api::SpriteSourceOutput::Pixels {
                        sprite: id("marker:block/own"),
                        width: 2,
                        height: 2,
                        rgba: [32, 64, 96, 255].repeat(4),
                    }),
                    "minecraft:gui" => {
                        marker.gui_asks += 1;
                        let width = 2 + marker.gui_asks;
                        output.push(mod_api::SpriteSourceOutput::Pixels {
                            sprite: id("marker:own"),
                            width,
                            height: 3,
                            rgba: [64, 96, 32, 255].repeat((width * 3) as usize),
                        });
                        // A sprite the first load publishes and the reload no longer does.
                        if marker.gui_asks == 1 {
                            output.push(mod_api::SpriteSourceOutput::Pixels {
                                sprite: id("marker:first_only"),
                                width: 2,
                                height: 2,
                                rgba: [9, 9, 9, 255].repeat(4),
                            });
                        }
                        // The replacement of a walked vanilla sprite, of another size.
                        output.push(mod_api::SpriteSourceOutput::Pixels {
                            sprite: id("container/slot"),
                            width: 5,
                            height: 5,
                            rgba: [255, 0, 255, 255].repeat(25),
                        });
                    }
                    _ => {}
                }
                Ok(output)
            });
        }
        fn texture_atlas_stitched(
            &self,
            host: &mut dyn ClientHost,
            event: &mut TextureAtlasStitchedEvent,
        ) {
            let mut marker = self.0.lock().unwrap();
            match event.atlas {
                TextureAtlas::Blocks => {
                    marker.block = host.texture_sprites(&["marker:block/own"])[0];
                }
                TextureAtlas::Gui => marker.gui = host.gui_sprite(&id("marker:own")),
                _ => {}
            }
        }
    }

    /// The prepared resources with the definitions `files` (relative path, bytes) laid over them:
    /// every other file is linked, never copied.
    #[cfg(unix)]
    fn shadowed(base: &std::path::Path, files: &[(&str, Vec<u8>)]) -> tempfile::TempDir {
        fn mirror(src: &std::path::Path, dst: &std::path::Path, files: &[(Vec<&str>, &[u8])]) {
            for entry in std::fs::read_dir(src).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let name = name.to_str().unwrap();
                let under: Vec<_> = files
                    .iter()
                    .filter(|(path, _)| path[0] == name)
                    .map(|(path, bytes)| (path[1..].to_vec(), *bytes))
                    .collect();
                let to = dst.join(name);
                match under.iter().find(|(rest, _)| rest.is_empty()) {
                    Some((_, bytes)) => std::fs::write(&to, bytes).unwrap(),
                    None if under.is_empty() => {
                        std::os::unix::fs::symlink(entry.path(), &to).unwrap();
                    }
                    None => {
                        std::fs::create_dir(&to).unwrap();
                        mirror(&entry.path(), &to, &under);
                    }
                }
            }
        }
        let root = tempfile::tempdir().unwrap();
        let files: Vec<(Vec<&str>, &[u8])> = files
            .iter()
            .map(|(path, bytes)| (path.split('/').collect(), bytes.as_slice()))
            .collect();
        mirror(base, root.path(), &files);
        root
    }

    /// The game's own definition of `atlas` with an entry of the mod's source type added.
    #[cfg(unix)]
    fn with_marker_entry(base: &std::path::Path, atlas: &str) -> (String, Vec<u8>) {
        let path = format!("assets/minecraft/atlases/{atlas}.json");
        let mut definition: serde_json::Value =
            serde_json::from_slice(&std::fs::read(base.join(&path)).unwrap()).unwrap();
        definition["sources"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"type": "marker:own"}));
        (path, serde_json::to_vec(&definition).unwrap())
    }

    /// A registered sprite source's output is published by atlas and id through the real host,
    /// at startup and after a reload: a block sprite no model names, a GUI sprite of another size
    /// each time, and a walked vanilla GUI sprite replaced by the source's pixels, all found by
    /// their ids; what the source never made, or made only before the reload, is none.
    #[test]
    #[cfg(unix)]
    #[ignore = "daily"]
    fn a_sprite_source_publishes_its_sprites_by_atlas_and_id_through_the_host() {
        use mod_api::__private::game::RegistrationContextNew;

        // Both resource loads need the source type, whose registry is process-wide.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::native_block_sprites::texture_atlas_stitched_tests::a_sprite_source_publishes_its_sprites_by_atlas_and_id_through_the_host",
            "SUNRISE_SPRITE_SOURCE_ATLAS_TEST_CHILD",
            &["atlas"],
        ) {
            return;
        }
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let files: Vec<_> = ["blocks", "gui"]
            .map(|atlas| with_marker_entry(&base, atlas))
            .into();
        let files: Vec<_> = files.iter().map(|(p, b)| (p.as_str(), b.clone())).collect();
        let root = shadowed(&base, &files);
        let marker = Arc::new(Mutex::new(Marker::default()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "marker",
            Box::new(Source(Arc::clone(&marker))),
        )]
        .into();
        with_mods(mods, || {
            let role = sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            let mut sources = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_register_sprite_sources(
                &mod_api::RegistrationContext::new(),
                &mut sources,
            );
            assert_eq!(
                sunrise_client_render::mod_sprite_sources::register(sources),
                ["marker:own"]
            );
            super::super::native_entity_refs::tests::with_game_role_client(
                root.path().to_owned(),
                role,
                |game| {
                    game.texture_atlases_stitched();
                    {
                        let mut marker = marker.lock().unwrap();
                        assert!(
                            marker.block.take().is_some(),
                            "no model names the block sprite"
                        );
                        assert_eq!(marker.gui.map(|s| s.size), Some([3, 3]));
                    }
                    assert_eq!(
                        game.gui_sprite(&id("container/slot")).map(|s| s.size),
                        Some([5, 5])
                    );
                    assert_eq!(game.gui_sprite(&id("marker:no_such_sprite")), None);
                    assert_eq!(
                        game.gui_sprite(&id("marker:first_only")).map(|s| s.size),
                        Some([2, 2])
                    );
                    game.spawn_pack_reload(super::super::packs::ReloadKind::Selection)
                        .unwrap();
                    let started = std::time::Instant::now();
                    while game.client.pack_reload.0.is_some() {
                        assert!(
                            started.elapsed().as_secs() < 300,
                            "the reload never finished"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        game.drive_pack_reload();
                    }
                    let marker = marker.lock().unwrap();
                    assert!(marker.block.is_some(), "published again after the reload");
                    assert_eq!(
                        game.gui_sprite(&id("marker:first_only")),
                        None,
                        "a sprite the reload no longer makes is gone"
                    );
                    assert_eq!(
                        marker.gui.map(|s| s.size),
                        Some([4, 3]),
                        "the reload's own answer, not the first load's"
                    );
                    assert_eq!(
                        game.gui_sprite(&id("container/slot")).map(|s| s.size),
                        Some([5, 5])
                    );
                },
            );
        });
    }

    /// `TextureAtlasStitchedEvent`: the mod hears the blocks' atlas and then the GUI's, finds a
    /// sprite of each, and hears both again after a resource reload.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_finds_its_sprites_each_time_an_atlas_is_stitched() {
        let kept = Arc::new(Mutex::new(Kept::default()));
        let mods: mod_api::Mods =
            vec![ModEntry::new("keeper", Box::new(Keeper(Arc::clone(&kept))))].into();
        with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                // The first load, as `run` fires it before `client_setup`.
                game.texture_atlases_stitched();
                {
                    let kept = kept.lock().unwrap();
                    assert_eq!(kept.atlases, [TextureAtlas::Blocks, TextureAtlas::Gui]);
                    assert!(kept.stone.is_some(), "stone is in the block arrays");
                    let slot = kept.slot.expect("the slot sprite is in the GUI atlas");
                    assert_eq!(slot.size, [18, 18], "the slot's own pixels");
                    let [u0, v0, u1, v1] = slot.uv;
                    assert!(0.0 <= u0 && u0 < u1 && u1 <= 1.0, "{:?}", slot.uv);
                    assert!(0.0 <= v0 && v0 < v1 && v1 <= 1.0, "{:?}", slot.uv);
                }
                // A mod's namespace is part of the name; one the atlas lacks is none.
                assert_eq!(game.gui_sprite(&id("mymod:container/slot")), None);
                assert_eq!(game.gui_sprite(&id("gui/no_such_sprite")), None);
                // F3+T: the reload stages on a worker and applies here.
                kept.lock().unwrap().atlases.clear();
                game.spawn_pack_reload(super::super::packs::ReloadKind::Selection)
                    .unwrap();
                let started = std::time::Instant::now();
                while game.client.pack_reload.0.is_some() {
                    assert!(
                        started.elapsed().as_secs() < 300,
                        "the reload never finished"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    game.drive_pack_reload();
                }
                let kept = kept.lock().unwrap();
                assert_eq!(kept.atlases, [TextureAtlas::Blocks, TextureAtlas::Gui]);
                assert_eq!(
                    kept.slot.map(|s| s.size),
                    Some([18, 18]),
                    "looked up again in the new atlas"
                );
            });
        });
    }
}

/// `FaceSprite::quad_on`, the copycat side of `block_sprites`.
#[cfg(test)]
mod quad_on_tests {
    use sunrise_core::geometry::Direction;

    fn face(uv: [f32; 6]) -> mod_api::FaceSprite {
        mod_api::FaceSprite {
            sprite: mod_api::SpriteRef {
                array: 0,
                layer: 0,
                animated: false,
                cutout: false,
            },
            texture: "block/oak_log".into(),
            rect: [0.0, 0.0, 1.0, 1.0],
            inset: 0.0,
            uv,
            tint: None,
            tint_index: -1,
            translucent: false,
        }
    }

    /// The point (a, b) of `side`, in its two axes, as `FaceSprite::uv` counts them.
    fn side_point(side: Direction, p: [f32; 3]) -> [f32; 2] {
        match side {
            Direction::Down | Direction::Up => [p[0], p[2]],
            Direction::North | Direction::South => [p[0], p[1]],
            Direction::West | Direction::East => [p[2], p[1]],
        }
    }

    /// Whatever the face's turn or mirror, the UV `quad` gives each corner (the rectangle's
    /// (u0, v0), (u0, v1), (u1, v1), (u1, v0) in turn) is the one the material's own side shows
    /// at that point, and the corners wind counterclockwise seen from outside.
    #[test]
    fn a_copycat_face_shows_its_material_turned_as_the_model_turns_it() {
        let upright = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        // u = b, v = 1 - a: a quarter turn (a log on its side).
        let turned = [0.0, 1.0, 0.0, -1.0, 1.0, 0.0];
        // u = 1 - a, v = b: mirrored.
        let mirrored = [1.0, 0.0, -1.0, 0.0, 0.0, 1.0];
        let boxes = [([0.0; 3], [1.0; 3]), ([0.0, 0.0, 0.0], [1.0, 0.5, 1.0])];
        for uv in [upright, turned, mirrored] {
            let face = face(uv);
            for side in Direction::ALL {
                for (from, to) in boxes {
                    let (corners, [u0, v0, u1, v1]) = face
                        .quad_on(side, from, to)
                        .expect("a turn of 90 degrees or a mirror fits a rectangle");
                    let given = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
                    for (corner, given) in corners.iter().zip(given) {
                        let [a, b] = side_point(side, *corner);
                        let want = [
                            (uv[0] + a * uv[2] + b * uv[4]) * 16.0,
                            (uv[1] + a * uv[3] + b * uv[5]) * 16.0,
                        ];
                        assert!(
                            (want[0] - given[0]).abs() < 1e-4 && (want[1] - given[1]).abs() < 1e-4,
                            "{side:?} {uv:?} {from:?}..{to:?}: {corner:?} shows {given:?}, not {want:?}"
                        );
                    }
                    let [p0, p1, p2, _] = corners;
                    let e = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
                    let f = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
                    let n = side.normal().map(|v| v as f32);
                    let out = (e[1] * f[2] - e[2] * f[1]) * n[0]
                        + (e[2] * f[0] - e[0] * f[2]) * n[1]
                        + (e[0] * f[1] - e[1] * f[0]) * n[2];
                    assert!(out > 0.0, "{side:?} {uv:?}: the face looks inward");
                }
            }
        }
        // The half box's north face is cut at half height: its corners lie on z = 0, y up to 0.5.
        let (corners, _) = face(upright)
            .quad_on(Direction::North, [0.0; 3], [1.0, 0.5, 1.0])
            .unwrap();
        assert!(corners.iter().all(|c| c[2] == 0.0 && c[1] <= 0.5));
        // A texture turned 22.5 degrees on the side, or sheared, fits no UV rectangle: no answer
        // rather than wrong UVs.
        let (s, c) = 22.5f32.to_radians().sin_cos();
        let slanted = [
            0.5 - 0.5 * c + 0.5 * s,
            0.5 - 0.5 * s - 0.5 * c,
            c,
            s,
            -s,
            c,
        ];
        let sheared = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0];
        for uv in [slanted, sheared] {
            assert_eq!(
                face(uv).quad_on(Direction::North, [0.0; 3], [1.0; 3]),
                None,
                "{uv:?}"
            );
        }
    }
}

/// `block_tint_color`, the colour a copycat multiplies its material's tinted faces by.
#[cfg(test)]
mod block_tint_color_tests {
    /// A grass block's tinted face takes a colour where it stands, and so does water's face
    /// (tint index 0, as `block_sprites` reports it); stone has no tint, and an untinted
    /// face (-1) none either.
    #[test]
    #[ignore = "daily"]
    fn a_tinted_face_takes_its_colour_where_it_stands() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let registry = game.client.client_pack.context.chunk.assets.registry;
            let grass = registry.default_state("minecraft:grass_block").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            use std::sync::Arc;
            use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local")
                .join(format!(
                    "tint-fixture-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
            let write = |path: &str, bytes: &[u8]| {
                let path = root.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            };
            let png = |rgb: Vec<u8>| {
                let mut bytes = Vec::new();
                let mut encoder = png::Encoder::new(&mut bytes, 256, 256);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&rgb).unwrap();
                writer.finish().unwrap();
                bytes
            };
            // Synthetic pixels only. Vanilla clamps t and r, then samples at
            // x = trunc(255 * (1 - t)), y = trunc(255 * (1 - r*t)).
            // (1,1) -> (0,0); (0.5,0.5) -> (127,191), index 49023.
            // Neither biome has a grass override/modifier. Uniform chunks make
            // the BiomeColors blend equal to that one pixel, independent of zoom.
            let mut pixels = vec![0; 256 * 256 * 3];
            pixels[..3].copy_from_slice(&[0x12, 0x34, 0x56]);
            pixels[49023 * 3..49023 * 3 + 3].copy_from_slice(&[0xAB, 0xCD, 0xEF]);
            let image = png(pixels);
            for name in ["grass", "foliage", "dry_foliage"] {
                write(
                    &format!("assets/minecraft/textures/colormap/{name}.png"),
                    &image,
                );
            }
            write(
                "data/minecraft/worldgen/biome/plains.json",
                br#"{"temperature":1.0,"downfall":1.0}"#,
            );
            write(
                "data/minecraft/worldgen/biome/cool.json",
                br#"{"temperature":0.5,"downfall":0.5}"#,
            );
            // A separate, constant water map: every sample and every blend is
            // 0x2468AC, never either grass pixel or a biome's default water colour.
            write(
                "assets/test/polytone/colormaps/water.png",
                &png([0x24, 0x68, 0xAC].repeat(256 * 256)),
            );
            write(
                "assets/test/polytone/colors.json",
                br#"{"biome_colors":{"water":"test:water"}}"#,
            );
            let biomes = Arc::new(
                sunrise_level::biome::Biomes::load(&ResourceRoot::directory(root.clone())).unwrap(),
            );
            let source = ResourceSource::directory(root.clone());
            let old = &game.client.client_pack.context;
            let tables = sunrise_client::mesher::MeshTables::new(
                &old.chunk.assets,
                &old.mesh.render,
                game.res.facts(),
                sunrise_client::models::load_visibility(&game.options.resources, registry).unwrap(),
                None,
            )
            .unwrap()
            .with_polytone_colormaps(&source, registry);
            game.client.client_pack.context =
                Arc::new(sunrise_client_render::streaming::WorkerContext {
                    chunk: sunrise_level::chunk_streamer::ChunkWorkerContext {
                        assets: old.chunk.assets.clone(),
                        biomes: biomes.clone(),
                        light: old.chunk.light.clone(),
                    },
                    mesh: sunrise_client_render::streaming::MeshWorkerContext {
                        render: old.mesh.render.clone(),
                        tables: Arc::new(tables),
                        hidden_states: old.mesh.hidden_states.clone(),
                        part_tables: old.mesh.part_tables.clone(),
                    },
                });
            game.world_biomes = Some(biomes.clone());
            game.client.streaming.biome_blend = 0;
            game.world.chunks.clear();
            for (cx, name) in [(0, "minecraft:plains"), (4, "minecraft:cool")] {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let mut chunk = sunrise_level::chunk::Chunk::empty(cx + dx, dz);
                        for y in 0..96 {
                            for z in 0..4 {
                                for x in 0..4 {
                                    chunk.set_biome_cell(x, y, z, biomes.id(name));
                                }
                            }
                        }
                        game.world.chunks.insert((cx + dx, dz), Arc::new(chunk));
                    }
                }
            }
            assert_eq!(game.block_tint_color(grass, 0, [8, 64, 8]), Some(0x123456));
            assert_eq!(game.block_tint_color(grass, 0, [72, 64, 8]), Some(0xABCDEF));
            for pos in [[8, 64, 8], [72, 64, 8]] {
                assert_eq!(game.block_tint_color(water, 0, pos), Some(0x2468AC));
                assert_eq!(game.block_tint_color(grass, -1, pos), None);
                let untinted = game.block_tint_color(stone, 0, pos);
                assert_eq!(untinted, None);
                assert_eq!(untinted.unwrap_or(0xFFFFFF), 0xFFFFFF);
                assert_eq!(game.block_tint_color(water, 1, pos), None);
            }
            std::fs::remove_dir_all(root).unwrap();
        });
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
