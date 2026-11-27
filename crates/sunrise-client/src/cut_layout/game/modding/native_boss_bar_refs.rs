#![cfg(test)]
use sunrise_sim::lan::playerdata::uuid_ints;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::__private::game::Fields;
    use mod_api::{BossBarColor, BossBarOverlay, BossBarRef};
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
    };
    use sunrise_protocol::packets::configuration::ClientInformation;
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use {
        crate::game::server_side::native_entity_refs::tests::with_game,
        sunrise_gameplay::game::server_side::lan::LanSettings,
    };

    fn guest(uuid: u128, conn: u32) -> Guest {
        let (tx, _) = std::sync::mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn,
                uuid,
                name: format!("guest{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [0.0; 3],
            10,
        );
        g.placed = true;
        g
    }

    #[test]
    #[ignore = "daily"]
    fn handles_create_update_list_and_go_stale_like_bossbar_commands() {
        with_game(|game| {
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 4,
                simulation_distance: 4,
                dimension: "minecraft:overworld".into(),
                allow_flight: true,
                seed: 0,
                max_players: 10,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            let mut net = NetConfig::lan(0, "test".into(), 10);
            net.listen = false;
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            game.lan_host(settings, net, config).unwrap();
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.push(guest(123, 1));
            hub.guests.push(guest(456, 2));
            let (a, b) = (game.player(123).unwrap(), game.player(456).unwrap());
            assert!(game.boss_bar("mod:bar").is_none());
            assert!(
                mod_api::prelude::BossBarsServerHost::create_boss_bar(
                    game,
                    "not a valid id",
                    mod_api::Text::literal("x")
                )
                .unwrap()
                .is_none()
            );
            let bar = mod_api::prelude::BossBarsServerHost::create_boss_bar(
                game,
                "mod:bar",
                mod_api::Text::literal("Hi"),
            )
            .unwrap()
            .unwrap();
            assert!(
                mod_api::prelude::BossBarsServerHost::create_boss_bar(
                    game,
                    "mod:bar",
                    mod_api::Text::literal("Again")
                )
                .unwrap()
                .is_none()
            );
            assert_eq!(game.boss_bar("mod:bar"), Some(bar));
            assert_eq!(game.boss_bars(), vec![bar]);
            assert_eq!(bar.id(game).as_deref(), Some("mod:bar"));
            assert_eq!(
                bar.name(game).unwrap().unwrap(),
                mod_api::Text::literal("Hi")
            );
            assert_eq!(bar.color(game), Some(BossBarColor::White));
            assert_eq!(bar.overlay(game), Some(BossBarOverlay::Progress));
            assert_eq!(bar.visible(game), Some(true));
            assert_eq!((bar.value(game), bar.max(game)), (Some(0), Some(100)));
            assert_eq!(bar.players(game), Some(vec![]));

            // Nobody sees it until a player is added.
            let seen = |game: &sunrise_gameplay::game::ServerGame<'_>, uuid: u128| {
                game.server.bosses.sent_state(uuid, bar_uuid(game, bar))
            };
            assert_eq!(seen(game, 123), None);
            assert!(bar.add_player(game, a));
            assert!(bar.add_player(game, a));
            assert_eq!(bar.players(game), Some(vec![a]));
            assert_eq!(seen(game, 123), Some((0.0, 6, 0, false, false, false)));
            assert_eq!(seen(game, 456), None);

            assert!(bar.set_name(game, mod_api::Text::literal("Boss")).unwrap());
            assert!(bar.set_color(game, BossBarColor::Red));
            assert!(bar.set_overlay(game, BossBarOverlay::Notched10));
            assert!(bar.set_darken_screen(game, true));
            assert!(bar.set_play_boss_music(game, true));
            assert!(bar.set_create_world_fog(game, true));
            assert!(bar.set_max(game, 50));
            assert!(bar.set_value(game, 25));
            assert_eq!(
                bar.name(game).unwrap().unwrap(),
                mod_api::Text::literal("Boss")
            );
            assert_eq!(bar.color(game), Some(BossBarColor::Red));
            assert_eq!(bar.overlay(game), Some(BossBarOverlay::Notched10));
            assert_eq!(
                (
                    bar.darken_screen(game),
                    bar.play_boss_music(game),
                    bar.create_world_fog(game)
                ),
                (Some(true), Some(true), Some(true))
            );
            assert_eq!((bar.value(game), bar.max(game)), (Some(25), Some(50)));
            assert_eq!(seen(game, 123), Some((0.5, 2, 2, true, true, true)));

            // Value and max are stored as given; only the progress is clamped.
            assert!(bar.set_value(game, 999));
            assert_eq!((bar.value(game), bar.max(game)), (Some(999), Some(50)));
            assert_eq!(seen(game, 123).unwrap().0, 1.0);
            assert!(bar.set_max(game, 10));
            assert_eq!((bar.value(game), bar.max(game)), (Some(999), Some(10)));
            assert!(bar.set_value(game, -4));
            assert_eq!(bar.value(game), Some(-4));
            assert_eq!(seen(game, 123).unwrap().0, 0.0);
            assert!(bar.set_max(game, 0));
            assert_eq!((bar.value(game), bar.max(game)), (Some(-4), Some(0)));
            assert!(bar.set_max(game, 100) && bar.set_value(game, 7));
            assert_eq!(seen(game, 123).unwrap().0, 0.07);

            // The command sees the mod's bar and the mod sees the command's changes.
            game.ui.registries.permission = 4;
            assert!(game.run_command("bossbar set mod:bar value 9"));
            assert_eq!(bar.value(game), Some(9));
            assert!(game.run_command("bossbar get mod:bar value"));
            assert!(!game.run_command("bossbar add mod:bar \"dup\""));
            assert!(game.run_command("bossbar set mod:bar players"));
            assert_eq!(bar.players(game), Some(vec![]));
            assert_eq!(seen(game, 123), None);

            // Hidden bars keep their players but are not sent.
            assert!(bar.add_player(game, b));
            assert!(bar.set_visible(game, false));
            assert_eq!(bar.visible(game), Some(false));
            assert_eq!(seen(game, 456), None);
            assert!(bar.set_visible(game, true));
            assert!(seen(game, 456).is_some());
            // A player handle from an earlier session cannot edit membership.
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(b.entity().epoch().get() + 1),
                    b.entity().id(),
                ),
                456,
            );
            assert!(!bar.remove_player(game, stale));
            assert!(!bar.add_player(game, stale));
            assert_eq!(bar.players(game), Some(vec![b]));
            assert!(bar.remove_player(game, b));
            assert_eq!(bar.players(game), Some(vec![]));
            let offline = mod_api::__private::game::player_ref(b.fields().entity, 999);
            assert!(bar.add_player(game, a));
            assert!(!bar.remove_player(game, offline));
            assert_eq!(bar.players(game), Some(vec![a]));
            assert!(bar.remove_player(game, a));

            // Saved with the level: a real write and read of the .dat file keeps the
            // fields and the players' UUIDs.
            assert!(bar.add_player(game, a));
            let dir = std::env::temp_dir().join(format!("boss-save-{}", std::process::id()));
            game.server.world_root = dir.clone();
            game.save_bossbars();
            let root = sunrise_sim::save::read_dat(&game.bossbar_path()).unwrap();
            let loaded = sunrise_game_data::bossbar::BossBars::unpack(root.get("data").unwrap());
            let kept = loaded.get("mod:bar").unwrap();
            assert_eq!((kept.value, kept.max, kept.color.as_str()), (9, 100, "red"));
            assert_eq!(kept.players, vec![uuid_ints(123)]);
            let _ = std::fs::remove_dir_all(dir);
            assert!(bar.remove_player(game, a));

            // Removal: the handle is stale, the id can be added again.
            assert!(bar.remove(game));
            assert!(!bar.remove(game));
            assert_eq!(bar.id(game), None);
            assert!(!bar.set_value(game, 3));
            assert!(!bar.add_player(game, a));
            assert!(game.boss_bar("mod:bar").is_none());
            let again = mod_api::prelude::BossBarsServerHost::create_boss_bar(
                game,
                "mod:bar",
                mod_api::Text::literal("New"),
            )
            .unwrap()
            .unwrap();
            assert_ne!(again, bar);
            assert_eq!(bar.name(game).unwrap(), None);
            assert_eq!(
                again.name(game).unwrap().unwrap(),
                mod_api::Text::literal("New")
            );
            assert!(game.run_command("bossbar remove mod:bar"));
            assert_eq!(again.id(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn generated_text_crossings_refuse_too_deep_before_boss_bar_writes() {
        with_game(|game| {
            let bar = mod_api::prelude::BossBarsServerHost::create_boss_bar(
                game,
                "mod:kept",
                mod_api::Text::literal("Kept"),
            )
            .unwrap()
            .unwrap();
            let before = game.boss_bars();
            let name = bar.name(game).unwrap().unwrap();
            let too_deep = || {
                let mut text = mod_api::Text::literal("leaf");
                for _ in 1..513 {
                    text = mod_api::Text::literal("parent").append(text);
                }
                text
            };

            assert!(matches!(
                mod_api::prelude::BossBarsServerHost::create_boss_bar(
                    game,
                    "mod:refused",
                    too_deep(),
                ),
                Err(mod_api::text::TextFormError::TooDeep)
            ));
            assert_eq!(game.boss_bars(), before);
            assert!(game.boss_bar("mod:refused").is_none());
            assert_eq!(bar.name(game).unwrap().unwrap(), name);

            assert!(matches!(
                bar.set_name(game, too_deep()),
                Err(mod_api::text::TextFormError::TooDeep)
            ));
            assert_eq!(game.boss_bars(), before);
            assert_eq!(bar.name(game).unwrap().unwrap(), name);

            use mod_api::{Tag, text::TextFormError};
            let bad_key =
                Tag::Compound([(sunrise_core::nbt::JavaString(vec![0xd800]), Tag::Byte(1))].into());
            for (index, (data, expected)) in [
                (Tag::Float(f32::NAN.to_bits()), TextFormError::NonFinite),
                (
                    Tag::Double(f64::INFINITY.to_bits()),
                    TextFormError::NonFinite,
                ),
                (
                    Tag::String(sunrise_core::nbt::JavaString(vec![0xd800])),
                    TextFormError::InvalidString,
                ),
                (bad_key, TextFormError::InvalidString),
            ]
            .into_iter()
            .enumerate()
            {
                let bad_text = || {
                    let mut text = mod_api::Text::literal("Unwritten");
                    text.metadata = Tag::compound([("test:bad", data.clone())]);
                    text
                };
                let id = format!("mod:refused_opaque_{index}");
                assert_eq!(
                    mod_api::prelude::BossBarsServerHost::create_boss_bar(game, &id, bad_text())
                        .err(),
                    Some(expected.clone())
                );
                assert!(game.boss_bar(&id).is_none());
                assert_eq!(bar.set_name(game, bad_text()).err(), Some(expected));
                assert_eq!(game.boss_bars(), before);
                assert_eq!(bar.name(game).unwrap().unwrap(), name);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn saved_bars_are_found_before_any_tick() {
        with_game(|game| {
            let dir = std::env::temp_dir().join(format!("boss-early-{}", std::process::id()));
            game.server.world_root = dir.clone();
            let bar = mod_api::prelude::BossBarsServerHost::create_boss_bar(
                game,
                "mod:saved",
                mod_api::Text::literal("Kept"),
            )
            .unwrap()
            .unwrap();
            assert!(bar.set_value(game, 7));
            game.save_bossbars();
            // The world opens again and loads the bars, as `finish_open_world` does.
            game.server.bosses = Default::default();
            game.load_bossbars();
            let found = game.boss_bar("mod:saved").unwrap();
            assert_eq!(found.value(game), Some(7));
            assert_eq!(game.boss_bars().len(), 1);
            let _ = std::fs::remove_dir_all(dir);
        });
    }

    fn bar_uuid(game: &sunrise_gameplay::game::ServerGame<'_>, bar: BossBarRef) -> u128 {
        game.boss_bar_state(bar).unwrap().uuid
    }
}
