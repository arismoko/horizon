#![cfg(test)]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_protocol::text::translatable;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::lan_actions::attack_checks::tests::disconnects;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::{configuration::ClientInformation, play};
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    /// handleAttack disconnects a player who attacks a dropped item, an experience orb,
    /// themselves, an arrow or a thrown trident (`invalid_entity_attacked`); a pig is
    /// attacked as before, and an item out of reach, or attacked with a spear in hand,
    /// ends the attack without a kick.
    #[test]
    #[ignore = "daily"]
    fn attacking_an_item_an_orb_oneself_or_an_arrow_kicks_the_player() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::env::temp_dir().join(format!("sunrise-attack-checks-unit-{}", std::process::id())),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap()
            .0;
        let mut chunk = Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        game.lan_host(
            sunrise_gameplay::game::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            NetConfig {
                announce: false,
                ..NetConfig::loopback(0, "attack-checks-test".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        let summoned = |game: &mut sunrise_gameplay::game::ServerGame<'_>, command: &str| {
            let before = game.server.next_entity;
            assert!(game.run_command(command), "{command}");
            game.lan_assign_ids();
            assert!(game.server.next_entity > before, "{command}");
            game.server.next_entity as i32
        };
        let pig = summoned(
            &mut game,
            "summon pig 6.5 200 4.5 {NoAI:1b,NoGravity:1b,PersistenceRequired:1b}",
        );
        let item = summoned(
            &mut game,
            "summon item 4.5 200 6 {NoGravity:1b,Item:{id:\"minecraft:stone\",count:1}}",
        );
        let far_item = summoned(
            &mut game,
            "summon item 4.5 200 12.5 {NoGravity:1b,Item:{id:\"minecraft:stone\",count:1}}",
        );
        let orb = summoned(
            &mut game,
            "summon experience_orb 3 200 4.5 {NoGravity:1b,Value:1s}",
        );
        let arrow = summoned(&mut game, "summon arrow 4.5 200.5 3 {NoGravity:1b}");
        let trident = summoned(&mut game, "summon trident 5.5 200.5 3 {NoGravity:1b}");
        let (tx, rx) = mpsc::channel();
        let fresh_guest = |conn, entity_id| {
            let mut guest = Guest::new(
                Joined {
                    conn,
                    uuid: 332,
                    name: "AttackGuest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx.clone(), Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                entity_id,
                [4.5, 200.0, 4.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.load_timeout = 0;
            guest
        };
        game.server.next_entity += 1;
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .push(fresh_guest(2, game.server.next_entity as i32));
        let attack = |game: &mut sunrise_gameplay::game::ServerGame<'_>, conn, id: i32| {
            game.lan_guest_packet(conn, S::Attack(play::Attack { id }));
        };
        let kicked = vec![translatable(
            "multiplayer.disconnect.invalid_entity_attacked",
            &[],
        )];

        let hp = |g: &sunrise_gameplay::game::ServerGame<'_>| {
            g.mobs.iter().find(|m| m.id as i32 == pig).unwrap().health
        };
        let full = hp(&game);
        attack(&mut game, 2, pig);
        assert_eq!(disconnects(&rx), vec![], "a pig is a target");
        assert!(hp(&game) < full, "the pig is not hurt: {full} health left");
        attack(&mut game, 2, far_item);
        assert_eq!(disconnects(&rx), vec![], "an item out of reach");
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                guest.inventory[0] = Some(ItemStack::new("minecraft:iron_spear", 1));
                guest.selected = 0;
            });
        attack(&mut game, 2, item);
        assert_eq!(disconnects(&rx), vec![], "a spear in hand");
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                guest.inventory[0] = None;
            });
        game.server.lan.as_mut().unwrap().hub.guests.remove(0);
        for (case, (what, target)) in [
            ("an item", Some(item)),
            ("an experience orb", Some(orb)),
            ("themselves", None),
            ("an arrow", Some(arrow)),
            ("a thrown trident", Some(trident)),
        ]
        .into_iter()
        .enumerate()
        {
            // Each kick ends a connection. A later invalid target needs a new
            // guest, not another packet from the already disconnected player.
            game.server.next_entity += 1;
            let me = game.server.next_entity as i32;
            let conn = 3 + case as u32;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .push(fresh_guest(conn, me));
            attack(&mut game, conn, target.unwrap_or(me));
            assert_eq!(disconnects(&rx), kicked, "{what}");
            game.server.lan.as_mut().unwrap().hub.guests.remove(0);
        }
    }
}
