#![cfg(test)]
use sunrise_gameplay_common::game::stations_common::Kind as StationKind;
use sunrise_gameplay_common::game::stations_common::Station;
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play;

#[cfg(test)]
pub(crate) mod anvil_tests {
    use super::*;
    use mod_api::item::menu::AnvilCraftInputs;
    use mod_api::{
        AnvilCraftPostEvent, AnvilCraftPreEvent, AnvilUpdateEvent, Flow, Hooks, HostSide, ModEntry,
        ServerHost,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize, Ordering},
    };
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;

    struct Observe(Arc<AtomicUsize>, Arc<AtomicI32>);

    impl Observe {
        fn check_inputs(&self, phase: usize, inputs: &AnvilCraftInputs<'_>) {
            let stack = |s: &mod_api::ItemStack| {
                (
                    mod_api::__private::item_key_id(s.item()),
                    s.count(),
                    s.damage(),
                )
            };
            assert_eq!(
                inputs.left.map(stack),
                Some((Some("minecraft:iron_sword"), 1, 80))
            );
            assert_eq!(
                inputs.right.map(stack),
                Some((Some("minecraft:iron_ingot"), 2, 0))
            );
            assert_eq!(stack(inputs.output), (Some("minecraft:iron_sword"), 1, 0));
            assert_eq!(self.0.load(Ordering::Relaxed), phase);
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn anvil_craft_pre(
            &self,
            _: &mut dyn ServerHost,
            event: &mut AnvilCraftPreEvent<'_>,
        ) -> Flow {
            self.check_inputs(0, event.inputs);
            Flow::Continue
        }

        fn anvil_update(&self, side: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
            if let HostSide::Server(host) = side
                && self.0.load(Ordering::Relaxed) == 1
                && event.inputs.right.is_none()
                && let Some(xp) = host
                    .player(event.player)
                    .and_then(|player| player.experience(host))
            {
                self.1.store(xp.level, Ordering::Relaxed);
            }
            Flow::Continue
        }

        fn anvil_craft_post(&self, _: &mut dyn ServerHost, event: &mut AnvilCraftPostEvent<'_>) {
            self.check_inputs(1, event.inputs);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn anvil_pre_and_post_read_pre_take_inputs() {
        const CONN: u32 = 12;
        const POS: [i32; 3] = [1, 64, 1];
        let calls = Arc::new(AtomicUsize::new(0));
        let during_update = Arc::new(AtomicI32::new(-1));
        let mods = vec![ModEntry::new(
            "anvil_observer",
            Box::new(Observe(calls.clone(), during_update.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let [x, y, z] = POS;
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:anvil")
                    .unwrap();
                game.world.set(x, y, z, state).unwrap();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(CONN, |guest| guest.xp_level = 10)
                    .unwrap();
                assert!(game.lan_open_station(CONN, &Station::new(StationKind::Anvil, POS)));
                assert!(game.lan_station_op(CONN, None, |menu, env| {
                    let mut sword = ItemStack::new("minecraft:iron_sword", 1);
                    sword.damage = 80;
                    menu.set_slot(env, 0, Some(sword));
                    menu.set_slot(env, 1, Some(ItemStack::new("minecraft:iron_ingot", 2)));
                }));
                let window = game.server.lan.as_ref().unwrap().menus[&CONN]
                    .station
                    .as_ref()
                    .unwrap()
                    .window;
                game.lan_guest_packet(
                    CONN,
                    play::Serverbound::ContainerClick(play::ContainerClick {
                        container: window,
                        state: 0,
                        slot: 2,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    }),
                );

                let host = game.server.lan.as_ref().unwrap();
                let menu = &host.menus[&CONN].station.as_ref().unwrap().menu;
                assert!((0..=2).all(|slot| menu.slot(slot).is_none()));
                let guest = &host.hub.guests[0];
                let carried = guest.carried.as_ref().expect("the guest took the output");
                assert_eq!(
                    (&*carried.item, carried.count, carried.damage),
                    ("minecraft:iron_sword", 1, 0)
                );
                assert_eq!(guest.xp_level, 8);
                assert_eq!(
                    during_update.load(Ordering::Relaxed),
                    8,
                    "paid before the material update"
                );
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    2,
                    "Pre and Post both read the pre-take inputs"
                );
            });
        });
    }
}
