//! A hit listener plays level sounds and draws the attacker's own random through its `HitHost`,
//! for each way the issue's mob scenario lands (player to mob, mob to mob, mob to player): the
//! sounds queue at the call with the level's sound seeds, the attacker's own stream advances, and
//! the level's random does not.
use super::native_entity_refs::tests::{bind_local_player, with_resident_game};
use mod_api::{
    EntityRef, Flow, HitHost, Hooks, LivingIncomingDamageEvent, SoundSource, WorldEpoch,
};
use std::sync::{Arc, Mutex};
use sunrise_command::args::GameMode;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use sunrise_sim::mobs::{Damage, DamageKind, Mob, Who};
use sunrise_sim::motion::PlayerMotion;

/// The handles the listener draws on, and what it saw of those it is not lent.
#[derive(Default)]
struct Handles {
    attacker: Option<EntityRef>,
    other: Option<EntityRef>,
    stale: Option<EntityRef>,
    refused: Option<Refusals>,
}

#[derive(Debug, PartialEq)]
struct Refusals {
    other_float: Option<f32>,
    other_sound: bool,
    stale_float: Option<f32>,
    stale_sound: bool,
    zero_bound: i32,
}

struct Probe(Arc<Mutex<Handles>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        level: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        let mut handles = self.0.lock().unwrap();
        let attacker = handles.attacker.unwrap();
        let (roll, zero_bound) = attacker
            .with_random(level, |rng| {
                let roll = rng.next_float();
                let zero_bound = rng.next_int_bound(0);
                let _ = rng.next_double();
                (roll, zero_bound)
            })
            .unwrap();
        let pitch = 0.8 + roll * 0.4;
        assert!(level.play_sound_at(
            "minecraft:entity.player.attack.strong",
            [1.0, 64.0, 2.0],
            SoundSource::Players,
            1.0,
            pitch,
        ));
        assert!(level.play_sound_from(
            attacker,
            "minecraft:entity.player.attack.weak",
            SoundSource::Players,
            0.5,
            1.0,
        ));
        let (other, stale) = (handles.other.unwrap(), handles.stale.unwrap());
        handles.refused = Some(Refusals {
            other_float: other.with_random(level, |rng| rng.next_float()),
            other_sound: level.play_sound_from(
                other,
                "minecraft:entity.pig.ambient",
                SoundSource::Neutral,
                1.0,
                1.0,
            ),
            stale_float: stale.with_random(level, |rng| rng.next_float()),
            stale_sound: level.play_sound_from(
                stale,
                "minecraft:entity.pig.ambient",
                SoundSource::Neutral,
                1.0,
                1.0,
            ),
            zero_bound,
        });
        event.container.set_new_damage(0.0).unwrap();
        Flow::Cancel
    }
}

/// Runs `hit` in a game with the probe installed: the attacker (a player or a mob) is the one
/// the listener draws on. Checks the two queued sounds, the seeds against a stream seeded
/// independently, the attacker's stream and the level's random.
fn check(
    attacker: i32,
    expected_at: [f64; 3],
    hit: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    let handles = Arc::new(Mutex::new(Handles::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(handles.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_resident_game(|game| {
            // Server-level sounds need the live server hub's sound-seed stream.
            game.lan_host(
                sunrise_gameplay::game::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                sunrise_sim::lan::NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..sunrise_sim::lan::NetConfig::lan(0, String::new(), 2)
                },
                sunrise_sim::lan::hub::HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: game.dim.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 2,
                    integrated: true,
                    levels: game.server.level_shapes.clone(),
                    level_seeds: game.server.levels.0.runtime.seeds(),
                },
            )
            .unwrap();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            bind_local_player(game, 7, 7);
            game.set_game_mode(GameMode::Survival);
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for id in [300, 301, 302] {
                game.mobs.push(Mob::new(id, kind.clone(), Vec3::ZERO, 0.0));
            }
            let epoch = game.entity(7).unwrap().epoch();
            {
                let mut handles = handles.lock().unwrap();
                handles.attacker = game.entity(attacker as u32);
                handles.other = game.entity(301);
                handles.stale = Some(EntityRef::new(WorldEpoch::new(epoch.get() + 1), 7));
            }
            // Independently seeded expectations.
            let mut attacker_random = if attacker == 7 {
                (*game.player.as_ref().unwrap().random).clone()
            } else {
                game.mobs
                    .iter()
                    .find(|m| m.id as i32 == attacker)
                    .unwrap()
                    .random
                    .clone()
            };
            let expected_pitch = 0.8 + attacker_random.next_float() * 0.4;
            let _ = attacker_random.next_double();
            let mut seeds = LegacyRandom::new(0x5eed);
            let expected_seeds = [seeds.next_long(), seeds.next_long()];
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .set_sound_seeds(LegacyRandom::new(0x5eed));
            let mut level_random = game.world.random.clone();
            game.server.pending_native_sounds.clear();

            hit(game);

            let sounds = &game.server.pending_native_sounds;
            assert_eq!(sounds.len(), 2);
            assert_eq!(sounds[0].sound, "minecraft:entity.player.attack.strong");
            assert_eq!(sounds[0].at, [1.0, 64.0, 2.0]);
            assert_eq!(sounds[0].pitch, expected_pitch);
            assert_eq!(sounds[0].source, SoundSource::Players.index() as i32);
            assert_eq!(sounds[0].entity, None);
            assert_eq!(sounds[1].sound, "minecraft:entity.player.attack.weak");
            assert_eq!(sounds[1].entity, Some(attacker));
            assert_eq!(sounds[1].at, expected_at);
            assert_eq!(sounds[1].volume, 0.5);
            assert_eq!([sounds[0].seed, sounds[1].seed], expected_seeds);
            // Two draws in one loan: the attacker's stream continues where the copy does.
            let after = if attacker == 7 {
                game.player.as_mut().unwrap().random.next_int()
            } else {
                game.mobs
                    .iter_mut()
                    .find(|m| m.id as i32 == attacker)
                    .unwrap()
                    .random
                    .next_int()
            };
            assert_eq!(after, attacker_random.next_int());
            assert_eq!(game.world.random.next_int(), level_random.next_int());
            // A mob the hit does not lend, and a handle of another open, refuse and queue nothing.
            assert_eq!(
                handles.lock().unwrap().refused,
                Some(Refusals {
                    other_float: None,
                    other_sound: false,
                    stale_float: None,
                    stale_sound: false,
                    zero_bound: 0,
                })
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_player_hit_on_a_mob_plays_sounds_and_draws_the_players_random() {
    check(7, [0.0, 0.0, 0.0], |game| {
        assert!(!game.mob_hits_mob(300, 4.0, Damage::player([0.0, 0.0, 0.0])));
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_mob_hit_on_a_mob_plays_sounds_and_draws_the_attacking_mobs_random() {
    check(302, [0.0, 0.0, 0.0], |game| {
        let source = Damage {
            kind: DamageKind::MobAttack,
            attacker: Some(Who::Mob(302)),
            from: Some([0.0, 0.0, 0.0]),
            direct: None,
            attacker_species: None,
        };
        assert!(!game.mob_hits_mob(300, 4.0, source));
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_mob_hit_on_the_player_plays_sounds_and_draws_the_attacking_mobs_random() {
    check(302, [0.0, 0.0, 0.0], |game| {
        game.mob_hits_player_caused(4.0, [0.0, 0.0, 0.0], "minecraft:mob_attack", Some(302));
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
