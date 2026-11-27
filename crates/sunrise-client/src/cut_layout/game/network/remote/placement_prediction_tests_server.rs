#![cfg(feature = "client")]
use crate::game::remote::placement_prediction_tests::top_of;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::first_person_sim::Hand;
use sunrise_sim::motion::PlayerMotion;

#[test]
#[ignore = "daily"]
fn an_active_vault_ends_a_held_item_s_click_on_the_client() {
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        let (mut peer, _writer) = super::tests::connect_menu_test(game);
        peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
            .unwrap();
        game.set_local_mode(sunrise_command::args::GameMode::Survival);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let hit = top_of(0, 0);
        game.set_test_forced_hit(Some(hit));
        let registry = game.pack.assets.registry;
        let vault = |state: &str| {
            registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:vault".into(),
                    properties: [
                        ("facing", "north"),
                        ("ominous", "false"),
                        ("vault_state", state),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap()
        };
        // VaultBlock.useItemOn: SUCCESS_SERVER for any held item on an ACTIVE vault, so the
        // click ends with UseItemOn and the hand swings only when the server says so.
        game.world.set(0, 64, 0, vault("active")).unwrap();
        for (hand, expected) in [
            (Hand::Main, &[(0, 1)][..]),
            (Hand::Off, &[(0, 1), (1, 2)][..]),
        ] {
            for item in ["minecraft:trial_key", "minecraft:stick"] {
                game.common.inventory[game.common.selected] =
                    (hand == Hand::Main).then(|| ItemStack::new(item, 1));
                game.offhand = (hand == Hand::Off).then(|| ItemStack::new(item, 1));
                let sequence = game.client.remote.as_ref().unwrap().sequence;
                let swings = game.swings_started;
                game.remote_use(mod_api::InteractionAttempt::Press);
                let mut uses = Vec::new();
                while let Ok((id, body)) = peer.recv() {
                    match S::decode(id, &body).unwrap() {
                        S::UseItemOn(packet) => {
                            assert_eq!(packet.pos, hit.pos);
                            uses.push((packet.hand, packet.sequence - sequence));
                        }
                        S::UseItem(_) => panic!("unexpected UseItem for {hand:?} {item}"),
                        _ => {}
                    }
                }
                assert_eq!(uses.as_slice(), expected, "{hand:?} {item}");
                assert_eq!(
                    game.client.remote.as_ref().unwrap().sequence - sequence,
                    expected.len() as i32
                );
                assert_eq!(game.swings_started, swings, "{hand:?} {item}");
            }
        }
        // TRY_WITH_EMPTY_HAND otherwise: the vault has no useWithoutItem, so the click goes on.
        let (empty, _) = game.remote_block_reacts(&hit, None, true);
        assert!(!empty.consumes_action());
        game.world.set(0, 64, 0, vault("unlocking")).unwrap();
        let key = ItemStack::new("minecraft:trial_key", 1);
        let (unlocking, _) = game.remote_block_reacts(&hit, Some(&key), true);
        assert!(!unlocking.consumes_action());
    });
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
