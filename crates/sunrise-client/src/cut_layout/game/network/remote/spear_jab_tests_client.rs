use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play;
use sunrise_sim::motion::PlayerMotion;

/// `MultiPlayerGameMode.piercingAttack`: a spear's jab sends its STAB action and nothing else,
/// even with the look changed since the last move packet, so the 20-tick position reminder
/// keeps its count.
#[test]
#[ignore = "daily"]
fn a_spear_jab_sends_only_its_stab() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let (mut peer, _writer) = super::connect_menu_test(game);
        peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        game.set_local_mode(sunrise_command::args::GameMode::Survival);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.inventory[0] = Some(ItemStack::new("minecraft:iron_spear", 1));
        game.selected = 0;
        game.client.hand_strength.ticker = 100;
        while peer.recv().is_ok() {}
        game.yaw = 37.0;
        assert!(game.remote_start_attack());
        let mut sent = Vec::new();
        while let Ok((id, bytes)) = peer.recv() {
            match play::Serverbound::decode(id, &bytes) {
                Ok(play::Serverbound::PlayerAction(a)) => sent.push(format!("action {}", a.action)),
                Ok(
                    play::Serverbound::MovePlayerPos(_)
                    | play::Serverbound::MovePlayerPosRot(_)
                    | play::Serverbound::MovePlayerRot(_),
                ) => sent.push("move".into()),
                _ => {}
            }
        }
        assert_eq!(sent, ["action 8"]);
    });
}
