use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{ModEntry, Mods};
use sunrise_items::items::ItemStack;
use sunrise_sim::menu::Input;

pub(crate) mod item_part_tests {
    use super::*;
    use mod_api::item::menu::StackedOnOtherDirection;
    use sunrise_gameplay::game::stacked_on_other_tests::item_part_tests::GlobalAndPartMod;
    use sunrise_gameplay::game::stacked_on_other_tests::item_part_tests::PartScript;
    use sunrise_gameplay::game::stacked_on_other_tests::item_part_tests::observations;
    use sunrise_gameplay::game::stacked_on_other_tests::item_part_tests::set_part_script;
    use sunrise_gameplay::game::stacked_on_other_tests::item_part_tests::trace;

    #[test]
    #[ignore = "daily"]
    fn item_part_client_prediction_reconciliation_and_creative_pickups() {
        use sunrise_client::creative::Creative;
        use sunrise_gameplay_common::game::Screen;
        use sunrise_protocol::{
            item::Stack,
            packets::play::{self, Clientbound as C, Serverbound as S},
        };
        // Install BOTH a cancelling global listener AND ItemParts so we prove client prediction
        // and creative pickups run ItemParts with HostSide::Client while keeping the global
        // listener authoritative-server-only.
        let mods: Mods = vec![ModEntry::new(
            "client_part_pred",
            Box::new(GlobalAndPartMod(true)),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let (mut reader, _writer) = crate::game::remote::tests::connect_menu_test(game);
                crate::game::remote::tests::spawn_menu_test(game);
                let mut body = sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0),
                )
                .unwrap();
                body.entity_id = 7;
                game.player = Some(body);
                reader
                    .set_read_timeout(Some(std::time::Duration::from_millis(200)))
                    .unwrap();
                let drain_packets = |reader: &mut sunrise_protocol::connection::ConnReader| {
                    let mut out = Vec::new();
                    while let Ok((id, body)) = reader.recv() {
                        if let Ok(pkt) = S::decode(id, &body) {
                            out.push(pkt);
                        }
                    }
                    out
                };
                let _ = drain_packets(&mut reader);

                // 1. Ordinary client prediction on Screen::Inventory dispatches ItemPart with
                //    HostSide::Client (never global listener), predicts the edit locally, and
                //    sends ContainerClick with the predicted slots/carried.
                game.creative = false;
                game.client.screen = Screen::Inventory;
                game.inventory = vec![None; game.inventory.len()];
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:diamond", 2))),
                    ..PartScript::default()
                });
                assert!(game.menu_ops(&[(Some(9), 0, Input::Pickup)]));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(
                    !observations()[0].is_server,
                    "client prediction must pass HostSide::Client"
                );
                assert!(game.carried.is_none());
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 2))
                );
                let packets = drain_packets(&mut reader);
                let click_pkt = packets
                    .iter()
                    .find_map(|p| match p {
                        S::ContainerClick(c) => Some(c),
                        _ => None,
                    })
                    .expect("expected ContainerClick for client prediction");
                assert_eq!(
                    (click_pkt.container, click_pkt.slot, click_pkt.input),
                    (0, 9, 0)
                );
                assert!(click_pkt.carried.is_none());
                assert_eq!(click_pkt.changed.len(), 1);
                assert_eq!(click_pkt.changed[0].0, 9);
                assert_eq!(click_pkt.changed[0].1.as_ref().map(|h| h.count), Some(2));

                // Server reconciliation stays authoritative over client prediction.
                let item_id = |game: &sunrise_client_gameplay::game::ClientGame<'_>, name: &str| {
                    game.client
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:item")
                        .and_then(|r| r.id(name))
                        .unwrap()
                };
                let emerald = item_id(game, "minecraft:emerald");
                let stone = item_id(game, "minecraft:stone");
                game.handle_packet(C::ContainerSetSlot(play::ContainerSetSlot {
                    container: 0,
                    state: 5,
                    slot: 9,
                    item: Some(Stack::new(emerald, 1)),
                }))
                .unwrap();
                game.handle_packet(C::SetCursorItem(play::SetCursorItem {
                    item: Some(Stack::new(stone, 3)),
                }))
                .unwrap();
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:emerald", 1))
                );
                assert_eq!(
                    game.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:stone", 3))
                );

                // 2. Client prediction for a 1-slot left drag resolves to a pickup and runs Part.
                game.inventory[10] = None;
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:diamond", 1))),
                    ..PartScript::default()
                });
                assert!(game.menu_ops(&[
                    (None, 0, Input::QuickCraft),
                    (Some(10), 1, Input::QuickCraft),
                    (None, 2, Input::QuickCraft),
                ]));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(!observations()[0].is_server);
                assert_eq!(
                    game.inventory[10].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 1))
                );
                let drag_clicks = drain_packets(&mut reader)
                    .into_iter()
                    .filter(|p| matches!(p, S::ContainerClick(_)))
                    .count();
                assert_eq!(drag_clicks, 3);

                // 2b. Multi-gesture batch on a remote inventory carries predicted `craft_result`
                //     from gesture 1 to gesture 2 so gesture 2 does not see a cleared result.
                game.client.remote.as_mut().unwrap().craft_result =
                    Some(ItemStack::new("minecraft:flint", 1));
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_clicked: Some(None),
                    ..PartScript::default()
                });
                assert!(game.remote_menu_ops(&[
                    (Some(200), 0, Input::Pickup),
                    (Some(200), 0, Input::Pickup),
                ]));
                assert_eq!(
                    observations().len(),
                    2,
                    "both gestures in the batch must dispatch CarriedOnOther"
                );
                assert_eq!(
                    observations()[0].clicked,
                    Some(("minecraft:flint".to_owned(), 1))
                );
                assert_eq!(
                    observations()[1].clicked,
                    None,
                    "second gesture in remote batch must observe updated craft_result"
                );
                assert_eq!(game.client.remote.as_ref().unwrap().craft_result, None);
                let _ = drain_packets(&mut reader);

                // 3. Creative Survival Inventory tab pickup runs ItemPart on HostSide::Client and
                //    sends SetCreativeModeSlot (never ContainerClick).
                game.creative = true;
                game.client.screen = Screen::Creative;
                game.refresh_creative_tabs();
                let inv_tab = game
                    .client
                    .creative_screen
                    .0
                    .tabs
                    .iter()
                    .position(|t| t.inventory)
                    .expect("creative inventory tab");
                game.client.creative_screen.0.select(inv_tab);
                assert!(game.creative_inventory_open());
                game.inventory[9] = Some(ItemStack::new("minecraft:flint", 1));
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:diamond", 4))),
                    ..PartScript::default()
                });
                assert!(game.click_open_inventory(Some(9), 0, Input::Pickup));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(!observations()[0].is_server);
                assert_eq!(
                    game.inventory[9].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 4))
                );
                let creative_pkts = drain_packets(&mut reader);
                assert!(
                    creative_pkts
                        .iter()
                        .any(|p| matches!(p, S::SetCreativeModeSlot(_))),
                    "creative inventory tab pickup must sync changed slots via SetCreativeModeSlot"
                );
                assert!(
                    !creative_pkts
                        .iter()
                        .any(|p| matches!(p, S::ContainerClick(_))),
                    "creative inventory tab pickup must not send ContainerClick"
                );

                // 4. Creative item-tab hotbar pickup runs ItemPart on HostSide::Client and sends
                //    SetCreativeModeSlot; creative catalogue display slots do NOT dispatch Parts.
                let item_tab = game
                    .client
                    .creative_screen
                    .0
                    .tabs
                    .iter()
                    .position(|t| !t.inventory && !t.search && !t.items.is_empty())
                    .expect("creative item tab");
                game.client.creative_screen.0.select(item_tab);
                assert!(game.creative_tab_open());
                game.inventory[0] = Some(ItemStack::new("minecraft:flint", 1));
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    set_carried: Some(None),
                    set_clicked: Some(Some(("minecraft:emerald", 2))),
                    ..PartScript::default()
                });
                assert!(game.click_open_inventory(Some(0), 0, Input::Pickup));
                assert_eq!(
                    trace(),
                    [("stick", StackedOnOtherDirection::CarriedOnOther)]
                );
                assert!(!observations()[0].is_server);
                assert_eq!(
                    game.inventory[0].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:emerald", 2))
                );
                let hotbar_pkts = drain_packets(&mut reader);
                assert!(
                    hotbar_pkts
                        .iter()
                        .any(|p| matches!(p, S::SetCreativeModeSlot(_)))
                );
                assert!(
                    !hotbar_pkts
                        .iter()
                        .any(|p| matches!(p, S::ContainerClick(_)))
                );

                // Creative catalogue display slot must NEVER fire pickup overrides.
                game.carried = Some(ItemStack::new("minecraft:stick", 1));
                set_part_script(PartScript {
                    cancel_carried: true,
                    ..PartScript::default()
                });
                assert!(game.click_open_inventory(Some(Creative::DISPLAY), 0, Input::Pickup));
                assert!(
                    trace().is_empty(),
                    "creative catalogue display slots must not dispatch pickup overrides"
                );
            });
        });
    }
}
