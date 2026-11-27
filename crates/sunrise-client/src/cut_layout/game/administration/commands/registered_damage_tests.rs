use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::{
    health::{damage_type, source_damage_type},
    mobs::{Damage, DamageKind, NamedDamage as Tags, Who, gear},
};
use sunrise_world::damage_source::Catalog;

#[path = "registered_damage_tests/creaking_borrowed.rs"]
pub(crate) mod creaking_borrowed;

#[test]
fn formerly_aliased_types_keep_their_id_and_registry_tags() {
    let registry_backed = ["minecraft:indirect_magic", "minecraft:fly_into_wall"];
    let named = [
        (
            "minecraft:dragon_breath",
            Tags::BYPASSES_ARMOR | Tags::NO_KNOCKBACK,
        ),
        (
            "minecraft:mob_projectile",
            Tags::IS_PROJECTILE | Tags::SULFUR_CUBE_WITH_BLOCK_IMMUNE_TO,
        ),
        (
            "minecraft:unattributed_fireball",
            Tags::IS_FIRE | Tags::IS_PROJECTILE,
        ),
        (
            "minecraft:explosion",
            Tags::IS_EXPLOSION | Tags::NO_KNOCKBACK,
        ),
        (
            "minecraft:player_explosion",
            Tags::IS_EXPLOSION | Tags::NO_KNOCKBACK,
        ),
        (
            "minecraft:bad_respawn_point",
            Tags::IS_EXPLOSION | Tags::NO_KNOCKBACK,
        ),
        (
            "minecraft:ender_pearl",
            Tags::IS_FALL | Tags::BYPASSES_ARMOR | Tags::NO_KNOCKBACK,
        ),
    ];
    // Model only the tags relevant to these cases. The raw tag fixture and
    // expected flags are separate so an omitted or misread tag fails the test.
    let types: Vec<_> = named
        .iter()
        .map(|(id, _)| *id)
        .chain(registry_backed)
        .map(|id| {
            serde_json::json!({
                "id": id, "message_id": "test", "exhaustion": "00000000",
                "scaling": "never", "effects": "hurt", "death_message_type": "default"
            })
        })
        .collect();
    let mut tags = serde_json::json!({
        "minecraft:bypasses_armor": r#"{"values":["dragon_breath","ender_pearl","fly_into_wall","indirect_magic"]}"#,
        "minecraft:no_knockback": r#"{"values":["dragon_breath","explosion","player_explosion","bad_respawn_point","ender_pearl","fly_into_wall"]}"#,
        "minecraft:witch_resistant_to": r#"{"values":["indirect_magic"]}"#,
        "minecraft:is_projectile": r#"{"values":["mob_projectile","unattributed_fireball"]}"#,
        "minecraft:is_fire": r#"{"values":["unattributed_fireball"]}"#,
        "minecraft:is_explosion": r#"{"values":["explosion","player_explosion","bad_respawn_point"]}"#,
        "minecraft:is_fall": r#"{"values":["ender_pearl"]}"#,
        "minecraft:sulfur_cube_with_block_immune_to": r#"{"values":["mob_projectile"]}"#,
    });
    let types = serde_json::json!({"data_version": sunrise_world::DATA_VERSION, "types": types});
    let catalog = Catalog::from_json(&types, &tags).unwrap();

    for (id, expected_tags) in named {
        let kind = sunrise_gameplay::game::ServerGame::damage_kind(&catalog, id);
        assert_eq!(kind, DamageKind::Named(Tags::new(id, expected_tags)));
        assert_eq!(damage_type(kind, false), id);
        assert_eq!(damage_type(kind, true), id);
    }
    let collision = Damage::of(sunrise_gameplay::game::ServerGame::damage_kind(
        &catalog,
        "minecraft:fly_into_wall",
    ));
    assert!(collision.bypasses_armor() && collision.no_knockback());
    assert!(!collision.causes_panic());
    let magic = Damage::of(sunrise_gameplay::game::ServerGame::damage_kind(
        &catalog,
        "minecraft:indirect_magic",
    ));
    assert!(magic.bypasses_armor() && magic.kind.witch_resistant_to());
    assert!(!magic.no_knockback());

    // A world's data pack can replace the defaults for both explicitly selected IDs.
    tags["minecraft:bypasses_armor"] = r#"{"values":["dragon_breath","ender_pearl"]}"#.into();
    tags["minecraft:bypasses_invulnerability"] =
        r#"{"values":["fly_into_wall","indirect_magic"]}"#.into();
    let overridden = Catalog::from_json(&types, &tags).unwrap();
    for id in registry_backed {
        let kind = sunrise_gameplay::game::ServerGame::damage_kind(&overridden, id);
        assert_eq!(damage_type(kind, false), id);
        assert_eq!(damage_type(kind, true), id);
        assert!(kind.bypasses_invulnerability(), "{id}");
        assert!(!Damage::of(kind).bypasses_armor(), "{id}");
    }
}

#[test]
#[ignore = "daily"]
fn every_world_damage_type_round_trips_with_or_without_an_attacker() {
    with_game(|game| {
        let catalog = &game.health_resources.damage;
        for (_, definition) in catalog.entries() {
            let kind = sunrise_gameplay::game::ServerGame::damage_kind(catalog, &definition.name);
            assert_eq!(damage_type(kind, false), definition.name);
            assert_eq!(damage_type(kind, true), definition.name);
        }
    });
}

#[test]
#[ignore = "daily"]
fn damage_command_uses_the_worlds_fall_tags() {
    with_game(|game| {
        game.build_registries();
        game.as_console(|game| {
            for (id, ty, health) in [
                (1, "minecraft:ender_pearl", 7.4),
                (2, "minecraft:fly_into_wall", 5.0),
            ] {
                assert!(game.run_command(&format!(
                    "summon pig 0.5 64 0.5 {{UUID:[I;83,0,0,{id}],NoAI:1b}}"
                )));
                let pig = game.mobs.last_mut().unwrap();
                pig.extra.set_slot_id(
                    sunrise_core::equipment_slot::EquipmentSlot::Feet as usize,
                    Some("minecraft:leather_boots".into()),
                );
                gear::upgrade(pig, gear::FEET, "minecraft:feather_falling", 4);
                assert!(
                    game.run_command(&format!("damage 00000053-0000-0000-0000-{id:012x} 5 {ty}"))
                );
                let pig = game.mobs.last().unwrap();
                assert_eq!(pig.health, health, "{ty}");
                assert_eq!(source_damage_type(pig.last_damage.unwrap().0), ty);
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn damage_command_keeps_explicit_explosion_with_an_attacker() {
    with_game(|game| {
        game.build_registries();
        game.as_console(|game| {
            assert!(game.run_command("summon pig 0.5 64 0.5 {UUID:[I;83,0,0,1],NoAI:1b}"));
            assert!(game.run_command("summon pig 2.5 64 0.5 {UUID:[I;83,0,0,2],NoAI:1b}"));
            assert!(game.run_command(
                "damage 00000053-0000-0000-0000-000000000001 1 minecraft:explosion by 00000053-0000-0000-0000-000000000002"
            ));
            let hit = game.mobs[0].last_damage.unwrap().0;
            assert_eq!(hit.attacker, Some(Who::Mob(game.mobs[1].id)));
            assert_eq!(source_damage_type(hit), "minecraft:explosion");
        });
    });
}

#[test]
fn projectile_and_command_targets_classify_actual_direct_entity_families() {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_sim::{
        health::{Attack, DirectEntity},
        mobs::{Damage, DamageKind, Who},
        projectiles::{Projectile, ProjectileKind},
    };

    let mut large = Projectile::new(
        ProjectileKind::LargeFireball { power: 1 },
        Vec3::ZERO,
        Some(Who::Player),
    );
    large.id = 10;
    let mut small = Projectile::new(ProjectileKind::SmallFireball, Vec3::ZERO, Some(Who::Player));
    small.id = 11;
    let mut arrow = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::ZERO,
        Some(Who::Player),
    );
    arrow.id = 12;
    let mut trident = Projectile::new(ProjectileKind::Trident, Vec3::ZERO, Some(Who::Player));
    trident.id = 13;
    let mut snowball = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, Some(Who::Mob(5)));
    snowball.id = 14;

    // Fireball caller wiring: only LargeFireball is DirectEntity::LargeFireball;
    // SmallFireball stays a plain Projectile even though both use DamageKind::Fireball.
    let mut large_attack = Attack {
        attacker: 7,
        ..Default::default()
    };
    sunrise_gameplay_common::game::CommonGame::set_projectile_source(&mut large_attack, &large);
    let mut small_attack = Attack {
        attacker: 7,
        ..Default::default()
    };
    sunrise_gameplay_common::game::CommonGame::set_projectile_source(&mut small_attack, &small);
    let fireball = Damage {
        kind: DamageKind::Fireball,
        attacker: Some(Who::Player),
        from: None,
        direct: None,
        attacker_species: None,
    };
    assert_eq!(large_attack.projectile_kind, DirectEntity::LargeFireball);
    assert!(large_attack.direct_is_large_fireball(&fireball));
    assert!(!large_attack.is_direct(&fireball));
    assert_eq!(small_attack.projectile_kind, DirectEntity::Projectile);
    assert!(!small_attack.direct_is_large_fireball(&fireball));
    assert!(small_attack.direct_entity(&fireball).is_projectile());
    assert!(!small_attack.is_direct(&fireball));

    // `/damage` target classification matches vanilla entity class families.
    let projectiles = [large, small, arrow, trident, snowball];
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Player),
            &projectiles
        ),
        DirectEntity::Living
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Mob(5)),
            &projectiles
        ),
        DirectEntity::Living
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Stand(6)),
            &projectiles
        ),
        DirectEntity::Living
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Dragon(7)),
            &projectiles
        ),
        DirectEntity::Living
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Projectile(10)),
            &projectiles
        ),
        DirectEntity::LargeFireball
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Projectile(11)),
            &projectiles
        ),
        DirectEntity::Projectile
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Projectile(12)),
            &projectiles
        ),
        DirectEntity::Arrow
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Projectile(13)),
            &projectiles
        ),
        DirectEntity::Arrow
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Projectile(14)),
            &projectiles
        ),
        DirectEntity::Projectile
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Trident(20)),
            &projectiles
        ),
        DirectEntity::Arrow
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Rocket(21)),
            &projectiles
        ),
        DirectEntity::Projectile
    );
    assert_eq!(
        sunrise_gameplay::game::ServerGame::target_direct_entity(
            Some(Target::Hook(22)),
            &projectiles
        ),
        DirectEntity::Projectile
    );
    for other in [
        None,
        Some(Target::Tnt(30)),
        Some(Target::Item(31)),
        Some(Target::Cloud(32)),
        Some(Target::Fangs(33)),
        Some(Target::Boat(34)),
        Some(Target::Crystal(35)),
    ] {
        assert_eq!(
            sunrise_gameplay::game::ServerGame::target_direct_entity(other, &projectiles),
            DirectEntity::Other,
            "{other:?}"
        );
    }
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::commands_server::world_cmds::registered_damage_tests::setup_live_damage_world;

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::commands_server::world_cmds::registered_damage_tests::spawn_test_projectile;

#[test]
#[ignore = "daily"]
fn live_damage_and_blast_creaking_reactions_and_timed_player_credit() {
    run_live_damage_and_blast_creaking_reactions_and_timed_player_credit();
}

pub(crate) fn run_live_damage_and_blast_creaking_reactions_and_timed_player_credit() {
    use mod_api::{ExplosionInteraction, ExplosionSettings};
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_sim::{mobs::creaking, projectiles::ProjectileKind};
    use {
        sunrise_gameplay_common::game::combat_common::PlayerCredit,
        sunrise_gameplay_common::game::commands_common::Target,
        sunrise_gameplay_common::game::side_ops::events::{
            TEST_POSTED_EVENT_ORDER, TEST_POSTED_EVENTS,
        },
    };

    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::commands::world_cmds::registered_damage_tests::live_damage_and_blast_creaking_reactions_and_timed_player_credit",
        "SUNRISE_CREAKING_LIVE_DAMAGE_CHILD",
        &["live-damage"],
    ) {
        return;
    }
    with_game(|game| {
        setup_live_damage_world(game);
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("time set 13000"));
        assert!(
            game.run_command("summon creaking 4.5 64 4.5 {UUID:[I;85,0,0,1],NoAI:1b,NoGravity:1b}")
        );
        let creaking_id = game.mobs.last().unwrap().id;
        let start_health = game.mobs.last().unwrap().health;
        // Its protecting heart: awake between pale oak logs, naming this creaking.
        for command in [
            "setblock 8 63 8 minecraft:pale_oak_log[axis=y]",
            "setblock 8 64 8 minecraft:creaking_heart[axis=y,creaking_heart_state=awake]{creaking:[I;85,0,0,1]}",
            "setblock 8 65 8 minecraft:pale_oak_log[axis=y]",
        ] {
            assert!(game.run_command(command), "{command}");
        }
        creaking::set_transient(
            game.mobs.iter_mut().find(|m| m.id == creaking_id).unwrap(),
            [8, 64, 8],
        );
        // `CreakingHeartBlockEntity.creakingHurt` from its protector starts the 100-tick emitter.
        use sunrise_level::containers::BlockEntity::CreakingHeart;
        let heart_emitter = |game: &sunrise_gameplay::game::ServerGame<'_>| match game
            .world
            .block_entities
            .get(&(8, 64, 8))
        {
            Some(CreakingHeart(heart)) => heart.emitter,
            _ => panic!("the protecting heart is gone"),
        };
        let clear_emitter = |game: &mut sunrise_gameplay::game::ServerGame<'_>| match game
            .world
            .block_entities
            .get_mut(&(8, 64, 8))
        {
            Some(CreakingHeart(heart)) => heart.emitter = 0,
            _ => panic!("the protecting heart is gone"),
        };

        assert!(game.run_command("summon pig 6.5 64 4.5 {UUID:[I;85,0,0,2],NoAI:1b,NoGravity:1b}"));
        let arrow_id = spawn_test_projectile(
            game,
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            Vec3::new(5.5, 64.8, 4.5),
            None,
            [85, 0, 0, 3],
        );
        game.add_primed_tnt(BlockPos::new(4, 64, 4), 0.0, 80, None, None);
        let unowned_tnt_id = game.tnts.last().unwrap().id;
        game.persist.uuids.insert(unowned_tnt_id, [85, 0, 0, 4]);

        assert!(
            game.run_command("summon wolf 6.5 64 5.5 {UUID:[I;85,0,0,5],NoAI:1b,NoGravity:1b}")
        );
        {
            let wolf = game.common.mobs.last_mut().unwrap();
            wolf.extra.tame = true;
            wolf.extra.owner = game.common.level.player_uuid;
        }
        assert!(
            game.run_command("summon wolf 6.5 64 6.5 {UUID:[I;85,0,0,6],NoAI:1b,NoGravity:1b}")
        );
        {
            let wolf = game.mobs.last_mut().unwrap();
            wolf.extra.tame = true;
            wolf.extra.owner = None;
        }

        let clear_anim = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            let c = game.mobs.iter_mut().find(|m| m.id == creaking_id).unwrap();
            c.extra.ints[0] = 0;
        };
        let creaking_state = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let c = game.mobs.iter().find(|m| m.id == creaking_id).unwrap();
            (c.health, c.extra.ints[0], c.ai.last_hurt_by_player)
        };

        // Vary damage type with fixed direct living and projectile entities: valid reactions return true.
        for ty in [
            "minecraft:generic",
            "minecraft:magic",
            "minecraft:explosion",
        ] {
            for direct in [
                "00000055-0000-0000-0000-000000000002",
                "00000055-0000-0000-0000-000000000003",
            ] {
                clear_anim(game);
                assert!(
                    game.run_command(&format!(
                        "damage 00000055-0000-0000-0000-000000000001 4 {ty} by {direct}"
                    )),
                    "{ty} by {direct}"
                );
                assert_eq!(
                    creaking_state(game),
                    (start_health, 8, 0),
                    "{ty} by {direct}"
                );
            }
        }

        // Vary entities with fixed mob_attack / mob_projectile: non-living/entityless hits return false.
        for ty in ["minecraft:mob_attack", "minecraft:mob_projectile"] {
            clear_anim(game);
            assert!(
                !game.run_command(&format!(
                    "damage 00000055-0000-0000-0000-000000000001 4 {ty}"
                )),
                "entityless {ty}"
            );
            assert_eq!(creaking_state(game), (start_health, 0, 0));

            clear_anim(game);
            assert!(
                !game.run_command(&format!(
                    "damage 00000055-0000-0000-0000-000000000001 4 {ty} by 00000055-0000-0000-0000-000000000004"
                )),
                "tnt {ty}"
            );
            assert_eq!(creaking_state(game), (start_health, 0, 0));
        }

        // A player hit posts ENTITY_ACTION before the heart reacts; `creakingHurt` places
        // and sounds the resin before the Creaking sways.
        assert_eq!(heart_emitter(game), 0);
        TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
        game.start_audio_capture();
        clear_anim(game);
        assert!(
            game.run_command(
                "damage 00000055-0000-0000-0000-000000000001 2 minecraft:generic by @p"
            )
        );
        assert_eq!(creaking_state(game), (start_health, 8, 100));
        assert_eq!(heart_emitter(game), 100);
        assert_eq!(game.server.lan_entity_events, [(creaking_id as i32, 66)]);
        let events = TEST_POSTED_EVENTS.with(|events| events.borrow_mut().take().unwrap());
        assert_eq!(
            events.first(),
            Some(&(
                "minecraft:entity_action",
                [4.5, 64.0, 4.5],
                Some(Who::Mob(creaking_id))
            ))
        );
        assert!(!events[1..].is_empty());
        assert!(
            events[1..]
                .iter()
                .all(|(event, _, source)| *event == "minecraft:block_place" && source.is_none())
        );
        let sounds = &game.server.pending_sounds;
        assert_eq!(
            sounds.last().unwrap().event,
            "minecraft:entity.creaking.sway"
        );
        assert!(
            sounds[..sounds.len() - 1]
                .iter()
                .all(|sound| sound.event == "minecraft:block.resin.place")
        );
        assert_eq!(sounds.len(), events.len());

        // Ownerless follow-up reacts while the timer is live and refuses once it expires.
        clear_anim(game);
        assert!(
            game.run_command("damage 00000055-0000-0000-0000-000000000001 1 minecraft:generic")
        );
        assert_eq!(creaking_state(game), (start_health, 8, 100));

        clear_anim(game);
        game.mobs
            .iter_mut()
            .find(|m| m.id == creaking_id)
            .unwrap()
            .ai
            .last_hurt_by_player = 0;
        game.server.last_hurt_by_player.remove(&creaking_id);
        assert!(
            !game.run_command("damage 00000055-0000-0000-0000-000000000001 1 minecraft:generic")
        );
        assert_eq!(creaking_state(game), (start_health, 0, 0));

        // Tame wolf owned by a live player sets player credit; an ownerless tame wolf clears it.
        clear_anim(game);
        assert!(game.run_command(
            "damage 00000055-0000-0000-0000-000000000001 2 minecraft:generic by 00000055-0000-0000-0000-000000000005"
        ));
        assert_eq!(creaking_state(game), (start_health, 8, 100));

        clear_anim(game);
        assert!(game.run_command(
            "damage 00000055-0000-0000-0000-000000000001 2 minecraft:generic by 00000055-0000-0000-0000-000000000006"
        ));
        assert_eq!(creaking_state(game), (start_health, 8, 0));

        // Live blasts: unowned TNT does not trigger without player credit, triggers with live
        // remembered player credit, and player-lit TNT replaces stale offline player credit.
        clear_anim(game);
        clear_emitter(game);
        game.server.last_hurt_by_player.remove(&creaking_id);
        game.native_explode(
            Some((unowned_tnt_id, Target::Tnt(unowned_tnt_id))),
            [4.5, 64.0, 4.5],
            3.0,
            false,
            ExplosionInteraction::None,
            ExplosionSettings::default(),
        )
        .unwrap();
        assert_eq!(creaking_state(game), (start_health, 0, 0));
        assert_eq!(heart_emitter(game), 0);

        assert!(
            game.run_command(
                "damage 00000055-0000-0000-0000-000000000001 1 minecraft:generic by @p"
            )
        );
        clear_anim(game);
        clear_emitter(game);
        game.native_explode(
            Some((unowned_tnt_id, Target::Tnt(unowned_tnt_id))),
            [4.5, 64.0, 4.5],
            3.0,
            false,
            ExplosionInteraction::None,
            ExplosionSettings::default(),
        )
        .unwrap();
        assert_eq!(creaking_state(game), (start_health, 8, 100));
        assert_eq!(heart_emitter(game), 100);

        clear_anim(game);
        clear_emitter(game);
        game.mobs
            .iter_mut()
            .find(|m| m.id == creaking_id)
            .unwrap()
            .ai
            .last_hurt_by_player = 50;
        game.server
            .last_hurt_by_player
            .insert(creaking_id, PlayerCredit(777));
        game.add_primed_tnt(BlockPos::new(4, 64, 4), 0.0, 80, Some(Who::Player), None);
        let player_tnt_id = game.tnts.last().unwrap().id;
        game.native_explode(
            Some((player_tnt_id, Target::Tnt(player_tnt_id))),
            [4.5, 64.0, 4.5],
            3.0,
            false,
            ExplosionInteraction::None,
            ExplosionSettings::default(),
        )
        .unwrap();
        assert_eq!(creaking_state(game), (start_health, 8, 100));
        assert_eq!(heart_emitter(game), 100);

        // The arrow's impact sound follows the Creaking's reaction to the live collision.
        // The heart is already emitting; observe the impact and the next real mob tick.
        clear_anim(game);
        game.load_ticking_neighbourhood(0, 0);
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(4, 4));
        assert!(game.creatures.projectiles.iter().any(|p| p.id == arrow_id));
        game.creatures.projectiles.for_each_mut(|p| {
            if p.id == arrow_id {
                p.velocity = Vec3::new(-1.0, 0.0, 0.0);
            }
        });
        game.server.pending_sounds.clear();
        game.server.lan_entity_events.clear();
        TEST_POSTED_EVENT_ORDER.with(|order| *order.borrow_mut() = Some(Vec::new()));
        let reaction = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let sounds = &game.server.pending_sounds;
            let is_hit_sound = |event: &str| {
                matches!(
                    event,
                    "minecraft:entity.creaking.sway" | "minecraft:entity.arrow.hit"
                )
            };
            let flashes = game.server.lan_entity_events.as_slice();
            let is_flash = |event: &(i32, i8)| *event == (creaking_id as i32, 66);
            let action = TEST_POSTED_EVENT_ORDER.with(|order| {
                order
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|(event, source, _, _)| {
                        *event == "minecraft:entity_action"
                            && *source == Some(Who::Mob(creaking_id))
                    })
                    .map(|(_, _, flash_cursor, sound_cursor)| {
                        [
                            flashes[..*flash_cursor]
                                .iter()
                                .filter(|e| is_flash(e))
                                .count(),
                            sounds[..*sound_cursor]
                                .iter()
                                .filter(|s| is_hit_sound(&s.event))
                                .count(),
                        ]
                    })
                    .collect::<Vec<_>>()
            });
            serde_json::json!({
                "action": action,
                "flashes": flashes.iter().filter(|e| is_flash(e)).count(),
                "sounds": sounds.iter().filter(|s| is_hit_sound(&s.event)).map(|s| &s.event).collect::<Vec<_>>(),
                "emitter": heart_emitter(game),
                "animation": creaking_state(game).1,
            })
        };
        game.tick_projectiles().unwrap();
        let impact = reaction(game);
        assert_eq!(
            impact,
            serde_json::json!({
                "action": [[1, 0]], "flashes": 1,
                "sounds": ["minecraft:entity.creaking.sway", "minecraft:entity.arrow.hit"],
                "emitter": 100, "animation": 8,
            })
        );
        game.mobs
            .iter_mut()
            .find(|m| m.id == creaking_id)
            .unwrap()
            .no_ai = false;
        game.tick_creature_entities().unwrap();
        let next_tick = reaction(game);
        TEST_POSTED_EVENT_ORDER.with(|order| *order.borrow_mut() = None);
        let mut expected = impact.clone();
        expected["animation"] = 7.into();
        assert_eq!(
            next_tick, expected,
            "the mob tick must not replay the hurt reaction"
        );
        if let Some(path) = std::env::var_os("SUNRISE_CREAKING_HIT_ORDER_OUTPUT") {
            std::fs::write(path, serde_json::json!([impact, next_tick]).to_string()).unwrap();
        }
    });
}

#[test]
#[ignore = "daily"]
fn live_damage_and_blast_ghast_reflected_fireball() {
    use mod_api::{ExplosionInteraction, ExplosionSettings};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_sim::projectiles::ProjectileKind;

    with_game(|game| {
        setup_live_damage_world(game);
        assert!(
            game.run_command("summon pig 10.5 64 4.5 {UUID:[I;86,0,0,1],NoAI:1b,NoGravity:1b}")
        );
        let pig_id = game.mobs.last().unwrap().id;
        let player_large_id = spawn_test_projectile(
            game,
            ProjectileKind::LargeFireball { power: 1 },
            Vec3::new(4.5, 64.0, 4.5),
            Some(Who::Player),
            [86, 0, 0, 2],
        );
        let _small_id = spawn_test_projectile(
            game,
            ProjectileKind::SmallFireball,
            Vec3::new(4.5, 64.0, 4.5),
            Some(Who::Player),
            [86, 0, 0, 3],
        );
        let pig_large_id = spawn_test_projectile(
            game,
            ProjectileKind::LargeFireball { power: 1 },
            Vec3::new(4.5, 64.0, 4.5),
            Some(Who::Mob(pig_id)),
            [86, 0, 0, 4],
        );

        let spawn_ghast = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                           n: u32,
                           health: f32|
         -> (u32, String) {
            assert!(game.run_command(&format!(
                "summon ghast 4.5 64 4.5 {{UUID:[I;86,0,0,{n}],NoAI:1b,NoGravity:1b}}"
            )));
            let g = game.mobs.last_mut().unwrap();
            g.health = health;
            (g.id, format!("00000056-0000-0000-0000-{n:012x}"))
        };

        // Vary damage type with fixed reflected LargeFireball from @p: every type deals 1000.
        for (i, ty) in [
            "minecraft:fireball",
            "minecraft:unattributed_fireball",
            "minecraft:in_fire",
            "minecraft:generic",
        ]
        .into_iter()
        .enumerate()
        {
            let (id, uuid) = spawn_ghast(game, 10 + i as u32, 10.0);
            assert!(
                game.run_command(&format!(
                    "damage {uuid} 1 {ty} by 00000056-0000-0000-0000-000000000002 from @p"
                )),
                "{ty}"
            );
            assert_eq!(
                game.mobs.iter().find(|m| m.id == id).unwrap().health,
                0.0,
                "{ty}"
            );
        }

        // Reflected LargeFireball still returns true under Fire Resistance and on hurt cooldown.
        let (resist_id, resist_uuid) = spawn_ghast(game, 20, 10.0);
        game.mobs
            .iter_mut()
            .find(|m| m.id == resist_id)
            .unwrap()
            .add_effect("minecraft:fire_resistance", -1, 0);
        assert!(game.run_command(&format!(
            "damage {resist_uuid} 1 minecraft:fireball by 00000056-0000-0000-0000-000000000002 from @p"
        )));
        assert_eq!(
            game.mobs.iter().find(|m| m.id == resist_id).unwrap().health,
            10.0
        );

        let (cool_id, cool_uuid) = spawn_ghast(game, 21, 2000.0);
        assert!(game.run_command(&format!(
            "damage {cool_uuid} 1 minecraft:generic by 00000056-0000-0000-0000-000000000002 from @p"
        )));
        assert_eq!(
            game.mobs.iter().find(|m| m.id == cool_id).unwrap().health,
            1000.0
        );
        assert!(game.run_command(&format!(
            "damage {cool_uuid} 1 minecraft:generic by 00000056-0000-0000-0000-000000000002 from @p"
        )));
        assert_eq!(
            game.mobs.iter().find(|m| m.id == cool_id).unwrap().health,
            1000.0
        );

        // Vary entities with fixed minecraft:fireball: SmallFireball, direct player, and non-player
        // LargeFireball are all refused by fire immunity.
        let (immune_id, immune_uuid) = spawn_ghast(game, 22, 10.0);
        for args in [
            "by 00000056-0000-0000-0000-000000000003 from @p",
            "by @p",
            "by 00000056-0000-0000-0000-000000000004 from 00000056-0000-0000-0000-000000000001",
        ] {
            assert!(
                !game.run_command(&format!("damage {immune_uuid} 1 minecraft:fireball {args}")),
                "{args}"
            );
            assert_eq!(
                game.mobs.iter().find(|m| m.id == immune_id).unwrap().health,
                10.0,
                "{args}"
            );
        }

        // Live blasts: a player-owned LargeFireball blast deals 1000 damage to a Ghast, while a
        // mob-owned LargeFireball blast deals ordinary explosion damage.
        let (blast_player_id, _) = spawn_ghast(game, 30, 500.0);
        game.native_explode(
            Some((player_large_id, Target::Projectile(player_large_id))),
            [4.5, 64.0, 4.5],
            3.0,
            false,
            ExplosionInteraction::None,
            ExplosionSettings::default(),
        )
        .unwrap();
        assert_eq!(
            game.mobs
                .iter()
                .find(|m| m.id == blast_player_id)
                .unwrap()
                .health,
            0.0
        );

        let (blast_mob_id, _) = spawn_ghast(game, 31, 500.0);
        game.native_explode(
            Some((pig_large_id, Target::Projectile(pig_large_id))),
            [4.5, 64.0, 4.5],
            3.0,
            false,
            ExplosionInteraction::None,
            ExplosionSettings::default(),
        )
        .unwrap();
        // At the centre (distance 0, exposure 1) a radius-3 blast deals (1 + 1) / 2 * 7 * 6 + 1.
        assert_eq!(
            game.mobs
                .iter()
                .find(|m| m.id == blast_mob_id)
                .unwrap()
                .health,
            457.0
        );
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
