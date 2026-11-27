//! A mod's fluid interaction in a real game: lava beside the mod's fluid turns to the block it
//! declared, `fluid_place_block` can veto it, vanilla's two lava rules stay first, and the water
//! tag is read again when the tables are installed again.
use mod_api::{
    Flow, FluidBecomes, FluidInteraction, FluidNeighbour, FluidPlaceBlockEvent, Hooks, LevelHost,
    ModEntry, PlacedState, Registrar, WorldFluid,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_items::items::TagRegistry;
use sunrise_level::chunk::Chunk;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};

static VETO: AtomicBool = AtomicBool::new(false);

struct Gluey;
#[mod_api::__subscriptions]
impl Hooks for Gluey {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let honey = r.declare_world_fluid("honey", WorldFluid::default());
        let lava = r.fluid_key("minecraft:lava");
        let glass = r.block_key("minecraft:glass");
        r.fluid_interaction(FluidInteraction::new(
            lava,
            FluidNeighbour::Fluid(honey),
            FluidBecomes::always(PlacedState::of(glass)),
        ));
    }
    fn fluid_place_block(&self, _: &dyn LevelHost, _: &mut FluidPlaceBlockEvent<'_>) -> Flow {
        if VETO.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_declared_interaction_runs_after_vanillas_and_a_veto_stops_it() {
    if run_fresh_cases(
        "cut_layout::game::fluid_interaction_tests::a_declared_interaction_runs_after_vanillas_and_a_veto_stops_it",
        "SUNRISE_FLUID_INTERACTION_CHILD",
        &["valid"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![ModEntry::new("gluey", Box::new(Gluey))].into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, |game| {
        game.options.dedicated = Some(Vec::new());
        for cx in -1..=1 {
            for cz in -1..=1 {
                game.world
                    .chunks
                    .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
            }
        }
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let (lava, water, honey, glass, air) = (
            state("minecraft:lava"),
            state("minecraft:water"),
            state("gluey:honey"),
            state("minecraft:glass"),
            state("minecraft:air"),
        );
        let (obsidian, stone) = (state("minecraft:obsidian"), state("minecraft:stone"));
        for x in -16..32 {
            for z in -16..32 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.install_fluid_interactions().unwrap();
        // Lava at (x, 64, z) with `beside` to its east; the beside block's placement tells
        // its neighbours, as a block set does.
        let meet_with =
            |game: &mut sunrise_gameplay::game::ServerGame<'_>, z: i32, liquid, beside| {
                game.world.set(4, 64, z, liquid).unwrap();
                game.world.set(5, 64, z, beside).unwrap();
                let at = BlockPos::new(5, 64, z);
                game.with_level(|b, level, ticks| {
                    b.update_neighbours_on_block_set(level, ticks, at, air)
                });
                game.world.block(4, 64, z)
            };

        let meet = |game: &mut sunrise_gameplay::game::ServerGame<'_>, z: i32, beside| {
            meet_with(game, z, lava, beside)
        };
        let flowing_lava = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:lava".into(),
                properties: [("level".to_string(), "1".to_string())].into(),
            })
            .unwrap();

        // The mod's fluid and a source of lava: glass, the block the mod declared.
        VETO.store(false, Ordering::Relaxed);
        assert_eq!(meet(game, 2, honey), glass);
        // Vanilla's rule still comes first and still makes obsidian of a lava source.
        assert_eq!(meet(game, 6, water), obsidian);

        // Flowing lava is the same fluid: cobblestone beside water, basalt over soul soil beside
        // blue ice, glass beside the mod's fluid.
        let cobblestone = state("minecraft:cobblestone");
        assert_eq!(meet_with(game, 22, flowing_lava, water), cobblestone);
        game.world
            .set(4, 63, 26, state("minecraft:soul_soil"))
            .unwrap();
        assert_eq!(
            meet_with(game, 26, flowing_lava, state("minecraft:blue_ice")),
            state("minecraft:basalt")
        );
        assert_eq!(meet_with(game, 30, flowing_lava, honey), glass);

        // The pure read answers what one cell becomes beside one neighbour, from the same table
        // and with no world: nothing is placed and the hook is not asked.
        let read = |game: &sunrise_gameplay::game::ServerGame<'_>, liquid, beside, below| {
            let source = game.res.behaviours().fluid(liquid);
            let source =
                sunrise_gameplay_common::game::native_world_read_common::api_fluid_state(source)
                    .unwrap();
            game.fluid_interaction_result(source, beside, below)
        };
        let soul_soil = state("minecraft:soul_soil");
        let blue_ice = state("minecraft:blue_ice");
        assert_eq!(read(game, lava, water, stone), Some(obsidian));
        assert_eq!(read(game, flowing_lava, water, stone), Some(cobblestone));
        assert_eq!(
            read(game, lava, blue_ice, soul_soil),
            Some(state("minecraft:basalt"))
        );
        assert_eq!(read(game, lava, blue_ice, stone), None);
        assert_eq!(read(game, lava, honey, stone), Some(glass));
        assert_eq!(read(game, lava, stone, stone), None);
        assert_eq!(read(game, water, lava, stone), None, "water has no entries");
        let waterlogged = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:oak_slab".into(),
                properties: [("waterlogged".to_string(), "true".to_string())].into(),
            })
            .unwrap();
        assert_eq!(read(game, lava, waterlogged, stone), Some(obsidian));
        let unknown = sunrise_world::registry::BlockStateId(u32::MAX);
        let source = sunrise_gameplay_common::game::native_world_read_common::api_fluid_state(
            game.res.behaviours().fluid(lava),
        );
        assert_eq!(
            game.fluid_interaction_result(source.unwrap(), unknown, stone),
            None,
            "a state the registry does not hold is no interaction"
        );

        // The hook vetoes what the table decided: nothing is written.
        VETO.store(true, Ordering::Relaxed);
        assert_eq!(meet(game, 10, honey), lava);
        VETO.store(false, Ordering::Relaxed);

        // The tags are read again when the tables are installed again. A pack that empties the
        // water tag ends lava meeting water, and the pack's tag back restores it.
        let items = game.items().clone();
        let tags = |lava: &[&str], water: &[&str]| {
            let list = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
            let mut edited = items.clone();
            edited.set_tags(
                TagRegistry::Fluid,
                [
                    ("minecraft:lava".to_string(), list(lava)),
                    ("minecraft:water".to_string(), list(water)),
                ]
                .into(),
            );
            edited
        };
        let both_lava = ["minecraft:lava", "minecraft:flowing_lava"];
        let both_water = ["minecraft:water", "minecraft:flowing_water"];
        game.install_fluid_interactions_from(&tags(&both_lava, &[]))
            .unwrap();
        assert_eq!(meet(game, 12, water), lava);
        game.install_fluid_interactions().unwrap();
        assert_eq!(meet(game, 14, water), obsidian);

        // A tag matches exactly the fluids it lists: source water alone is not flowing water.
        let flowing_water = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:water".into(),
                properties: [("level".to_string(), "1".to_string())].into(),
            })
            .unwrap();
        game.install_fluid_interactions_from(&tags(&both_lava, &["minecraft:water"]))
            .unwrap();
        assert_eq!(meet(game, 16, flowing_water), lava);
        assert_eq!(meet(game, 18, water), obsidian);

        // Vanilla's rules follow the lava tag: lava taken out of it no longer meets water, and
        // only the fluid it lists does.
        game.install_fluid_interactions_from(&tags(&[], &both_water))
            .unwrap();
        assert_eq!(meet(game, 20, water), lava);
        game.install_fluid_interactions_from(&tags(&["minecraft:lava"], &both_water))
            .unwrap();
        assert_eq!(meet(game, 24, water), obsidian);
        assert_eq!(meet_with(game, 28, flowing_lava, water), flowing_lava);
        game.install_fluid_interactions().unwrap();
    });
}

/// A handler whose test panics beside gold, registered by a mod of its own.
struct Panicky;
impl mod_api::FluidInteractionHandler for Panicky {
    fn test(&self, level: &dyn LevelHost, at: mod_api::FluidInteractionAt) -> bool {
        let [x, y, z] = [at.relative.x, at.relative.y, at.relative.z];
        if level.block_state([x, y, z]) == level.block_state_id("minecraft:gold_block") {
            panic!("a fluid interaction test that panics");
        }
        false
    }
    fn interact(&self, _: &mut dyn mod_api::LevelWriteHost, _: mod_api::FluidInteractionAt) {}
}
struct PanickyMod;
#[mod_api::__subscriptions]
impl Hooks for PanickyMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let lava = r.fluid_key("minecraft:lava");
        r.fluid_interaction_handler(lava, Arc::new(Panicky));
    }
}

/// Lava beside packed ice or gold becomes glass, written by the handler itself.
struct Quench;
impl mod_api::FluidInteractionHandler for Quench {
    fn test(&self, level: &dyn LevelHost, at: mod_api::FluidInteractionAt) -> bool {
        let [x, y, z] = [at.relative.x, at.relative.y, at.relative.z];
        let beside = level.block_state([x, y, z]);
        [
            level.block_state_id("minecraft:packed_ice"),
            level.block_state_id("minecraft:gold_block"),
        ]
        .contains(&beside)
    }
    fn interact(&self, level: &mut dyn mod_api::LevelWriteHost, at: mod_api::FluidInteractionAt) {
        let glass = level.block_state_id("minecraft:glass").unwrap();
        let flags = mod_api::BlockUpdateFlags::NEIGHBORS | mod_api::BlockUpdateFlags::CLIENTS;
        assert!(level.set_block([at.pos.x, at.pos.y, at.pos.z], glass, flags));
    }
}
struct Quenchy;
#[mod_api::__subscriptions]
impl Hooks for Quenchy {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let lava = r.fluid_key("minecraft:lava");
        r.fluid_interaction_handler(lava, Arc::new(Quench));
    }
}

#[test]
#[ignore = "daily"]
fn a_native_handler_tests_and_acts_in_its_place_and_a_panicking_one_does_not_apply() {
    if run_fresh_cases(
        "cut_layout::game::fluid_interaction_tests::a_native_handler_tests_and_acts_in_its_place_and_a_panicking_one_does_not_apply",
        "SUNRISE_FLUID_HANDLER_CHILD",
        &["valid"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![
        ModEntry::new("panicky", Box::new(PanickyMod)),
        ModEntry::new("quenchy", Box::new(Quenchy)),
    ]
    .into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, |game| {
        game.options.dedicated = Some(Vec::new());
        for cx in -1..=1 {
            for cz in -1..=1 {
                game.world
                    .chunks
                    .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
            }
        }
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let (lava, water, air, stone) = (
            state("minecraft:lava"),
            state("minecraft:water"),
            state("minecraft:air"),
            state("minecraft:stone"),
        );
        for x in -16..32 {
            for z in -16..32 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.install_fluid_interactions().unwrap();
        let meet = |game: &mut sunrise_gameplay::game::ServerGame<'_>, z: i32, beside| {
            game.world.set(4, 64, z, lava).unwrap();
            game.world.set(5, 64, z, beside).unwrap();
            let at = BlockPos::new(5, 64, z);
            game.with_level(|b, level, ticks| {
                b.update_neighbours_on_block_set(level, ticks, at, air)
            });
            game.world.block(4, 64, z)
        };
        let glass = state("minecraft:glass");
        // The handler's own write, with no fizz or veto in between.
        assert_eq!(meet(game, 2, state("minecraft:packed_ice")), glass);
        // Vanilla's lava rules come before the mods' entries.
        assert_eq!(meet(game, 6, water), state("minecraft:obsidian"));
        // A neighbour no entry takes leaves the lava to spread.
        assert_eq!(meet(game, 10, stone), lava);
        // The earlier handler panics beside gold: it does not apply, and the next one still does.
        assert_eq!(meet(game, 14, state("minecraft:gold_block")), glass);
        // The pure read passes over handlers.
        let source = sunrise_gameplay_common::game::native_world_read_common::api_fluid_state(
            game.res.behaviours().fluid(lava),
        );
        assert_eq!(
            game.fluid_interaction_result(source.unwrap(), state("minecraft:packed_ice"), stone),
            None
        );
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;
