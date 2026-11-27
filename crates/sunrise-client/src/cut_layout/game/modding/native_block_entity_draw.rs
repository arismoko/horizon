#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_client_gameplay::game::client_side::native_block_entity_draw::tests::draw;

    /// A mod registers a layer (`register_layer_definitions`) at start: the game bakes it by its
    /// id and draws it, before and after a resource reload, beside vanilla's layers. The cube's
    /// south face, (0, 0, 1) to (1, 1, 1) in its part's frame, sits a block east of the part's
    /// origin and then the frame's 10.
    #[test]
    #[ignore = "daily"]
    fn a_registered_layer_bakes_and_draws_before_and_after_a_reload() {
        use mod_api::{CubeListBuilder, LayerDefinition, MeshDefinition, PartPose};
        use sunrise_core::geometry::Direction as Face;
        struct Golem;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Golem {
            fn register_layer_definitions(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::RegisterLayerDefinitionsEvent<'_>,
            ) {
                event.register("golem_mod:golem#main", || {
                    let mut mesh = MeshDefinition::new();
                    let root = mesh.root();
                    mesh.add_or_replace_child(
                        root,
                        "body",
                        CubeListBuilder::create().add_box_faces(
                            [0.0; 3],
                            [16.0; 3],
                            &[Face::South],
                        ),
                        PartPose::offset(16.0, 0.0, 0.0),
                    );
                    LayerDefinition::create(mesh, 64, 64)
                });
            }
        }
        let drawn = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            let part = game
                .bake_layer("golem_mod:golem#main")
                .expect("the mod's layer bakes");
            let mut d = draw();
            assert!(game.native_draw_model_part(&mut d, &part, "entity/zombie/zombie", 0xFF_FFFF));
            d.into_vertices().iter().map(|v| v.pos).collect::<Vec<_>>()
        };
        let face = [
            [11.0, 0.0, 1.0],
            [12.0, 0.0, 1.0],
            [12.0, 1.0, 1.0],
            [11.0, 1.0, 1.0],
        ];
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("golem_mod", Box::new(Golem))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                sunrise_client_gameplay::game::client_side::mods_client::register_layer_definitions(
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
                    &game.vanilla_root(),
                )
                .unwrap();
                assert_eq!(drawn(game), face);
                assert!(game.bake_layer("golem_mod:other#main").is_none());
                assert!(game.bake_layer("minecraft:zombie#main").is_some());
                game.spawn_pack_reload(
                    sunrise_client_gameplay::game::client_side::packs::ReloadKind::Selection,
                )
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
                assert_eq!(drawn(game), face, "the same layer after the reload");
            });
        });
    }

    const CORNERS: [[f32; 3]; 4] = [
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ];
    const UNIT_UV: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];

    /// What `crumbling(true)` covers, and no more: `quad`, `cube`, `block`, `model` and
    /// `terrain_model` drawn under it are projected onto the crack's destroy stage
    /// (`breaking::entity_geometry`), the same draws under `crumbling(false)` are not, and
    /// `crumbling_stage` reads the stage the frame gave the block (none for a posed frame).
    #[test]
    #[ignore = "daily"]
    fn crumbling_overlay_covers_quad_cube_block_model_and_terrain_model_only_while_on() {
        use glam::{Mat4, Vec3};
        use mod_api::{ClientReloadHost as _, Identifier};
        use sunrise_client::{block_entity_render::diffuse_lights, mesher::Vertex};
        use sunrise_client_gameplay::game::client_side::native_block_entity_draw::{
            BlockEntityDraw,
            tests::{sprite, test_crack_overlay},
        };
        use sunrise_world::registry::BlockStateId;

        let _ = (CORNERS, UNIT_UV);
        let _fixture =
            sunrise_client::models::standalone_fixture(vec!["minecraft:block/grass_block".into()]);
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.client
                .gui
                .atlas
                .sprites
                .insert("mymod:block/casing".into(), sprite());
            let id = Identifier::parse("minecraft:block/grass_block").unwrap();
            let model = game.render_assets().model(&id).unwrap();
            let game = &*game;
            let mut faces = [None; 6];
            faces[1] = Some(mod_api::CubeFace::new("mymod:block/casing"));
            let all_kinds = |surface: &mut BlockEntityDraw| {
                game.test_with_draw_context(surface, |drawn| {
                    assert!(drawn.quad(
                        "mymod:block/casing",
                        [[0.0, 1.0, 0.0], [0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                        [0.0, 0.0, 16.0, 16.0],
                        0xFFFFFF,
                    ));
                    assert!(drawn.cube([0.0; 3], [16.0; 3], faces, 0xFFFFFF));
                    assert_eq!(drawn.model(model, u32::MAX), Ok(()));
                    assert_eq!(
                        drawn.terrain_model(model, mod_api::TerrainModelOptions::default()),
                        Ok(())
                    );
                });
                assert!(game.test_native_draw_block(surface, BlockStateId(1)));
            };

            let posed = BlockEntityDraw::test_posed(
                0.0,
                0,
                0xF0F0,
                [0; 3],
                Mat4::IDENTITY,
                diffuse_lights(false),
                Vec::new(),
            );
            assert_eq!(game.test_native_draw_crumbling_stage(&posed), None);

            let mut surface = draw();
            surface.test_set_crumbling_stage(Some(4));
            assert_eq!(game.test_native_draw_crumbling_stage(&surface), Some(4));
            all_kinds(&mut surface);
            let before = surface.test_vertices().len();
            assert!(
                before > 0 && surface.test_crumbled().is_empty(),
                "off by default"
            );
            surface.test_crumbling(true);
            all_kinds(&mut surface);
            surface.test_crumbling(false);
            let covered = surface.test_vertices().len() - before;
            assert!(covered > 0);
            all_kinds(&mut surface);
            assert_eq!(
                surface.test_crumbled(),
                [(before, before + covered)],
                "the five draws under crumbling(true) and no others"
            );

            let stage = [0.25, 0.5];
            let mut overlay = Vec::new();
            test_crack_overlay(
                surface.test_vertices(),
                surface.test_crumbled(),
                Vec3::ZERO,
                stage,
                &mut overlay,
            );
            assert_eq!(overlay.len(), covered);
            let mut expected = surface.test_vertices()[before..before + covered].to_vec();
            sunrise_client::breaking::entity_geometry(&mut expected, Vec3::ZERO, stage);
            let bytes = |vertices: &[Vertex]| bytemuck::cast_slice::<_, u8>(vertices).to_vec();
            assert_eq!(bytes(&overlay), bytes(&expected));
            assert!(overlay.iter().all(|v| {
                v.layer == 11 && v.light == sunrise_client::breaking::stage_light(stage)
            }));
        });
    }

    /// `BlockEntityDrawContext::block_entity` draws a chest, a sign, a banner, a shulker box,
    /// a skull and a mod's block entity into pixels, draws nothing for a bed (its renderer is
    /// its block model) and for a type nobody renders, and refuses a call from inside its own
    /// dispatch of a mod's submit.
    #[test]
    #[ignore = "daily"]
    fn carried_block_entities_draw_visible_pixels() {
        if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
            "cut_layout::game::client_side::native_block_entity_draw::tests::carried_block_entities_draw_visible_pixels",
            "SUNRISE_CARRIED_BLOCK_ENTITY_CHILD",
            &["carried"],
        ) {
            return;
        }
        use glam::{Mat4, Quat, Vec3};
        use mod_api::{
            BlockEntityDrawRefusal, BlockEntityPart, BlockEntitySubmitEvent, BlockPart, ClientHost,
            Hooks, Identifier, ModEntry, Mods, Registrar, RegistrationContext, SceneBlockEntity,
        };
        use std::sync::{Arc, Mutex};
        use sunrise_client::mesher::Vertex;
        use sunrise_core::nbt::{Compound, Tag};
        use sunrise_gameplay_common::game::api_values::TryIntoApi;

        struct Carried {
            inner: Arc<Mutex<Option<SceneBlockEntity>>>,
            nested: Arc<Mutex<Option<Result<(), BlockEntityDrawRefusal>>>>,
        }
        #[mod_api::sunrise_part]
        impl BlockPart for Carried {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for Carried {
            fn block_entity_submit(
                &self,
                _: &dyn ClientHost,
                event: &mut BlockEntitySubmitEvent<'_>,
            ) {
                let _ = event.draw.quad(
                    "block/stone",
                    [[0.0, 1.0, 0.0], [0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                    [0.0, 0.0, 16.0, 16.0],
                    0xFF_FFFF,
                );
                if let Some(inner) = self.inner.lock().unwrap().clone() {
                    *self.nested.lock().unwrap() = Some(event.draw.block_entity(&inner));
                }
            }
        }
        struct CarriedMod {
            inner: Arc<Mutex<Option<SceneBlockEntity>>>,
            nested: Arc<Mutex<Option<Result<(), BlockEntityDrawRefusal>>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for CarriedMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let kind = r
                    .block_entity_type("carried")
                    .blocks(&["minecraft:stone"])
                    .build();
                r.block_entity_part(
                    &kind,
                    Carried {
                        inner: self.inner.clone(),
                        nested: self.nested.clone(),
                    },
                )
                .unwrap();
            }
        }
        let inner = Arc::new(Mutex::new(None));
        let nested = Arc::new(Mutex::new(None));
        let mods: Mods = vec![ModEntry::new(
            "carried_be",
            Box::new(CarriedMod {
                inner: inner.clone(),
                nested: nested.clone(),
            }),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();

        mod_platform::testing::with_mods(mods, || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                let game = &*game;
                let entity = |game: &sunrise_client_gameplay::game::ClientGame<'_>,
                              id: &str,
                              block: &str,
                              nbt: Compound| {
                    let state = game
                        .common
                        .pack
                        .assets
                        .registry
                        .default_state(block)
                        .unwrap_or_else(|| panic!("no block {block}"))
                        .try_into_api()
                        .unwrap();
                    SceneBlockEntity::new(
                        Identifier::parse(id).unwrap(),
                        state,
                        mod_api::item::containers::ModBlockEntityFields {
                            nbt,
                            items: Vec::new(),
                            data: Vec::new(),
                        },
                    )
                };
                // The vertices `block_entity` adds under an identity pose, lit fully.
                let drawn = |entity: &SceneBlockEntity| {
                    let mut surface = draw();
                    surface.test_set_pose_light(Mat4::IDENTITY, 240 | 240 << 8);
                    let result =
                        game.test_with_draw_context(&mut surface, |ctx| ctx.block_entity(entity));
                    (result, surface.test_into_vertices_raw())
                };
                // Lit pixels of a 64 by 64 picture of `vertices` over a white atlas.
                let lit_pixels = |vertices: &[Vertex]| {
                    let atlas = &game.client.gui.atlas;
                    let white = vec![255; (atlas.width * atlas.height * 4) as usize];
                    let view = Mat4::from_scale_rotation_translation(
                        Vec3::new(0.5, 0.5, 0.2),
                        Quat::IDENTITY,
                        Vec3::new(-0.25, -0.5, 0.4),
                    );
                    let rendered = sunrise_client::gfx::entity_pixel_test::render(
                        vertices,
                        [atlas.width, atlas.height],
                        &white,
                        view,
                        false,
                    )
                    .expect("software Vulkan entity pixels");
                    rendered
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0)
                        .count()
                };

                let mut sign = Compound::new();
                let mut front = Compound::new();
                front.insert(
                    "messages".into(),
                    Tag::List(vec![
                        Tag::String("carried".into()),
                        Tag::String("".into()),
                        Tag::String("".into()),
                        Tag::String("".into()),
                    ]),
                );
                // Black text (the default) is the picture's own black: dye it so it shows.
                front.insert("color".into(), Tag::String("white".into()));
                sign.insert("front_text".into(), Tag::Compound(front));
                let rendered = [
                    ("minecraft:chest", "minecraft:chest", Compound::new()),
                    ("minecraft:sign", "minecraft:oak_sign", sign),
                    ("minecraft:banner", "minecraft:red_banner", Compound::new()),
                    (
                        "minecraft:shulker_box",
                        "minecraft:shulker_box",
                        Compound::new(),
                    ),
                    (
                        "minecraft:skull",
                        "minecraft:skeleton_skull",
                        Compound::new(),
                    ),
                    ("carried_be:carried", "minecraft:stone", Compound::new()),
                ];
                for (id, block, nbt) in rendered {
                    let carried = entity(game, id, block, nbt);
                    let (result, vertices) = drawn(&carried);
                    assert_eq!(result, Ok(()), "{id}");
                    let pixels = lit_pixels(&vertices);
                    assert!(pixels >= 8, "{id} on {block} drew {pixels} pixels");
                }

                // A bed's renderer is its block model, and a stone block entity of another
                // mod has no Part: both draw nothing and are no error.
                for (id, block) in [
                    ("minecraft:bed", "minecraft:red_bed"),
                    ("other_mod:machine", "minecraft:dirt"),
                ] {
                    let (result, vertices) = drawn(&entity(game, id, block, Compound::new()));
                    assert_eq!(result, Ok(()), "{id}");
                    assert!(vertices.is_empty(), "{id} drew {} vertices", vertices.len());
                }

                // The mod's submit draws a chest of its own: that is the nested call, refused,
                // while the same chest drawn from outside draws.
                *inner.lock().unwrap() = Some(entity(
                    game,
                    "minecraft:chest",
                    "minecraft:chest",
                    Compound::new(),
                ));
                let carried = entity(
                    game,
                    "carried_be:carried",
                    "minecraft:stone",
                    Compound::new(),
                );
                let (result, vertices) = drawn(&carried);
                assert_eq!(result, Ok(()));
                assert_eq!(
                    *nested.lock().unwrap(),
                    Some(Err(BlockEntityDrawRefusal::Nested))
                );
                let (_, chest) = drawn(&inner.lock().unwrap().clone().unwrap());
                assert_eq!(
                    vertices.len(),
                    4,
                    "only the mod's quad: the nested chest drew nothing, a chest alone {}",
                    chest.len()
                );
                assert!(!chest.is_empty());
            });
        });
    }
}
