#![cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn;
    use sunrise_gameplay_common::game::server_side::native_merchant_refs::tests::offer;

    use sunrise_gameplay_common::game::TryIntoApi;
    use sunrise_gameplay_common::game::tables::GameItems;

    use sunrise_gameplay::game::server_side::lan_trading::tests::open_trade;

    use super::super::native_entity_refs::tests::with_game;

    use super::*;

    use sunrise_protocol::packets::play;

    use sunrise_sim::merchant;

    #[test]
    #[ignore = "daily"]
    fn changing_an_open_merchant_to_an_unknown_profession_closes_without_trading() {
        use sunrise_core::nbt::Tag;
        with_game(|game| {
            let (_villager, rx, mut offer) = open_trade(game, 1);
            offer.uses = 3;
            let window = {
                let gm = game
                    .server
                    .lan
                    .as_mut()
                    .unwrap()
                    .menus
                    .get_mut(&12)
                    .unwrap()
                    .merchant
                    .as_mut()
                    .unwrap();
                gm.merchant.session.offers[0] = offer.clone();
                gm.merchant.session.xp = 17;
                gm.slots[1] = Some(ItemStack::new("minecraft:coal", 2));
                gm.window
            };
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(ItemStack::new("minecraft:apple", 3))
            });
            let wheat = ItemStack::new("minecraft:wheat", 1);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .get_mut(&12)
                .unwrap()
                .merchant
                .as_mut()
                .unwrap()
                .slots[0] = Some(wheat);
            let mut tag = game
                .command_target_nbt(sunrise_gameplay_common::game::commands_common::Target::Mob(
                    40,
                ))
                .unwrap();
            if let Tag::Compound(data) = &mut tag {
                data.insert(
                    "VillagerData".into(),
                    Tag::compound([
                        ("type", Tag::String("minecraft:plains".into())),
                        ("profession", Tag::String("example:engineer".into())),
                        ("level", Tag::Int(4)),
                    ]),
                );
            }
            game.apply_entity_nbt(
                sunrise_gameplay_common::game::commands_common::Target::Mob(40),
                tag,
            )
            .unwrap();
            assert!(
                game.server.lan.as_ref().unwrap().menus[&12]
                    .merchant
                    .is_none()
            );
            game.lan_guest_packet(
                12,
                play::Serverbound::SelectTrade(play::SelectTrade { item: 0 }),
            );
            for input in [0, 1] {
                game.lan_guest_packet(
                    12,
                    play::Serverbound::ContainerClick(play::ContainerClick {
                        container: window,
                        state: 0,
                        slot: 2,
                        button: 0,
                        input,
                        changed: vec![],
                        carried: None,
                    }),
                );
            }
            assert_eq!(game.mobs[0].extra.profession.id(), "example:engineer");
            assert_eq!(game.mobs[0].extra.villager_level, 4);
            assert_eq!(game.mobs[0].extra.trade.as_ref().unwrap().xp, 17);
            assert!(rx.try_iter().any(|out| match out {
                sunrise_sim::lan::Out::Packet(body) => matches!(
                    play::Clientbound::decode(body[0] as i32, &body[1..]),
                    Ok(play::Clientbound::ContainerClose(_))
                ),
                _ => false,
            }));
            assert_eq!(game.server.missing_professions.len(), 1);
            let tag = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [0; 4],
                None,
                Some(game.pack.assets.registry),
                |_| None,
                &Default::default(),
            );
            game.apply_entity_nbt(
                sunrise_gameplay_common::game::commands_common::Target::Mob(40),
                tag,
            )
            .unwrap();
            assert_eq!(game.server.missing_professions.len(), 1);
            let second = spawn(game, 41, "minecraft:villager");
            let mut tag = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[1],
                [0; 4],
                None,
                Some(game.pack.assets.registry),
                |_| None,
                &Default::default(),
            );
            if let Tag::Compound(data) = &mut tag {
                data.insert(
                    "VillagerData".into(),
                    Tag::compound([
                        ("type", Tag::String("minecraft:plains".into())),
                        ("profession", Tag::String("example:engineer".into())),
                        ("level", Tag::Int(1)),
                    ]),
                );
            }
            game.apply_entity_nbt(
                sunrise_gameplay_common::game::commands_common::Target::Mob(second.entity().id()),
                tag.clone(),
            )
            .unwrap();
            assert_eq!(game.server.missing_professions.len(), 1);
            game.server.missing_professions.clear();
            game.apply_entity_nbt(
                sunrise_gameplay_common::game::commands_common::Target::Mob(second.entity().id()),
                tag,
            )
            .unwrap();
            assert_eq!(game.server.missing_professions.len(), 1);
            assert_eq!(
                game.mobs[0].extra.trade.as_ref().unwrap().offers,
                vec![offer]
            );
            let inventory = &game.server.lan.as_ref().unwrap().hub.guests[0].inventory;
            for (id, count) in [
                ("minecraft:wheat", 1),
                ("minecraft:coal", 2),
                ("minecraft:apple", 3),
            ] {
                assert_eq!(
                    inventory
                        .iter()
                        .flatten()
                        .filter(|s| &*s.item == id)
                        .map(|s| s.count)
                        .sum::<u32>(),
                    count
                );
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn explicit_nbt_offers_and_xp_override_the_live_trade() {
        use sunrise_core::nbt::Tag;
        with_game(|game| {
            let (_, _, _) = open_trade(game, 1);
            let session = &mut game
                .server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .get_mut(&12)
                .unwrap()
                .merchant
                .as_mut()
                .unwrap()
                .merchant
                .session;
            session.offers[0].uses = 3;
            session.xp = 17;
            let target = sunrise_gameplay_common::game::commands_common::Target::Mob(40);
            let old = game.command_target_nbt(target).unwrap();
            let replacement = offer("minecraft:coal", "minecraft:diamond");
            let patch = Tag::compound([
                (
                    "VillagerData",
                    Tag::compound([
                        ("profession", Tag::String("example:engineer".into())),
                        ("level", Tag::Int(4)),
                    ]),
                ),
                (
                    "Offers",
                    sunrise_sim::entity_nbt::offers_to_nbt(
                        std::slice::from_ref(&replacement),
                        &Default::default(),
                    ),
                ),
                ("Xp", Tag::Int(9)),
            ]);
            game.apply_entity_nbt(target, sunrise_sim::data_cmd::merged(&old, &patch))
                .unwrap();
            let trade = game.mobs[0].extra.trade.as_ref().unwrap();
            assert_eq!(trade.offers, vec![replacement]);
            assert_eq!(trade.xp, 9);
            assert_eq!(game.mobs[0].extra.villager_level, 4);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_merchant_edit_reaches_the_guest_trading_with_it() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::lan_trading::tests::a_merchant_edit_reaches_the_guest_trading_with_it",
            "SUNRISE_MERCHANT_EDIT_CHILD",
            &["game"],
        ) {
            return;
        }
        with_game(|game| {
            let (villager, rx, first) = open_trade(game, 1);

            // MerchantOffers packets (id 53) the guest was sent since last asked.
            let sent_offers = || {
                let mut n = 0;
                while let Ok(out) = rx.try_recv() {
                    let bodies = match out {
                        sunrise_sim::lan::Out::Packet(body) => vec![body],
                        sunrise_sim::lan::Out::Packets(bodies) => bodies,
                        _ => Vec::new(),
                    };
                    n += bodies.iter().filter(|b| b.first() == Some(&53)).count();
                }
                n
            };
            sent_offers();
            let player = villager.trading_player(game).unwrap();
            assert_eq!(player.uuid(), 12);
            let second = offer("minecraft:coal", "minecraft:diamond");
            assert!(
                villager
                    .add_offer(game, second.try_into_api().unwrap())
                    .unwrap()
            );
            assert_eq!(sent_offers(), 1);
            let mut changed = first.clone();
            changed.uses = 9;
            assert!(
                villager
                    .set_offer(game, 0, changed.try_into_api().unwrap())
                    .unwrap()
            );
            assert!(villager.set_xp(game, 40));
            assert!(villager.set_level(game, 3));
            // The profession cannot change under an open screen.
            assert!(!villager.set_profession(game, "minecraft:cleric"));
            assert!(
                !villager
                    .set_offer(game, 7, changed.try_into_api().unwrap())
                    .unwrap()
            );
            assert_eq!(sent_offers(), 3);
            // The mob's offers are the screen's (the brain restocks them).
            let live = vec![changed.clone(), second.clone()];
            assert_eq!(game.mobs[0].extra.trade.as_ref().unwrap().offers, live);
            assert_eq!(villager.offers(game), Some(live.try_into_api().unwrap()));
            assert_eq!(villager.xp(game), Some(40));
            assert_eq!(villager.level(game), Some(3));
            {
                let gm = game.server.lan.as_ref().unwrap().menus[&12]
                    .merchant
                    .as_ref()
                    .unwrap();
                assert_eq!(gm.sent, live);
                assert_eq!(gm.sent_level, 3);
                assert_eq!(gm.merchant.session.offers, live);
            }
            assert!(villager.remove_offer(game, 1));
            let gm = game.server.lan.as_ref().unwrap().menus[&12]
                .merchant
                .as_ref()
                .unwrap();
            assert_eq!(gm.sent, vec![changed.clone()]);
            assert_eq!(sent_offers(), 1);

            // Closing the screen gives the mob the edited trades.
            let mut gm = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .get_mut(&12)
                .unwrap()
                .merchant
                .take()
                .unwrap();
            game.merchant_closed(&mut gm.merchant);
            let trade = game.mobs[0].extra.trade.as_ref().unwrap();
            assert_eq!((trade.offers.clone(), trade.xp), (vec![changed], 40));
            assert_eq!(game.mobs[0].extra.villager_level, 3);
            assert_eq!(villager.trading_player(game), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_lowered_level_still_levels_up_with_the_next_trade_set() {
        with_game(|game| {
            let (villager, _rx, first) = open_trade(game, 3);
            let before = game.lan_trade_session(12).unwrap().sequences.len();
            assert_eq!(before, 2, "levels 4 and 5 were lent");
            assert!(villager.set_level(game, 1));
            assert!(villager.set_xp(game, 5));
            assert_eq!(game.lan_trade_session(12).unwrap().sequences.len(), 4);
            // The trade makes 12 experience: past level 1's 10, so level 2 is rolled.
            let mut gm = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .get_mut(&12)
                .unwrap()
                .merchant
                .take()
                .unwrap();
            let items = game.items();
            let trade_env = merchant::TradeEnv {
                trades: &game.stations().trades,
                enchantments: &game.stations().enchantments,
                maps: None,
            };
            let session = &mut gm.merchant.session;
            session.origin = [0.0; 3];
            assert!(session.notify_trade(0, Some(&trade_env), items));
            assert_eq!(session.level, 2);
            assert!(session.offers.len() > 1, "level 2 offers were added");
            assert_eq!(session.offers[0].uses, first.uses + 1);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .menus
                .get_mut(&12)
                .unwrap()
                .merchant
                .restore(Some(gm));
            // Level 7: nothing more to lend, and no further level-up.
            assert!(villager.set_level(game, 7));
            assert_eq!(villager.level(game), Some(7));
            assert!(game.lan_trade_session(12).unwrap().sequences.is_empty());
            assert!(villager.set_level(game, i32::MAX));
            assert_eq!(villager.level(game), Some(i32::MAX));
            assert!(game.lan_trade_session(12).unwrap().sequences.is_empty());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_type_change_reaches_the_open_session_before_a_level_up() {
        with_game(|game| {
            let (villager, _rx, _) = open_trade(game, 4);
            assert!(villager.set_villager_type(game, "minecraft:swamp"));
            assert_eq!(
                game.lan_trade_session(12).unwrap().variant,
                "minecraft:swamp"
            );
            assert_eq!(game.mobs[0].extra.variant, 5);
            assert_eq!(
                villager.villager_type(game).as_deref(),
                Some("minecraft:swamp")
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn a_trade_hook_fired_by_a_real_select_can_write_the_open_trade() {
        use mod_api::{self, Hooks, TradeSelectEvent};
        struct Observer(Seen);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn trade_select(&self, host: &mut dyn ServerHost, event: &mut TradeSelectEvent<'_>) {
                let merchant = event
                    .villager
                    .and_then(|id| host.entity(id))
                    .and_then(|entity| host.merchant(entity));
                let wrote = merchant.is_some_and(|m| {
                    m.add_offer(
                        host,
                        offer("minecraft:wheat", "minecraft:emerald")
                            .try_into_api()
                            .unwrap(),
                    )
                    .unwrap()
                });
                let player = merchant.is_some_and(|m| m.trading_player(host).is_some());
                self.0
                    .lock()
                    .unwrap()
                    .push((event.villager, event.index, wrote, player));
            }
        }
        let seen = Seen::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "merchant-observer",
            Box::new(Observer(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (villager, _rx, first) = open_trade(game, 1);
                let second = offer("minecraft:coal", "minecraft:diamond");
                assert!(
                    villager
                        .add_offer(game, second.try_into_api().unwrap())
                        .unwrap()
                );
                game.lan_select_trade(12, 1);
                // The selected menu is restored before the hook, so its live
                // session and trading player are readable and writable there.
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 1);
                assert_eq!(seen[0], (Some(40), 1, true, true));
                assert_eq!(
                    villager.offers(game),
                    Some(
                        vec![first, second, offer("minecraft:wheat", "minecraft:emerald")]
                            .try_into_api()
                            .unwrap()
                    )
                );
            });
        });
    }
}
