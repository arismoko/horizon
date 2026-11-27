//! `register_entity_layers` on the client (#860 stage 2): the event tells what renderers this load
//! built and refuses a target that is no living entity type by name; a living type's layers draw
//! over it in the order they were added, a player's are its model's, a type with none draws none,
//! an `Err` skips only that draw, and a panic disables only that layer for the session.
#![cfg(feature = "client")]
use mod_api::{
    ArmorLayerPass, ArmorLayerSlot, BlockEntityDrawContext, EntityLayers, EntityRenderLayer,
    EntityRenderState, EntityRenderer, FrameModel, Hand, Hooks, LayerTarget, PlayerModel,
    RegisterEntityLayersEvent, RegistrationContext, StockLayer, StockLayerDraw,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use sunrise_client::mesher::Vertex;
use sunrise_client_gameplay::game::native_entity_layers::{LayerRegistration, StockLayerContext};
use sunrise_client_gameplay::game::native_entity_renderers::RegisteredEntityRenderer;
use sunrise_gameplay_common::game::entity_type_rows::EntityTypeRows;
use sunrise_sim::mobs::Mob;

/// A load's entity types: a boat, a zombie, an armour stand, the ender dragon, the player and a mod's
/// living type.
fn rows() -> EntityTypeRows {
    EntityTypeRows::parse(
        br#"{"minecraft:oak_boat": {"width": 1.375},
             "minecraft:zombie": {"attributes": {}},
             "minecraft:armor_stand": {"attributes": {}},
             "minecraft:ender_dragon": {"attributes": {}},
             "minecraft:player": {"attributes": {}},
             "ruby:golem": {"template": "minecraft:zombie", "attributes": {}}}"#,
    )
}

/// The living types drawn as mobs, where layers run: not the armour stand or the ender dragon.
fn routed(id: &str) -> bool {
    ["minecraft:zombie", "ruby:golem"].contains(&id)
}

/// Draws one stone quad a block wide, `height` blocks above the entity's feet; refuses its first
/// `refuse` calls with an `Err`. `calls` counts the calls.
struct Raised {
    pub height: f32,
    refuse: u32,
    calls: AtomicU32,
    /// The entity id of the last call.
    id: AtomicU32,
}
impl Raised {
    fn new(height: f32) -> Arc<Self> {
        Self::refusing(height, 0)
    }
    fn refusing(height: f32, refuse: u32) -> Arc<Self> {
        Arc::new(Self {
            height,
            refuse,
            calls: AtomicU32::new(0),
            id: AtomicU32::new(u32::MAX),
        })
    }
    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}
impl EntityRenderLayer for Raised {
    fn submit(
        &self,
        entity: &EntityRenderState,
        _: Option<&dyn FrameModel>,
        draw: &mut dyn BlockEntityDrawContext,
        _: &mut dyn StockLayerDraw,
    ) -> Result<(), String> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        self.id.store(entity.id, Ordering::Relaxed);
        let (low, high) = (self.height, self.height + 1.0);
        let corners = [
            [0.0, high, 0.0],
            [0.0, low, 0.0],
            [1.0, low, 0.0],
            [1.0, high, 0.0],
        ];
        draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
        if call < self.refuse {
            return Err("refused this frame".into());
        }
        Ok(())
    }
}

/// Draws one stone quad a block wide, `height` blocks above the entity's feet, then panics.
struct PanickingLayer {
    height: f32,
    calls: AtomicU32,
}
impl PanickingLayer {
    fn new(height: f32) -> Arc<Self> {
        Arc::new(Self {
            height,
            calls: AtomicU32::new(0),
        })
    }
    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}
impl EntityRenderLayer for PanickingLayer {
    fn submit(
        &self,
        _: &EntityRenderState,
        _: Option<&dyn FrameModel>,
        draw: &mut dyn BlockEntityDrawContext,
        _: &mut dyn StockLayerDraw,
    ) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let (low, high) = (self.height, self.height + 1.0);
        let corners = [
            [0.0, high, 0.0],
            [0.0, low, 0.0],
            [1.0, low, 0.0],
            [1.0, high, 0.0],
        ];
        draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
        panic!("panicking entity layer");
    }
}

#[test]
fn the_event_tells_the_renderers_this_load_built() {
    let rows = rows();
    let mut renderers: BTreeMap<String, RegisteredEntityRenderer> = BTreeMap::new();
    struct Nothing;
    impl EntityRenderer for Nothing {
        fn submit(
            &self,
            _: &EntityRenderState,
            _: &mut dyn BlockEntityDrawContext,
        ) -> Result<(), String> {
            Ok(())
        }
    }
    renderers.insert(
        "ruby:golem".into(),
        RegisteredEntityRenderer::new(Arc::new(Nothing), None),
    );
    renderers.insert(
        "nope:ghost".into(),
        RegisteredEntityRenderer::new(Arc::new(Nothing), None),
    );
    let stock = BTreeMap::from([
        (
            "minecraft:zombie".into(),
            vec![
                StockLayer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Base),
                StockLayer::HeldItem(Hand::Main),
            ],
        ),
        (
            "ruby:golem".into(),
            vec![StockLayer::Armor(
                ArmorLayerSlot::Head,
                ArmorLayerPass::Base,
            )],
        ),
    ]);
    let event = LayerRegistration::new(&rows, &renderers, &routed, &stock);
    let types: &[String] = event.entity_types();
    assert!(std::ptr::eq(types, rows.non_player_ids.as_slice()));
    assert_eq!(
        types,
        [
            "minecraft:armor_stand",
            "minecraft:ender_dragon",
            "minecraft:oak_boat",
            "minecraft:zombie",
            "ruby:golem"
        ]
        .as_slice()
    );
    assert!(
        event.is_living("minecraft:armor_stand"),
        "living, whatever its route"
    );
    assert!(event.is_living("minecraft:zombie"));
    assert!(event.is_living("ruby:golem"));
    assert!(!event.is_living("minecraft:oak_boat"));
    assert!(!event.is_living("minecraft:player"));
    assert!(!event.is_living("nope:ghost"));
    let models: &[PlayerModel] = event.player_models();
    assert_eq!(models, [PlayerModel::Wide, PlayerModel::Slim].as_slice());
    assert!(event.has_mod_renderer("ruby:golem"));
    assert!(!event.has_mod_renderer("minecraft:zombie"));
    assert!(!event.has_mod_renderer("nope:ghost"), "no entity type");
    assert_eq!(
        event.stock_layers(&LayerTarget::Entity("minecraft:zombie".into())),
        stock["minecraft:zombie"].as_slice()
    );
    assert!(
        event
            .stock_layers(&LayerTarget::Entity("ruby:golem".into()))
            .is_empty(),
        "a mod-replaced renderer has no stock layers"
    );
    for empty in [
        "minecraft:oak_boat",
        "minecraft:player",
        "minecraft:armor_stand",
        "nope:ghost",
    ] {
        assert!(
            event
                .stock_layers(&LayerTarget::Entity(empty.into()))
                .is_empty(),
            "{empty}"
        );
    }
    for model in [PlayerModel::Wide, PlayerModel::Slim] {
        let player_layers = event.stock_layers(&LayerTarget::Player(model));
        assert!(player_layers.contains(&StockLayer::Cape));
        assert!(player_layers.contains(&StockLayer::Wings));
        assert!(player_layers.contains(&StockLayer::Armor(
            ArmorLayerSlot::Chest,
            ArmorLayerPass::Base
        )));
        assert!(!player_layers.contains(&StockLayer::SnowGolemHead));
    }
}

#[test]
fn a_target_that_is_no_living_entity_type_is_refused_by_name() {
    let rows = rows();
    let renderers = BTreeMap::new();
    let stock = BTreeMap::from([(
        "minecraft:zombie".into(),
        vec![StockLayer::Armor(
            ArmorLayerSlot::Chest,
            ArmorLayerPass::Base,
        )],
    )]);
    let mut event = LayerRegistration::new(&rows, &renderers, &routed, &stock);
    let mut add = |target: &str| event.add(LayerTarget::Entity(target.into()), Raised::new(1.0));
    assert_eq!(
        add("minecraft:oak_boat"),
        Err("the entity layer for minecraft:oak_boat is refused: its renderer is not a living entity's, and only those take layers".into())
    );
    assert_eq!(
        add("nope:ghost"),
        Err("the entity layer for nope:ghost is refused: it is no entity type".into())
    );
    assert_eq!(
        add("minecraft:player"),
        Err("the entity layer for minecraft:player is refused: a player takes layers through its model (wide or slim)".into())
    );
    assert_eq!(
        add("minecraft:armor_stand"),
        Err("the entity layer for minecraft:armor_stand is refused: it draws through a route of its own, which does not run layers yet".into())
    );
    assert_eq!(
        add("minecraft:ender_dragon"),
        Err("the entity layer for minecraft:ender_dragon is refused: it draws through a route of its own, which does not run layers yet".into())
    );
    assert_eq!(add("minecraft:zombie"), Ok(()));
    assert_eq!(add("ruby:golem"), Ok(()), "a mod's living type");
    assert_eq!(
        event.add(LayerTarget::Player(PlayerModel::Slim), Raised::new(1.0)),
        Ok(())
    );
}

#[test]
fn hiding_stock_layers_validates_target_and_supported_layers() {
    let rows = rows();
    let mut renderers = BTreeMap::new();
    struct Nothing;
    impl EntityRenderer for Nothing {
        fn submit(
            &self,
            _: &EntityRenderState,
            _: &mut dyn BlockEntityDrawContext,
        ) -> Result<(), String> {
            Ok(())
        }
    }
    renderers.insert(
        "ruby:golem".into(),
        RegisteredEntityRenderer::new(Arc::new(Nothing), None),
    );
    let stock = BTreeMap::from([
        (
            "minecraft:zombie".into(),
            vec![StockLayer::Armor(
                ArmorLayerSlot::Chest,
                ArmorLayerPass::Base,
            )],
        ),
        (
            "ruby:golem".into(),
            vec![StockLayer::Armor(
                ArmorLayerSlot::Chest,
                ArmorLayerPass::Base,
            )],
        ),
    ]);
    let mut event = LayerRegistration::new(&rows, &renderers, &routed, &stock);
    assert_eq!(
        event.hide(
            LayerTarget::Entity("minecraft:zombie".into()),
            StockLayer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Base)
        ),
        Ok(())
    );
    assert_eq!(
        event.hide(
            LayerTarget::Player(PlayerModel::Wide),
            StockLayer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Base)
        ),
        Ok(())
    );
    assert!(
        event
            .hide(
                LayerTarget::Player(PlayerModel::Wide),
                StockLayer::SnowGolemHead
            )
            .is_err(),
        "players have no SnowGolemHead layer"
    );
    assert!(
        event
            .hide(
                LayerTarget::Entity("minecraft:zombie".into()),
                StockLayer::SnowGolemHead
            )
            .is_err(),
        "zombies have no SnowGolemHead layer"
    );
    assert!(
        event
            .hide(
                LayerTarget::Entity("ruby:golem".into()),
                StockLayer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Base)
            )
            .is_err(),
        "mod-replaced renderers have no stock layers"
    );
    for bad in [
        "minecraft:oak_boat",
        "minecraft:player",
        "minecraft:armor_stand",
        "nope:ghost",
    ] {
        assert!(
            event
                .hide(LayerTarget::Entity(bad.into()), StockLayer::CustomHead)
                .is_err(),
            "{bad}"
        );
    }
}

type LayerList = Vec<(LayerTarget, Arc<dyn EntityRenderLayer>)>;

/// The layers the scene's mods initially add: the first mod `zombie` 100 blocks up, `wide`
/// 300 up, `slim` 400 up, `pig` (refusing its first call) 500 up, and on `sheep` a panicking layer
/// at 600 up followed by a healthy sibling at 650 up; the second mod `second` on `zombie` 200 up
/// and `sheep_other` on `sheep` 700 up.
struct Scene {
    first_layers: Arc<Mutex<LayerList>>,
    pub zombie: Arc<Raised>,
    second: Arc<Raised>,
    wide: Arc<Raised>,
    slim: Arc<Raised>,
    pig: Arc<Raised>,
    sheep_panic: Arc<PanickingLayer>,
    sheep_sibling: Arc<Raised>,
    sheep_other: Arc<Raised>,
}

/// The camera, south of the entity and looking at it.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);
/// Where the scene's entity stands.
const FEET: [f64; 3] = [0.5, 64.0, 0.5];
/// Full light, as the entity pass packs it.
const LIGHT: u32 = 0x00F0_00F0;

/// How high above the entity's feet a vertex is, in blocks.
fn above(v: &Vertex) -> f32 {
    v.pos[1] - (FEET[1] - CAMERA.y) as f32
}

/// The heights the quads `out` holds start at, in order.
fn quads(out: &[Vertex]) -> Vec<f32> {
    out.chunks(4)
        .map(|quad| quad.iter().map(above).fold(f32::MAX, f32::min).round())
        .collect()
}

/// A mob of `kind` (a player of the slim model when `slim`), standing at [`FEET`].
fn mob(game: &sunrise_client_gameplay::game::ClientGame<'_>, kind: &str, slim: bool) -> Mob {
    let kind = if kind == "minecraft:player" {
        game.client_res().player_type().clone().unwrap()
    } else {
        game.res.mob_types().get(kind).unwrap().clone()
    };
    let at = sunrise_core::geometry::Vec3::new(FEET[0], FEET[1], FEET[2]);
    let mut mob = Mob::new(90, kind, at, 0.0);
    mob.slim = slim;
    mob
}

/// The mods' layers one frame draws over a mob of `kind` (a player of the slim model when
/// `slim`).
fn layer_frame(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    kind: &str,
    slim: bool,
) -> Vec<Vertex> {
    let mut out = Vec::new();
    let mob = mob(game, kind, slim);
    let layers = game.mod_layers(&mob);
    let ctx = StockLayerContext::none(1.0, CAMERA, LIGHT);
    game.mod_layer_geometry(&mob, None, None, layers, &ctx, 1.0, CAMERA, LIGHT, &mut out);
    out
}

/// Runs `check` on a client game with prepared resources and the [`Scene`]'s mods, resolved as
/// the first load does. In a child process running only `test`: the registrations of a process
/// are its own.
fn in_scene(
    test: &str,
    check: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &Scene),
) {
    if std::env::var_os("SUNRISE_ENTITY_LAYERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("cut_layout::game::native_entity_layers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_ENTITY_LAYERS_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use mod_api::{Hooks, RegisterEntityLayersEvent, RegistrationContext};
    let scene = Scene {
        first_layers: Arc::default(),
        zombie: Raised::new(100.0),
        second: Raised::new(200.0),
        wide: Raised::new(300.0),
        slim: Raised::new(400.0),
        pig: Raised::refusing(500.0, 1),
        sheep_panic: PanickingLayer::new(600.0),
        sheep_sibling: Raised::new(650.0),
        sheep_other: Raised::new(700.0),
    };
    /// A mod adding `layers` on every load.
    struct Layers(Arc<Mutex<LayerList>>);
    #[mod_api::__subscriptions]
    impl Hooks for Layers {
        fn register_entity_layers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterEntityLayersEvent<'_>,
        ) {
            for (target, layer) in self.0.lock().unwrap().iter() {
                event.layers.add(target.clone(), layer.clone()).unwrap();
            }
        }
    }
    let entity = |id: &str| LayerTarget::Entity(id.into());
    *scene.first_layers.lock().unwrap() = vec![
        (entity("minecraft:sheep"), scene.sheep_panic.clone()),
        (entity("minecraft:sheep"), scene.sheep_sibling.clone()),
        (
            entity("minecraft:zombie"),
            scene.zombie.clone() as Arc<dyn EntityRenderLayer>,
        ),
        (
            LayerTarget::Player(PlayerModel::Wide),
            scene.wide.clone() as Arc<dyn EntityRenderLayer>,
        ),
        (
            LayerTarget::Player(PlayerModel::Slim),
            scene.slim.clone() as Arc<dyn EntityRenderLayer>,
        ),
        (
            entity("minecraft:pig"),
            scene.pig.clone() as Arc<dyn EntityRenderLayer>,
        ),
    ];
    let first = Layers(scene.first_layers.clone());
    let second = Layers(Arc::new(Mutex::new(vec![
        (
            entity("minecraft:zombie"),
            scene.second.clone() as Arc<dyn EntityRenderLayer>,
        ),
        (
            entity("minecraft:sheep"),
            scene.sheep_other.clone() as Arc<dyn EntityRenderLayer>,
        ),
    ])));
    let mods = vec![
        mod_api::ModEntry::new("first", Box::new(first)),
        mod_api::ModEntry::new("second", Box::new(second)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.resolve_entity_renderers();
            check(game, &scene);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_living_type_draws_its_layers_in_the_order_they_were_added() {
    in_scene(
        "a_living_type_draws_its_layers_in_the_order_they_were_added",
        |game, scene| {
            let zombie = layer_frame(game, "minecraft:zombie", false);
            assert_eq!(
                quads(&zombie),
                [100.0, 200.0],
                "the first mod's, then the second's"
            );
            assert_eq!((scene.zombie.calls(), scene.second.calls()), (1, 1));
            assert!(
                layer_frame(game, "minecraft:cow", false).is_empty(),
                "a type with none"
            );
        },
    );
}

/// The one lookup a mob gets a frame finds its own type's or player model's layers and nothing for a
/// type without: what decides whether the draw keeps its final model, and what draws over it.
#[test]
#[ignore = "daily"]
fn a_mob_looks_up_only_its_own_types_or_models_layers() {
    in_scene(
        "a_mob_looks_up_only_its_own_types_or_models_layers",
        |game, _| {
            let count = |kind: &str, slim: bool| {
                game.mod_layers(&mob(game, kind, slim))
                    .map(|layers| layers.len())
            };
            assert_eq!(count("minecraft:zombie", false), Some(2));
            assert_eq!(
                count("minecraft:cow", false),
                None,
                "no parts kept, nothing drawn"
            );
            assert_eq!(
                count("minecraft:player", false),
                Some(1),
                "the wide model's"
            );
            assert_eq!(count("minecraft:player", true), Some(1), "the slim model's");
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_player_draws_the_layers_of_its_model() {
    in_scene("a_player_draws_the_layers_of_its_model", |game, scene| {
        assert_eq!(
            quads(&layer_frame(game, "minecraft:player", false)),
            [300.0]
        );
        assert_eq!(quads(&layer_frame(game, "minecraft:player", true)), [400.0]);
        assert_eq!((scene.wide.calls(), scene.slim.calls()), (1, 1));
        assert_eq!(
            scene.zombie.calls(),
            0,
            "the zombie's layers are not a player's"
        );
    });
}

#[test]
#[ignore = "daily"]
fn this_windows_player_is_told_its_own_entity_id() {
    in_scene(
        "this_windows_player_is_told_its_own_entity_id",
        |game, scene| {
            // This window's own entity id, as the server gave it.
            match game.client.remote.as_mut() {
                Some(remote) => remote.entity_id = 4321,
                None => {
                    let at = sunrise_core::geometry::Vec3::ZERO;
                    let mut player =
                        sunrise_sim::motion::PlayerMotion::new(game.motion, at).unwrap();
                    player.entity_id = 4321;
                    game.player = Some(player);
                }
            }
            let own = u32::try_from(game.own_entity_id()).unwrap();
            assert_eq!(own, 4321);
            // Drawn from a model made for the frame, with id 0, as `tick_player_model` makes it.
            let mut model = mob(game, "minecraft:player", false);
            model.id = 0;
            let who = sunrise_client_gameplay::game::native_mob_draw::PlayerRender {
                name: "own",
                local: true,
            };
            let layers = game.mod_layers(&model);
            let mut out = Vec::new();
            let ctx = StockLayerContext::none(1.0, CAMERA, LIGHT);
            game.mod_layer_geometry(
                &model,
                Some(who),
                None,
                layers,
                &ctx,
                1.0,
                CAMERA,
                LIGHT,
                &mut out,
            );
            assert_eq!(scene.wide.id.load(Ordering::Relaxed), own);
            // Another player keeps its own mob's id.
            let other = mob(game, "minecraft:player", false);
            let layers = game.mod_layers(&other);
            let who = sunrise_client_gameplay::game::native_mob_draw::PlayerRender {
                name: "other",
                local: false,
            };
            game.mod_layer_geometry(
                &other,
                Some(who),
                None,
                layers,
                &ctx,
                1.0,
                CAMERA,
                LIGHT,
                &mut out,
            );
            assert_eq!(scene.wide.id.load(Ordering::Relaxed), 90);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_failing_layer_skips_only_that_frame_and_stays_active() {
    in_scene(
        "a_failing_layer_skips_only_that_frame_and_stays_active",
        |game, scene| {
            let _ = mod_api::__private::take_hook_panics();
            assert!(
                layer_frame(game, "minecraft:pig", false).is_empty(),
                "the failing frame drops only its own geometry"
            );
            assert_eq!(scene.pig.calls(), 1);
            assert!(
                mod_api::__private::take_hook_panics().is_empty(),
                "an Err return never queues a panic notice"
            );
            assert_eq!(
                quads(&layer_frame(game, "minecraft:pig", false)),
                [500.0],
                "the next frame asks the layer again without waiting for reload"
            );
            assert_eq!(scene.pig.calls(), 2);
            sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload(game);
            assert_eq!(quads(&layer_frame(game, "minecraft:pig", false)), [500.0]);
            assert_eq!(
                scene.pig.calls(),
                3,
                "the reload replaced and kept it active"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_panicking_layer_stays_disabled_across_reload_without_affecting_siblings() {
    in_scene(
        "a_panicking_layer_stays_disabled_across_reload_without_affecting_siblings",
        |game, scene| {
            let _ = mod_api::__private::take_hook_panics();
            assert_eq!(
                quads(&layer_frame(game, "minecraft:sheep", false)),
                [650.0, 700.0],
                "the panicking layer rolls back while siblings from the same and other mods draw"
            );
            assert_eq!(scene.sheep_panic.calls(), 1);
            assert_eq!(
                (scene.sheep_sibling.calls(), scene.sheep_other.calls()),
                (1, 1)
            );
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "first",
                    hook: "EntityRenderLayer::submit",
                }],
                "the shared journal records the mod and retained operation once"
            );

            assert_eq!(
                quads(&layer_frame(game, "minecraft:sheep", false)),
                [650.0, 700.0],
                "disabled on subsequent frames while siblings continue"
            );
            assert_eq!(scene.sheep_panic.calls(), 1);
            assert_eq!(
                (scene.sheep_sibling.calls(), scene.sheep_other.calls()),
                (2, 2)
            );
            assert!(mod_api::__private::take_hook_panics().is_empty());

            // The same callbacks now take each other's slots on the sheep renderer.
            scene.first_layers.lock().unwrap().swap(0, 1);
            sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload(game);
            assert_eq!(
                quads(&layer_frame(game, "minecraft:sheep", false)),
                [650.0, 700.0],
                "the healthy layer takes the old slot without inheriting poison"
            );
            assert_eq!(
                scene.sheep_panic.calls(),
                1,
                "the moved callback stays disabled"
            );
            assert_eq!(
                (scene.sheep_sibling.calls(), scene.sheep_other.calls()),
                (3, 3)
            );
            assert!(mod_api::__private::take_hook_panics().is_empty());

            let replacement = Raised::new(625.0);
            scene.first_layers.lock().unwrap()[1].1 = replacement.clone();
            sunrise_client_gameplay::game::client_side::model_baking_completed_tests::reload(game);
            assert_eq!(
                quads(&layer_frame(game, "minecraft:sheep", false)),
                [650.0, 625.0, 700.0],
                "a fresh callback in the disabled one's slot draws in registration order"
            );
            assert_eq!(replacement.calls(), 1);
            assert_eq!(scene.sheep_panic.calls(), 1);
            assert!(mod_api::__private::take_hook_panics().is_empty());
        },
    );
}

type VertexKey = ([u32; 3], [u32; 2], u32, u32, u32, [i8; 4], [u32; 2]);

fn vertex_keys(verts: &[Vertex]) -> Vec<VertexKey> {
    verts
        .iter()
        .map(|v| {
            (
                v.pos.map(f32::to_bits),
                v.uv.map(f32::to_bits),
                v.layer,
                v.color,
                v.light,
                v.normal,
                v.glint_uv.map(f32::to_bits),
            )
        })
        .collect()
}

fn draw_single_mob(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    mob: Mob,
) -> Vec<Vertex> {
    game.mobs.clear();
    game.mobs.push(mob);
    game.prepare_entity_culling();
    let mut out = Vec::new();
    game.mob_geometry(CAMERA, 1.0, &mut out);
    out
}

fn in_child(test: &str, check: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    if std::env::var_os("SUNRISE_ENTITY_LAYERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("cut_layout::game::native_entity_layers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("SUNRISE_ENTITY_LAYERS_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    crate::game::server_side::native_entity_refs::tests::with_client_game(check);
}

fn wide_player(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    id: u32,
    chestplate: bool,
) -> Mob {
    let kind = game.client_res().player_type().clone().unwrap();
    let at = sunrise_core::geometry::Vec3::new(FEET[0], FEET[1], FEET[2]);
    let mut m = Mob::new(id, kind, at, 0.0);
    m.slim = false;
    if chestplate {
        m.extra.set_slot_stack(
            sunrise_items::equipment::CHEST,
            Some(sunrise_items::items::ItemStack::new(
                "minecraft:diamond_chestplate",
                1,
            )),
        );
    }
    m
}

#[test]
#[ignore = "daily"]
fn hiding_snow_golem_head_removes_the_carved_pumpkin_and_lists_it() {
    in_child(
        "hiding_snow_golem_head_removes_the_carved_pumpkin_and_lists_it",
        |game| {
            let snow_golem = |game: &sunrise_client_gameplay::game::ClientGame<'_>,
                              sheared: bool| {
                let mut m = mob(game, "minecraft:snow_golem", false);
                m.extra.sheared = sheared;
                m
            };
            let with_pumpkin = draw_single_mob(game, snow_golem(game, false));
            let sheared_only = draw_single_mob(game, snow_golem(game, true));
            assert!(
                with_pumpkin.len() > sheared_only.len(),
                "unsheared snow golem draws its carved pumpkin"
            );

            #[derive(Default)]
            struct Observed {
                snow_golem: Vec<StockLayer>,
                zombie: Vec<StockLayer>,
                horse: Vec<StockLayer>,
                player_wide: Vec<StockLayer>,
            }
            struct HidePumpkin(Arc<Mutex<Observed>>);
            #[mod_api::__subscriptions]
            impl Hooks for HidePumpkin {
                fn register_entity_layers(
                    &self,
                    _: &RegistrationContext,
                    event: &mut RegisterEntityLayersEvent<'_>,
                ) {
                    let mut obs = self.0.lock().unwrap();
                    obs.snow_golem = event
                        .layers
                        .stock_layers(&LayerTarget::Entity("minecraft:snow_golem".into()))
                        .to_vec();
                    obs.zombie = event
                        .layers
                        .stock_layers(&LayerTarget::Entity("minecraft:zombie".into()))
                        .to_vec();
                    obs.horse = event
                        .layers
                        .stock_layers(&LayerTarget::Entity("minecraft:horse".into()))
                        .to_vec();
                    obs.player_wide = event
                        .layers
                        .stock_layers(&LayerTarget::Player(PlayerModel::Wide))
                        .to_vec();
                    event
                        .layers
                        .hide(
                            LayerTarget::Entity("minecraft:snow_golem".into()),
                            StockLayer::SnowGolemHead,
                        )
                        .unwrap();
                }
            }
            let observed = Arc::new(Mutex::new(Observed::default()));
            let mods = vec![mod_api::ModEntry::new(
                "hide_pumpkin",
                Box::new(HidePumpkin(observed.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                game.resolve_entity_renderers();
                let obs = observed.lock().unwrap();
                assert!(obs.snow_golem.contains(&StockLayer::SnowGolemHead));
                assert!(!obs.zombie.contains(&StockLayer::SnowGolemHead));
                assert!(obs.zombie.contains(&StockLayer::Armor(
                    ArmorLayerSlot::Chest,
                    ArmorLayerPass::Base
                )));
                assert!(obs.zombie.contains(&StockLayer::HeldItem(Hand::Main)));
                assert!(obs.zombie.contains(&StockLayer::CustomHead));
                assert!(obs.horse.is_empty());
                assert!(obs.player_wide.contains(&StockLayer::Cape));
                assert!(obs.player_wide.contains(&StockLayer::Wings));
                assert!(obs.player_wide.contains(&StockLayer::Arrow));
                assert!(obs.player_wide.contains(&StockLayer::BeeStinger));
                drop(obs);

                let hidden_pumpkin = draw_single_mob(game, snow_golem(game, false));
                assert_eq!(
                    vertex_keys(&hidden_pumpkin),
                    vertex_keys(&sheared_only),
                    "hiding SnowGolemHead removes only the carved pumpkin"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn hiding_player_chestplate_and_redrawing_conditionally_matches_baseline() {
    in_child(
        "hiding_player_chestplate_and_redrawing_conditionally_matches_baseline",
        |game| {
            const HIDE_ID: u32 = 201;
            const KEEP_ID: u32 = 202;
            let bare_hidden = draw_single_mob(game, wide_player(game, HIDE_ID, false));
            let baseline_kept = draw_single_mob(game, wide_player(game, KEEP_ID, true));
            let bare_kept = draw_single_mob(game, wide_player(game, KEEP_ID, false));
            assert!(
                baseline_kept.len() > bare_kept.len(),
                "wearing a diamond chestplate emits armour vertices"
            );

            struct ConditionalChestplate {
                keep_id: u32,
            }
            impl EntityRenderLayer for ConditionalChestplate {
                fn submit(
                    &self,
                    entity: &EntityRenderState,
                    _: Option<&dyn FrameModel>,
                    _: &mut dyn BlockEntityDrawContext,
                    stock: &mut dyn StockLayerDraw,
                ) -> Result<(), String> {
                    if entity.id == self.keep_id {
                        stock.draw_stock_layer(StockLayer::Armor(
                            ArmorLayerSlot::Chest,
                            ArmorLayerPass::Base,
                        ))?;
                        stock.draw_stock_layer(StockLayer::Armor(
                            ArmorLayerSlot::Chest,
                            ArmorLayerPass::Glint,
                        ))?;
                        stock.draw_stock_layer(StockLayer::Armor(
                            ArmorLayerSlot::Chest,
                            ArmorLayerPass::Trim,
                        ))?;
                    }
                    Ok(())
                }
            }
            struct HideChestplate;
            #[mod_api::__subscriptions]
            impl Hooks for HideChestplate {
                fn register_entity_layers(
                    &self,
                    _: &RegistrationContext,
                    event: &mut RegisterEntityLayersEvent<'_>,
                ) {
                    for model in [PlayerModel::Wide, PlayerModel::Slim] {
                        let target = LayerTarget::Player(model);
                        for pass in [
                            ArmorLayerPass::Base,
                            ArmorLayerPass::Glint,
                            ArmorLayerPass::Trim,
                        ] {
                            event
                                .layers
                                .hide(
                                    target.clone(),
                                    StockLayer::Armor(ArmorLayerSlot::Chest, pass),
                                )
                                .unwrap();
                        }
                        event
                            .layers
                            .add(target, Arc::new(ConditionalChestplate { keep_id: KEEP_ID }))
                            .unwrap();
                    }
                }
            }
            let mods = vec![mod_api::ModEntry::new(
                "conditional_armor",
                Box::new(HideChestplate),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                game.resolve_entity_renderers();
                let drawn_hidden = draw_single_mob(game, wide_player(game, HIDE_ID, true));
                assert_eq!(
                    vertex_keys(&drawn_hidden),
                    vertex_keys(&bare_hidden),
                    "player {HIDE_ID} draws no chestplate vertices"
                );
                let drawn_kept = draw_single_mob(game, wide_player(game, KEEP_ID, true));
                assert_eq!(
                    vertex_keys(&drawn_kept),
                    vertex_keys(&baseline_kept),
                    "player {KEEP_ID} redraws the chestplate identically to the unmodded baseline"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn refused_stock_hides_and_draws_and_rollback_on_layer_failure() {
    in_child(
        "refused_stock_hides_and_draws_and_rollback_on_layer_failure",
        |game| {
            const PROBE_ID: u32 = 301;
            const ERR_ID: u32 = 302;
            const PANIC_ID: u32 = 303;
            let bare_probe = draw_single_mob(game, wide_player(game, PROBE_ID, false));
            let bare_err = draw_single_mob(game, wide_player(game, ERR_ID, false));
            let bare_panic = draw_single_mob(game, wide_player(game, PANIC_ID, false));

            #[derive(Default)]
            struct Results {
                hide_zombie_pumpkin: Option<Result<(), String>>,
                hide_player_pumpkin: Option<Result<(), String>>,
                draw_unhidden_cape: Option<Result<(), String>>,
                draw_unsupported_pumpkin: Option<Result<(), String>>,
            }
            struct ProbingLayer(Arc<Mutex<Results>>);
            impl EntityRenderLayer for ProbingLayer {
                fn submit(
                    &self,
                    entity: &EntityRenderState,
                    _: Option<&dyn FrameModel>,
                    draw: &mut dyn BlockEntityDrawContext,
                    stock: &mut dyn StockLayerDraw,
                ) -> Result<(), String> {
                    match entity.id {
                        PROBE_ID => {
                            let mut r = self.0.lock().unwrap();
                            r.draw_unhidden_cape = Some(stock.draw_stock_layer(StockLayer::Cape));
                            r.draw_unsupported_pumpkin =
                                Some(stock.draw_stock_layer(StockLayer::SnowGolemHead));
                            Ok(())
                        }
                        ERR_ID => {
                            stock.draw_stock_layer(StockLayer::Armor(
                                ArmorLayerSlot::Chest,
                                ArmorLayerPass::Base,
                            ))?;
                            draw.quad(
                                "block/stone",
                                [
                                    [0.0, 101.0, 0.0],
                                    [0.0, 100.0, 0.0],
                                    [1.0, 100.0, 0.0],
                                    [1.0, 101.0, 0.0],
                                ],
                                [0.0, 0.0, 16.0, 16.0],
                                0xFF_FFFF,
                            );
                            Err("rollback both stock and custom draw".into())
                        }
                        PANIC_ID => {
                            stock.draw_stock_layer(StockLayer::Armor(
                                ArmorLayerSlot::Chest,
                                ArmorLayerPass::Base,
                            ))?;
                            draw.quad(
                                "block/stone",
                                [
                                    [0.0, 101.0, 0.0],
                                    [0.0, 100.0, 0.0],
                                    [1.0, 100.0, 0.0],
                                    [1.0, 101.0, 0.0],
                                ],
                                [0.0, 0.0, 16.0, 16.0],
                                0xFF_FFFF,
                            );
                            panic!("panic after stock draw");
                        }
                        _ => Ok(()),
                    }
                }
            }
            struct ProbeMod(Arc<Mutex<Results>>);
            #[mod_api::__subscriptions]
            impl Hooks for ProbeMod {
                fn register_entity_layers(
                    &self,
                    _: &RegistrationContext,
                    event: &mut RegisterEntityLayersEvent<'_>,
                ) {
                    let mut r = self.0.lock().unwrap();
                    r.hide_zombie_pumpkin = Some(event.layers.hide(
                        LayerTarget::Entity("minecraft:zombie".into()),
                        StockLayer::SnowGolemHead,
                    ));
                    r.hide_player_pumpkin = Some(event.layers.hide(
                        LayerTarget::Player(PlayerModel::Wide),
                        StockLayer::SnowGolemHead,
                    ));
                    drop(r);
                    let target = LayerTarget::Player(PlayerModel::Wide);
                    event
                        .layers
                        .hide(
                            target.clone(),
                            StockLayer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Base),
                        )
                        .unwrap();
                    event
                        .layers
                        .add(target, Arc::new(ProbingLayer(self.0.clone())))
                        .unwrap();
                }
            }
            let results = Arc::new(Mutex::new(Results::default()));
            let mods = vec![mod_api::ModEntry::new(
                "probe_stock",
                Box::new(ProbeMod(results.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                let _ = mod_api::__private::take_hook_panics();
                game.resolve_entity_renderers();

                let out_probe = draw_single_mob(game, wide_player(game, PROBE_ID, true));
                assert_eq!(vertex_keys(&out_probe), vertex_keys(&bare_probe));
                {
                    let r = results.lock().unwrap();
                    assert!(r.hide_zombie_pumpkin.as_ref().unwrap().is_err());
                    assert!(r.hide_player_pumpkin.as_ref().unwrap().is_err());
                    assert!(r.draw_unhidden_cape.as_ref().unwrap().is_err());
                    assert!(r.draw_unsupported_pumpkin.as_ref().unwrap().is_err());
                }

                let out_err = draw_single_mob(game, wide_player(game, ERR_ID, true));
                assert_eq!(
                    vertex_keys(&out_err),
                    vertex_keys(&bare_err),
                    "Err return rolls back both draw_stock_layer and custom draw vertices"
                );
                assert!(mod_api::__private::take_hook_panics().is_empty());

                let out_panic = draw_single_mob(game, wide_player(game, PANIC_ID, true));
                assert_eq!(
                    vertex_keys(&out_panic),
                    vertex_keys(&bare_panic),
                    "panic rolls back both draw_stock_layer and custom draw vertices"
                );
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "probe_stock",
                        hook: "EntityRenderLayer::submit",
                    }]
                );
            });
        },
    );
}

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
