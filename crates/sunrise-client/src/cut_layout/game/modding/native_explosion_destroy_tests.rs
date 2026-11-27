//! `LevelRef::explosion_destroy_blocks` through the native handle on a fresh game.
#![cfg(test)]
use mod_api::{
    BlockPart, EntityRef, ExplosionDestroyError, ExplosionDetonateEvent, ExplosionInteraction,
    ExplosionStartEvent, Flow, ModEntry, OnBlockExplodedEvent, Registrar, RegistrationContext,
    ServerHost,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use sunrise_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
use sunrise_level::chunk::Chunk;
use sunrise_sim::{mobs::Who, motion::PlayerMotion};
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

/// One `explosion_destroy_blocks` call.
struct Blast {
    pub source: Option<EntityRef>,
    center: [f64; 3],
    radius: f32,
    fire: bool,
    interaction: ExplosionInteraction,
}

/// A sourceless blast of radius 4 at `[5.5, 80.5, 5.5]` with `interaction`.
fn blast(interaction: ExplosionInteraction) -> Blast {
    Blast {
        source: None,
        center: [5.5, 80.5, 5.5],
        radius: 4.0,
        fire: false,
        interaction,
    }
}

/// Destroys `positions` with `blast` in the overworld and checks the answer.
fn destroy(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    positions: &[[i32; 3]],
    blast: Blast,
    expected: Result<u32, ExplosionDestroyError>,
) {
    let level = game.level("minecraft:overworld").unwrap();
    assert_eq!(
        level.explosion_destroy_blocks(
            game,
            positions,
            blast.source,
            blast.center,
            blast.radius,
            blast.fire,
            blast.interaction
        ),
        expected
    );
}

/// What the test mod does and saw.
#[derive(Clone, Default)]
struct Watch {
    /// `explosion_start` cancels.
    pub cancel: Arc<AtomicBool>,
    /// `explosion_detonate` takes `[1, 80, 1]` out, prepends `[3, 80, 3]` (plus a duplicate),
    /// and appends an unloaded and out-of-height position.
    edit: Arc<AtomicBool>,
    /// The detonate lists heard.
    detonated: Arc<Mutex<Vec<Vec<[i32; 3]>>>>,
    /// Where dirt's part heard `on_block_exploded`.
    exploded: Arc<Mutex<Vec<[i32; 3]>>>,
}
#[derive(Clone)]
struct Marker(Arc<Mutex<Vec<[i32; 3]>>>);
#[mod_api::sunrise_part]
impl BlockPart for Marker {
    fn on_block_exploded(&self, _: &mut dyn ServerHost, event: &mut OnBlockExplodedEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push([event.pos.x, event.pos.y, event.pos.z]);
    }
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Watch {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let dirt = r.block_key("minecraft:dirt");
        r.block_part(dirt, Marker(self.exploded.clone()));
    }
    fn explosion_start(&self, _: &mut dyn ServerHost, _: &mut ExplosionStartEvent<'_>) -> Flow {
        if self.cancel.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn explosion_detonate(&self, _: &mut dyn ServerHost, event: &mut ExplosionDetonateEvent<'_>) {
        self.detonated.lock().unwrap().push(event.blocks.clone());
        if self.edit.load(Ordering::Relaxed) {
            event.blocks.retain(|p| *p != [1, 80, 1]);
            event.blocks.insert(0, [3, 80, 3]);
            event.blocks.push([3, 80, 3]);
            event.blocks.push([100, 80, 100]);
            event.blocks.push([1, 400, 1]);
        }
    }
}

/// Runs `test` on a fresh server game with the watching mod and a loaded, empty overworld chunk
/// (0, 0).
fn with_watch(
    test: impl Fn(&mut sunrise_gameplay::game::ServerGame<'_>, &Watch) + Send + Sync + 'static,
) {
    run_with_game_stack(move || {
        let watch = Watch::default();
        let mods: mod_api::Mods = vec![ModEntry::new("watch", Box::new(watch.clone()))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            with_game(|game| {
                game.build_registries();
                game.load_server_loot();
                game.set_allow_commands(true);
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                test(game, &watch);
            });
        });
    });
}

fn name(game: &sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

fn set(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3], block: &str) {
    assert!(game.run_command(&format!(
        "setblock {} {} {} minecraft:{block}",
        pos[0], pos[1], pos[2]
    )));
}

/// How many of `item` lie dropped.
fn dropped(game: &sunrise_gameplay::game::ServerGame<'_>, item: &str) -> u32 {
    game.dropped
        .iter()
        .filter(|e| &*e.stack.item == item)
        .map(|e| e.stack.count)
        .sum()
}

/// The 1000 cells of a 10-block cube in chunk (0, 0).
fn cube() -> Vec<[i32; 3]> {
    (0..10)
        .flat_map(|x| (70..80).flat_map(move |y| (0..10).map(move |z| [x, y, z])))
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_thousand_stone_blocks_turn_to_air_with_drops_decayed_by_the_rule() {
    with_watch(|game, _| {
        for decay in [true, false] {
            assert!(game.run_command("fill 0 70 0 9 79 9 minecraft:stone"));
            game.dropped.clear();
            assert!(game.ui.rules.set(
                "block_explosion_drop_decay",
                sunrise_game_data::gamerules::Value::Bool(decay),
            ));
            // Under decay a stack survives one time in `radius`: none of 1000 at a radius of
            // a billion. Without it every stone drops its cobblestone.
            let mut blast = blast(ExplosionInteraction::Block);
            blast.radius = 1.0e9;
            destroy(game, &cube(), blast, Ok(1000));
            assert!(cube().iter().all(|&p| name(game, p) == "minecraft:air"));
            let expected = if decay { 0 } else { 1000 };
            assert_eq!(dropped(game, "minecraft:cobblestone"), expected, "{decay}");
        }
    });
}

#[test]
#[ignore = "daily"]
fn the_keep_interaction_changes_nothing() {
    with_watch(|game, _| {
        set(game, [1, 80, 1], "stone");
        destroy(
            game,
            &[[1, 80, 1]],
            blast(ExplosionInteraction::None),
            Ok(0),
        );
        assert_eq!(name(game, [1, 80, 1]), "minecraft:stone");
    });
}

#[test]
#[ignore = "daily"]
fn a_part_hears_its_block_exploded_once() {
    with_watch(|game, watch| {
        set(game, [2, 80, 2], "dirt");
        // A position listed twice counts once.
        let positions = [[2, 80, 2], [2, 80, 2]];
        destroy(game, &positions, blast(ExplosionInteraction::Block), Ok(1));
        assert_eq!(*watch.exploded.lock().unwrap(), [[2, 80, 2]]);
        assert_eq!(*watch.detonated.lock().unwrap(), [vec![[2, 80, 2]]]);
        assert_eq!(name(game, [2, 80, 2]), "minecraft:air");
    });
}

#[test]
#[ignore = "daily"]
fn a_detonate_listener_edits_reorders_and_adds_positions() {
    with_watch(|game, watch| {
        assert!(game.ui.rules.set(
            "block_explosion_drop_decay",
            sunrise_game_data::gamerules::Value::Bool(false),
        ));
        for p in [[1, 80, 1], [2, 80, 1], [3, 80, 3]] {
            set(game, p, "cobblestone");
        }
        watch.edit.store(true, Ordering::Relaxed);
        // Java Random seed 7 on the 3-entry list [[3,80,3], [2,80,1], [3,80,3]]:
        // nextInt(3) is 1 and nextInt(2) is 1, leaving [3, 80, 3] first after shuffle.
        game.world.random.set_seed(7);
        let positions = [[1, 80, 1], [2, 80, 1]];
        destroy(game, &positions, blast(ExplosionInteraction::Block), Ok(2));
        assert_eq!(name(game, [1, 80, 1]), "minecraft:cobblestone", "spared");
        assert_eq!(name(game, [2, 80, 1]), "minecraft:air");
        assert_eq!(name(game, [3, 80, 3]), "minecraft:air", "added, destroyed");
        assert!(!game.world.chunks.contains_key(&(6, 6)), "never loaded");
        // Reordered [3, 80, 3] first -> merged cobblestone drop pops from [3, 3], not [2, 1].
        let drops: Vec<_> = game
            .dropped
            .iter()
            .map(|e| (e.stack.item.to_string(), e.stack.count, e.position))
            .collect();
        assert_eq!(drops.len(), 1, "{drops:?}");
        let (item, count, at) = &drops[0];
        assert_eq!((item.as_str(), *count), ("minecraft:cobblestone", 2));
        assert_eq!([at.x.floor() as i32, at.z.floor() as i32], [3, 3], "{at:?}");

        // Multiplicity: Java Random seed 2 draws nextInt(3) = 1, then 0.
        // Caller duplicates are normalized before the hook (1 draw -> stays air), while a
        // listener-added duplicate stays in the list and gets a second ignition roll (ignites).
        set(game, [3, 79, 3], "stone");
        watch.edit.store(false, Ordering::Relaxed);
        game.world.random.set_seed(2);
        destroy(
            game,
            &[[3, 80, 3], [3, 80, 3]],
            Blast {
                fire: true,
                ..blast(ExplosionInteraction::None)
            },
            Ok(0),
        );
        assert_eq!(
            name(game, [3, 80, 3]),
            "minecraft:air",
            "caller duplicate normalized before hook"
        );
        watch.edit.store(true, Ordering::Relaxed);
        game.world.random.set_seed(2);
        destroy(
            game,
            &[],
            Blast {
                fire: true,
                ..blast(ExplosionInteraction::None)
            },
            Ok(0),
        );
        assert_eq!(
            name(game, [3, 80, 3]),
            "minecraft:fire",
            "listener-added duplicate kept for second fire roll"
        );
    });
}

#[test]
#[ignore = "daily"]
fn tnt_primes_owned_by_the_causing_entity() {
    with_watch(|game, _| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(14.5, 80.0, 14.5)).unwrap());
        bind_local_player(game, 11, 123);
        set(game, [5, 80, 5], "tnt");
        let mut blast = blast(ExplosionInteraction::Tnt);
        // A living source is its own causing entity.
        blast.source = game.entity(11);
        assert!(blast.source.is_some());
        destroy(game, &[[5, 80, 5]], blast, Ok(1));
        assert_eq!(name(game, [5, 80, 5]), "minecraft:air");
        let owners: Vec<Option<Who>> = game.tnts.iter().map(|t| t.owner).collect();
        assert_eq!(owners, [Some(Who::Player)]);
    });
}

#[test]
#[ignore = "daily"]
fn drops_of_one_item_merge_at_the_first_blocks_position() {
    with_watch(|game, _| {
        assert!(game.ui.rules.set(
            "block_explosion_drop_decay",
            sunrise_game_data::gamerules::Value::Bool(false),
        ));
        let positions = [[1, 80, 1], [8, 80, 8]];
        for p in positions {
            set(game, p, "cobblestone");
        }
        // Java Random seed 7: nextInt(2) is 1, so the first cell stays first.
        game.world.random.set_seed(7);
        destroy(game, &positions, blast(ExplosionInteraction::Block), Ok(2));
        // The merged stack still uses Block.popResource: three nextDouble offsets
        // after the shuffle, but its constructor motion uses its own random.
        assert_eq!(game.world.random.next_int(), -1_105_573_998);
        let drops: Vec<_> = game
            .dropped
            .iter()
            .map(|e| (e.stack.item.to_string(), e.stack.count, e.position))
            .collect();
        assert_eq!(drops.len(), 1, "{drops:?}");
        let (item, count, at) = &drops[0];
        assert_eq!((item.as_str(), *count), ("minecraft:cobblestone", 2));
        // Popped from the independently calculated first shuffled cell.
        let cell = [at.x.floor() as i32, at.z.floor() as i32];
        assert_eq!(cell, [1, 1], "{at:?}");
        // Block.popResource spends the same three position doubles even
        // when the gamerule prevents constructing the merged item entity.
        assert!(game.ui.rules.set(
            "block_drops",
            sunrise_game_data::gamerules::Value::Bool(false)
        ));
        for p in positions {
            set(game, p, "cobblestone");
        }
        game.world.random.set_seed(7);
        let before = game.dropped.len();
        destroy(game, &positions, blast(ExplosionInteraction::Block), Ok(2));
        assert_eq!(game.dropped.len(), before, "no new item entity");
        assert_eq!(
            game.world.random.next_int(),
            -1_105_573_998,
            "offsets still drawn"
        );
    });
}

#[test]
#[ignore = "daily"]
fn an_unloaded_position_is_skipped_and_not_counted() {
    with_watch(|game, _| {
        set(game, [1, 80, 1], "stone");
        let positions = [[1, 80, 1], [100, 80, 100]];
        destroy(game, &positions, blast(ExplosionInteraction::Block), Ok(1));
        assert_eq!(name(game, [1, 80, 1]), "minecraft:air");
        assert!(!game.world.chunks.contains_key(&(6, 6)), "never loaded");
    });
}

#[test]
#[ignore = "daily"]
fn more_than_the_old_cap_of_positions_is_accepted() {
    with_watch(|game, watch| {
        set(game, [1, 80, 1], "stone");
        let positions = vec![[1, 80, 1]; 65_537];
        destroy(game, &positions, blast(ExplosionInteraction::Block), Ok(1));
        assert_eq!(name(game, [1, 80, 1]), "minecraft:air");
        assert_eq!(*watch.detonated.lock().unwrap(), [vec![[1, 80, 1]]]);
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_start_destroys_nothing() {
    with_watch(|game, watch| {
        set(game, [1, 80, 1], "stone");
        watch.cancel.store(true, Ordering::Relaxed);
        destroy(
            game,
            &[[1, 80, 1]],
            blast(ExplosionInteraction::Block),
            Ok(0),
        );
        assert_eq!(name(game, [1, 80, 1]), "minecraft:stone");
        assert!(watch.detonated.lock().unwrap().is_empty());
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
