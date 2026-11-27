//! The End fight's hooks and its level reads and writes (`EnderDragonFight`).
use super::*;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use mod_api::{
    DragonFightState, DragonFightTickEvent, DragonFightWonEvent, DragonRespawnStage,
    DragonRespawnStageEvent, EndCrystalDestroyedEvent, Flow, Hooks, Host, ServerHost,
};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::{
    Dim, native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
};
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;

#[derive(Default)]
struct Log {
    cancel: bool,
    ticks: usize,
    stages: Vec<DragonRespawnStage>,
    answer: Option<bool>,
    wins: Vec<(BlockPos, bool)>,
    crystals: Vec<(BlockPos, bool)>,
}

struct Probe(Arc<Mutex<Log>>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn dragon_fight_tick(&self, _: &mut dyn ServerHost, _: &mut DragonFightTickEvent) -> Flow {
        let mut log = self.0.lock().unwrap();
        log.ticks += 1;
        if log.cancel {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn dragon_respawn_stage(&self, _: &mut dyn ServerHost, event: &mut DragonRespawnStageEvent) {
        self.0.lock().unwrap().stages.push(event.stage);
    }
    fn end_crystal_destroyed(&self, _: &mut dyn ServerHost, event: &mut EndCrystalDestroyedEvent) {
        let mut log = self.0.lock().unwrap();
        log.crystals.push((event.pos, event.respawn_crystal));
    }
    fn dragon_fight_won(&self, _: &dyn Host, event: &mut DragonFightWonEvent) -> Option<bool> {
        let mut log = self.0.lock().unwrap();
        log.wins.push((event.egg_pos, event.previously_won));
        log.answer
    }
}

#[test]
#[ignore = "daily"]
fn the_end_fight_hooks_cancel_answer_and_hear_the_fight() {
    if run_fresh_cases(
        "game::server_side::dragon::fight_hooks_tests::the_end_fight_hooks_cancel_answer_and_hear_the_fight",
        "SUNRISE_DRAGON_FIGHT_HOOKS_CHILD",
        &["end"],
    ) {
        return;
    }
    let log = Arc::new(Mutex::new(Log::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            assert_eq!(game.enter_command_level(Dim::End), Some(Dim::Overworld));
            let level = game.level("minecraft:the_end").unwrap();
            assert_eq!(
                level.dragon_fight(game),
                None,
                "not loaded before its first tick"
            );
            game.common.player =
                Some(PlayerMotion::new(game.common.motion, Vec3::new(0.5, 128.0, 0.5)).unwrap());
            // The chunks `EnderDragonFight.isArenaLoaded` waits for.
            {
                let streaming = &mut game.server.streaming;
                let world = &mut game.common.world;
                for z in -2..=1 {
                    for x in -2..=1 {
                        streaming.insert_remote(world, Chunk::empty(x, z));
                    }
                }
            }

            // Canceled ticks scan no players, so no dragon is made.
            log.lock().unwrap().cancel = true;
            for _ in 0..20 {
                game.tick_fight();
            }
            assert_eq!(log.lock().unwrap().ticks, 20);
            assert!(
                game.common.dragon.is_none(),
                "a canceled fight makes no dragon"
            );
            // `updatePlayers` every 20 ticks, then `findOrCreateDragon`.
            log.lock().unwrap().cancel = false;
            for _ in 0..20 {
                game.tick_fight();
            }
            assert!(game.common.dragon.is_some(), "the fight made its dragon");
            let fight = level.dragon_fight(game).unwrap();
            assert!(!fight.won && !fight.previously_won);
            assert_eq!(fight.respawn_stage, None);
            assert!(
                !level.dragon_fight_try_respawn(game),
                "no respawn while unwon"
            );

            // The first win with no answer: vanilla's egg on top of the podium's pillar
            // (origin to origin + 3), the MOTION_BLOCKING height of the column.
            let origin = [0, 70, 0];
            game.set_exit_portal(origin);
            game.dragon_killed();
            let egg = BlockPos::new(0, origin[1] + 4, 0);
            assert_eq!(log.lock().unwrap().wins, [(egg, false)]);
            assert_eq!(&*game.block_name_at([0, egg.y, 0]), "minecraft:dragon_egg");
            let air = game
                .common
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .unwrap();
            let _ = game.level_set([0, egg.y, 0], air);
            // A later win places none unless a listener answers.
            game.dragon_killed();
            assert_eq!(&*game.block_name_at([0, egg.y, 0]), "minecraft:air");
            log.lock().unwrap().answer = Some(true);
            game.dragon_killed();
            assert_eq!(&*game.block_name_at([0, egg.y, 0]), "minecraft:dragon_egg");
            assert_eq!(log.lock().unwrap().wins[1..], [(egg, true), (egg, true)]);
            assert_eq!(
                level.dragon_fight(game),
                Some(DragonFightState {
                    won: true,
                    previously_won: true,
                    respawn_stage: None,
                    alive_crystals: game.server.fight.alive_crystals,
                })
            );

            // A crystal on each side of the exit portal: `tryRespawn` starts.
            let mut ids = Vec::new();
            for [dx, dz] in [[0, -1], [0, 1], [-1, 0], [1, 0]] {
                game.server.next_entity += 1;
                ids.push(game.server.next_entity);
                let at = Vec3::new(
                    f64::from(origin[0] + dx * 3) + 0.5,
                    f64::from(origin[1] + 1),
                    f64::from(origin[2] + dz * 3) + 0.5,
                );
                game.common
                    .crystals
                    .push(Crystal::new(game.server.next_entity, at, false, 0));
            }
            assert!(level.dragon_fight_try_respawn(game));
            assert!(
                !level.dragon_fight_try_respawn(game),
                "one respawn at a time"
            );
            assert_eq!(log.lock().unwrap().stages, [DragonRespawnStage::Start]);
            let fight = level.dragon_fight(game).unwrap();
            assert_eq!(fight.respawn_stage, Some(DragonRespawnStage::Start));
            // Destroying one aborts it, and the listener hears it after.
            game.kill_crystal(ids[0]);
            let first = BlockPos::new(origin[0], origin[1] + 1, origin[2] - 3);
            assert_eq!(log.lock().unwrap().crystals, [(first, true)]);
            assert_eq!(level.dragon_fight(game).unwrap().respawn_stage, None);
            assert!(level.dragon_fight_reset_spike_crystals(game));

            // A crystal destroyed outside the End is no fight's.
            game.enter_command_level(Dim::Overworld);
            assert!(
                !level.dragon_fight_reset_spike_crystals(game),
                "the End is parked"
            );
            assert!(level.dragon_fight(game).is_some(), "read while parked");
            game.server.next_entity += 1;
            let stray = game.server.next_entity;
            game.common
                .crystals
                .push(Crystal::new(stray, Vec3::new(0.5, 64.0, 0.5), false, 0));
            game.kill_crystal(stray);
            assert_eq!(log.lock().unwrap().crystals.len(), 1);
        });
    });
}
