#![cfg(test)]
use mod_api::EntityRef;
use mod_api::LivingRef;
use mod_api::MobRef;
use sunrise_gameplay_common::game::block_props::BlockPropsGameExt2 as _;
use sunrise_sim::mobs::Who;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6 as _;

#[cfg(test)]
pub(crate) mod tests {
    use super::super::native_entity_refs::tests::with_resident_game;
    use super::*;
    use mod_api::WorldEpoch;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::mobs;
    use sunrise_sim::{entities::ArmorStand, mobs::Mob};

    pub(crate) fn add_mob(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        id: u32,
        name: &str,
    ) -> MobRef {
        let kind = game.res.mob_types().get(name).unwrap().clone();
        game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
        game.mob(game.entity(id).unwrap()).unwrap()
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn fox_enderman_and_zombified_piglin_take_targets_with_their_side_effects() {
        use sunrise_core::random::{LegacyRandom, RandomSource};
        let mut client_events = Vec::new();
        with_resident_game(|game| {
            for (id, name) in [
                (12, "minecraft:fox"),
                (13, "minecraft:enderman"),
                (14, "minecraft:zombified_piglin"),
                (15, "minecraft:zombie"),
            ] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let handle = |game: &sunrise_gameplay::game::ServerGame<'_>, id| {
                game.mob(game.entity(id).unwrap()).unwrap()
            };
            let (fox, enderman, piglin) = (handle(game, 12), handle(game, 13), handle(game, 14));
            let other = game.living(game.entity(15).unwrap()).unwrap();
            let state = |game: &sunrise_gameplay::game::ServerGame<'_>, id| {
                game.mobs.iter().find(|m| m.id == id).unwrap().clone()
            };

            // Fox.setTarget: a target leaves defending as it is; clearing ends it.
            game.mobs
                .iter_mut()
                .find(|m| m.id == 12)
                .unwrap()
                .extra
                .ints[0] |= mobs::fox::DEFENDING;
            mobs::fox::add_trusted_entity(
                game.mobs.iter_mut().find(|m| m.id == 12).unwrap(),
                [1, 2, 3, 4],
            );
            assert!(fox.set_target(game, Some(other)));
            assert_eq!(fox.target(game), Some(other));
            assert!(mobs::fox::flag(&state(game, 12), mobs::fox::DEFENDING));
            assert!(fox.set_target(game, None));
            assert_eq!(fox.target(game), None);
            assert!(!mobs::fox::flag(&state(game, 12), mobs::fox::DEFENDING));
            assert!(mobs::fox::trusts(&state(game, 12), [1, 2, 3, 4]));

            // Enderman target changes also update the synced flags and attack speed.
            let enderman_state = game.mobs.iter_mut().find(|m| m.id == 13).unwrap();
            enderman_state.age = 1234;
            enderman_state.extra.flag2 = true;
            game.start_audio_capture();
            assert!(enderman.set_target(game, Some(other)));
            client_events =
                std::mem::take(&mut game.mobs.iter_mut().find(|m| m.id == 13).unwrap().outbox);
            game.handle_mob_events(client_events.clone()).unwrap();
            assert!(
                game.server.pending_sounds.is_empty(),
                "stare stays client-local"
            );
            let m = state(game, 13);
            assert!(m.extra.flag, "DATA_CREEPY");
            assert_eq!(m.extra.counter, 1234, "targetChangeTime");
            assert!(mobs::enderman::attacking_speed(&m) > 0.0);
            assert!(m.extra.flag2, "retargeting preserves DATA_STARED_AT");
            assert!(m.movement_speed() > f64::from(0.3f32));
            assert!(enderman.set_target(game, None));
            let m = state(game, 13);
            assert!(!m.extra.flag);
            assert_eq!(m.extra.counter, 0);
            assert!(!m.extra.flag2, "clearing resets DATA_STARED_AT");
            assert_eq!(mobs::enderman::attacking_speed(&m), 0.0);

            // A zombified piglin samples both delays only when acquiring its first target.
            let mut expected = LegacyRandom::new(2026);
            let sound_delay = expected.next_int_bound(21).unwrap();
            let alert_delay = 80 + expected.next_int_bound(41).unwrap();
            game.mobs.iter_mut().find(|m| m.id == 14).unwrap().random = LegacyRandom::new(2026);
            assert!(piglin.set_target(game, Some(other)));
            assert_eq!(piglin.target(game), Some(other));
            let first = state(game, 14);
            assert_eq!(first.extra.counter, sound_delay);
            assert_eq!(first.extra.counter2, alert_delay);
            assert!(piglin.set_target(game, Some(other)));
            let changed = state(game, 14);
            assert_eq!(changed.extra.counter, first.extra.counter);
            assert_eq!(changed.extra.counter2, first.extra.counter2);
            assert!(piglin.set_target(game, None));
            assert_eq!(piglin.target(game), None);
        });
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.client.audio.0.capture = Some(Vec::new());
            game.handle_client_mob_events(&mut client_events);
            assert_eq!(
                game.client
                    .audio
                    .0
                    .heard
                    .get("minecraft:entity.enderman.stare"),
                Some(&1)
            );
            assert!(game.client.audio.0.capture.as_ref().unwrap().is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn redirected_fake_target_is_refused_with_the_original_target_intact() {
        use std::sync::{Arc, Mutex};
        struct Redirect(Arc<Mutex<Option<LivingRef>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Redirect {
            fn living_change_target(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::LivingChangeTargetEvent<'_>,
            ) -> mod_api::Flow {
                assert!(event.mob.living().health(host).is_some());
                *event.new_target = *self.0.lock().unwrap();
                mod_api::Flow::Continue
            }
        }
        let redirect = Arc::new(Mutex::new(None));
        let mods = vec![mod_api::ModEntry::new(
            "target-test",
            Box::new(Redirect(redirect.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_resident_game(|game| {
                sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(game);
                let level = game.level(game.dim.id()).unwrap();
                let fake = game.fake_player(level, 1234, "fake").unwrap();
                *redirect.lock().unwrap() = Some(LivingRef::new(fake.entity()));
                let mob = add_mob(game, 12, "minecraft:zombie");
                let existing = add_mob(game, 13, "minecraft:sheep");
                game.mobs[0].ai.target = Some(Who::Mob(13));
                assert!(!mob.set_target(game, None));
                assert_eq!(
                    game.mobs[0].ai.target,
                    Some(Who::Mob(existing.entity().id()))
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn sitting_pose_reads_and_writes_a_tamable_animal_only() {
        with_resident_game(|game| {
            let wolf = add_mob(game, 12, "minecraft:wolf");
            let horse = add_mob(game, 13, "minecraft:horse");
            assert_eq!(wolf.is_in_sitting_pose(game), Some(false));
            assert!(wolf.set_in_sitting_pose(game, true));
            assert!(game.mobs[0].extra.sitting_pose);
            assert!(game.mobs[0].needs_sync);
            assert_eq!(wolf.is_in_sitting_pose(game), Some(true));
            assert!(wolf.set_in_sitting_pose(game, false));
            assert!(!game.mobs[0].extra.sitting_pose);
            assert_eq!(wolf.is_in_sitting_pose(game), Some(false));
            // A horse is tamed another way and has no TamableAnimal pose.
            assert_eq!(horse.is_in_sitting_pose(game), None);
            assert!(!horse.set_in_sitting_pose(game, true));
            assert!(!game.mobs[1].extra.sitting_pose);
            // A cat is tamable too.
            let cat = add_mob(game, 14, "minecraft:cat");
            assert_eq!(cat.is_in_sitting_pose(game), Some(false));
            assert!(cat.set_in_sitting_pose(game, true));
            assert!(game.mobs[2].extra.sitting_pose);
            // A removed handle is stale: it reads nothing and writes nothing.
            game.mobs[0].removed = true;
            assert_eq!(wolf.is_in_sitting_pose(game), None);
            assert!(!wolf.set_in_sitting_pose(game, true));
        })
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn leash_holder_reads_live_state() {
        use mod_api::LeashHolder;
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{
            Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig, playerdata::uuid_ints,
        };
        use sunrise_sim::mobs::leash::{Holder, Pending};
        use {
            sunrise_gameplay::game::server_side::lan::LanSettings,
            sunrise_gameplay_common::game::boats_common::Boat,
        };

        with_resident_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
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
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 20,
                    uuid: 20,
                    name: "holder".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                20,
                [0.0, 0.0, 0.0],
                10,
            );
            guest.placed = true;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            let cow = add_mob(game, 12, "minecraft:cow");
            let other = add_mob(game, 13, "minecraft:cow");
            game.boats
                .push(Boat::new(14, "oak".into(), Vec3::ZERO, 0.0));
            let boat = game.entity(14).unwrap();
            assert_eq!(
                cow.entity().leash_holder(game),
                Some(LeashHolder::Unleashed)
            );
            let motion = sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(20, |guest| guest.motion = Some(Box::new(motion)))
                .unwrap();
            game.as_guest(20, |game| {
                assert!(game.interact_leash(0, "minecraft:lead"));
                assert_eq!(
                    cow.entity().leash_holder(game),
                    Some(LeashHolder::Entity(game.entity(20).unwrap()))
                );
            })
            .unwrap();
            for (holder, expected) in [
                (Holder::Mob(13), LeashHolder::Entity(other.entity())),
                (Holder::Boat(14), LeashHolder::Entity(boat)),
                (Holder::Knot([1, 2, 3]), LeashHolder::Knot([1, 2, 3])),
            ] {
                game.mobs[0].extra.leash = Some(Holder::Player);
                game.mobs[0].extra.leash_player = Some(uuid_ints(20));
                game.as_guest(20, |game| {
                    assert!(game.tie_player_leads(glam::DVec3::ZERO, holder, None));
                    assert_eq!(cow.entity().leash_holder(game), Some(expected));
                })
                .unwrap();
            }
            game.mobs[0].extra.leash = None;
            game.mobs[0].extra.leash_pending = Some(Pending::Uuid([0, 0, 0, 99]));
            assert_eq!(cow.entity().leash_holder(game), None);
            let stale = EntityRef::new(WorldEpoch::new(999), 12);
            assert_eq!(stale.leash_holder(game), None);
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert_eq!(cow.entity().leash_holder(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_portal_eligibility_matches_gameplay() {
        with_resident_game(|game| {
            let cow = add_mob(game, 12, "minecraft:cow");
            let wither = add_mob(game, 13, "minecraft:wither");
            let check =
                |game: &sunrise_gameplay::game::ServerGame<'_>, handle: MobRef, expected| {
                    let mob = game
                        .mobs
                        .iter()
                        .find(|m| m.id == handle.entity().id())
                        .unwrap();
                    assert_eq!(game.mob_portal_eligible(mob), expected);
                    let host: &dyn mod_api::Host = game;
                    assert_eq!(handle.can_use_portal(host), Some(expected));
                };
            check(game, cow, true);
            check(game, wither, false);
            game.mobs[0].extra.mount = Some(99);
            check(game, cow, false);
            game.mobs[0].extra.mount = None;
            game.riding = Some(12);
            check(game, cow, false);
            game.riding = None;
            check(game, cow, true);
            let stale = MobRef::new(mod_api::EntityRef::new(WorldEpoch::new(999), 12));
            let host: &dyn mod_api::Host = game;
            assert_eq!(stale.can_use_portal(host), None);
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            let host: &dyn mod_api::Host = game;
            assert_eq!(cow.can_use_portal(host), None);
            game.enter_command_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            check(game, cow, true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_custom_persistence_is_independent_of_stored_flags() {
        with_resident_game(|game| {
            let mob = add_mob(game, 12, "minecraft:cow");
            for no_ai in [false, true] {
                assert!(mob.set_no_ai(game, no_ai));
                for (persistent, leash, expected) in [
                    (false, None, false),
                    (false, Some(mobs::leash::Holder::Knot([5, 64, 5])), true),
                    (true, None, false),
                    (true, Some(mobs::leash::Holder::Knot([5, 64, 5])), true),
                ] {
                    game.mobs[0].persistent = persistent;
                    game.mobs[0].extra.leash = leash;
                    assert_eq!(mob.requires_custom_persistence(game), Some(expected));
                    assert_eq!(mob.persistence_required(game), Some(persistent));
                    assert_eq!(mob.no_ai(game), Some(no_ai));
                }
            }
            let enderman = add_mob(game, 13, "minecraft:enderman");
            assert_eq!(enderman.requires_custom_persistence(game), Some(false));
            game.mobs[1].extra.carried = Some(
                game.pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap(),
            );
            assert_eq!(enderman.requires_custom_persistence(game), Some(true));
            assert_eq!(enderman.persistence_required(game), Some(false));
            game.mobs[1].extra.carried = None;
            assert!(enderman.set_persistence_required(game));
            assert_eq!(enderman.requires_custom_persistence(game), Some(false));
            assert_eq!(enderman.persistence_required(game), Some(true));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_baby_setter_uses_species_age_and_attribute_boundaries() {
        use sunrise_sim::mobs::finalize::{Attr, base_value};

        with_resident_game(|game| {
            let mut id = 12;
            for (no_ai, persistent) in [(false, false), (false, true), (true, false), (true, true)]
            {
                for (name, baby_age, damage) in [
                    ("minecraft:sniffer", -48000, None),
                    ("minecraft:goat", -24000, Some((1.0, 2.0))),
                    ("minecraft:hoglin", -24000, Some((0.5, 6.0))),
                    // Non-ageable babies keep a sentinel age instead of growing up.
                    ("minecraft:zombie", -1073741824, None),
                    ("minecraft:piglin", -1073741824, None),
                ] {
                    let mob = add_mob(game, id, name);
                    id += 1;
                    assert!(mob.set_no_ai(game, no_ai));
                    if persistent {
                        assert!(mob.set_persistence_required(game));
                    }
                    for baby in [true, false, true, false] {
                        assert!(mob.set_baby(game, baby), "{name}");
                        let state = game.mobs.last().unwrap();
                        assert_eq!(state.extra.age, if baby { baby_age } else { 0 }, "{name}");
                        assert_eq!(mob.living().baby(game), Some(baby), "{name}");
                        if let Some((baby_damage, adult_damage)) = damage {
                            assert_eq!(
                                base_value(state, Attr::AttackDamage),
                                if baby { baby_damage } else { adult_damage },
                                "{name}"
                            );
                        }
                        assert_eq!(mob.no_ai(game), Some(no_ai), "{name}");
                        assert_eq!(mob.persistence_required(game), Some(persistent), "{name}");
                    }
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_baby_setter_runs_ghastling_setup_in_both_directions() {
        use sunrise_sim::mobs::{control::MoveKind, nav::NavKind};

        with_resident_game(|game| {
            let ghast = add_mob(game, 12, "minecraft:happy_ghast");
            assert!(ghast.set_no_ai(game, true));
            assert!(ghast.set_persistence_required(game));
            assert!(game.mobs[0].ai.goals.has("HappyGhastFloatGoal"));
            for baby in [true, false, true, false] {
                assert!(ghast.set_baby(game, baby));
                let state = &game.mobs[0];
                assert_eq!(ghast.living().baby(game), Some(baby));
                assert_eq!(state.ai.goals.has("HappyGhastFloatGoal"), !baby);
                assert_eq!(state.ai.goals.has("RandomFloatAroundGoal"), !baby);
                if baby {
                    assert!(matches!(state.ai.nav.kind, NavKind::Flying));
                    assert!(matches!(state.ai.mov.kind, MoveKind::Flying { .. }));
                } else {
                    assert!(matches!(state.ai.nav.kind, NavKind::Ground));
                    assert!(matches!(state.ai.mov.kind, MoveKind::Custom));
                }
                assert_eq!(ghast.no_ai(game), Some(true));
                assert_eq!(ghast.persistence_required(game), Some(true));
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_flags_baby_setter_refuses_no_baby_species_and_unavailable_mobs() {
        with_resident_game(|game| {
            for (id, name) in [(12, "minecraft:skeleton"), (13, "minecraft:parrot")] {
                let mob = add_mob(game, id, name);
                assert!(mob.set_no_ai(game, true));
                assert!(mob.set_persistence_required(game));
                for baby in [true, false] {
                    assert!(!mob.set_baby(game, baby), "{name}");
                    assert_eq!(game.mobs.last().unwrap().extra.age, 0, "{name}");
                    assert_eq!(mob.living().baby(game), Some(false), "{name}");
                    assert_eq!(mob.no_ai(game), Some(true), "{name}");
                    assert_eq!(mob.persistence_required(game), Some(true), "{name}");
                }
            }
            let mob = add_mob(game, 14, "minecraft:sniffer");
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(mob.entity().epoch().get() + 1),
                14,
            ));
            assert_eq!(stale.requires_custom_persistence(game), None);
            assert!(!stale.set_baby(game, true));
            game.server.held_out.push(14);
            assert_eq!(mob.requires_custom_persistence(game), None);
            assert!(!mob.set_baby(game, true));
            game.server.held_out.pop();
            game.mobs.last_mut().unwrap().removed = true;
            assert_eq!(mob.requires_custom_persistence(game), None);
            assert!(!mob.set_baby(game, true));
            assert_eq!(game.mobs.last().unwrap().extra.age, 0);
            game.mobs.last_mut().unwrap().removed = false;
            let taken = game.mobs.lift();
            assert_eq!(mob.requires_custom_persistence(game), None);
            assert!(!mob.set_baby(game, true));
            game.mobs = taken;
            assert!(mob.set_baby(game, true));

            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 15;
            game.dragon.replace(dragon);
            let dragon = game.mob(game.entity(15).unwrap()).unwrap();
            assert_eq!(dragon.requires_custom_persistence(game), None);
            assert!(!dragon.set_baby(game, true));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_target_round_trips_through_the_mob_binding() {
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

        with_resident_game(|game| {
            let settings = sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                announce: false,
                ..NetConfig::lan(0, "MobRefTest".into(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 1,
                    uuid: 123,
                    name: "guest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                20,
                [0.0; 3],
                8,
            );
            guest.placed = true;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            let player = game.living(game.entity(20).unwrap()).unwrap();
            assert!(mob.set_target(game, Some(player)));
            assert_eq!(mob.target(game), Some(player));
            assert_eq!(
                game.server.lan.as_ref().unwrap().mob_players.get(&12),
                Some(&1)
            );
            // `ZombifiedPiglin.setTarget` does not impersonate a player hit:
            // existing kill credit and its remaining lifetime are unchanged.
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombified_piglin")
                .unwrap()
                .clone();
            let mut piglin_state = Mob::new(13, kind, Vec3::ZERO, 0.0);
            piglin_state.ai.last_hurt_by_player = 42;
            game.mobs.push(piglin_state);
            let piglin = game.mob(game.entity(13).unwrap()).unwrap();
            assert!(piglin.set_target(game, Some(player)));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 13)
                    .unwrap()
                    .ai
                    .last_hurt_by_player,
                42
            );
            assert!(!game.server.last_hurt_by_player.contains_key(&13));
            assert!(piglin.set_target(game, None));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 13)
                    .unwrap()
                    .ai
                    .last_hurt_by_player,
                42
            );
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 9999, "fake").unwrap();
            let fake_living = LivingRef::new(fake.entity());
            assert!(!piglin.set_target(game, Some(fake_living)));
            assert_eq!(piglin.target(game), None);
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 13)
                    .unwrap()
                    .ai
                    .last_hurt_by_player,
                42
            );
            // The alert recipient is allied with the host, not with the
            // unteamed guest targeted by the source piglin.
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombified_piglin")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(14, kind, Vec3::ZERO, 0.0));
            game.persist.uuids.insert(14, [0, 0, 0, 14]);
            let host_name = game.acting_player_name().to_owned();
            game.score.board.add_team("red");
            game.score.board.add_to_team(&host_name, "red");
            game.score
                .board
                .add_to_team("00000000-0000-0000-0000-00000000000e", "red");
            let recipient = game.mob(game.entity(14).unwrap()).unwrap();
            assert!(piglin.set_target(game, Some(player)));
            let alert = || {
                mobs::MobEvent::PiglinAlertTarget(mobs::undead::PiglinAlert {
                    mob: 14,
                    source: 13,
                    target: Who::Player,
                })
            };
            game.handle_mob_events(vec![alert()]).unwrap();
            assert_eq!(recipient.target(game), Some(player));
            assert!(recipient.set_target(game, None));
            game.score.board.add_to_team("guest", "red");
            game.handle_mob_events(vec![alert()]).unwrap();
            assert_eq!(recipient.target(game), None);
            {
                let value = 1;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.game_mode = value);
            }
            assert!(!mob.set_target(game, Some(player)));
            assert_eq!(mob.target(game), None);
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
            {
                let value = 0;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.game_mode = value);
            }
            {
                let value = "minecraft:the_nether".into();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            assert!(!mob.set_target(game, Some(player)));
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
            game.server.lan.as_mut().unwrap().mob_players.clear();
            assert_eq!(mob.target(game), None);
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn brain_species_take_a_written_attack_target_memory() {
        with_resident_game(|game| {
            for (id, name) in [
                (12, "minecraft:warden"),
                (13, "minecraft:breeze"),
                (14, "minecraft:axolotl"),
                (15, "minecraft:piglin"),
                (16, "minecraft:hoglin"),
                (17, "minecraft:creaking"),
                (18, "minecraft:zoglin"),
                (19, "minecraft:frog"),
                (20, "minecraft:zombie"),
                (21, "minecraft:iron_golem"),
            ] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let handle = |game: &sunrise_gameplay::game::ServerGame<'_>, id| {
                game.mob(game.entity(id).unwrap()).unwrap()
            };
            let golem = game.living(game.entity(21).unwrap()).unwrap();
            // Each brain keeps the memory where its behaviours read it, and the
            // read handle answers it back; clearing erases it.
            for id in [12, 13, 14, 15, 16, 17, 18] {
                let mob = handle(game, id);
                assert!(mob.set_brain_target(game, Some(golem)), "mob {id}");
                assert_eq!(mob.target(game), Some(golem), "mob {id}");
                assert_eq!(
                    mobs::ai::brain_target(game.mobs.iter().find(|m| m.id == id).unwrap()),
                    Some(Some(Who::Mob(21))),
                    "mob {id}"
                );
                assert!(mob.set_brain_target(game, None), "mob {id}");
                assert_eq!(mob.target(game), None, "mob {id}");
            }
            // A plain memory write: the warden's own setAttackTarget effects
            // (the sonic-boom cooldown among them) do not run.
            let warden = handle(game, 12);
            assert!(warden.set_brain_target(game, Some(golem)));
            let st = game.mobs.iter().find(|m| m.id == 12).unwrap();
            assert!(
                !st.extra
                    .warden
                    .as_ref()
                    .unwrap()
                    .brain
                    .mem
                    .sonic_boom_cooldown
                    .present()
            );
            // The brains act on it: a tick keeps a hoglin's and a breeze's
            // written target (their target-validity behaviours accept an iron
            // golem), as a memory their own sensing set would be kept.
            for id in [13, 16] {
                assert!(handle(game, id).set_brain_target(game, Some(golem)));
            }
            game.tick_mobs().unwrap();
            for id in [13, 16] {
                assert_eq!(
                    handle(game, id).target(game),
                    Some(golem),
                    "mob {id} after a tick"
                );
            }
            // The frog's target belongs to its tongue attack; a mob without
            // an attack-target brain takes `set_target` instead.
            assert!(!handle(game, 19).set_brain_target(game, Some(golem)));
            assert!(!handle(game, 20).set_brain_target(game, Some(golem)));
            assert!(handle(game, 20).set_target(game, Some(golem)));
            // The ordinary setter still refuses the brain species.
            assert!(!handle(game, 13).set_target(game, Some(golem)));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_handles_read_write_and_refuse_unavailable_entities() {
        with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie.clone(), Vec3::ZERO, 0.0));
            game.mobs.push(Mob::new(13, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(14, Vec3::ZERO, 0.0));
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            // Reserve the dragon's eight consecutive part IDs away from the mobs.
            dragon.id = 40;
            game.dragon.replace(dragon);
            assert!(
                game.mob(game.entity(40).unwrap()).is_some(),
                "the dragon is a Mob"
            );
            let entity = game.entity(12).unwrap();
            let mob = game.mob(entity).unwrap();
            let other = game.living(game.entity(13).unwrap()).unwrap();
            let stand = game.living(game.entity(14).unwrap()).unwrap();
            for (id, name) in [
                (16, "minecraft:creaking"),
                (17, "minecraft:creeper"),
                (18, "minecraft:goat"),
                (19, "minecraft:warden"),
                (21, "minecraft:ghast"),
            ] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let brain = game.mob(game.entity(16).unwrap()).unwrap();
            mobs::monster_brain::set_attack_target(
                game.mobs.iter_mut().find(|m| m.id == 16).unwrap(),
                Some(Who::Mob(13)),
            );
            assert_eq!(brain.target(game), Some(other));
            assert!(!brain.set_target(game, None));
            assert_eq!(brain.target(game), Some(other));
            let warden = game.mob(game.entity(19).unwrap()).unwrap();
            let ghast = game.living(game.entity(21).unwrap()).unwrap();
            assert!(warden.set_brain_target(game, Some(ghast)));
            assert_eq!(
                warden.target(game),
                Some(ghast),
                "warden can target a ghast, unlike Mob.canAttack"
            );
            assert!(!warden.set_target(game, None));
            let creeper = game.mob(game.entity(17).unwrap()).unwrap();
            let goat = game.living(game.entity(18).unwrap()).unwrap();
            assert!(creeper.set_target(game, Some(other)));
            assert!(!creeper.set_target(game, Some(goat)));
            assert_eq!(creeper.target(game), Some(other));
            assert!(game.mob(stand.entity()).is_none());
            assert_eq!(mob.living().entity(), entity);
            assert_eq!(game.mob_is_aggressive(mob), Some(false));
            assert!(game.mob_set_aggressive(mob, true));
            assert_eq!(game.mob_is_aggressive(mob), Some(true));
            assert_eq!(game.mob_persistence_required(mob), Some(false));
            assert!(game.mob_set_persistence_required(mob));
            assert_eq!(game.mob_persistence_required(mob), Some(true));
            assert!(mob.set_target(game, Some(other)));
            assert_eq!(mob.target(game), Some(other));
            assert!(!game.mob_set_target(mob, Some(stand)));
            assert_eq!(game.mob_target(mob), Some(other));
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                12,
            ));
            assert!(game.mob(stale.entity()).is_none());
            assert!(!game.mob_set_target(stale, None));
            assert_eq!(game.mob_is_aggressive(stale), None);
            let taken = game.mobs.lift();
            assert_eq!(game.mob_target(mob), None);
            assert!(!game.mob_set_target(mob, None));
            assert_eq!(taken[0].ai.target, Some(Who::Mob(13)));
            game.mobs = taken;
            assert!(game.mob_set_target(mob, None));
            assert_eq!(game.mob_target(mob), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn look_and_jump_go_through_the_mobs_own_controls() {
        with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let creaking = game
                .res
                .mob_types()
                .get("minecraft:creaking")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(16, creaking, Vec3::new(9.5, 65.0, 9.5), 0.0));
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            assert!(game.mob_look_at(mob, [10.5, 66.0, -2.5]));
            let (wanted, cooldown, speed) = {
                let control = &game.mobs[0].ai.look;
                (control.wanted, control.cooldown, control.y_max_rot_speed)
            };
            assert_eq!(wanted, [10.5, 66.0, -2.5]);
            assert_eq!(cooldown, 2);
            assert_eq!(speed, game.mobs[0].kind.head_rot_speed);
            assert!(!game.mob_look_at(mob, [f64::NAN, 66.0, 0.0]));
            assert_eq!(game.mobs[0].ai.look.wanted, [10.5, 66.0, -2.5]);
            // A brain species writes through the same look control, so its
            // handle takes the wish too.
            let brain = game.mob(game.entity(16).unwrap()).unwrap();
            assert!(game.mob_look_at(brain, [0.0, 66.0, 0.0]));
            assert_eq!(game.mobs[1].ai.look.wanted, [0.0, 66.0, 0.0]);
            assert!(game.mob_jump(brain));
            assert!(game.mobs[1].ai.jump);
            assert!(game.mob_jump(mob));
            assert!(game.mobs[0].ai.jump);
            // JumpControl.tick consumes the wish as the next tick's jump.
            game.mobs[0].jump_control_tick();
            assert!(game.mobs[0].ai.jumping);
            assert!(!game.mobs[0].ai.jump);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_restriction_round_trips_through_the_handle_and_saves() {
        use sunrise_core::nbt::Tag;
        with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            assert_eq!(game.mob_restrict_center(mob), Some([0, 0, 0]));
            assert_eq!(game.mob_restrict_radius(mob), Some(-1));
            assert_eq!(game.mob_has_restriction(mob), Some(false));
            assert_eq!(
                game.mob_is_within_restriction(mob, [900, 90, 900]),
                Some(true),
                "an unrestricted mob is within everywhere"
            );
            assert!(game.mob_restrict_to(mob, [10, 64, -5], 6));
            assert_eq!(game.mob_has_restriction(mob), Some(true));
            assert_eq!(game.mob_restrict_center(mob), Some([10, 64, -5]));
            assert_eq!(game.mob_restrict_radius(mob), Some(6));
            assert_eq!(
                game.mob_is_within_restriction(mob, [12, 64, -2]),
                Some(true)
            );
            assert_eq!(
                game.mob_is_within_restriction(mob, [20, 64, 0]),
                Some(false)
            );
            // The home area is saved with the mob.
            let saved = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [0; 4],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            assert_eq!(
                saved.get("home_pos"),
                Some(&Tag::IntArray(vec![10, 64, -5]))
            );
            assert_eq!(saved.get("home_radius"), Some(&Tag::Int(6)));
            // clearRestriction keeps the center; every position is within
            // again.
            assert!(game.mob_clear_restriction(mob));
            assert_eq!(game.mob_has_restriction(mob), Some(false));
            assert_eq!(game.mob_restrict_center(mob), Some([10, 64, -5]));
            assert_eq!(game.mob_restrict_radius(mob), Some(-1));
            assert_eq!(game.mob_is_within_restriction(mob, [20, 64, 0]), Some(true));
            let saved = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [0; 4],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            assert!(saved.get("home_pos").is_none());
            assert!(saved.get("home_radius").is_none());
            // restrictTo with radius -1 clears while keeping the new center.
            assert!(game.mob_restrict_to(mob, [1, 2, 3], -1));
            assert_eq!(game.mob_has_restriction(mob), Some(false));
            assert_eq!(game.mob_restrict_center(mob), Some([1, 2, 3]));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_extreme_restriction_answers_as_vanilla() {
        with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, zombie, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            // `Vec3i.distSqr` in doubles: the far corner of the int range is
            // outside, with no overflow on the way.
            assert!(game.mob_restrict_to(mob, [i32::MIN, 0, 0], 6));
            assert_eq!(
                game.mob_is_within_restriction(mob, [i32::MAX, 0, 0]),
                Some(false)
            );
            assert!(game.mob_restrict_to(mob, [i32::MIN, i32::MIN, i32::MIN], 6));
            assert_eq!(
                game.mob_is_within_restriction(mob, [i32::MAX, i32::MAX, i32::MAX]),
                Some(false)
            );
            // The radius squares as a Java `int`: 65 536² wraps to 0, so not
            // even the center is within.
            assert!(game.mob_restrict_to(mob, [0, 0, 0], 65_536));
            assert_eq!(game.mob_is_within_restriction(mob, [0, 0, 0]), Some(false));
            // The largest radius that squares without wrapping still holds.
            assert!(game.mob_restrict_to(mob, [0, 0, 0], 46_340));
            assert_eq!(
                game.mob_is_within_restriction(mob, [46_339, 0, 0]),
                Some(true)
            );
            assert_eq!(
                game.mob_is_within_restriction(mob, [46_340, 0, 0]),
                Some(false)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn hand_and_loot_flags_live_on_the_mob_handle() {
        use sunrise_core::nbt::Tag;
        with_resident_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let mut dragon =
                sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 15;
            game.dragon.replace(dragon);
            let entity = game.entity(12).unwrap();
            let mob = game.mob(entity).unwrap();
            assert_eq!(game.mob_left_handed(mob), Some(false));
            assert!(game.mob_set_left_handed(mob, true));
            assert!(game.mobs[0].extra.left_handed && game.mobs[0].main_arm_left());
            assert_eq!(game.mob_left_handed(mob), Some(true));
            assert!(game.mob_set_can_pick_up_loot(mob, true));
            assert!(game.mobs[0].extra.pick_up_loot);
            assert_eq!(game.mob_can_pick_up_loot(mob), Some(true));
            // Saved as LeftHanded / CanPickUpLoot.
            let saved = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [0; 4],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            assert_eq!(saved.get("LeftHanded"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("CanPickUpLoot"), Some(&Tag::Byte(1)));
            assert!(game.mob_set_left_handed(mob, false));
            assert!(game.mob_set_can_pick_up_loot(mob, false));
            assert_eq!(game.mob_left_handed(mob), Some(false));
            assert_eq!(game.mob_can_pick_up_loot(mob), Some(false));
            // The dragon is not a Mob for these.
            let dragon = game.mob(game.entity(15).unwrap()).unwrap();
            assert_eq!(game.mob_left_handed(dragon), None);
            assert!(!game.mob_set_left_handed(dragon, true));
            assert_eq!(game.mob_can_pick_up_loot(dragon), None);
            assert!(!game.mob_set_can_pick_up_loot(dragon, true));
            assert!(!game.mob_look_at(dragon, [0.0; 3]));
            assert!(!game.mob_jump(dragon));
            assert!(!game.mob_restrict_to(dragon, [0, 0, 0], 5));
            assert_eq!(game.mob_has_restriction(dragon), None);
            // A stale handle changes no other entity.
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                12,
            ));
            assert_eq!(game.mob_left_handed(stale), None);
            assert!(!game.mob_set_left_handed(stale, true));
            assert_eq!(game.mob_can_pick_up_loot(stale), None);
            assert!(!game.mob_set_can_pick_up_loot(stale, true));
            assert!(!game.mob_look_at(stale, [0.0; 3]));
            assert!(!game.mob_jump(stale));
            assert!(!game.mob_restrict_to(stale, [0, 0, 0], 5));
            assert!(!game.mob_clear_restriction(stale));
            assert_eq!(game.mob_restrict_center(stale), None);
            assert_eq!(game.mob_restrict_radius(stale), None);
            assert_eq!(game.mob_has_restriction(stale), None);
            assert_eq!(game.mob_is_within_restriction(stale, [0, 0, 0]), None);
            assert!(!game.mobs[0].extra.left_handed);
            assert!(!game.mobs[0].extra.pick_up_loot);
        });
    }
}
