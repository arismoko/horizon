#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_resident_game;

    /// A client reads its own player's damage cooldown as `LocalPlayer.hurtTo` sets it from
    /// the server's health (20 on a drop, 10 on a rise), its invulnerable time, and a tracked
    /// mob's cooldown from its damage event; an entity it doesn't keep reads None (#5694).
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_client_reads_its_players_and_a_kept_mobs_damage_timers() {
        use super::super::native_entity_refs::tests::with_client_game;
        use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
        use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        use sunrise_sim::motion::PlayerMotion;
        mod_platform::testing::with_mods(Default::default(), || {
            with_client_game(|game| {
                let _server = logged_client(game);
                game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                let local = game.local_player().unwrap().entity();
                let health = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                              health: f32| {
                    game.handle_packet(C::SetHealth(play::SetHealth {
                        health,
                        food: 20,
                        saturation: 5.0,
                    }))
                    .unwrap();
                };
                // The first health after joining sets it without a flash.
                health(game, 20.0);
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(local.damage_cooldown_time(host), Some(0));
                health(game, 15.0);
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(local.damage_cooldown_time(host), Some(20));
                health(game, 18.0);
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(local.damage_cooldown_time(host), Some(10));
                game.player.as_mut().unwrap().health.invulnerable_time = 6;
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(local.invulnerable_time(host), Some(6));

                let zombie = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:zombie")
                    .unwrap();
                game.handle_packet(C::AddEntity(play::AddEntity {
                    id: 302,
                    uuid: 0x5694_0002,
                    kind: zombie,
                    position: [2.5, 64.0, 2.5],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
                let zombie = game.entity(302).expect("the tracked zombie");
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(zombie.damage_cooldown_time(host), Some(0));
                assert_eq!(zombie.invulnerable_time(host), Some(0));
                game.handle_packet(C::DamageEvent(play::DamageEvent {
                    id: 302,
                    source_type: 0,
                    cause: None,
                    direct: None,
                    position: None,
                }))
                .unwrap();
                let host = &*game as &dyn mod_api::ClientHost;
                assert_eq!(zombie.damage_cooldown_time(host), Some(20));

                let untracked = EntityRef::new(local.epoch(), 99);
                assert_eq!(untracked.damage_cooldown_time(host), None);
                assert_eq!(untracked.invulnerable_time(host), None);
            });
        });
    }

    use mod_api::{EntityRef, WorldEpoch};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6;

    use sunrise_sim::mobs::Mob;

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_native_mods_data_damage_type_hurts_and_names_the_death() {
        use sunrise_client::mod_content_client::resolved_resources;
        use sunrise_mod_package::{Package, content::Content};
        use sunrise_sim::mod_content::ContentMod;

        let scratch = tempfile::tempdir().unwrap();
        let manifest = format!(
            "id = 'testmod'\nname = 'Damage fixture'\nversion = '1'\napi = '0.2'\n\
             side = 'both'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\n\
             sunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let path = scratch.path().join("testmod.smod");
        let archive = Package::write_files(
            &manifest,
            &[
                (
                    "Cargo.toml",
                    b"[package]\nname='testmod'\nversion='1.0.0'\n",
                ),
                ("src/lib.rs", b""),
                (
                    "data/testmod/damage_type/matter_cannon.json",
                    br#"{"message_id":"testmod.matter_cannon","scaling":"never","exhaustion":0.0}"#,
                ),
            ],
        )
        .unwrap();
        std::fs::write(&path, archive).unwrap();
        let package = Package::read(&path).unwrap();
        let content = ContentMod {
            id: package.manifest.id.clone(),
            content: Content::read(&path, &package.manifest).unwrap(),
            jars: Vec::new(),
        };
        let _publication = sunrise_sim::datapack_net::isolate_server_publication();
        with_resident_game(|game| {
            let root = game.options.resources.clone();
            // Use the package's passive-data reader and the world-open path, not
            // an entry injected into the damage catalog the hurt call reads.
            let resources = resolved_resources(
                &root,
                root.clone(),
                scratch.path(),
                &[content],
                scratch.path().to_owned(),
            )
            .unwrap();
            let stack = sunrise_gameplay::game::functions::pack_stack(
                scratch.path(),
                &root,
                resources.data(),
                resources.data_changes(),
                false,
                &[],
                &[],
            );
            let staged = game.stage_world_tables(&stack, false, &root).unwrap();
            game.open_world_tables(staged).unwrap();
            // Entities are found in resident columns only (`Visibility.isAccessible`).
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );

            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs
                .push(Mob::new(300001, pig.clone(), Vec3::ZERO, 0.0));
            game.mobs.push(Mob::new(300002, pig, Vec3::ZERO, 0.0));
            let victim = game.entity(300001).unwrap();
            let control = game.entity(300002).unwrap();
            assert_eq!(
                victim.hurt(game, "testmod:matter_cannon", 2.0, None, None),
                Some(true)
            );
            assert_eq!(game.mobs[0].health, 8.0);
            assert_eq!(
                control.hurt(game, "minecraft:player_attack", 2.0, None, None),
                Some(true)
            );
            assert_eq!(game.mobs[1].health, 8.0);

            // A stronger hit lands even inside the first hit's cooldown.
            assert_eq!(
                victim.hurt(game, "testmod:matter_cannon", 10.0, None, None),
                Some(true)
            );
            let dead = game.mobs.remove(0);
            assert_eq!(dead.health, 0.0);
            assert_eq!(
                game.mob_death_message(&dead, None).component()["translate"],
                "death.attack.testmod.matter_cannon"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn review_unknown_damage_type_is_refused() {
        with_resident_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(300001, pig, Vec3::ZERO, 0.0));
            let victim = game.entity(300001).unwrap();
            assert_eq!(
                victim.hurt(game, "missing:unknown", 5.0, None, None),
                Some(false)
            );
            assert_eq!(game.mobs[0].health, 10.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn review_fire_immune_query() {
        with_resident_game(|game| {
            let blaze = game.res.mob_types().get("minecraft:blaze").unwrap().clone();
            game.mobs.push(Mob::new(300001, blaze, Vec3::ZERO, 0.0));
            let victim = game.entity(300001).unwrap();
            assert_eq!(
                victim.invulnerable_to_damage(game, "minecraft:in_fire"),
                Some(true)
            );
        });
    }

    struct SourceProbe(std::sync::Arc<std::sync::Mutex<Vec<(i32, i32)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for SourceProbe {
        fn living_incoming_damage(
            &self,
            _: &mut dyn mod_api::HitHost,
            event: &mut mod_api::LivingIncomingDamageEvent<'_>,
        ) -> mod_api::Flow {
            self.0
                .lock()
                .unwrap()
                .push((event.source.direct, event.source.causing));
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn review_direct_entity_preserved() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(SourceProbe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_resident_game(|game| {
                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                for id in 300001..=300003 {
                    game.mobs.push(Mob::new(id, pig.clone(), Vec3::ZERO, 0.0));
                }
                let victim = game.entity(300001).unwrap();
                let direct = game.entity(300002).unwrap();
                let causing = game.entity(300003).unwrap();
                for damage_type in [
                    "minecraft:arrow",
                    "minecraft:mob_attack",
                    "minecraft:generic",
                ] {
                    assert!(game.entity_set_damage_cooldown_time(victim, 0));
                    assert_eq!(
                        victim.hurt(game, damage_type, 1.0, Some(direct), Some(causing)),
                        Some(true)
                    );
                }
                let mut player =
                    sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
                player.entity_id = 7;
                game.player = Some(player);
                sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 7, 1);
                game.set_game_mode(sunrise_command::args::GameMode::Survival);
                let victim = game.entity(7).unwrap();
                assert_eq!(
                    victim.hurt(game, "minecraft:arrow", 1.0, Some(direct), Some(causing)),
                    Some(true)
                );
            })
        });
        assert_eq!(*heard.lock().unwrap(), vec![(300002, 300003); 4]);
    }

    /// Incoming-damage notifications, in order, with the id the hook heard.
    struct Probe(std::sync::Arc<std::sync::Mutex<Vec<(u32, String)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Probe {
        fn living_incoming_damage(
            &self,
            _: &mut dyn mod_api::HitHost,
            event: &mut mod_api::LivingIncomingDamageEvent<'_>,
        ) -> mod_api::Flow {
            self.0
                .lock()
                .unwrap()
                .push((event.entity as u32, event.source.damage_type.to_owned()));
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn registered_vanilla_damage_keeps_its_hook_and_death_message() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(Probe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_resident_game(|game| {
                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs.push(Mob::new(300001, pig, Vec3::ZERO, 0.0));
                let victim = game.entity(300001).unwrap();
                assert_eq!(
                    victim.hurt(game, "minecraft:dragon_breath", 10.0, None, None),
                    Some(true)
                );
                let dead = game.mobs.remove(0);
                assert_eq!(dead.health, 0.0);
                assert_eq!(
                    game.mob_death_message(&dead, None).component()["translate"],
                    "death.attack.dragonBreath"
                );
            });
        });
        assert_eq!(
            *heard.lock().unwrap(),
            vec![(300001, "minecraft:dragon_breath".to_owned())]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn hurt_routes_to_the_engines_damage_paths() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "probe",
            Box::new(Probe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_resident_game(|game| {
                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs.push(Mob::new(300001, pig, Vec3::ZERO, 0.0));
                let victim = game.entity(300001).unwrap();
                // An accepted hit: taken, the health drops, the damage hooks
                // the engine's own path fires are the ones that fire.
                assert_eq!(
                    victim.hurt(game, "minecraft:generic", 5.0, None, None),
                    Some(true)
                );
                assert_eq!(
                    game.mobs[0].health, 5.0,
                    "the pig took it (10 max health, no armor)"
                );
                // The cooldown: a weaker hit inside the window is refused
                // (`LivingEntity.hurtServer`'s `damageCooldownTime` rule).
                assert_eq!(
                    victim.hurt(game, "minecraft:generic", 1.0, None, None),
                    Some(false)
                );
                assert_eq!(game.mobs[0].health, 5.0, "the window held");
                // The flag refuses all but what bypasses it, and the query
                // answers the same decision the hit ran.
                assert!(game.entity_set_invulnerable(victim, true));
                assert_eq!(
                    victim.hurt(game, "minecraft:generic", 5.0, None, None),
                    Some(false)
                );
                assert_eq!(
                    victim.invulnerable_to_damage(game, "minecraft:generic"),
                    Some(true)
                );
                assert_eq!(
                    victim.invulnerable_to_damage(game, "minecraft:out_of_world"),
                    Some(false),
                    "out of world bypasses the flag"
                );
                assert_eq!(
                    victim.hurt(game, "minecraft:out_of_world", 100.0, None, None),
                    Some(true),
                    "and so does the hit"
                );
                assert!(game.mobs[0].health <= 0.0);
                // The piercing query follows the flag.
                assert_eq!(victim.invulnerable_to_piercing_weapon(game), Some(true));
                // A stale handle reads None and hurts nothing.
                let stale = EntityRef::new(WorldEpoch::new(victim.epoch().get() + 1), 300001);
                assert_eq!(stale.hurt(game, "minecraft:generic", 5.0, None, None), None);
                assert_eq!(
                    stale.invulnerable_to_damage(game, "minecraft:generic"),
                    None
                );
                assert_eq!(stale.invulnerable_to_piercing_weapon(game), None);
            });
        });
        // Every hit that entered the engine's pipeline fired its incoming
        // hook once — the accepted hit, the weaker one the cooldown window
        // refused (the window is checked inside the pipeline) and the
        // flag-bypassing kill — while the flag-refused one never entered.
        assert_eq!(
            *heard.lock().unwrap(),
            vec![
                (300001, "minecraft:generic".to_owned()),
                (300001, "minecraft:generic".to_owned()),
                (300001, "minecraft:out_of_world".to_owned()),
            ]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn teams_allow_damage_without_a_team() {
        with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(300001, zombie.clone(), Vec3::ZERO, 0.0));
            game.mobs.push(Mob::new(300002, zombie, Vec3::ZERO, 0.0));
            let a = game.entity(300001).unwrap();
            let b = game.entity(300002).unwrap();
            // No team on either side: damage is allowed, and a mob without
            // the flag answers the piercing query false.
            assert_eq!(a.entities_team_damage_allowed(game, b), Some(true));
            assert_eq!(a.invulnerable_to_piercing_weapon(game), Some(false));
            let stale = EntityRef::new(WorldEpoch::new(a.epoch().get() + 1), 300001);
            assert_eq!(stale.entities_team_damage_allowed(game, b), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mob_keeps_its_flag_and_damage_cooldown() {
        with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let mob = game.entity(12).unwrap();
            assert_eq!(game.entity_permanently_invulnerable(mob), Some(false));
            assert!(game.entity_set_invulnerable(mob, true));
            assert_eq!(game.entity_permanently_invulnerable(mob), Some(true));
            assert!(game.mobs[0].invulnerable_flag);
            assert_eq!(game.entity_damage_cooldown_time(mob), Some(0));
            assert!(game.entity_set_damage_cooldown_time(mob, 20));
            assert_eq!(game.mobs[0].invulnerable, 20);
            assert_eq!(game.entity_damage_cooldown_time(mob), Some(20));
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), 12);
            assert_eq!(game.entity_permanently_invulnerable(stale), None);
            assert!(!game.entity_set_invulnerable(stale, false));
            assert_eq!(game.entity_damage_cooldown_time(stale), None);
            assert!(!game.entity_set_damage_cooldown_time(stale, 0));
            assert!(game.mobs[0].invulnerable_flag);
        });
    }

    /// #916: the permanent flag reaches players and non-mob kinds, and a
    /// player's damage is really refused by it.
    #[test]
    #[ignore = "daily"]
    fn the_flag_reaches_players_and_non_mobs_and_refuses_player_damage() {
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            game.armor_stands
                .push(sunrise_sim::entities::ArmorStand::new(14, Vec3::ZERO, 0.0));
            let stand = game.entity(14).unwrap();

            // A player's flag, on their body, refusing their damage — and
            // `#bypasses_invulnerability` still getting past.
            assert_eq!(game.entity_permanently_invulnerable(player), Some(false));
            assert!(game.entity_set_invulnerable(player, true));
            assert_eq!(game.entity_permanently_invulnerable(player), Some(true));
            game.hurt_player_by("minecraft:generic", 5.0);
            assert_eq!(
                game.player.as_ref().unwrap().health.health(),
                20.0,
                "the flag refused it"
            );
            game.hurt_player_by("minecraft:out_of_world", 100.0);
            assert!(
                game.player.as_ref().unwrap().health.health() <= 0.0,
                "out of world gets past the flag"
            );

            // A stand without saved data reads false, and the write lands in
            // the saved `Invulnerable` tag the item damage path refuses by.
            assert_eq!(game.entity_permanently_invulnerable(stand), Some(false));
            assert!(game.entity_set_invulnerable(stand, true));
            assert_eq!(game.entity_permanently_invulnerable(stand), Some(true));
            let raw = game.persist.raw.get(&14).cloned().unwrap();
            assert!(sunrise_sim::entity_nbt::flag(&raw, "Invulnerable"));
            // The timer tag rides along the same compound.
            assert!(game.entity_set_invulnerable_time(stand, 12));
            assert_eq!(game.entity_invulnerable_time(stand), Some(12));
            let raw = game.persist.raw.get(&14).cloned().unwrap();
            assert!(matches!(
                raw.get("invulnerable_time"),
                Some(sunrise_core::nbt::Tag::Int(12))
            ));
        });
    }

    /// A lit stone floor under the origin, so a mob's tick survives it.
    fn floor(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
            let mut chunk = sunrise_level::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in -8..8 {
            for z in -8..8 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
    }

    /// #916: `Entity.invulnerableTime` is its own field — never the living
    /// damage cooldown — on a mob and a player alike, and both survive a
    /// save and load.
    #[test]
    #[ignore = "daily"]
    fn the_entity_timer_never_shares_the_living_cooldown() {
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            // The #170 recipe: two zombies on a lit stone floor, kept from
            // despawning while their tick runs.
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(13, zombie, Vec3::new(5.5, 64.0, 0.5), 0.0));
            floor(game);
            let mob = game.entity(12).unwrap();
            let mob_ref = game.mob(game.entity(12).unwrap()).unwrap();
            assert!(mob_ref.set_persistence_required(game));
            assert!(mob_ref.set_no_action_time(game, 650));

            // Different fields, counted down side by side by the base tick.
            assert!(game.entity_set_damage_cooldown_time(mob, 15));
            assert!(game.entity_set_invulnerable_time(mob, 5));
            game.tick_mobs().unwrap();
            assert_eq!(game.entity_damage_cooldown_time(mob), Some(14));
            assert_eq!(game.entity_invulnerable_time(mob), Some(4));
            game.mobs.clear();

            // The player's two, on the same clock.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            assert!(game.entity_set_damage_cooldown_time(player, 7));
            assert!(game.entity_set_invulnerable_time(player, 3));
            game.player.as_mut().unwrap().health.advance_clock();
            assert_eq!(game.entity_damage_cooldown_time(player), Some(6));
            assert_eq!(game.entity_invulnerable_time(player), Some(2));

            // A mob's flag and timer survive a save and a load, as vanilla
            // writes them (`Invulnerable`, `invulnerable_time`).
            game.common.mobs.push(Mob::new(
                30,
                game.res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone(),
                Vec3::new(0.5, 64.0, 0.5),
                0.0,
            ));
            game.mobs[0].invulnerable_flag = true;
            game.mobs[0].invulnerable_time = 9;
            let tag = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [0; 4],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            let back = sunrise_sim::entity_nbt::mob_from_nbt(&tag, game.res.mob_types(), 30, None)
                .unwrap();
            assert!(back.invulnerable_flag);
            assert_eq!(back.invulnerable_time, 9);
        });
    }

    /// #916, review: the window really refuses. A mod sets the timer and the
    /// entity no longer takes damage — nothing but what bypasses
    /// invulnerability — and enemies no longer see it as a target, until the
    /// window closes.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn the_window_refuses_damage_and_hides_the_mob_from_enemies() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::{Damage, DamageKind, Mob};
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            // The #170 recipe: two zombies on a lit stone floor, kept from
            // despawning while their tick runs.
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(13, zombie, Vec3::new(5.5, 64.0, 0.5), 0.0));
            floor(game);
            let victim = game.entity(12).unwrap();
            let enemy = game.entity(13).unwrap();
            let keep = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                let m = game.mob(game.entity(12).unwrap()).unwrap();
                m.set_persistence_required(game);
                m.set_no_action_time(game, 650);
            };
            keep(game);

            // The window refuses a hit nothing-bypassing; the void goes
            // through (`Entity.isInvulnerableToBase`).
            assert!(game.entity_set_invulnerable_time(victim, 60));
            let before = game.mobs[0].health;
            assert!(!game.mobs[0].damage(
                sunrise_gameplay::game::server_side::entity_server::damage_level_of!(
                    game,
                    &sunrise_sim::health::NO_ATTACK,
                    no_push,
                    None
                ),
                5.0,
                Damage::of(DamageKind::Generic),
            ));
            assert_eq!(game.mobs[0].health, before, "the window refused it");
            assert!(game.mobs[0].damage(
                sunrise_gameplay::game::server_side::entity_server::damage_level_of!(
                    game,
                    &sunrise_sim::health::NO_ATTACK,
                    no_push,
                    None
                ),
                100.0,
                Damage::of(DamageKind::OutOfWorld),
            ));
            assert!(game.mobs[0].health <= 0.0, "the void gets past");

            // `canBeSeenAsEnemy`: the facts enemies ask about carry the window
            // too (`Mob::seen`, `isInvulnerable`).
            game.mobs[0].health = 20.0;
            assert!(game.mobs[0].invulnerable_time > 0);
            assert!(game.mobs[0].is_invulnerable());
            assert!(game.mobs[0].seen(None, game.items()).invulnerable);
            // ...and once the window has run out, the enemy sees it again.
            game.mobs[0].invulnerable_time = 1;
            game.tick_mobs().unwrap();
            assert_eq!(game.mobs[0].invulnerable_time, 0);
            assert!(!game.mobs[0].seen(None, game.items()).invulnerable);
            let _ = enemy;
            keep(game);
        });
    }

    /// #916, review round 2: `Entity.setInvulnerable` on a player is the
    /// permanent flag, apart from `Abilities.invulnerable` — the abilities
    /// stay untouched, a survival hit is refused and a creative player's
    /// passes the flag (`isInvulnerableToBase`'s one exception), while the
    /// ability gate (a creative player's own mode) holds even a creative
    /// hit back, as vanilla `Player.hurtServer` does.
    #[test]
    #[ignore = "daily"]
    fn the_players_permanent_flag_is_apart_from_their_abilities() {
        use sunrise_sim::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();

            // The window refuses an ordinary hit; a creative player's
            // `player_attack` passes it (`DamageSource.isCreativePlayer`).
            assert!(game.entity_set_invulnerable_time(player, 60));
            game.mob_hits_player(5.0, [0.0; 3], "minecraft:player_attack");
            assert_eq!(
                game.player.as_ref().unwrap().health.health(),
                20.0,
                "the window refused it"
            );
            game.mob_hits_player_from(5.0, [0.0; 3], "minecraft:player_attack", true);
            assert_eq!(
                game.player.as_ref().unwrap().health.health(),
                15.0,
                "a creative player's hit passes the window"
            );
            game.player.as_mut().unwrap().health.invulnerable_time = 0;
            // The landed hit started the hurt cooldown: a later hit must
            // out-damage it to land at all (`LivingEntity.hurtServer`).
            game.player.as_mut().unwrap().health.hurt.cooldown = 0;
            game.player.as_mut().unwrap().health.hurt.last_hurt = 0.0;

            // The permanent flag is not the abilities: set_invulnerable
            // leaves `Abilities.invulnerable` alone.
            assert!(game.entity_set_invulnerable(player, true));
            assert!(
                !game.player.as_ref().unwrap().invulnerable,
                "the abilities are untouched"
            );
            game.mob_hits_player(5.0, [0.0; 3], "minecraft:player_attack");
            assert_eq!(
                game.player.as_ref().unwrap().health.health(),
                15.0,
                "the flag refused the survival hit"
            );
            game.mob_hits_player_from(5.0, [0.0; 3], "minecraft:player_attack", true);
            assert_eq!(
                game.player.as_ref().unwrap().health.health(),
                10.0,
                "and a creative player's hit passes the flag"
            );
            game.player.as_mut().unwrap().health.hurt.cooldown = 0;
            game.player.as_mut().unwrap().health.hurt.last_hurt = 0.0;

            // The ability flag is another gate (`Player.hurtServer`): even a
            // creative player's hit does not pass it, as in vanilla.
            game.player.as_mut().unwrap().invulnerable = true;
            game.mob_hits_player_from(5.0, [0.0; 3], "minecraft:player_attack", true);
            assert_eq!(
                game.player.as_ref().unwrap().health.health(),
                10.0,
                "the ability flag holds even a creative hit back"
            );
        });
    }

    /// #916, review: a non-mob's saved window counts down each tick, and the
    /// item damage path refuses by it.
    #[test]
    #[ignore = "daily"]
    fn a_non_mobs_saved_window_counts_down_and_refuses() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::entities::ArmorStand;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            game.armor_stands
                .push(ArmorStand::new(14, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let stand = game.entity(14).unwrap();
            assert!(game.entity_set_invulnerable_time(stand, 20));
            // Entity.baseTick, over the saved tag.
            game.tick_saved_invulnerable_time(14);
            game.tick_saved_invulnerable_time(14);
            assert_eq!(game.entity_invulnerable_time(stand), Some(18));
            // Down to zero it stays, and reads as no window.
            for _ in 0..18 {
                game.tick_saved_invulnerable_time(14);
            }
            assert_eq!(game.entity_invulnerable_time(stand), Some(0));
        });
    }

    /// #916, coderabbit: an item outside the entity-ticking chunks does not
    /// tick, so its saved window does not count down in that pass — while
    /// one the pass reaches does.
    #[test]
    #[ignore = "daily"]
    fn an_item_outside_the_ticking_chunks_keeps_its_window() {
        use sunrise_items::items::ItemStack;
        use sunrise_sim::motion::PlayerMotion;
        with_resident_game(|game| {
            // A player's tickets: chunks out of the simulation distance
            // around them do not tick entities.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.update_chunk_tickets();
            let stack = || ItemStack {
                item: "minecraft:stone".into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            game.spawn_item(stack(), [0.5, 64.0, 0.5], [0.0; 3], 10);
            // The far item's column is resident but outside every ticket.
            game.world.chunks.insert(
                (6250, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(6250, 0)),
            );
            game.spawn_item(stack(), [100000.5, 64.0, 0.5], [0.0; 3], 10);
            let near = game.entity(game.dropped[0].id).unwrap();
            let far = game.entity(game.dropped[1].id).unwrap();
            assert!(game.entity_set_invulnerable_time(near, 5));
            assert!(game.entity_set_invulnerable_time(far, 5));
            game.tick_items();
            assert_eq!(game.entity_invulnerable_time(near), Some(4));
            assert_eq!(
                game.entity_invulnerable_time(far),
                Some(5),
                "outside the ticking chunks, the window holds"
            );
        });
    }

    /// #916, review round 2: the piercing query is `Entity.isInvulnerable()`
    /// — the flag or the open window — for every kind that keeps either.
    #[test]
    #[ignore = "daily"]
    fn the_piercing_query_follows_the_flag_or_the_window_of_every_kind() {
        use sunrise_sim::entities::ArmorStand;
        use sunrise_sim::motion::PlayerMotion;
        with_resident_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(300001, pig, Vec3::ZERO, 0.0));
            let mob = game.entity(300001).unwrap();
            assert_eq!(mob.invulnerable_to_piercing_weapon(game), Some(false));
            // The window alone, without the flag, answers it.
            assert!(game.entity_set_invulnerable_time(mob, 3));
            assert_eq!(mob.invulnerable_to_piercing_weapon(game), Some(true));
            // A player's flag, apart from the abilities.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            assert!(game.entity_set_invulnerable(player, true));
            assert_eq!(player.invulnerable_to_piercing_weapon(game), Some(true));
            // A saved kind's window, over its tags.
            game.armor_stands
                .push(ArmorStand::new(14, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let stand = game.entity(14).unwrap();
            assert!(game.entity_set_invulnerable_time(stand, 5));
            assert_eq!(stand.invulnerable_to_piercing_weapon(game), Some(true));
        });
    }

    /// #916, review round 2: a player the flag or the window covers is no
    /// mob's target — the write handle's facts, the goal view, and
    /// `asValidTarget` behind both.
    #[test]
    #[ignore = "daily"]
    fn a_flagged_player_is_no_ones_target() {
        use sunrise_gameplay_common::game::commands_common::Target;
        use sunrise_sim::motion::PlayerMotion;
        with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(300001, zombie, Vec3::new(0.5, 64.0, 0.5), 0.0));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            // Survival and alive: a zombie takes the player as a target.
            assert!(game.native_mob_valid_target(&game.mobs[0], Target::Player));
            // The flag alone, with every ability untouched, refuses it.
            assert!(game.entity_set_invulnerable(player, true));
            assert!(
                !game.player.as_ref().unwrap().invulnerable,
                "the abilities stay untouched"
            );
            assert!(!game.native_mob_valid_target(&game.mobs[0], Target::Player));
            assert!(
                game.player_view().unwrap().invulnerable,
                "the goal view agrees"
            );
            // The window refuses it too, until it closes.
            assert!(game.entity_set_invulnerable(player, false));
            assert!(game.entity_set_invulnerable_time(player, 2));
            assert!(!game.native_mob_valid_target(&game.mobs[0], Target::Player));
            assert!(game.entity_set_invulnerable_time(player, 0));
            assert!(game.native_mob_valid_target(&game.mobs[0], Target::Player));
        });
    }

    /// #916, review round 2: a stand the flag or the window covers refuses a
    /// survival punch — no wobble, no break — while a creative player's hit
    /// passes both, as `DamageSource.isCreativePlayer` does.
    #[test]
    #[ignore = "daily"]
    fn a_saved_stand_refuses_the_punch_that_would_break_it() {
        use sunrise_sim::entities::ArmorStand;
        with_resident_game(|game| {
            game.armor_stands
                .push(ArmorStand::new(14, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let stand = game.entity(14).unwrap();
            // The window refuses even the first hit's wobble.
            assert!(game.entity_set_invulnerable_time(stand, 5));
            assert!(!game.punch_armor_stand(0));
            assert_eq!(game.armor_stands.len(), 1);
            // The flag holds the breaking second hit back in survival.
            assert!(game.entity_set_invulnerable(stand, true));
            assert!(!game.punch_armor_stand(0));
            assert!(!game.punch_armor_stand(0));
            assert_eq!(game.armor_stands.len(), 1, "still standing");
            // A creative player's hit passes the flag.
            game.creative = true;
            assert!(game.punch_armor_stand(0));
            assert_eq!(game.armor_stands.len(), 0);
        });
    }

    /// #916, review round 2: the player's flag and window ride the save —
    /// `sync_level` writes them, `load_saved_state_once` reads them back.
    #[test]
    #[ignore = "daily"]
    fn the_players_flag_and_window_ride_the_save() {
        use sunrise_core::nbt::Tag;
        use sunrise_sim::motion::PlayerMotion;
        with_resident_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            assert!(game.entity_set_invulnerable(player, true));
            assert!(game.entity_set_invulnerable_time(player, 7));
            // Entity.saveWithoutId: the flag always, the window while open.
            game.sync_level();
            let saved = game.level.player_extra.clone().unwrap();
            assert!(matches!(saved.get("Invulnerable"), Some(Tag::Byte(1))));
            assert!(matches!(saved.get("invulnerable_time"), Some(Tag::Int(7))));
            // A later load restores both onto the body.
            game.player
                .as_mut()
                .unwrap()
                .health
                .permanently_invulnerable = false;
            game.player.as_mut().unwrap().health.invulnerable_time = 0;
            let mut pending = true;
            game.common
                .player
                .as_mut()
                .unwrap()
                .load_saved_state_once(&mut pending, &game.common.level)
                .unwrap();
            assert!(
                game.player
                    .as_ref()
                    .unwrap()
                    .health
                    .permanently_invulnerable
            );
            assert_eq!(game.entity_invulnerable_time(player), Some(7));
        });
    }

    /// #916, review round 2: the dragon's saved flag refuses `/damage`
    /// except from a creative player — the one refusal every saved kind
    /// shares (`saved_refuses_damage`).
    #[test]
    #[ignore = "daily"]
    fn the_dragons_saved_flag_refuses_all_but_a_creative_player() {
        use sunrise_gameplay_common::game::dragon_common::Dragon;
        use sunrise_sim::mobs::DamageKind;
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 15;
            game.dragon.replace(dragon);
            let entity = game.entity(15).unwrap();
            assert!(game.entity_set_invulnerable(entity, true));

            // Match cmd_damage's normalized direct and causing player source.
            let source = (
                Some(sunrise_gameplay_common::game::commands_common::Target::Player),
                Some(sunrise_gameplay_common::game::commands_common::Target::Player),
                None,
            );
            let before = game.dragon.as_ref().unwrap().health;
            // A survival player's blow: refused.
            game.creative = false;
            assert!(!game.command_damage_dragon(
                15,
                10.0,
                "minecraft:generic",
                DamageKind::Generic,
                source,
            ));
            assert_eq!(game.dragon.as_ref().unwrap().health, before);
            // A creative player's blow passes the flag (`isCreativePlayer`).
            game.creative = true;
            assert!(game.command_damage_dragon(
                15,
                10.0,
                "minecraft:generic",
                DamageKind::Generic,
                source,
            ));
            assert!(game.dragon.as_ref().unwrap().health < before);
            // And the void needs no creative help (`#bypasses_invulnerability`).
            game.creative = false;
            assert!(game.command_damage_dragon(
                15,
                1000.0,
                "minecraft:out_of_world",
                DamageKind::OutOfWorld,
                source,
            ));
        });
    }
}
