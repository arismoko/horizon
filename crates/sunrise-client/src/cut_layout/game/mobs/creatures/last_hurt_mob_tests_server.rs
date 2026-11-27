use mod_api::{EntityRef, LivingRef};
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::MOB;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::MOB_AGE;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::PIG;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::PLAYER;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::PLAYER_AGE;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::WOLF;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::add_mob;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::arrow;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::living;
use sunrise_gameplay::game::creatures::last_hurt_mob_tests::shoot;
use sunrise_gameplay_common::game::Dim;
use sunrise_level::chunk::Chunk;
use sunrise_sim::mobs::Damage;
use sunrise_sim::mobs::MobEvent;
use sunrise_sim::mobs::Who;
use sunrise_sim::motion::PlayerMotion;
use sunrise_sim::projectiles::ProjectileKind;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

#[test]
#[ignore = "daily"]
fn last_hurt_mob_follows_the_attacking_entity() {
    const CASE: &str = "SUNRISE_LAST_HURT_MOB_CASE";
    if run_fresh_cases(
        "cut_layout::game::server_side::creatures_server::last_hurt_mob_tests::last_hurt_mob_follows_the_attacking_entity",
        CASE,
        &[
            "arrow",
            "wind_charge",
            "melee",
            "spear_order",
            "other_level",
            "wolf",
        ],
    ) {
        return;
    }
    with_game(|game| {
        game.enter_level(Dim::Overworld).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(game.entity_ticking_area()(1, 0));
        game.world.game_time = 6000;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 80.0, 0.5)).unwrap());
        bind_local_player(game, PLAYER, 123);
        game.player.as_mut().unwrap().health.tick_count = PLAYER_AGE;
        add_mob(game, PIG, "minecraft:pig", 4.0);
        let player = living(game, PLAYER);
        let pig = game.entity(PIG).unwrap();
        assert_eq!(player.entity().tick_count(game), Some(PLAYER_AGE));
        assert_eq!(player.last_hurt_mob(game), None);
        assert_eq!(living(game, PIG).last_hurt_mob(game), None);

        match std::env::var(CASE).unwrap().as_str() {
            "arrow" => {
                game.mobs[0].invulnerable_flag = true;
                let health = game.mobs[0].health;
                shoot(game, Who::Player, arrow());
                assert_eq!(game.mobs[0].health, health, "the pig refused damage");
                assert_eq!(player.last_hurt_mob(game), Some((pig, PLAYER_AGE)));
                game.player.as_mut().unwrap().health.tick_count += 200;
                game.tick_mob_effects();
                assert_eq!(
                    player.last_hurt_mob(game),
                    Some((pig, PLAYER_AGE)),
                    "no expiry"
                );
                game.mobs[0].health = 0.0;
                assert_eq!(
                    player.last_hurt_mob(game),
                    None,
                    "dead targets read as absent immediately"
                );
                game.tick_mob_effects();
                game.mobs[0].health = health;
                assert_eq!(
                    player.last_hurt_mob(game),
                    None,
                    "the player's base tick cleared it"
                );
                let stale = LivingRef::new(EntityRef::new(player.entity().epoch(), 111));
                assert_eq!(stale.last_hurt_mob(game), None);
            }
            "wind_charge" => {
                add_mob(game, MOB, "minecraft:breeze", 2.0);
                game.mobs[1].age = MOB_AGE;
                game.mobs[0].invulnerable_flag = true;
                let health = game.mobs[0].health;
                shoot(game, Who::Mob(MOB), ProjectileKind::BreezeWindCharge);
                let breeze = living(game, MOB);
                assert_eq!(game.mobs[0].health, health, "the pig refused damage");
                assert_eq!(breeze.entity().tick_count(game), Some(MOB_AGE as i32));
                assert_eq!(breeze.last_hurt_mob(game), Some((pig, MOB_AGE as i32)));
                assert_eq!(
                    player.last_hurt_mob(game),
                    None,
                    "the shooter was the breeze"
                );
                game.mobs[0].health = 0.0;
                assert_eq!(breeze.last_hurt_mob(game), None);
                game.tick_creature_entities().unwrap();
                let breeze = game.mobs.iter().find(|m| m.id == MOB).unwrap();
                assert_eq!(breeze.last_hurt_mob, None, "the mob's base tick cleared it");
            }
            "melee" => {
                game.mobs[0].invulnerable_flag = true;
                game.server.attack_ticks = 20;
                game.attack_mob(0);
                assert_eq!(
                    player.last_hurt_mob(game),
                    None,
                    "a refused hit cannot set it"
                );
                game.mobs[0].invulnerable_flag = false;
                game.server.attack_ticks = 20;
                game.attack_mob(0);
                assert_eq!(player.last_hurt_mob(game), Some((pig, PLAYER_AGE)));

                add_mob(game, MOB, "minecraft:zombie", 3.0);
                game.mobs[1].age = MOB_AGE;
                let zombie = living(game, MOB);
                let swing = MobEvent::AttackMob {
                    target: PIG,
                    amount: 1.0,
                    source: Damage::mob(MOB, [3.0, 80.0, 0.5]),
                    remember_target: true,
                };
                game.mobs[0].invulnerable_flag = true;
                game.handle_mob_events(vec![swing.clone()]).unwrap();
                assert_eq!(zombie.last_hurt_mob(game), None);
                game.mobs[0].invulnerable_flag = false;
                game.mobs[0].invulnerable = 0;
                game.handle_mob_events(vec![swing]).unwrap();
                assert_eq!(zombie.last_hurt_mob(game), Some((pig, MOB_AGE as i32)));
            }
            "spear_order" => {
                add_mob(game, MOB, "minecraft:zombie", 3.0);
                game.mobs[1].age = MOB_AGE;
                let zombie = living(game, MOB);
                let stab = |target, contact, attempted_damage, otherwise_affected| {
                    vec![MobEvent::StabMemory {
                        attacker: MOB,
                        target,
                        contact,
                        attempted_damage,
                        otherwise_affected,
                    }]
                };
                // The pig was the second contact; the player's row, the first contact, is
                // delivered after it with that player's actions and must not overwrite it.
                game.handle_mob_events(stab(Who::Mob(PIG), 1, false, true))
                    .unwrap();
                game.handle_mob_events(stab(Who::Player, 0, false, true))
                    .unwrap();
                assert_eq!(zombie.last_hurt_mob(game), Some((pig, MOB_AGE as i32)));
                // A later contact whose only effect was a refused hurt writes nothing.
                game.server.combat.0.hit_landed = false;
                game.handle_mob_events(stab(Who::Player, 2, true, false))
                    .unwrap();
                assert_eq!(zombie.last_hurt_mob(game), Some((pig, MOB_AGE as i32)));
                // A later contact that pushed the player is the last one.
                game.handle_mob_events(stab(Who::Player, 2, false, true))
                    .unwrap();
                assert_eq!(
                    zombie.last_hurt_mob(game),
                    Some((player.entity(), MOB_AGE as i32))
                );
            }
            "other_level" => {
                game.server.attack_ticks = 20;
                game.attack_mob(0);
                assert_eq!(player.last_hurt_mob(game), Some((pig, PLAYER_AGE)));
                game.enter_level(Dim::Nether).unwrap();
                game.tick_mob_effects();
                assert_eq!(
                    game.player.as_ref().unwrap().health.last_hurt_mob,
                    Some((PIG, PLAYER_AGE)),
                    "the pig is alive in the level the player left"
                );
                game.enter_level(Dim::Overworld).unwrap();
                assert_eq!(player.last_hurt_mob(game), Some((pig, PLAYER_AGE)));
            }
            "wolf" => {
                add_mob(game, WOLF, "minecraft:wolf", 1.5);
                let owner = game.acting_uuid();
                let wolf = &mut game.mobs[1];
                sunrise_sim::mobs::tame::set_tame(wolf, true);
                wolf.extra.owner = Some(owner);
                wolf.no_ai = false;
                game.tick_creature_entities().unwrap();
                assert!(
                    game.mobs
                        .iter()
                        .find(|m| m.id == WOLF)
                        .unwrap()
                        .ai
                        .target
                        .is_none()
                );
                shoot(game, Who::Player, arrow());
                for _ in 0..2 {
                    game.tick_creature_entities().unwrap();
                }
                let wolf = game.mobs.iter().find(|m| m.id == WOLF).unwrap();
                assert_eq!(wolf.ai.target, Some(Who::Mob(PIG)));
            }
            case => panic!("unexpected fixture case {case}"),
        }
    });
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
