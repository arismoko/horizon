//! The `living_travel` census (#2414 PR 2): the mob's travel fires it once,
//! the move the travel ends in fires `entity_move` beside it - and a NoAI
//! mob, which never travels, fires nobody. The same file carries the
//! pipeline checks a replacement and a move veto need.
use mod_api::{EntityMoveEvent, Flow, Hooks, LivingTravelEvent, LoopContext, TravelMode};
use std::sync::{Arc, Mutex};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Travel(u32, TravelMode),
    Move(u32),
}

struct Recorder(Arc<Mutex<Vec<Event>>>);

#[mod_api::__subscriptions]
impl Hooks for Recorder {
    fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push(Event::Travel(event.entity, event.mode));
        Flow::Continue
    }
    fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
        self.0.lock().unwrap().push(Event::Move(event.entity));
        Flow::Continue
    }
}

/// One zombie that travels (a motion to spend), one that never does (NoAI):
/// the moving one fires `living_travel` exactly once - in the air branch,
/// over the floor, on the server - and the move its travel ends in fires
/// `entity_move` once beside it. The NoAI one fires neither: vanilla's
/// `isEffectiveAi` gate keeps its tick from travel altogether.
#[test]
#[ignore = "daily"]
fn one_travel_fires_once_and_its_move_beside_and_noai_fires_nobody() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "recorder",
        Box::new(Recorder(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(sunrise_gameplay_common::game::Dim::Overworld);
            game.build_registries();
            game.reset_entity_residency();
            game.set_allow_commands(true);
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 8, z, stone.0);
                }
            }
            game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
            game.summon_with_nbt(
                "minecraft:zombie",
                [4.5, 40.5, 5.5],
                "{Motion:[0.5d,0.0d,0.0d]}",
            )
            .unwrap_or_else(|e| panic!("summon zombie: {e:?}"));
            let zombie = game.mobs.iter().next().expect("normal zombie spawned").id;
            game.summon_with_nbt(
                "minecraft:zombie",
                [6.5, 40.5, 5.5],
                "{NoAI:1b,Motion:[0.5d,0.0d,0.0d]}",
            )
            .unwrap_or_else(|e| panic!("summon noai zombie: {e:?}"));
            heard.lock().unwrap().clear();
            let _ = game.tick_creatures();
            assert_eq!(
                *heard.lock().unwrap(),
                vec![Event::Travel(zombie, TravelMode::Air), Event::Move(zombie)],
                "only the normal zombie fires, with travel before its nested move"
            );
        });
    });
}

/// A travel replacement supplies the branch's starting values: a zombie
/// spending 0.5 eastbound leaves its travel with a westbound velocity, and
/// the tick that follows walks it west, not east.
#[test]
#[ignore = "daily"]
fn a_replaced_travel_velocity_steer_its_tick() {
    struct Westward;
    #[mod_api::__subscriptions]
    impl Hooks for Westward {
        fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
            if event.kind == "minecraft:zombie" {
                *event.velocity = [-1.0, 0.0, 0.0];
            }
            Flow::Continue
        }
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("westward", Box::new(Westward))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(sunrise_gameplay_common::game::Dim::Overworld);
            game.build_registries();
            game.reset_entity_residency();
            game.set_allow_commands(true);
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 8, z, stone.0);
                }
            }
            game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
            game.summon_with_nbt(
                "minecraft:zombie",
                [8.5, 9.5, 5.5],
                "{Motion:[0.5d,0.0d,0.0d]}",
            )
            .unwrap_or_else(|e| panic!("summon zombie: {e:?}"));
            let before: Vec<f64> = game.mobs.iter().map(|m| m.position.x).collect();
            let _ = game.tick_creatures();
            let _ = game.tick_creatures();
            let after: Vec<f64> = game.mobs.iter().map(|m| m.position.x).collect();
            assert!(
                after[0] < before[0],
                "the westward replacement steered the tick, not the eastbound motion"
            );
        });
    });
}

/// A travel that runs may still meet a move veto (design #2308: a travel
/// replacement is not the last word): a listener that cancels the zombie's
/// moves leaves it where its travel found it, while the travel itself fired.
#[test]
#[ignore = "daily"]
fn a_move_veto_still_holds_a_travel_that_ran() {
    struct MoveCanceler(Arc<Mutex<usize>>);
    #[mod_api::__subscriptions]
    impl Hooks for MoveCanceler {
        fn living_travel(&self, _: &dyn LoopContext, _: &mut LivingTravelEvent<'_>) -> Flow {
            Flow::Continue
        }
        fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
            if event.kind == "minecraft:zombie" {
                event.cancelled = true;
                *self.0.lock().unwrap() += 1;
            }
            Flow::Continue
        }
    }
    let travels = Arc::new(Mutex::new(0usize));
    let vetoes = Arc::new(Mutex::new(0usize));
    struct TravelTally(Arc<Mutex<usize>>);
    #[mod_api::__subscriptions]
    impl Hooks for TravelTally {
        fn living_travel(&self, _: &dyn LoopContext, _: &mut LivingTravelEvent<'_>) -> Flow {
            *self.0.lock().unwrap() += 1;
            Flow::Continue
        }
    }
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("tally", Box::new(TravelTally(travels.clone()))),
        mod_api::ModEntry::new("veto", Box::new(MoveCanceler(vetoes.clone()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(sunrise_gameplay_common::game::Dim::Overworld);
            game.build_registries();
            game.reset_entity_residency();
            game.set_allow_commands(true);
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 8, z, stone.0);
                }
            }
            game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
            game.summon_with_nbt(
                "minecraft:zombie",
                [4.5, 9.5, 5.5],
                "{Motion:[0.5d,0.0d,0.0d]}",
            )
            .unwrap_or_else(|e| panic!("summon zombie: {e:?}"));
            let x_before: Vec<f64> = game.mobs.iter().map(|m| m.position.x).collect();
            let _ = game.tick_creatures();
            let x_after: Vec<f64> = game.mobs.iter().map(|m| m.position.x).collect();
            assert_eq!(*travels.lock().unwrap(), 1, "the travel ran");
            assert_eq!(*vetoes.lock().unwrap(), 1, "its move was vetoed");
            assert_eq!(
                x_before, x_after,
                "the move veto held the zombie where its travel found it"
            );
        });
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
