#![cfg(test)]
#[cfg(test)]
pub(crate) mod select_on_a_server_tests {

    /// Picking a trade on a server moves payment locally, sends SelectTrade, and leaves effects to server.
    #[test]
    #[ignore = "daily"]
    fn picking_a_trade_on_a_server_moves_the_payment_here_too() {
        use crate::game::server_side::native_entity_refs::tests::with_client_game;
        use sunrise_items::items::ItemStack;
        use sunrise_protocol::packets::play::Serverbound as S;
        use sunrise_sim::merchant::{ItemCost, MerchantOffer};
        with_client_game(|game| {
            let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
            peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                .unwrap();
            game.remote_open_merchant(1, String::new(), sunrise_game_data::text::Text::literal(""));
            let offer = MerchantOffer {
                base_cost_a: ItemCost {
                    stack: ItemStack::new("minecraft:emerald", 1),
                },
                cost_b: None,
                result: ItemStack::new("minecraft:bread", 6),
                uses: 0,
                max_uses: 12,
                reward_exp: true,
                special_price_diff: 0,
                demand: 0,
                price_multiplier: 0.05,
                xp: 1,
            };
            if let Some(m) = game.station.as_mut().and_then(|s| s.merchant.as_mut()) {
                m.session.offers = vec![offer];
            }
            game.inventory[0] = Some(ItemStack::new("minecraft:emerald", 5));
            while peer.recv().is_ok() {}
            game.try_move_items(0);
            let mut payment = None;
            game.menu_session(|menu, _, _| {
                payment = menu.slot(0).map(|s| (s.item.to_string(), s.count));
                Some(false)
            });
            assert_eq!(payment, Some(("minecraft:emerald".to_owned(), 5)));
            let mut picked = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                if let Ok(S::SelectTrade(p)) = S::decode(id, &bytes) {
                    picked.push(p.item);
                }
            }
            assert_eq!(picked, [0]);
            // Shift-clicking result predicts the move while merchant effects stay on the server.
            let traded = game.stats.get("custom", "minecraft:traded_with_villager");
            game.remote_menu_ops(&[(
                Some(sunrise_gameplay_common::game::stations_common::RESULT),
                0,
                sunrise_sim::menu::Input::QuickMove,
            )]);
            let mut changed = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                if let Ok(S::ContainerClick(p)) = S::decode(id, &bytes) {
                    changed.extend(
                        p.changed
                            .iter()
                            .map(|(slot, stack)| (*slot, stack.is_some())),
                    );
                }
            }
            changed.sort();
            assert_eq!(changed, [(0, false), (2, false), (38, true)]);
            assert!(game.orbs.is_empty());
            assert_eq!(
                game.stats.get("custom", "minecraft:traded_with_villager"),
                traded
            );
        });
    }
}
