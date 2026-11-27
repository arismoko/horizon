//! Actual murder notification, with no server or world tick.
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::villager_brain;
use sunrise_sim::mobs::{Damage, Who};
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_acting_guest,
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

#[test]
#[ignore = "daily"]
fn murder_uses_only_the_victims_sensor_memory() {
    crate::game::server_side::kill_die_vibration_tests::game(|game| {
        game.mobs.clear();
        let player = [0, 0, 0, 101];
        game.level.player_uuid = Some(player);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:villager")
            .unwrap()
            .clone();
        let mut victim = Mob::new(91, kind.clone(), Vec3::new(1.0, 100.0, 1.0), 0.0);
        game.mobs
            .push(Mob::new(92, kind, Vec3::new(2.0, 100.0, 1.0), 0.0));
        game.villager_murdered(&victim, true, None);
        assert_eq!(
            game.villager_reputation_of(92, player),
            0,
            "a nearby villager absent from the last scan is not a witness"
        );
        let snapshot: Vec<_> = game
            .mobs
            .iter()
            .map(|m| m.seen(None, game.items()))
            .collect();
        let world = game.mob_world_base(&snapshot);
        let mut level = sunrise_sim::health::TestLevel::new(0);
        let mut events = Vec::new();
        for _ in 0..20 {
            villager_brain::tick(&mut victim, &world, &mut level.level(), &mut events);
        }
        assert_eq!(
            villager_brain::murder_witnesses(&mut victim, &world),
            vec![92]
        );
        game.villager_murdered(&victim, true, None);
        assert_eq!(game.villager_reputation_of(92, player), -125);
    });
}

/// Accepted damage writes the hit's own gossip in `Villager.setLastHurtByMob`, before
/// either the next hit or removal of its attacker. No world tick or artificial reset.
#[test]
#[ignore = "daily"]
fn every_accepted_hurt_counts_for_its_own_attacker() {
    if run_fresh_cases(
        "cut_layout::game::server_side::villagers::witness_tests::every_accepted_hurt_counts_for_its_own_attacker",
        "SUNRISE_VILLAGER_HURT_SEQUENCE_CHILD",
        &["same-tick"],
    ) {
        return;
    }
    with_game(|game| {
        game.mobs.clear();
        let kind = game
            .res
            .mob_types()
            .get("minecraft:villager")
            .unwrap()
            .clone();
        let at = Vec3::new(1.0, 100.0, 1.0);
        game.discard_resident_dragon();
        game.server.next_entity = 100;
        let dragon = game
            .spawn_mob_spawned(
                "minecraft:ender_dragon",
                at.components(),
                sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                    sunrise_sim::mobs::spawn_rules::Reason::SpawnItemUse,
                ),
            )
            .expect("a dragon admitted by the egg path");
        let mob_uuid = game
            .known_attacker_uuid(dragon)
            .expect("the dragon has its UUID before tracking or saving");
        for id in [91, 92] {
            game.mobs.push(Mob::new(id, kind.clone(), at, 0.0));
        }
        game.persist.uuids.insert(92, [0, 0, 0, 4]);
        game.admit_new_mobs(0);
        game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, at).unwrap());
        let player_hit = |game: &mut sunrise_gameplay::game::ServerGame<'_>, uuid, amount| {
            bind_local_player(game, 300, uuid);
            game.mob_hits_mob(91, amount, Damage::player([at.x, at.y, at.z]))
        };
        assert!(player_hit(game, 2, 1.0));
        assert!(player_hit(game, 3, 3.0));
        let mob = Damage::mob(dragon, at.components());
        assert!(game.mob_hits_mob(91, 5.0, mob));
        assert!(!game.mob_hits_mob(91, 2.0, Damage::mob(92, [at.x, at.y, at.z])));
        assert!(game.mob_hits_mob(91, 7.0, mob));
        game.discard_resident_dragon();
        assert!(player_hit(game, 5, 1000.0));
        assert!(!game.mobs[0].alive());
        let got = [
            mob_uuid,
            [0, 0, 0, 2],
            [0, 0, 0, 3],
            [0, 0, 0, 4],
            [0, 0, 0, 5],
        ]
        .map(|uuid| game.villager_reputation_of(91, uuid));
        let expected: Vec<i32> = match std::env::var_os("SUNRISE_REPUTATION_ORACLE") {
            Some(oracle) => std::fs::read_to_string(oracle)
                .unwrap()
                .lines()
                .find_map(|l| l.strip_prefix("hurt "))
                .unwrap()
                .trim_matches(['[', ']'])
                .split(',')
                .map(|s| s.parse().unwrap())
                .collect(),
            None => vec![-50, -25, -25, 0, -25],
        };
        assert_eq!(got.as_slice(), expected);
    });
}

/// The no-knockback-credit sting during the borrowed pufferfish tick uses the UUID
/// allocated when that fish entered the world, not a new identity at the point of damage.
#[test]
#[ignore = "daily"]
fn two_hits_of_a_mob_without_uuid_share_one() {
    if run_fresh_cases(
        "cut_layout::game::server_side::villagers::witness_tests::two_hits_of_a_mob_without_uuid_share_one",
        "SUNRISE_VILLAGER_HURT_STING_CHILD",
        &["sting"],
    ) {
        return;
    }
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.build_registries();
        let (common, server) = sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
        server
            .streaming
            .insert_remote(&mut common.world, sunrise_level::chunk::Chunk::empty(0, 0));
        game.mobs.clear();
        let fish = game
            .res
            .mob_types()
            .get("minecraft:pufferfish")
            .unwrap()
            .clone();
        let villager = game
            .res
            .mob_types()
            .get("minecraft:villager")
            .unwrap()
            .clone();
        let mut fish = Mob::new(90, fish, Vec3::new(1.5, 80.0, 1.5), 0.0);
        fish.extra.size = 2;
        fish.no_ai = true;
        fish.no_gravity = true;
        game.mobs.push(fish);
        for (id, x) in [(91, 1.25), (92, 1.75)] {
            let mut target = Mob::new(id, villager.clone(), Vec3::new(x, 80.0, 1.5), 0.0);
            target.no_ai = true;
            target.no_gravity = true;
            game.mobs.push(target);
        }
        game.admit_new_mobs(0);
        let uuid = game.persist.uuids[&90];
        game.tick_creature_entities().unwrap();
        assert_eq!(game.persist.uuids[&90], uuid);
        assert_eq!(game.villager_reputation_of(91, uuid), -25);
        assert_eq!(game.villager_reputation_of(92, uuid), -25);
    });
}

/// An indirect hit credits the cloud's guest owner, not the player in the acting body.
#[test]
#[ignore = "daily"]
fn hurt_gossip_belongs_to_the_hits_own_player() {
    if run_fresh_cases(
        "cut_layout::game::server_side::villagers::witness_tests::hurt_gossip_belongs_to_the_hits_own_player",
        "SUNRISE_VILLAGER_HURT_CLOUD_CHILD",
        &["cloud"],
    ) {
        return;
    }
    with_game(|game| {
        game.mobs.clear();
        let kind = game
            .res
            .mob_types()
            .get("minecraft:villager")
            .unwrap()
            .clone();
        let at = Vec3::new(1.0, 100.0, 1.0);
        game.mobs.push(Mob::new(91, kind, at, 0.0));
        game.admit_new_mobs(0);
        game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, at).unwrap());
        bind_local_player(game, 301, 9);
        game.server.lan_acting = Some(7);
        game.server.lan_acting_name = Some("Cloud owner".into());
        bind_acting_guest(game);
        game.server.lan_acting = None;
        bind_local_player(game, 302, 2);
        game.cloud_affect(
            sunrise_gameplay_common::game::potions_common::Victim::Mob(91),
            &[("instant_damage", 1, 0)],
            (&[], &[], 1.0),
            [at.x, at.y, at.z],
            &sunrise_game_data::text::Text::literal("Cloud"),
            (Some(Who::Player), Some(7)),
            None,
        );
        assert_eq!(game.villager_reputation_of(91, [0, 0, 0, 9]), -25);
        assert_eq!(game.villager_reputation_of(91, [0, 0, 0, 2]), 0);
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
