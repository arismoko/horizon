//! `crop_grow_pre` and `crop_grow_post` reach mods from the server's random ticks.
use mod_api::{CropGrowPostEvent, CropGrowPreEvent, Hooks, LevelRandomHost, Registrar, TriState};
use std::sync::Mutex;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::LevelDimension;

static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ORIGINALS: Mutex<Vec<(BlockPos, mod_api::BlockState)>> = Mutex::new(Vec::new());

/// Stops the sugar cane at x 0, forces the vine at x 4, and logs what it hears.
struct Gardener;
#[mod_api::__subscriptions]
impl Hooks for Gardener {
    fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}
    fn crop_grow_pre(&self, _: &mut dyn LevelRandomHost, event: &mut CropGrowPreEvent<'_>) {
        let BlockPos { x, y, z } = event.pos;
        HEARD
            .lock()
            .unwrap()
            .push(format!("pre {:?} {x} {y} {z}", event.dimension));
        if x == 0 {
            *event.result = TriState::False;
        } else if x == 4 {
            *event.result = TriState::True;
        }
    }
    fn crop_grow_post(&self, _: &mut dyn LevelRandomHost, event: &mut CropGrowPostEvent) {
        let BlockPos { x, y, z } = event.pos;
        ORIGINALS
            .lock()
            .unwrap()
            .push((event.pos, event.original_state));
        let same = event.original_state == event.state;
        HEARD
            .lock()
            .unwrap()
            .push(format!("post {x} {y} {z} same {same}"));
    }
}

#[test]
#[ignore = "daily"]
fn a_mod_stops_one_cane_and_hears_the_other_grow() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("gardener", Box::new(Gardener))].into();
    mod_platform::testing::with_mods(mods, || {
        HEARD.lock().unwrap().clear();
        ORIGINALS.lock().unwrap().clear();
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let registry = &game.pack.assets.registry;
            let sand = registry.default_state("minecraft:sand").unwrap();
            let cane = registry.default_state("minecraft:sugar_cane").unwrap();
            let vine_soil = registry.default_state("minecraft:warped_nylium").unwrap();
            let ripe = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:sugar_cane".into(),
                    properties: [("age".to_string(), "15".to_string())].into(),
                })
                .unwrap();
            let vine = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:twisting_vines".into(),
                    properties: [("age".to_string(), "5".to_string())].into(),
                })
                .unwrap();
            let grown_vine = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:twisting_vines".into(),
                    properties: [("age".to_string(), "6".to_string())].into(),
                })
                .unwrap();
            for x in [0, 2] {
                game.world.set(x, 63, 0, sand).unwrap();
                game.world.set(x, 64, 0, ripe).unwrap();
            }
            game.world.set(4, 63, 0, vine_soil).unwrap();
            game.world.set(4, 64, 0, vine).unwrap();
            let originals = game.with_level(|_, level, _| {
                [
                    (
                        BlockPos::new(2, 65, 0),
                        mod_api::LevelHost::block_state(level, [2, 64, 0]).unwrap(),
                    ),
                    (
                        BlockPos::new(4, 65, 0),
                        mod_api::LevelHost::block_state(level, [4, 64, 0]).unwrap(),
                    ),
                ]
            });
            game.with_level(|behaviours, level, ticks| {
                for x in [0, 2, 4] {
                    behaviours.random_tick(level, ticks, BlockPos::new(x, 64, 0));
                }
            });
            assert_eq!(
                game.world.block(0, 64, 0),
                ripe,
                "the stopped cane keeps its age"
            );
            assert_ne!(game.world.block(0, 65, 0), cane);
            assert_eq!(game.world.block(2, 65, 0), cane, "the other grows");
            assert_eq!(game.world.block(4, 65, 0), grown_vine);
            assert_eq!(*ORIGINALS.lock().unwrap(), originals);
            assert_eq!(
                *HEARD.lock().unwrap(),
                [
                    format!("pre {:?} 0 64 0", LevelDimension::Overworld),
                    format!("pre {:?} 2 64 0", LevelDimension::Overworld),
                    "post 2 65 0 same false".to_string(),
                    format!("pre {:?} 4 65 0", LevelDimension::Overworld),
                    "post 4 65 0 same false".to_string(),
                ]
            );
        });
    });
}
