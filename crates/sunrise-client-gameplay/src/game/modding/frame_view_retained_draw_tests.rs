//! Behavior tests for #5077: `WorldRenderView` on `BlockEntityDrawContext`,
//! per-view `EntityRenderState` placement, and posed retained draws
//! (`BlockEntityDrawContext::instance_group` and `ClientHost::prepare_item` /
//! `BlockEntityDrawContext::prepared_item`).
use super::{
    api_values_client::aggregates::scene_blocks, native_block_entity_draw::BlockEntityDraw,
    native_entity_renderers::RegisteredEntityRenderer, native_mob_draw::entity_render_state,
};
use crate::game::host::draw::GameBlockEntityDrawContext;
use glam::{DVec3, Mat4, Quat, Vec3};
use mod_api::{
    BlockEntityDrawContext, BlockEntityPart, BlockEntitySubmitEvent, BlockPart, Camera, ClientHost,
    EntityRenderLayer, EntityRenderState, EntityRenderer, FrameModel, GroupError, Hooks,
    IDENTITY_TRANSFORM, Identifier, InstanceGroups, ItemDisplayContext, ItemStack, LayerTarget,
    LightCoords, ModEntry, ModelError, ModelInstance, Mods, Overlay, RegisterEntityLayersEvent,
    Registrar, RegistrationContext, RenderAssets, Scene, SceneBlock, SceneBlockEntity, SceneClock,
    SceneEntity, SceneView, WorldRenderView,
};
use std::sync::{Arc, Mutex};
use sunrise_client_render::{block_entity_render::diffuse_lights, hud::HudBatch};
use sunrise_gameplay_common::game::api_values::TryIntoApi as _;
use sunrise_level::containers::{BlockEntity, ModBlockEntity};
use sunrise_sim::mobs::Mob;

fn test_camera(position: [f64; 3], yaw: f32, pitch: f32, roll: f32) -> Camera {
    let view = if roll == 0.0 {
        sunrise_client_render::camera::view_matrix(yaw, pitch)
    } else {
        sunrise_level::camera_math::view_matrix_rolled(yaw, pitch, roll)
    };
    Camera {
        position,
        eye_height: 1.62,
        partial_tick: 0.5,
        yaw,
        pitch,
        roll,
        fov_degrees: 70.0,
        near: 0.05,
        far: 128.0,
        view: view.to_cols_array(),
        projection: Mat4::IDENTITY.to_cols_array(),
        dimension: "minecraft:overworld".into(),
    }
}

#[test]
fn world_render_view_basis_matches_engine_camera_across_angles_and_roll() {
    for yaw in [-135.0f32, 0.0, 45.0, 90.0, 180.0, 270.0] {
        for pitch in [-89.0f32, -30.0, 0.0, 35.0, 89.0] {
            for roll in [-45.0f32, 0.0, 22.5, 90.0] {
                let mut engine = sunrise_client_render::camera::Camera::default();
                engine.set_rotation_with_roll(yaw, pitch, roll);
                let cam = test_camera([1.0, 2.0, 3.0], yaw, pitch, roll);
                let view = WorldRenderView::new(&cam, None, 0.25, 10, 10.25, false, &[]);
                assert_eq!(view.camera_rotation(), [yaw, pitch, roll]);
                for (got, expected, label) in [
                    (view.camera_forward(), engine.forwards, "forward"),
                    (view.camera_up(), engine.up, "up"),
                    (view.camera_left(), engine.left, "left"),
                ] {
                    for axis in 0..3 {
                        assert!(
                            (got[axis] - expected[axis]).abs() < 1e-5,
                            "{label}[{axis}] mismatch at ({yaw}, {pitch}, {roll}): {got:?} vs {expected:?}"
                        );
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ObservedView {
    camera_position: [f64; 3],
    camera_rotation: [f32; 3],
    camera_entity_eye: Option<[f64; 3]>,
    partial_tick: f32,
    game_time: i64,
    animation_time: f64,
}

fn observe(draw: &dyn BlockEntityDrawContext) -> ObservedView {
    let view = draw.view();
    assert_eq!(draw.partial(), view.partial_tick);
    assert_eq!(draw.game_time(), view.game_time);
    assert_eq!(draw.animation_time(), view.animation_time);
    ObservedView {
        camera_position: view.camera_position(),
        camera_rotation: view.camera_rotation(),
        camera_entity_eye: view.camera_entity_eye,
        partial_tick: view.partial_tick,
        game_time: view.game_time,
        animation_time: view.animation_time,
    }
}

#[test]
#[ignore = "daily"]
fn block_entity_entity_renderer_scene_and_two_view_isolation() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::client_side::frame_view_retained_draw_tests::block_entity_entity_renderer_scene_and_two_view_isolation",
        "SUNRISE_FRAME_VIEW_BLOCK_ENTITY_CHILD",
        &["block-entity-and-entity"],
    ) {
        return;
    }

    let block_observations = Arc::new(Mutex::new(Vec::<ObservedView>::new()));
    struct BlockObserverPart {
        log: Arc<Mutex<Vec<ObservedView>>>,
    }
    #[mod_api::sunrise_part]
    impl BlockPart for BlockObserverPart {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for BlockObserverPart {
        fn block_entity_submit(&self, _: &dyn ClientHost, event: &mut BlockEntitySubmitEvent<'_>) {
            self.log.lock().unwrap().push(observe(event.draw));
            let _ = event.draw.quad(
                "block/stone",
                [[0.0, 1.0, 0.0], [0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                [0.0, 0.0, 16.0, 16.0],
                0xFF_FFFF,
            );
        }
    }
    struct BlockObserverMod {
        log: Arc<Mutex<Vec<ObservedView>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for BlockObserverMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let kind = r
                .block_entity_type("observer")
                .blocks(&["minecraft:stone"])
                .build();
            r.block_entity_part(
                &kind,
                BlockObserverPart {
                    log: self.log.clone(),
                },
            )
            .unwrap();
        }
    }

    let mods: Mods = vec![ModEntry::new(
        "view_observer",
        Box::new(BlockObserverMod {
            log: block_observations.clone(),
        }),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();

    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            let stone_state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let pos = [4, 64, 8];
            game.world.block_entities.insert(
                (pos[0], pos[1], pos[2]),
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "view_observer:observer".into(),
                    Default::default(),
                ))),
            );

            let cam_a = test_camera([1.0, 65.0, 2.0], 30.0, 10.0, 5.0);
            let view_a = WorldRenderView::new(
                &cam_a,
                Some([1.0, 66.62, 2.0]),
                0.25,
                100,
                100.25,
                false,
                &[],
            );
            let cam_b = test_camera([12.0, 70.0, 20.0], 180.0, -15.0, -10.0);
            let view_b = WorldRenderView::new(
                &cam_b,
                Some([12.0, 71.62, 20.0]),
                0.75,
                101,
                101.75,
                true,
                &[],
            );

            let blocks = [(pos, stone_state.0)];
            let mut out_a = Vec::new();
            game.mod_block_entity_geometry_with_view(
                blocks.iter().map(|b| (b, true)),
                DVec3::from(cam_a.position),
                view_a.partial_tick,
                Some(&view_a),
                &mut out_a,
            );
            let mut out_b = Vec::new();
            game.mod_block_entity_geometry_with_view(
                blocks.iter().map(|b| (b, true)),
                DVec3::from(cam_b.position),
                view_b.partial_tick,
                Some(&view_b),
                &mut out_b,
            );

            // Draw the same block entity in a screen scene: camera_entity_eye must be None
            // and view must carry the scene's camera and clock.
            let mut batch = HudBatch::new([800, 600], game.client.settings.gui_scale_inputs());
            let scene_state = stone_state.try_into_api().unwrap();
            let scene_entity = SceneBlockEntity::new(
                Identifier::parse("view_observer:observer").unwrap(),
                scene_state,
                Default::default(),
            );
            let scene_block = SceneBlock::new([0, 0, 0], scene_state, Some(scene_entity));
            let scene = Scene::new(vec![scene_block])
                .with_clock(SceneClock::new(7, 55, 0.5).animate_blocks(false));
            let scene_view = SceneView::new([0.5, 0.5, 0.5], 45.0, 25.0, 16.0);
            let prepared = scene_blocks(&scene.blocks).unwrap();
            game.native_draw_scene(
                &mut batch,
                [0.0, 0.0, 200.0, 200.0],
                &prepared,
                &scene,
                scene_view,
            )
            .unwrap();

            let seen = block_observations.lock().unwrap().clone();
            assert_eq!(seen.len(), 3);
            assert_eq!(seen[0].camera_position, cam_a.position);
            assert_eq!(seen[0].camera_rotation, [30.0, 10.0, 5.0]);
            assert_eq!(seen[0].camera_entity_eye, Some([1.0, 66.62, 2.0]));
            assert_eq!(seen[0].partial_tick, 0.25);
            assert_eq!(seen[0].game_time, 100);
            assert_eq!(seen[0].animation_time, 100.25);

            assert_eq!(seen[1].camera_position, cam_b.position);
            assert_eq!(seen[1].camera_rotation, [180.0, -15.0, -10.0]);
            assert_eq!(seen[1].camera_entity_eye, Some([12.0, 71.62, 20.0]));
            assert_eq!(seen[1].partial_tick, 0.75);
            assert_eq!(seen[1].game_time, 101);
            assert_eq!(seen[1].animation_time, 101.75);

            assert_eq!(seen[2].camera_position, [0.5, 0.5, 0.5]);
            assert_eq!(seen[2].camera_rotation, [45.0, 25.0, 0.0]);
            assert_eq!(seen[2].camera_entity_eye, None);
            assert_eq!(seen[2].partial_tick, 0.5);
            assert_eq!(seen[2].game_time, 55);
            assert_eq!(seen[2].animation_time, 55.5);

            // EntityRenderer: two views of the same moving mob yield distinct views and
            // per-view distance_to_camera_sq, plus accurate interpolated position/size/eye_height.
            let entity_observations =
                Arc::new(Mutex::new(Vec::<(EntityRenderState, ObservedView)>::new()));
            struct MobObserver {
                log: Arc<Mutex<Vec<(EntityRenderState, ObservedView)>>>,
            }
            impl EntityRenderer for MobObserver {
                fn submit(
                    &self,
                    entity: &EntityRenderState,
                    draw: &mut dyn BlockEntityDrawContext,
                ) -> Result<(), String> {
                    self.log
                        .lock()
                        .unwrap()
                        .push((entity.clone(), observe(draw)));
                    Ok(())
                }
            }
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(
                77,
                kind,
                sunrise_core::geometry::Vec3::new(10.0, 64.0, 10.0),
                0.0,
            );
            mob.previous = sunrise_core::geometry::Vec3::new(6.0, 64.0, 10.0);
            let renderer = RegisteredEntityRenderer::new(
                Arc::new(MobObserver {
                    log: entity_observations.clone(),
                }),
                None,
            );
            for (cam, view) in [(&cam_a, &view_a), (&cam_b, &view_b)] {
                let state =
                    entity_render_state(&mob, mob.id, view.partial_tick, DVec3::from(cam.position));
                let feet = DVec3::from_array(state.position);
                let mut out = Vec::new();
                assert!(game.mod_entity_renderer_geometry_with_view(
                    &renderer,
                    &state,
                    &mob.kind.id,
                    feet - DVec3::from(cam.position),
                    Some(feet),
                    DVec3::from(cam.position),
                    Some(view),
                    [
                        feet.x.floor() as i32,
                        feet.y.floor() as i32,
                        feet.z.floor() as i32,
                    ],
                    0xF0F0,
                    &mut out,
                ));
            }

            let mob_seen = entity_observations.lock().unwrap();
            assert_eq!(mob_seen.len(), 2);
            // At partial 0.25: x = 6 + 0.25 * (10 - 6) = 7.0
            assert_eq!(mob_seen[0].0.position, [7.0, 64.0, 10.0]);
            assert_eq!(mob_seen[0].0.size, [mob.width(), mob.height()]);
            assert_eq!(mob_seen[0].0.eye_height, mob.eye_height() as f32);
            let expected_dist_a =
                (DVec3::new(7.0, 64.0, 10.0) - DVec3::from(cam_a.position)).length_squared();
            assert!((mob_seen[0].0.distance_to_camera_sq - expected_dist_a).abs() < 1e-9);
            assert_eq!(mob_seen[0].1.camera_position, cam_a.position);

            // At partial 0.75: x = 6 + 0.75 * (10 - 6) = 9.0
            assert_eq!(mob_seen[1].0.position, [9.0, 64.0, 10.0]);
            let expected_dist_b =
                (DVec3::new(9.0, 64.0, 10.0) - DVec3::from(cam_b.position)).length_squared();
            assert!((mob_seen[1].0.distance_to_camera_sq - expected_dist_b).abs() < 1e-9);
            assert_eq!(mob_seen[1].1.camera_position, cam_b.position);
        });
    });
}

#[test]
#[ignore = "daily"]
fn entity_layer_and_scene_entity_receive_their_view_and_placement() {
    if super::native_entity_refs::tests::run_fresh_cases(
        "game::client_side::frame_view_retained_draw_tests::entity_layer_and_scene_entity_receive_their_view_and_placement",
        "SUNRISE_FRAME_VIEW_LAYER_CHILD",
        &["layer-and-scene"],
    ) {
        return;
    }

    super::native_entity_refs::tests::with_client_game(|game| {
        let log = Arc::new(Mutex::new(Vec::<(EntityRenderState, ObservedView)>::new()));
        struct LayerObserver {
            log: Arc<Mutex<Vec<(EntityRenderState, ObservedView)>>>,
        }
        impl EntityRenderLayer for LayerObserver {
            fn submit(
                &self,
                entity: &EntityRenderState,
                _: Option<&dyn FrameModel>,
                draw: &mut dyn BlockEntityDrawContext,
                _: &mut dyn mod_api::StockLayerDraw,
            ) -> Result<(), String> {
                self.log
                    .lock()
                    .unwrap()
                    .push((entity.clone(), observe(draw)));
                Ok(())
            }
        }
        struct RegisterLayer {
            layer: Arc<dyn EntityRenderLayer>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for RegisterLayer {
            fn register_entity_layers(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterEntityLayersEvent<'_>,
            ) {
                event
                    .layers
                    .add(
                        LayerTarget::Entity("minecraft:zombie".into()),
                        self.layer.clone(),
                    )
                    .unwrap();
            }
        }

        let layer: Arc<dyn EntityRenderLayer> = Arc::new(LayerObserver { log: log.clone() });
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "layer_view",
                Box::new(RegisterLayer { layer }),
            )]
            .into(),
            || {
                game.resolve_entity_renderers();

                // 1. World draw in two views via registered_layer_geometry_with_view.
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mob = Mob::new(
                    88,
                    kind,
                    sunrise_core::geometry::Vec3::new(3.0, 64.0, 4.0),
                    0.0,
                );
                let layers = game.mod_layers(&mob).expect("zombie layer resolved");
                let cam_1 = test_camera([0.0, 64.0, 0.0], 0.0, 0.0, 0.0);
                let view_1 = WorldRenderView::new(
                    &cam_1,
                    Some([0.0, 65.62, 0.0]),
                    0.5,
                    10,
                    10.5,
                    false,
                    &[],
                );
                let mut out = Vec::new();
                let stock_ctx = super::native_entity_layers::StockLayerContext::none(
                    0.5,
                    DVec3::from(cam_1.position),
                    0xF0F0,
                );
                game.registered_layer_geometry_with_view(
                    &mob,
                    mob.id,
                    &layers,
                    None,
                    &stock_ctx,
                    0.5,
                    DVec3::from(cam_1.position),
                    Some(&view_1),
                    0xF0F0,
                    &mut out,
                );

                // 2. Living scene entity drawn through draw_scene: camera_entity_eye must be None.
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let scene_entity = SceneEntity::new(
                    mod_api::GuiEntity::kind("minecraft:zombie"),
                    [1.5, 1.0, 2.5],
                );
                let scene = Scene::new(vec![SceneBlock::new(
                    [0, 0, 0],
                    stone.try_into_api().unwrap(),
                    None,
                )])
                .with_entities(vec![scene_entity])
                .with_clock(SceneClock::new(9, 12, 0.25).animate_blocks(false));
                let scene_view = SceneView::new([0.5, 1.0, 0.5], 60.0, 20.0, 16.0);
                let prepared = scene_blocks(&scene.blocks).unwrap();
                let mut batch = HudBatch::new([800, 600], game.client.settings.gui_scale_inputs());
                game.native_draw_scene(
                    &mut batch,
                    [0.0, 0.0, 200.0, 200.0],
                    &prepared,
                    &scene,
                    scene_view,
                )
                .unwrap();
            },
        );

        let seen = log.lock().unwrap();
        assert_eq!(seen.len(), 2);
        // World layer draw: distance from (3,64,4) to (0,64,0) is 5 -> sq = 25.
        assert_eq!(seen[0].0.position, [3.0, 64.0, 4.0]);
        assert!((seen[0].0.distance_to_camera_sq - 25.0).abs() < 1e-9);
        assert_eq!(seen[0].1.camera_entity_eye, Some([0.0, 65.62, 0.0]));

        // Scene layer draw: camera_entity_eye is None, scene camera and clock propagate,
        // distance from (1.5, 1.0, 2.5) to scene center (0.5, 1.0, 0.5) is 1^2 + 2^2 = 5.
        assert_eq!(seen[1].0.position, [1.5, 1.0, 2.5]);
        assert!((seen[1].0.distance_to_camera_sq - 5.0).abs() < 1e-9);
        assert_eq!(seen[1].1.camera_entity_eye, None);
        assert_eq!(seen[1].1.camera_rotation, [60.0, 20.0, 0.0]);
        assert_eq!(seen[1].1.game_time, 12);
        assert_eq!(seen[1].1.partial_tick, 0.25);
        assert_eq!(seen[1].1.animation_time, 12.25);
    });
}

#[test]
#[ignore = "daily"]
fn posed_instance_group_draws_under_pose_and_rejects_stale_or_disabled() {
    super::native_entity_refs::tests::with_client_game(|game| {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let model = game
            .block_state_model(stone.try_into_api().unwrap())
            .unwrap();
        let group = game.create_group(model, 2).unwrap();
        let copy = ModelInstance::new(
            [1.0, 2.0, 0.0],
            IDENTITY_TRANSFORM,
            0xFFFF_FFFF,
            LightCoords::FULL_BRIGHT,
            Overlay::NONE,
        );
        group.write(game, 0, &[copy]).unwrap();

        let cam = test_camera([0.0, 0.0, 0.0], 0.0, 0.0, 0.0);
        let view = WorldRenderView::new(&cam, None, 0.0, 0, 0.0, false, &[]);

        // Reference draw at identity pose with copy at (1, 2, 0).
        let mut base = BlockEntityDraw::posed_with_view(
            view,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        );
        GameBlockEntityDrawContext::new(game, &mut base)
            .instance_group(group)
            .unwrap();
        let base_verts = base.into_vertices();
        assert!(!base_verts.is_empty(), "group emitted geometry");

        // Posed draw: translate by (10, 0, -5) and rotate 90 degrees about +Y.
        let pose = Mat4::from_translation(Vec3::new(10.0, 0.0, -5.0))
            * Mat4::from_quat(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        let mut posed = BlockEntityDraw::posed_with_view(
            view,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        );
        {
            let mut ctx = GameBlockEntityDrawContext::new(game, &mut posed);
            ctx.translate([10.0, 0.0, -5.0]);
            ctx.rotate_about([0.0, 1.0, 0.0], 90.0, [0.0; 3]);
            ctx.instance_group(group).unwrap();
        }
        let posed_verts = posed.into_vertices();
        assert_eq!(posed_verts.len(), base_verts.len());
        for (a, b) in base_verts.iter().zip(posed_verts.iter()) {
            let expected = pose.transform_point3(Vec3::from_array(a.pos));
            let got = Vec3::from_array(b.pos);
            assert!(
                (got - expected).length() < 1e-4,
                "posed vertex {got:?} != expected {expected:?}"
            );
        }

        // Destroyed group returns Stale without appending vertices.
        group.destroy(game).unwrap();
        let mut after_destroy = BlockEntityDraw::posed_with_view(
            view,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        );
        assert_eq!(
            GameBlockEntityDrawContext::new(game, &mut after_destroy).instance_group(group),
            Err(GroupError::Stale)
        );
        assert!(after_destroy.into_vertices().is_empty());

        // Disabled owner retires its groups.
        let owned = {
            let _entered =
                mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                    id: "disabled_group_owner",
                    hook: "test",
                    part: None,
                });
            let g = game.create_group(model, 1).unwrap();
            g.write(game, 0, &[copy]).unwrap();
            g
        };
        game.client.instance_groups.borrow_mut().retire_disabled(
            &std::collections::BTreeSet::from(["disabled_group_owner".into()]),
        );
        let mut after_disable = BlockEntityDraw::posed_with_view(
            view,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        );
        assert_eq!(
            GameBlockEntityDrawContext::new(game, &mut after_disable).instance_group(owned),
            Err(GroupError::Stale)
        );
        assert!(after_disable.into_vertices().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn prepare_item_and_prepared_item_match_draw_item_and_invalidate() {
    super::native_entity_refs::tests::with_client_game(|game| {
        let host: &dyn ClientHost = game;
        let air = ItemStack::of(
            host.item_key(&Identifier::parse("minecraft:air").unwrap())
                .unwrap(),
        );
        assert_eq!(
            host.prepare_item(&air, ItemDisplayContext::Fixed, 0),
            Err(ModelError::UnknownModel)
        );

        let stone_key = host
            .item_key(&Identifier::parse("minecraft:stone").unwrap())
            .unwrap();
        let stack = ItemStack::of(stone_key);
        let prepared = host
            .prepare_item(&stack, ItemDisplayContext::Fixed, 7)
            .unwrap();

        let cam = test_camera([0.0, 0.0, 0.0], 0.0, 0.0, 0.0);
        let view = WorldRenderView::new(&cam, None, 0.0, 0, 0.0, false, &[]);

        for unlit in [false, true] {
            let mut expected_draw = BlockEntityDraw::posed_with_view(
                view,
                0xC080,
                [0; 3],
                Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
                diffuse_lights(false),
                Vec::new(),
            );
            {
                let mut ctx = GameBlockEntityDrawContext::new(game, &mut expected_draw);
                ctx.unlit(unlit);
                assert!(ctx.item(&stack, ItemDisplayContext::Fixed, None, 7));
            }
            let mut got_draw = BlockEntityDraw::posed_with_view(
                view,
                0xC080,
                [0; 3],
                Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
                diffuse_lights(false),
                Vec::new(),
            );
            {
                let mut ctx = GameBlockEntityDrawContext::new(game, &mut got_draw);
                ctx.unlit(unlit);
                ctx.prepared_item(prepared).unwrap();
            }
            let expected = expected_draw.into_vertices();
            let got = got_draw.into_vertices();
            assert!(!expected.is_empty());
            assert_eq!(got.len(), expected.len());
            for (g, e) in got.iter().zip(expected.iter()) {
                assert_eq!(g.pos, e.pos);
                assert_eq!(g.uv, e.uv);
                assert_eq!(g.color, e.color);
                assert_eq!(g.light, e.light);
                assert_eq!(g.normal, e.normal);
            }
        }

        // Destroy invalidates the handle and a second destroy fails with StaleModel.
        prepared.destroy(game).unwrap();
        assert_eq!(prepared.destroy(game), Err(ModelError::StaleModel));
        let mut stale_draw = BlockEntityDraw::posed_with_view(
            view,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        );
        assert_eq!(
            GameBlockEntityDrawContext::new(game, &mut stale_draw).prepared_item(prepared),
            Err(ModelError::StaleModel)
        );
        assert!(stale_draw.into_vertices().is_empty());

        // Reload (clearing prepared items) invalidates handles.
        let reloaded = host
            .prepare_item(&stack, ItemDisplayContext::Ground, 0)
            .unwrap();
        game.client.prepared_items.borrow_mut().clear();
        let mut reload_draw = BlockEntityDraw::posed_with_view(
            view,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        );
        assert_eq!(
            GameBlockEntityDrawContext::new(game, &mut reload_draw).prepared_item(reloaded),
            Err(ModelError::StaleModel)
        );
        assert!(reload_draw.into_vertices().is_empty());

        // Owner mod disable retires only that mod's prepared items.
        let owned = {
            let _entered =
                mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                    id: "disabled_item_owner",
                    hook: "test",
                    part: None,
                });
            host.prepare_item(&stack, ItemDisplayContext::Fixed, 0)
                .unwrap()
        };
        let surviving = host
            .prepare_item(&stack, ItemDisplayContext::Fixed, 0)
            .unwrap();
        game.client
            .prepared_items
            .borrow_mut()
            .retire_disabled(&std::collections::BTreeSet::from([
                "disabled_item_owner".into()
            ]));
        let mut disable_draw = BlockEntityDraw::posed_with_view(
            view,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        );
        assert_eq!(
            GameBlockEntityDrawContext::new(game, &mut disable_draw).prepared_item(owned),
            Err(ModelError::StaleModel)
        );
        assert!(disable_draw.into_vertices().is_empty());

        let mut survivor_draw = BlockEntityDraw::posed_with_view(
            view,
            0xF0F0,
            [0; 3],
            Mat4::IDENTITY,
            diffuse_lights(false),
            Vec::new(),
        );
        GameBlockEntityDrawContext::new(game, &mut survivor_draw)
            .prepared_item(surviving)
            .unwrap();
        assert!(!survivor_draw.into_vertices().is_empty());
    });
}
