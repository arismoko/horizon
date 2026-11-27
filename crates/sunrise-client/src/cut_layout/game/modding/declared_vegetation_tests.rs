//! Explicit vegetation behaves the same on different templates and is not inferred from Parts.
#![cfg(feature = "client")]
use mod_api::{BlockPart, Hooks, LevelSide, MayPlaceOnEvent, Registrar};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_core::{
    geometry::{Direction, Vec3},
    position::{BlockPos, ChunkPos},
};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::api_values::TryIntoApi as _;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_world::level::{PlaceContext, PlaceOutcome};

struct Plant(Arc<AtomicUsize>);
#[mod_api::sunrise_part]
impl BlockPart for Plant {
    fn may_place_on(&self, _: LevelSide<'_>, _: &mut MayPlaceOnEvent) -> Option<bool> {
        self.0.fetch_add(1, Ordering::Relaxed);
        None // Leave the declaration's default supports_vegetation rule in place.
    }
}
struct Soil(bool);
#[mod_api::sunrise_part]
impl BlockPart for Soil {
    fn can_sustain_plant(
        &self,
        _: LevelSide<'_>,
        _: &mut mod_api::CanSustainPlantEvent,
    ) -> Option<bool> {
        Some(self.0)
    }
}
struct Garden(Arc<AtomicUsize>);
const PLANTS: [(&str, &str); 5] = [
    ("garden:glass", "minecraft:glass"),
    ("garden:stairs", "minecraft:oak_stairs"),
    ("garden:poppy", "minecraft:poppy"),
    ("garden:farmland", "minecraft:farmland"),
    ("garden:path", "minecraft:dirt_path"),
];
#[mod_api::__subscriptions]
impl Hooks for Garden {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for (id, _) in PLANTS
            .into_iter()
            .chain([("garden:ordinary_glass", "minecraft:glass")])
        {
            let key = r.block_key(id);
            r.block_part(key, Plant(Arc::clone(&self.0)));
        }
        r.block_part_tag("minecraft:flowers", Plant(Arc::clone(&self.0)));
        for (id, answer) in [
            ("minecraft:gold_block", true),
            ("minecraft:grass_block", false),
        ] {
            let key = r.block_key(id);
            r.block_part(key, Soil(answer));
        }
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        for (id, template) in PLANTS
            .into_iter()
            .chain([("garden:bare", "minecraft:glass")])
        {
            let mut block = mod_api::Block::new(id, template);
            block.properties.vegetation = true;
            event.register(block).unwrap();
        }
        event
            .register(mod_api::Block::new(
                "garden:ordinary_glass",
                "minecraft:glass",
            ))
            .unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn declared_vegetation_places_and_breaks_independently_of_its_template() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::declared_vegetation_tests::declared_vegetation_places_and_breaks_independently_of_its_template",
        "SUNRISE_VEGETATION_CHILD",
        &["run"],
    ) {
        return;
    }
    let heard = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "garden",
        Box::new(Garden(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        sunrise_gameplay::game::native_entity_refs::tests::with_game_at(
            root.prepared().to_path_buf(),
            |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                let registry = game.pack.assets.registry;
                let air = registry.default_state("minecraft:air").unwrap();
                let stone = registry.default_state("minecraft:stone").unwrap();
                let pos = BlockPos::new(5, 65, 5);
                for (plant, _) in PLANTS {
                    for (soil, allowed, asks_plant) in [
                        ("minecraft:stone", false, true),
                        ("minecraft:dirt", true, true),
                        ("minecraft:gold_block", true, false),
                        ("minecraft:grass_block", false, false),
                    ] {
                        game.world.set(5, 65, 5, air).unwrap();
                        game.world
                            .set(5, 64, 5, registry.default_state(soil).unwrap())
                            .unwrap();
                        let candidate = registry
                            .default_state(plant)
                            .unwrap()
                            .try_into_api()
                            .unwrap();
                        let level = game.level(game.dim.id()).unwrap();
                        let before = heard.load(Ordering::Relaxed);
                        assert_eq!(
                            level.can_survive(game, [pos.x, pos.y, pos.z], candidate),
                            Ok(allowed),
                            "candidate {plant} on {soil}"
                        );
                        assert_eq!(
                            heard.load(Ordering::Relaxed) > before,
                            asks_plant,
                            "soil Part decides before the plant Part"
                        );
                        let context = PlaceContext {
                            clicked_pos: BlockPos::new(5, 64, 5),
                            face: Direction::Up,
                            hit: Vec3::new(5.5, 65.0, 5.5),
                            inside: false,
                            source: sunrise_world::level::PlacementSource::Player {
                                yaw: 0.0,
                                pitch: 0.0,
                                sneaking: false,
                            },
                            item: plant.into(),
                            can_use_game_master_blocks: false,
                        };
                        let before = heard.load(Ordering::Relaxed);
                        let placed = game
                            .with_level(|b, level, ticks| b.place_block(level, ticks, &context));
                        assert_eq!(
                            matches!(placed, PlaceOutcome::Placed { .. }),
                            allowed,
                            "{plant} on {soil}"
                        );
                        assert_eq!(
                            heard.load(Ordering::Relaxed) > before,
                            asks_plant,
                            "soil decides before the plant"
                        );
                        let placed = game.world.block(pos.x, pos.y, pos.z);
                        if allowed {
                            assert_eq!(registry.state(placed).unwrap().name, plant);
                            game.with_level(|b, level, ticks| {
                                b.set_block(level, ticks, BlockPos::new(5, 64, 5), stone, 3);
                            });
                            assert_eq!(
                                game.world.block(pos.x, pos.y, pos.z),
                                air,
                                "{plant} pops off after its soil changes"
                            );
                        } else {
                            assert_eq!(placed, air, "{plant} on {soil} must not write a fallback");
                        }
                    }
                }
                // A registered Part does not opt this block into vegetation.
                let ordinary = registry.default_state("garden:ordinary_glass").unwrap();
                game.world.set(5, 64, 5, stone).unwrap();
                assert!(
                    game.with_level(|b, level, ticks| b.can_survive(level, ticks, pos, ordinary))
                );
                // Nor does the same Part reached by a vanilla block tag.
                let leaf = registry.default_state("minecraft:cherry_leaves").unwrap();
                assert!(
                    sunrise_gameplay_common::game::api_values::block_state::block_listens(
                        mod_api::HookId::MayPlaceOn,
                        leaf
                    )
                );
                game.world.set(5, 65, 5, leaf).unwrap();
                game.with_level(|b, level, ticks| {
                    b.set_block(level, ticks, BlockPos::new(5, 64, 5), air, 3);
                });
                assert_ne!(game.world.block(5, 65, 5), air);
            },
        );
        crate::game::server_side::native_entity_refs::tests::with_game_role_client(
            root.prepared().to_path_buf(),
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |game| {
                use sunrise_items::items::ItemStack;
                let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "VegetationPrediction").unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.set_local_mode(sunrise_command::args::GameMode::Survival);
                let hit = sunrise_gameplay_common::game::Hit {
                    pos: [5, 64, 5],
                    face: [0, 1, 0],
                    point: [5.5, 65.0, 5.5],
                    inside: false,
                };
                game.client.remote.as_mut().unwrap().logged_in = true;
                let level = game.native_client_level().unwrap();
                let plant = game
                    .pack
                    .assets
                    .registry
                    .default_state("garden:glass")
                    .unwrap()
                    .try_into_api()
                    .unwrap();
                for (soil, allowed) in [("minecraft:stone", false), ("minecraft:dirt", true)] {
                    let support = game.pack.assets.registry.default_state(soil).unwrap();
                    game.world.set(5, 64, 5, support).unwrap();
                    assert_eq!(
                        level.can_survive(game, [5, 65, 5], plant),
                        Ok(allowed),
                        "client's loaded replica: {soil}"
                    );
                }
                for (soil, allowed) in [("minecraft:stone", false), ("minecraft:dirt", true)] {
                    let ground = game.pack.assets.registry.default_state(soil).unwrap();
                    game.world.set(5, 64, 5, ground).unwrap();
                    for id in [
                        "garden:glass",
                        "garden:bare",
                        "garden:farmland",
                        "garden:path",
                    ] {
                        let before = heard.load(Ordering::Relaxed);
                        let placement = game.remote_can_place(&hit, &ItemStack::new(id, 1));
                        assert_eq!(
                            placement.is_some(),
                            allowed,
                            "prediction for {id} on {soil}"
                        );
                        assert_eq!(
                            heard.load(Ordering::Relaxed) > before,
                            id != "garden:bare",
                            "the client asks only blocks with a subscribed Part: {id}"
                        );
                    }
                }
            },
        );
    });
}
