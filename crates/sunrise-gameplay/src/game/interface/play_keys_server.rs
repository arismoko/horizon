impl crate::game::ServerGame<'_> {
    /// handlePlayerAction SWAP_ITEM_WITH_OFFHAND for the window's own player
    /// with no server: the mods hear it as a server's players
    /// (`player_swap_hands`), and may stop it or change what each hand gets.
    /// Then setItemInHand for the off hand, then the main hand. The stacks are
    /// copied only for a listener: `heard` holds the old main and off hands.
    #[allow(dead_code)]
    pub fn own_swap_hands(&mut self) {
        if self.common.spectator() {
            return;
        }
        let heard = mod_api::__private::listened(mod_api::HookId::PlayerSwapHands).then(|| {
            (
                self.common.inventory[self.common.selected].clone(),
                self.common.offhand.clone(),
            )
        });
        if let Some((old_main, old_off)) = &heard {
            let (mut to_main, mut to_off, mut cancel) = (old_off.clone(), old_main.clone(), false);
            let (uuid, name) = (self.own_uuid(), self.own_profile_name());
            self.native_player_swap_hands(uuid, &name, &mut to_main, &mut to_off, &mut cancel);
            if cancel {
                return;
            }
            (
                self.common.inventory[self.common.selected],
                self.common.offhand,
            ) = (to_main, to_off);
        } else {
            std::mem::swap(
                &mut self.common.offhand,
                &mut self.common.inventory[self.common.selected],
            );
        }
        let (offhand, main) = (
            self.common.offhand.clone(),
            self.common.inventory[self.common.selected].clone(),
        );
        let (old_main, old_off) = match &heard {
            Some((old_main, old_off)) => (old_main.as_ref(), old_off.as_ref()),
            None => (offhand.as_ref(), main.as_ref()),
        };
        self.player_on_equip("offhand", old_off, offhand.as_ref());
        self.player_on_equip("mainhand", old_main, main.as_ref());
        // handlePlayerAction ends with stopUsingItem.
        if self.common.using.is_some() {
            self.stop_active_item_use();
        }
    }
}

#[cfg(test)]
#[cfg(any())]
pub(in crate::game) mod swap_hands_tests {
    use crate::game::client_side::play_keys::swap_hands_tests::Torch;
    use crate::game::native_entity_refs::tests::with_game;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use sunrise_items::items::ItemStack;

    /// The swap through a LAN host's registered connection: the packet goes
    /// through the hub's inbox and the game's packet pass
    /// (`tick_lan_packets`), and what the guest's client is sent comes off
    /// its sender. The hook is heard once a press; a cancel sends the client
    /// both hands as they were, and a listener's stack reaches its off-hand
    /// slot.
    #[test]
    #[ignore = "daily"]
    fn a_guest_swap_through_its_connection_is_heard_once_and_sends_the_hands() {
        use crate::game::play_ping_tests::{fixture, output};
        use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
        let (cancel, calls) = (
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicUsize::new(0)),
        );
        let torch = Torch(Arc::clone(&cancel), Arc::clone(&calls));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("torch", Box::new(torch))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let (mut game, receivers) = fixture(&mut resources);
            let selected = game.side.lan.as_ref().unwrap().hub.guests[0].selected;
            game.side.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
                g.offhand = None;
            });
            // One press through the connection's inbox; the hands (main, off)
            // in the last inventory the client was sent.
            let press = |game: &mut crate::game::ServerGame<'_>| {
                let swap = S::PlayerAction(play::PlayerAction {
                    action: 7,
                    pos: [0; 3],
                    direction: 0,
                    sequence: 0,
                });
                let host = game.server.lan.as_mut().unwrap();
                host.hub
                    .guests
                    .update_connection(1, |g| {
                        g.inbox
                            .push_back(sunrise_sim::lan::guest::InboxItem::Packet(swap, None))
                    })
                    .unwrap();
                game.server.level_pass.first = false;
                game.server.level_pass.packets_drained = false;
                game.tick_lan_packets();
                let hub = &game.server.lan.as_ref().unwrap().hub;
                sunrise_sim::lan::flush_all(hub.guests.iter().map(|g| &g.sender));
                output(&receivers[0])
                    .into_iter()
                    .filter_map(|p| match p {
                        C::ContainerSetContent(c) if c.container == 0 => {
                            Some((c.items[36 + selected].clone(), c.items[45].clone()))
                        }
                        _ => None,
                    })
                    .next_back()
                    .expect("the client was sent its inventory")
            };
            let hub = &mut game.side.lan.as_mut().unwrap().hub;
            let sword = hub.stack(&Some(ItemStack::new("minecraft:diamond_sword", 1)));
            let torch = hub.stack(&Some(ItemStack::new("minecraft:torch", 1)));
            assert!(sword.is_some() && torch.is_some());
            // Cancelled: both hands as they were.
            assert_eq!(press(&mut game), (sword.clone(), None));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            let g = &game.side.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(
                g.inventory[selected].as_ref().map(|s| &*s.item),
                Some("minecraft:diamond_sword")
            );
            assert!(g.offhand.is_none());
            // A listener's torch for the off hand: the sword is gone and the
            // client is sent the torch.
            cancel.store(false, Ordering::SeqCst);
            assert_eq!(press(&mut game), (None, torch));
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            let g = &game.side.lan.as_ref().unwrap().hub.guests[0];
            assert!(g.inventory[selected].is_none());
            assert_eq!(
                g.offhand.as_ref().map(|s| &*s.item),
                Some("minecraft:torch")
            );
        });
    }
}

use sunrise_gameplay_common::game::player_equip::PlayerEquipGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
