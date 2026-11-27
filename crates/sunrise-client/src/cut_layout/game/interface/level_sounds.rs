#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use super::super::native_entity_refs::tests::{with_game, with_resident_game};
    use mod_api::{HostSide, SoundSource};
    use std::sync::{Arc, Mutex};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::server_side::lan::LanSettings;
    use sunrise_gameplay_common::game::creatures::ModGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_protocol::packets::play::Clientbound;
    use sunrise_sim::lan::{NetConfig, hub::HubConfig};
    use sunrise_sim::mobs::{Mob, MobEvent};
    use {
        sunrise_gameplay_common::game::server_side::native_sound_refs::tests::guest,
        sunrise_gameplay_common::game::server_side::native_sound_refs::tests::packets,
    };

    /// Quietens and moves the locked chest, cancels the pig, and turns a
    /// sound bound to an entity into a cow's; logs what it was asked.
    struct Sounds(Arc<Mutex<Vec<(&'static str, String, Option<u32>)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Sounds {
        fn play_level_sound_at_position(
            &self,
            host: HostSide<'_>,
            e: &mut mod_api::PlayLevelSoundAtPositionEvent,
        ) -> mod_api::Flow {
            assert!(matches!(host, HostSide::Server(_)));
            self.0
                .lock()
                .unwrap()
                .push(("position", e.sound.to_string(), None));
            match e.sound.to_string().as_str() {
                "minecraft:block.chest.locked" => {
                    assert_eq!((e.original_volume, e.original_pitch), (1.0, 1.0));
                    (e.volume, e.pitch, *e.source) = (0.25, 2.0, SoundSource::Ambient);
                    mod_api::Flow::Continue
                }
                "minecraft:entity.pig.ambient" => mod_api::Flow::Cancel,
                _ => mod_api::Flow::Continue,
            }
        }
        fn play_level_sound_at_entity(
            &self,
            host: HostSide<'_>,
            e: &mut mod_api::PlayLevelSoundAtEntityEvent,
        ) -> mod_api::Flow {
            assert!(matches!(host, HostSide::Server(_)));
            self.0
                .lock()
                .unwrap()
                .push(("entity", e.sound.to_string(), Some(e.entity.id())));
            *e.sound = mod_api::Identifier::parse("minecraft:entity.cow.ambient").unwrap();
            mod_api::Flow::Continue
        }
    }

    /// Opens the world to the network in place, with no one joined yet.
    fn host(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        let net = NetConfig {
            listen: false,
            announce: false,
            lod_distance: 0,
            ..NetConfig::lan(0, String::new(), 3)
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
            max_players: 3,
            integrated: true,
            levels: game.server.level_shapes.clone(),
            level_seeds: game.server.levels.0.runtime.seeds(),
        };
        let settings = LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        game.lan_host(settings, net, config).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn experience_level_up_reaches_only_the_gaining_players_level() {
        with_game(|game| {
            host(game);
            let overworld = game.dim;
            let nether = sunrise_gameplay_common::game::Dim::Nether;
            game.enter_level(nether).unwrap();
            game.enter_level(overworld).unwrap();

            let at = [1.0, 64.0, 2.0];
            let (mut target, rx_target) = guest(21, 21, at);
            target.dimension = nether.id().into();
            target.xp_level = 4;
            let (mut nearby, rx_nearby) = guest(22, 22, at);
            nearby.dimension = nether.id().into();
            let (caller, rx_caller) = guest(23, 23, at);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .extend([target, nearby, caller]);
            game.as_guest(21, |game| {
                for _ in 0..101 {
                    game.server.level_up_sound.tick();
                }
            })
            .unwrap();

            let host: &mut dyn mod_api::ServerHost = game;
            let target = host.player(21).unwrap();
            assert!(target.give_experience_points(host, 15));
            assert_eq!(target.experience(host).unwrap().level, 5);
            assert_eq!(game.dim, overworld);
            game.lan_broadcast_effects();
            game.flush_native_sounds();
            let id = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .sync_data()
                .id("minecraft:sound_event", "minecraft:entity.player.levelup")
                .unwrap();

            for rx in [&rx_target, &rx_nearby] {
                let sounds: Vec<_> = packets(rx)
                    .into_iter()
                    .filter_map(|packet| match packet {
                        Clientbound::Sound(sound) => Some(sound),
                        _ => None,
                    })
                    .collect();
                let [sound] = sounds.as_slice() else {
                    panic!("expected one nether sound")
                };
                assert_eq!(sound.sound, (id, None));
                assert_eq!((sound.x, sound.y, sound.z), (8, 512, 16));
                assert_eq!((sound.volume, sound.pitch), (0.125, 1.0));
                assert_eq!(sound.source, SoundSource::Players.index() as i32);
            }
            assert!(
                !packets(&rx_caller)
                    .iter()
                    .any(|p| matches!(p, Clientbound::Sound(_)))
            );
        });
    }

    /// Vanilla's `playSound(except, entity, ...)` sites ask
    /// `play_level_sound_at_entity` with their entity, at the end of the
    /// tick, in vanilla's source: a goat's ram (the mobs' `BoundSound`) and
    /// a dispenser shearing a snow golem. Each reaches the players once.
    #[test]
    #[ignore = "daily"]
    fn entity_bound_sites_ask_at_their_entity() {
        let heard = Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "sounds",
            Box::new(Sounds(Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                host(game);
                let key = (0, 0);
                if !game.world.chunks.contains_key(&key) {
                    let chunk = sunrise_level::chunk::Chunk::with_layout(0, 0, game.world.layout());
                    game.world.chunks.insert(key, Arc::new(chunk));
                }
                let (a, rx) = guest(21, 21, [1.0, 64.0, 2.0]);
                game.server.lan.as_mut().unwrap().hub.guests.push(a);
                let mob = |game: &mut sunrise_gameplay::game::ServerGame<'_>, id, kind: &str, x| {
                    let kind = game.res.mob_types().get(kind).unwrap().clone();
                    game.mobs
                        .push(Mob::new(id, kind, Vec3::new(x, 64.0, 2.0), 0.0));
                };
                mob(game, 50, "minecraft:goat", 2.0);
                mob(game, 51, "minecraft:snow_golem", 4.0);

                // RamTarget: level.playSound(null, goat, RAM_IMPACT, NEUTRAL, 1, 1).
                let goat = game.mobs.iter().find(|m| m.id == 50).unwrap();
                let ram = MobEvent::bound_sound(
                    goat,
                    "entity.goat.ram_impact",
                    SoundSource::Neutral,
                    1.0,
                    1.0,
                );
                game.handle_mob_events(vec![ram]).unwrap();
                // SnowGolem.shear from a dispenser: BLOCKS.
                let shears = sunrise_items::items::ItemStack::new("minecraft:shears", 1);
                assert!(game.shear_entity((4, 64, 2), &shears).unwrap());
                assert!(
                    heard.lock().unwrap().is_empty(),
                    "asked at the end of the tick"
                );
                assert!(
                    game.server.pending_sounds.is_empty(),
                    "not also sent as a positional sound"
                );

                game.lan_broadcast_effects();
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        (
                            "entity",
                            "minecraft:entity.goat.ram_impact".into(),
                            Some(50)
                        ),
                        (
                            "entity",
                            "minecraft:entity.snow_golem.shear".into(),
                            Some(51)
                        ),
                    ]
                );
                // The mod made both a cow's; the sources stay vanilla's.
                let sources: Vec<_> = packets(&rx)
                    .into_iter()
                    .filter_map(|p| match p {
                        Clientbound::Sound(p) => Some(p.source),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    sources,
                    [SoundSource::Neutral as i32, SoundSource::Blocks as i32]
                );
                assert!(game.server.bound_sounds.is_empty());
            });
        });
    }

    /// `PlayLevelSoundEvent`: the tick's sounds are asked at a position, a
    /// sound bound to an entity at that entity; the players get the mod's
    /// sound, and none for a cancelled one.
    #[test]
    #[ignore = "daily"]
    fn a_mod_changes_and_cancels_level_sounds() {
        let heard = Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "sounds",
            Box::new(Sounds(Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_resident_game(|game| {
                let net = NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..NetConfig::lan(0, String::new(), 3)
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
                    max_players: 3,
                    integrated: true,
                    levels: game.server.level_shapes.clone(),
                    level_seeds: game.server.levels.0.runtime.seeds(),
                };
                let settings = LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                };
                game.lan_host(settings, net, config).unwrap();
                let (a, rx) = guest(21, 21, [1.0, 64.0, 2.0]);
                game.server.lan.as_mut().unwrap().hub.guests.push(a);
                let (b, rx_b) = guest(22, 22, [1.0, 64.0, 3.0]);
                game.server.lan.as_mut().unwrap().hub.guests.push(b);
                let at = [2.0, 64.0, 2.0];

                // The tick's sounds, at their positions.
                game.server.pending_sounds.extend(
                    [
                        "minecraft:block.chest.locked",
                        "minecraft:entity.pig.ambient",
                    ]
                    .into_iter()
                    .map(|event| {
                        sunrise_game_data::sound_source::ServerSound {
                            event: event.into(),
                            position: at,
                            volume: 1.0,
                            pitch: 1.0,
                            source: SoundSource::Blocks as usize,
                            entity: None,
                            except_entity: None,
                            seed: None,
                        }
                    }),
                );
                game.lan_broadcast_effects();
                let got = packets(&rx);
                assert_eq!(got.len(), 1, "the pig is cancelled");
                let Clientbound::Sound(p) = &got[0] else {
                    panic!("expected a sound")
                };
                assert_eq!((p.volume, p.pitch), (0.25, 2.0));
                assert_eq!(p.source, SoundSource::Ambient as i32);

                // A mod's sound bound to a cow, asked at the cow.
                let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                game.mobs
                    .push(Mob::new(40, kind, Vec3::new(2.0, 64.0, 2.0), 0.0));
                let cow = game.entity(40).unwrap();
                assert!(game.entity_play_bound_sound(
                    cow,
                    "minecraft:entity.player.burp",
                    SoundSource::Neutral,
                    1.0,
                    1.0
                ));
                game.flush_native_sounds();
                let got = packets(&rx);
                assert_eq!(got.len(), 1);
                let Clientbound::SoundEntity(p) = &got[0] else {
                    panic!("expected a bound sound")
                };
                assert_eq!(p.id, 40);
                let cow_ambient = game.server.lan.as_ref().unwrap().hub.sound_packet(
                    "minecraft:entity.cow.ambient",
                    at,
                    1.0,
                    1.0,
                    0,
                    0,
                );
                let Clientbound::Sound(expected) = cow_ambient else {
                    panic!("expected a sound")
                };
                assert_eq!(format!("{:?}", p.sound), format!("{:?}", expected.sound));

                // `Player.playSound` for the first guest (eating): the other
                // guest hears it, and the mods were asked first.
                game.as_guest(21, |game| {
                    game.server.guest_player_sounds.push((
                        "minecraft:entity.generic.eat".into(),
                        at,
                        1.0,
                        1.0,
                    ));
                })
                .unwrap();
                let got = packets(&rx_b);
                let Some(Clientbound::Sound(p)) = got.last() else {
                    panic!("expected the guest's sound")
                };
                assert_eq!(p.source, SoundSource::Players as i32);
                assert!(packets(&rx).is_empty(), "not the guest who made it");

                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("position", "minecraft:block.chest.locked".into(), None),
                        ("position", "minecraft:entity.pig.ambient".into(), None),
                        ("entity", "minecraft:entity.player.burp".into(), Some(40)),
                        ("position", "minecraft:entity.generic.eat".into(), None),
                    ]
                );
            });
        });
    }
}
