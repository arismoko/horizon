//! Production singleton installation/removal and id-allocation contracts.
#[cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_entity_refs::tests::{with_game, with_resident_game};
use sunrise_gameplay_common::game::creatures::SpawnSite;
use sunrise_gameplay_common::game::dragon_common::Dragon;

#[test]
#[ignore = "daily"]
fn singleton_dragon_spawn_install_refusal_and_kill() {
    with_game(|game| {
        game.dragon.discard();
        let before = game.server.next_entity;
        let id = game
            .spawn_dragon_spawned([0.0, 80.0, 0.0], SpawnSite::Plain)
            .unwrap();
        assert_eq!(id, before + 1);
        assert_eq!(game.server.next_entity, before + 9);
        for part in id + 1..=id + 8 {
            assert!(game.dragon_part(part).is_some());
        }
        assert!(
            game.spawn_dragon_spawned([1.0, 80.0, 0.0], SpawnSite::Plain)
                .is_none()
        );
        assert_eq!(game.server.next_entity, before + 9);
        let mut replacement = Dragon::new(Vec3::ZERO, 0.0, 0);
        replacement.id = 900;
        let refused = game.dragon.install(replacement).err().unwrap();
        assert_eq!(refused.id, 900);
        assert_eq!(game.dragon.as_ref().unwrap().id, id);
        game.kill_dragon(id + 1);
        assert_eq!(game.dragon.as_ref().unwrap().id, id);
        game.kill_dragon(id);
        assert!(game.dragon.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn singleton_dragon_load_keeps_state_and_reserves_nine_ids() {
    with_resident_game(|game| {
        game.dragon.discard();
        let mut saved = Dragon::new(Vec3::new(2.0, 90.0, 3.0), 25.0, 0);
        saved.health = 73.0;
        let uuid = [17, 0, 0, 3];
        let tag = saved.to_nbt(uuid, None, &Default::default());
        let before = game.server.next_entity;
        game.load_dragon(tag.clone(), Some(uuid));
        let dragon = game.dragon.as_ref().unwrap();
        assert_eq!(dragon.id, before + 1);
        assert_eq!(dragon.position, saved.position);
        assert_eq!(dragon.health, 73.0);
        assert!(dragon.join.is_joined());
        assert_eq!(game.dragon.locate(dragon.join), Some(0));
        assert_eq!(game.server.next_entity, before + 9);
        assert_eq!(game.persist.uuids.get(&dragon.id), Some(&uuid));
        game.load_dragon(tag, Some(uuid));
        assert_eq!(game.server.next_entity, before + 9);
        assert_eq!(game.dragon.as_ref().unwrap().id, before + 1);
        let chunks = game.entity_chunks(&[(0, 0)].into_iter().collect(), false);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].1.len(), 1);
        assert_eq!(
            chunks[0].1[0].get("Health"),
            Some(&Tag::Float(73.0f32.to_bits()))
        );
        assert_eq!(
            chunks[0].1[0].get("UUID"),
            Some(&Tag::IntArray(uuid.to_vec()))
        );
        let entity = game.entity(before + 1).unwrap();
        assert!(entity.discard(game));
        game.native_removals_flush();
        assert!(game.dragon.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn singleton_fight_dragon_install_reserves_nine_ids() {
    with_game(|game| {
        game.dragon.discard();
        let before = game.server.next_entity;
        game.create_new_dragon();
        let dragon = game.dragon.as_ref().unwrap();
        assert_eq!(dragon.id, before + 1);
        assert_eq!(game.server.next_entity, before + 9);
        assert!(dragon.join.is_joined());
        assert_eq!(
            game.server.fight.dragon_uuid,
            game.persist.uuids.get(&dragon.id).copied()
        );
    });
}

#[cfg(feature = "client")]
fn install_server_dragon(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32, at: [f64; 3]) {
    let mut dragon = Dragon::new(Vec3::new(at[0], at[1], at[2]), 0.0, 0);
    dragon.id = id;
    game.replace_resident_dragon(dragon);
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn singleton_client_dragon_install_explicitly_replaces_network_owner() {
    with_client_game(|game| {
        game.dragon.discard();
        game.spawn_remote_dragon(420, [1.0, 80.0, 2.0], 45.0);
        assert_eq!(game.dragon.as_ref().unwrap().id, 420);
        game.spawn_remote_dragon(840, [3.0, 90.0, 4.0], 90.0);
        let dragon = game.dragon.as_ref().unwrap();
        assert_eq!(dragon.id, 840);
        assert_eq!(dragon.position, Vec3::new(3.0, 90.0, 4.0));
        game.remove_remote_dragon(420);
        assert_eq!(game.dragon.as_ref().unwrap().id, 840);
        game.remove_remote_dragon(840);
        assert!(game.dragon.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn singleton_late_dragon_id_reserves_all_eight_parts_once() {
    with_game(|game| {
        game.dragon.discard();
        assert!(game.dragon.install(Dragon::new(Vec3::ZERO, 0.0, 0)).is_ok());
        let before = game.server.next_entity;
        game.assign_dragon_and_crystal_ids();
        assert_eq!(game.dragon.as_ref().unwrap().id, before + 1);
        assert_eq!(game.server.next_entity, before + 9);
        game.assign_dragon_and_crystal_ids();
        assert_eq!(game.server.next_entity, before + 9);
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn singleton_lifecycle_commands_unload_and_parked_owners() {
    crate::game::server_side::entity_server::taken_pass_tests::LifecycleFixture {
        seed: |game, entries| {
            game.discard_resident_dragon();
            for &(id, at) in entries {
                install_server_dragon(game, id, at);
            }
        },
        ids: |game| game.dragon.as_ref().map(|d| vec![d.id]).unwrap_or_default(),
        target: sunrise_gameplay_common::game::commands_common::Target::Dragon,
        unloads_with_chunk: true,
        remote: None,
    }
    .run_singleton();
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn singleton_death_fight_scan_and_resets_discard_owners() {
    with_game(|game| {
        install_server_dragon(game, 601, [1.0, 80.0, 1.0]);
        game.dragon.as_mut().unwrap().death_time = 199;
        game.tick_dragon_death();
        assert!(game.dragon.is_none(), "death completion");
        install_server_dragon(game, 602, [1.0, 80.0, 1.0]);
        game.scan_state();
        assert!(game.dragon.is_none(), "fight scan without exit portal");
        install_server_dragon(game, 603, [1.0, 80.0, 1.0]);
        game.clear_entities();
        assert!(game.dragon.is_none(), "world reset");
        install_server_dragon(game, 605, [1.0, 80.0, 1.0]);
        game.set_allow_commands(true);
        assert!(game.run_command("kill @e[type=minecraft:ender_dragon]"));
        assert!(game.dragon.is_none(), "command selector");
        install_server_dragon(game, 606, [1.0, 80.0, 1.0]);
        game.world.modified.clear();
        game.tick_dragon();
        assert!(
            game.world.modified.contains(&(0, 0)),
            "tick dirties the owner's chunk"
        );
    });
    with_client_game(|game| {
        game.spawn_remote_dragon(604, [1.0, 80.0, 1.0], 0.0);
        game.clear_remote_projectiles();
        assert!(game.dragon.is_none(), "remote reset");
    });
}

#[test]
#[ignore = "daily"]
fn singleton_join_rejection_discards_the_installed_dragon() {
    struct Veto;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Veto {
        fn entity_join_level(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityJoinLevelEvent<'_>,
        ) -> mod_api::Flow {
            if event.kind == "minecraft:ender_dragon" {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("dragon-veto", Box::new(Veto))].into(),
        || {
            with_game(|game| {
                assert!(
                    game.spawn_dragon_spawned([1.0, 80.0, 1.0], SpawnSite::Plain)
                        .is_none()
                );
                assert!(
                    game.dragon.is_none(),
                    "join rejection left its owner installed"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn singleton_live_owner_is_not_reported_as_departed() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Witness(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Witness {
        fn entity_leave_level(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::EntityLeaveLevelEvent<'_>,
        ) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let leaves = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "dragon-leaves",
            Box::new(Witness(leaves.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                let id = game
                    .spawn_dragon_spawned([1.0, 80.0, 1.0], SpawnSite::Plain)
                    .unwrap();
                game.native_leaves_flush();
                assert_eq!(leaves.load(Ordering::Relaxed), 0);
                game.kill_dragon(id);
                game.native_leaves_flush();
                assert_eq!(leaves.load(Ordering::Relaxed), 1);
            });
        },
    );
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

/// #5479: The killing blow publishes its source and saved weapon through the ordered death hooks.
#[test]
#[ignore = "daily"]
fn the_dragons_killing_blow_fires_the_living_death_hooks() {
    use mod_api::{
        DragonPhase, Flow, HitHost, Hooks, LivingDamagePostEvent, LivingDeathEvent,
        LivingDeathPostEvent, ModEntry, ServerHost,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use sunrise_gameplay::game::ServerGame;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_gameplay_common::game::dragon_common::{Crystal, FightWho, Phase, SPECIAL};
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
    use sunrise_gameplay_common::game::{Dim, TryIntoApi as _};
    use sunrise_items::items::ItemStack;
    use sunrise_sim::mobs::{DamageKind, Mob, Who};
    use sunrise_sim::motion::PlayerMotion;
    use sunrise_sim::projectiles::{Projectile, ProjectileKind};

    #[derive(Clone, Debug, PartialEq)]
    struct SeenDeath {
        kind: String,
        damage_type: String,
        causing: i32,
        direct: i32,
        direct_type: Option<String>,
        weapon: Option<mod_api::ItemStack>,
        health: Option<f32>,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct SeenPost {
        damage_type: String,
        attacker: u32,
        credited_player: Option<u128>,
    }

    #[derive(Default)]
    struct Probe {
        order: Mutex<Vec<&'static str>>,
        deaths: Mutex<Vec<SeenDeath>>,
        posts: Mutex<Vec<SeenPost>>,
        cancel: AtomicBool,
        discard: AtomicBool,
        sit_on_death: AtomicBool,
    }

    struct Deaths(Arc<Probe>);
    #[mod_api::__subscriptions]
    impl Hooks for Deaths {
        fn living_death(
            &self,
            host: &mut dyn ServerHost,
            event: &mut LivingDeathEvent<'_>,
        ) -> Flow {
            self.0.order.lock().unwrap().push("death");
            let entity = host.entity(event.entity).expect("dying dragon resolves");
            let health = host.living(entity).and_then(|living| living.health(host));
            self.0.deaths.lock().unwrap().push(SeenDeath {
                kind: event.kind.to_owned(),
                damage_type: event.source.damage_type.to_owned(),
                causing: event.source.causing,
                direct: event.source.direct,
                direct_type: event.source.direct_type.map(str::to_owned),
                weapon: event.weapon.cloned(),
                health,
            });
            if self.0.discard.load(Ordering::SeqCst) {
                assert!(entity.discard(host), "listener discards the dragon");
                return Flow::Continue;
            }
            if self.0.sit_on_death.load(Ordering::SeqCst) {
                let dragon = host.dragon(entity).expect("dragon handle");
                assert!(dragon.set_phase(host, DragonPhase::Hover));
                return Flow::Continue;
            }
            if self.0.cancel.load(Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn living_death_post(&self, _: &mut dyn ServerHost, event: &mut LivingDeathPostEvent<'_>) {
            self.0.order.lock().unwrap().push("post");
            self.0.posts.lock().unwrap().push(SeenPost {
                damage_type: event.damage_type.to_owned(),
                attacker: event.attacker,
                credited_player: event.credited_player,
            });
        }
        fn living_damage_post(&self, _: &mut dyn HitHost, _: &mut LivingDamagePostEvent<'_>) {
            self.0.order.lock().unwrap().push("damage_post");
        }
    }

    let probe = Arc::new(Probe::default());
    let mods = vec![ModEntry::new(
        "dragon_deaths",
        Box::new(Deaths(probe.clone())),
    )]
    .into();
    let clear = |probe: &Probe| {
        probe.order.lock().unwrap().clear();
        probe.deaths.lock().unwrap().clear();
        probe.posts.lock().unwrap().clear();
    };
    let holding = || Phase::Holding {
        path: None,
        target: None,
        clockwise: false,
    };
    let spawn_with_phase = |game: &mut ServerGame<'_>, phase: Phase| -> u32 {
        game.dragon.discard();
        let id = game
            .spawn_dragon_spawned([0.0, 80.0, 0.0], SpawnSite::Plain)
            .unwrap();
        let d = game.dragon.as_mut().unwrap();
        d.health = 1.0;
        d.set_phase(phase);
        id
    };
    let spawn_one_hp = |game: &mut ServerGame<'_>| -> u32 { spawn_with_phase(game, holding()) };

    crate::entry::install_client_hooks();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            fixture_resident_column(&mut game.world, (0, 0));
            let host_uuid = 77u128;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 55;
            game.level.player_uuid = Some(sunrise_sim::lan::playerdata::uuid_ints(host_uuid));

            // 1a. Flying dragon: acceptance vs cancellation, health/phase, order, and 20-tick cleanup.
            for cancelled in [false, true] {
                probe.cancel.store(cancelled, Ordering::SeqCst);
                probe.discard.store(false, Ordering::SeqCst);
                probe.sit_on_death.store(false, Ordering::SeqCst);
                clear(&probe);
                let id = spawn_one_hp(game);
                game.server.fight.dragon_uuid = game.persist.uuids.get(&id).copied();
                game.server.fight.killed = false;
                game.server.fight.previously_killed = false;
                game.orbs.clear();
                game.hurt_dragon(
                    0,
                    50.0,
                    true,
                    Some(FightWho::Host),
                    DamageKind::PlayerAttack,
                );
                let d = game.dragon.as_ref().unwrap();
                let deaths = probe.deaths.lock().unwrap();
                assert_eq!(deaths.len(), 1);
                assert_eq!(deaths[0].kind, "minecraft:ender_dragon");
                assert_eq!(deaths[0].damage_type, "minecraft:player_attack");
                assert_eq!(
                    deaths[0].health,
                    Some(0.0),
                    "living_death observes 0 health before handleKillingBlow"
                );
                drop(deaths);
                if cancelled {
                    assert_eq!(d.health, 0.0, "cancelled death preserves zero-health body");
                    assert!(d.death_canceled, "cancelled death records veto");
                    assert!(
                        matches!(d.phase, Phase::Holding { .. }),
                        "cancelled death skips handleKillingBlow"
                    );
                    assert_eq!(*probe.order.lock().unwrap(), ["death", "damage_post"]);
                    assert!(probe.posts.lock().unwrap().is_empty());
                    // Engine persistence stores a genuine veto in versioned metadata, not raw entity NBT.
                    game.save_entities(true).unwrap();
                    let records = game.server.save.0.load_entities(0, 0).unwrap();
                    assert_eq!(records.len(), 1);
                    assert!(
                        records[0].get("sunrise:death_canceled").is_none(),
                        "veto is not written as a raw entity NBT field"
                    );
                    let meta_path = game
                        .server
                        .save
                        .0
                        .root
                        .join("data/sunrise/entity_metadata.dat");
                    let meta = sunrise_sim::save::read_dat(&meta_path).unwrap();
                    assert_eq!(meta.get("version"), Some(&Tag::Int(1)));
                    game.dragon.discard();
                    game.persist.dragon_death_canceled = None;
                    game.persist.entity_metadata_on_disk = false;
                    game.world.queue_entity_nbt((0, 0), records);
                    game.spawn_loaded_entities();
                    assert!(
                        game.dragon.as_ref().unwrap().death_canceled,
                        "persisted veto survives save/load through versioned engine metadata"
                    );
                    // Advancing 20 ticks removes the unhealed body without XP or fight completion.
                    for _ in 0..19 {
                        game.tick_dragon();
                    }
                    assert!(game.dragon.is_some());
                    assert_eq!(game.dragon.as_ref().unwrap().death_time, 19);
                    assert!(game.orbs.is_empty());
                    assert!(!game.server.fight.killed);
                    game.tick_dragon();
                    assert!(game.dragon.is_none(), "cancelled dragon removed at tick 20");
                    assert!(game.orbs.is_empty(), "cancelled dragon drops no XP");
                    assert!(
                        !game.server.fight.killed,
                        "cancelled dragon does not win the fight"
                    );
                    assert!(probe.posts.lock().unwrap().is_empty());
                    game.save_entities(true).unwrap();
                    assert!(
                        !meta_path.exists(),
                        "empty engine metadata file is cleaned up on save"
                    );
                } else {
                    assert_eq!(
                        d.health, 1.0,
                        "accepted death resets flying dragon to 1 health"
                    );
                    assert!(!d.death_canceled);
                    assert!(
                        matches!(d.phase, Phase::Dying { .. }),
                        "accepted death enters dying flight"
                    );
                    assert_eq!(
                        *probe.order.lock().unwrap(),
                        ["death", "post", "damage_post"]
                    );
                    assert_eq!(
                        *probe.posts.lock().unwrap(),
                        [SeenPost {
                            damage_type: "minecraft:player_attack".to_owned(),
                            attacker: 0,
                            credited_player: Some(host_uuid),
                        }]
                    );
                }
            }

            // 1b. Perched dragon: stays at 0 health on acceptance, skips post/credit on cancel.
            for cancelled in [false, true] {
                probe.cancel.store(cancelled, Ordering::SeqCst);
                clear(&probe);
                let kills_before = game.stats.get("killed", "minecraft:ender_dragon");
                spawn_with_phase(game, Phase::Scanning { time: 0 });
                game.hurt_dragon(
                    0,
                    50.0,
                    true,
                    Some(FightWho::Host),
                    DamageKind::PlayerAttack,
                );
                let d = game.dragon.as_ref().unwrap();
                assert_eq!(d.health, 0.0);
                assert!(!matches!(d.phase, Phase::Dying { .. }));
                if cancelled {
                    assert!(d.death_canceled);
                    assert_eq!(*probe.order.lock().unwrap(), ["death", "damage_post"]);
                    assert!(probe.posts.lock().unwrap().is_empty());
                    assert_eq!(
                        game.stats.get("killed", "minecraft:ender_dragon"),
                        kills_before
                    );
                } else {
                    assert!(!d.death_canceled);
                    assert_eq!(
                        *probe.order.lock().unwrap(),
                        ["death", "post", "damage_post"]
                    );
                    assert_eq!(
                        game.stats.get("killed", "minecraft:ender_dragon"),
                        kills_before + 1
                    );
                }
            }

            // 1c. Pre-existing raw sunrise:death_canceled fields stay intact and never act as vetoes.
            let meta_path = game
                .server
                .save
                .0
                .root
                .join("data/sunrise/entity_metadata.dat");
            for (i, raw_val) in [Tag::Byte(1), Tag::Byte(0)].into_iter().enumerate() {
                game.dragon.discard();
                game.persist.dragon_death_canceled = None;
                game.persist.entity_metadata_on_disk = false;
                let mut legacy = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 0).to_nbt(
                    [9, 0, 0, (i + 1) as i32],
                    None,
                    game.res.items(),
                );
                if let Tag::Compound(map) = &mut legacy {
                    map.insert("Health".into(), Tag::Float(0.0f32.to_bits()));
                    map.insert("sunrise:death_canceled".into(), raw_val.clone());
                }
                game.world.queue_entity_nbt((0, 0), vec![legacy]);
                game.spawn_loaded_entities();
                assert!(
                    !game.dragon.as_ref().unwrap().death_canceled,
                    "raw sunrise:death_canceled field is never treated as a veto"
                );
                game.save_entities(true).unwrap();
                let resaved = game.server.save.0.load_entities(0, 0).unwrap();
                assert_eq!(
                    resaved[0].get("sunrise:death_canceled"),
                    Some(&raw_val),
                    "existing raw sunrise:death_canceled field survives save intact"
                );
                assert!(
                    !meta_path.exists(),
                    "loading/saving non-vetoed dragon creates no engine metadata file"
                );
            }
            game.dragon.discard();
            let summoned = game
                .summon_with_nbt(
                    "minecraft:ender_dragon",
                    [0.0, 80.0, 0.0],
                    r#"{Health:0f,"sunrise:death_canceled":1b}"#,
                )
                .unwrap();
            assert!(summoned.is_some());
            assert!(
                !game.dragon.as_ref().unwrap().death_canceled,
                "/summon raw sunrise:death_canceled is never treated as a veto"
            );

            // 2a. Lethal projectile shot after the shooter switched items keeps projectile ID, type, and saved weapon.
            probe.cancel.store(false, Ordering::SeqCst);
            clear(&probe);
            spawn_one_hp(game);
            let mut bow = ItemStack::new("minecraft:bow", 1);
            bow.enchant("minecraft:power", 2);
            let expected_bow = bow.try_into_api().unwrap();
            let mut shot = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.0, 80.0, 0.0),
                Some(Who::Player),
            );
            shot.id = 912;
            shot.arrow.as_mut().unwrap().fired_from_weapon = Some(bow);
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:stick", 1));
            game.hit_special(SPECIAL, 50.0, true, DamageKind::Arrow, Some(&shot));
            {
                let deaths = probe.deaths.lock().unwrap();
                assert_eq!(deaths.len(), 1);
                assert_eq!(deaths[0].damage_type, "minecraft:arrow");
                assert_eq!(deaths[0].causing, 55);
                assert_eq!(deaths[0].direct, 912);
                assert_eq!(deaths[0].direct_type.as_deref(), Some("minecraft:arrow"));
                assert_eq!(deaths[0].weapon.as_ref(), Some(&expected_bow));
            }

            // 2b. Lethal blast carries producer direct ID and type to hooks while keeping baseline last_source.
            clear(&probe);
            spawn_one_hp(game);
            game.explosion_hits_special(
                [0.0, 80.0, 0.0],
                6.0,
                &[],
                "minecraft:bad_respawn_point",
                Some((930, Some("minecraft:end_crystal"))),
            );
            {
                let deaths = probe.deaths.lock().unwrap();
                assert_eq!(deaths.len(), 1);
                assert_eq!(deaths[0].damage_type, "minecraft:bad_respawn_point");
                assert_eq!(deaths[0].direct, 930);
                assert_eq!(
                    deaths[0].direct_type.as_deref(),
                    Some("minecraft:end_crystal")
                );
            }
            assert_eq!(
                game.dragon.as_ref().unwrap().last_source,
                Some(("minecraft:explosion", None, None)),
                "baseline LAN damage source stays separate from hook blast payload"
            );

            // 2c. Lethal firework rocket blast carries rocket ID and type while keeping baseline last_source.
            clear(&probe);
            spawn_one_hp(game);
            let reach = game.dragon.as_ref().unwrap().whole_box();
            game.rocket_blast_dragon(
                Vec3::new(0.0, 80.0, 0.0),
                7.0,
                reach,
                Some(FightWho::Host),
                941,
                Some(Who::Player),
            );
            {
                let deaths = probe.deaths.lock().unwrap();
                assert_eq!(deaths.len(), 1);
                assert_eq!(deaths[0].damage_type, "minecraft:fireworks");
                assert_eq!(deaths[0].causing, 55);
                assert_eq!(deaths[0].direct, 941);
                assert_eq!(
                    deaths[0].direct_type.as_deref(),
                    Some("minecraft:firework_rocket")
                );
            }
            assert_eq!(
                game.dragon.as_ref().unwrap().last_source,
                Some(("minecraft:fireworks", Some(55), None)),
                "baseline LAN damage source stays separate from hook rocket payload"
            );

            // 2d. Lethal linked-crystal destruction carries crystal ID and type while keeping baseline last_source.
            clear(&probe);
            let prev_dim = game.dim;
            game.dim = Dim::End;
            game.reset_entity_residency();
            spawn_one_hp(game);
            let crystal_tag =
                game.dragon
                    .as_ref()
                    .unwrap()
                    .to_nbt([2, 0, 0, 1], None, game.res.items());
            let mut crystal = Crystal::from_nbt(&crystal_tag, 0).unwrap();
            crystal.id = 942;
            game.dragon.as_mut().unwrap().nearest_crystal = Some(0);
            game.crystal_destroyed(0, &crystal, Some(FightWho::Host), false);
            game.dim = prev_dim;
            game.reset_entity_residency();
            {
                let deaths = probe.deaths.lock().unwrap();
                assert_eq!(deaths.len(), 1);
                assert_eq!(deaths[0].damage_type, "minecraft:player_explosion");
                assert_eq!(deaths[0].causing, 55);
                assert_eq!(deaths[0].direct, 942);
                assert_eq!(
                    deaths[0].direct_type.as_deref(),
                    Some("minecraft:end_crystal")
                );
            }
            assert_eq!(
                game.dragon.as_ref().unwrap().last_source,
                Some(("minecraft:player_explosion", Some(55), None)),
                "baseline LAN damage source stays separate from hook crystal payload"
            );

            // 3. Lethal explicit-source hit carries its type, cause, and direct entity ID.
            clear(&probe);
            let dragon_id = spawn_one_hp(game);
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(
                920,
                zombie.clone(),
                Vec3::new(0.0, 80.0, 0.0),
                0.0,
            ));
            game.mobs
                .push(Mob::new(921, zombie, Vec3::new(1.0, 80.0, 0.0), 0.0));
            assert!(game.hurt_target(
                Target::Dragon(dragon_id),
                50.0,
                "minecraft:explosion",
                Some(Target::Mob(921)),
                Some(Target::Mob(920)),
                None,
            ));
            {
                let deaths = probe.deaths.lock().unwrap();
                assert_eq!(deaths.len(), 1);
                assert_eq!(deaths[0].damage_type, "minecraft:explosion");
                assert_eq!(deaths[0].causing, 920);
                assert_eq!(deaths[0].direct, 921);
            }
            assert_eq!(
                *probe.posts.lock().unwrap(),
                [SeenPost {
                    damage_type: "minecraft:explosion".to_owned(),
                    attacker: 920,
                    credited_player: None,
                }]
            );

            // 4a. Phase change during living_death with Continue rejects the stale lethal tail.
            probe.sit_on_death.store(true, Ordering::SeqCst);
            clear(&probe);
            let kills_before = game.stats.get("killed", "minecraft:ender_dragon");
            spawn_one_hp(game);
            game.hurt_dragon(
                0,
                50.0,
                true,
                Some(FightWho::Host),
                DamageKind::PlayerAttack,
            );
            {
                let d = game.dragon.as_ref().unwrap();
                assert_eq!(d.health, 0.0);
                assert!(matches!(d.phase, Phase::Hover { .. }));
                assert_eq!(
                    d.sitting_damage, 0.0,
                    "phase change during living_death skips finish_sitting_damage"
                );
            }
            assert!(probe.posts.lock().unwrap().is_empty());
            assert_eq!(
                game.stats.get("killed", "minecraft:ender_dragon"),
                kills_before
            );
            probe.sit_on_death.store(false, Ordering::SeqCst);

            // 4b. Discard-and-continue skips kill credit and living_death_post.
            probe.discard.store(true, Ordering::SeqCst);
            clear(&probe);
            let kills_before = game.stats.get("killed", "minecraft:ender_dragon");
            spawn_one_hp(game);
            game.hurt_dragon(
                0,
                50.0,
                true,
                Some(FightWho::Host),
                DamageKind::PlayerAttack,
            );
            assert!(
                probe.posts.lock().unwrap().is_empty(),
                "discarded dragon emits no living_death_post"
            );
            assert_eq!(
                game.stats.get("killed", "minecraft:ender_dragon"),
                kills_before,
                "discarded dragon awards no kill credit"
            );
            game.native_removals_flush();
            assert!(game.dragon.is_none());
            probe.discard.store(false, Ordering::SeqCst);

            // 5. Guest-turn and missing-guest kill credit match posted UUID and awarded kill.
            sunrise_gameplay::game::native_player_death::tests::add_guest(game);
            clear(&probe);
            let host_kills_before = game.stats.get("killed", "minecraft:ender_dragon");
            game.as_guest(1, |g| {
                spawn_one_hp(g);
                g.hurt_dragon(
                    0,
                    50.0,
                    true,
                    Some(g.acting_player()),
                    DamageKind::PlayerAttack,
                );
            })
            .unwrap();
            assert_eq!(
                probe
                    .posts
                    .lock()
                    .unwrap()
                    .last()
                    .and_then(|p| p.credited_player),
                Some(123)
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().play[&1]
                    .stats
                    .get("killed", "minecraft:ender_dragon"),
                1
            );
            assert_eq!(
                game.stats.get("killed", "minecraft:ender_dragon"),
                host_kills_before
            );

            clear(&probe);
            spawn_one_hp(game);
            {
                let d = game.dragon.as_mut().unwrap();
                d.hurt_by_player = Some((FightWho::Guest(99), d.ticks));
            }
            game.hurt_dragon(0, 50.0, true, None, DamageKind::Explosion);
            assert_eq!(
                probe
                    .posts
                    .lock()
                    .unwrap()
                    .last()
                    .and_then(|p| p.credited_player),
                Some(host_uuid)
            );
            assert_eq!(
                game.stats.get("killed", "minecraft:ender_dragon"),
                host_kills_before + 1
            );
        });
    });
}
