//! `PlayerRef::container_counter` and `PlayerRef::next_container_counter`
//! (`ServerPlayer.containerCounter` and `nextContainerCounter`), and `MenuRef::close_container`
//! inside the player's own turn (`ServerPlayer.closeContainer`, which vanilla's `openMenu`
//! calls from inside the player's use action), through the handles a mod calls.

use mod_api::{EntityRef, Host, MenuReplace, ServerHost, WorldEpoch};
use std::sync::{Arc, mpsc};
use sunrise_gameplay_common::game::server_side::native_menus::tests::sent;
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_sim::lan::{Out, PlayerSender};
use {
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

/// Guest 1 (UUID 123) on a world open to LAN, and what their client is sent.
fn watched_guest(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> mpsc::Receiver<Out> {
    open_to_lan(game);
    let (tx, rx) = mpsc::channel();
    let mut player = guest(1);
    player.sender = PlayerSender::new(tx, Arc::default());
    game.server.lan.as_mut().unwrap().hub.guests.push(player);
    rx
}

/// A one-row chest display for UUID 123 (`PlayerRef::open_chest_menu`), replacing the open menu: its window.
fn open(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    let player = game.player(123).unwrap();
    let host: &mut dyn ServerHost = game;
    let menu = player
        .open_chest_menu(
            host,
            1,
            mod_api::Text::literal("chest"),
            Vec::new(),
            MenuReplace::CloseFirst,
        )
        .unwrap();
    menu.container_id(host).unwrap()
}

/// The counter is the window the last open took; an advance moves it as an open does
/// (`counter % 100 + 1`, so 100 wraps to 1) without opening or closing anything, and the next
/// open takes the id after it.
#[test]
#[ignore = "daily"]
fn the_container_counter_is_the_last_window_and_advances_as_an_open_does() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_menus::container_counter_tests::the_container_counter_is_the_last_window_and_advances_as_an_open_does",
        || {
            with_game(|game| {
                let rx = watched_guest(game);
                let player = game.player(123).unwrap();
                let counter = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                    player.container_counter(game as &dyn Host)
                };
                assert_eq!(counter(game), Some(0), "no menu has opened yet");
                let first = open(game);
                assert_eq!((first, counter(game)), (1, Some(1)));
                sent(&rx);

                let host: &mut dyn ServerHost = game;
                assert_eq!(player.next_container_counter(host), Some(2));
                assert_eq!(
                    player
                        .container_menu(host)
                        .and_then(|menu| menu.container_id(host)),
                    Some(first),
                    "the open menu keeps its window"
                );
                assert!(sent(&rx).is_empty(), "an advance sends nothing");
                assert_eq!(counter(game), Some(2));
                assert_eq!(
                    open(game),
                    3,
                    "the next open takes the id after the advance"
                );

                let host: &mut dyn ServerHost = game;
                let ids: Vec<i32> = (0..98)
                    .map(|_| player.next_container_counter(host).unwrap())
                    .collect();
                assert_eq!(ids, (4..=100).chain([1]).collect::<Vec<_>>());
                assert_eq!(open(game), 2);

                let stale = mod_api::__private::game::player_ref(
                    EntityRef::new(
                        WorldEpoch::new(player.entity().epoch().get() + 1),
                        player.entity().id(),
                    ),
                    player.uuid(),
                );
                let host: &mut dyn ServerHost = game;
                assert_eq!(stale.container_counter(host), None);
                assert_eq!(stale.next_container_counter(host), None);
                assert_eq!(counter(game), Some(2), "a stale handle advances nothing");
            });
        },
    );
}

/// A close inside the player's own turn happens at the call, as vanilla's `openMenu` closes the
/// open menu from inside the player's use action: the client is told, the inventory menu is the
/// open one at once, and the cursor goes back to the player's own inventory.
#[test]
#[ignore = "daily"]
fn a_menu_closes_at_the_call_inside_its_players_own_turn() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_menus::container_counter_tests::a_menu_closes_at_the_call_inside_its_players_own_turn",
        || {
            with_game(|game| {
                let rx = watched_guest(game);
                let window = open(game);
                let stone = ItemStack::new("minecraft:stone", 4);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.carried = Some(stone.clone()));
                sent(&rx);
                let player = game.player(123).unwrap();
                let in_turn = game
                    .as_guest(1, |game| {
                        let host: &mut dyn ServerHost = game;
                        let menu = player.container_menu(host).unwrap();
                        let closed = menu.close_container(host);
                        let now = player
                            .container_menu(host)
                            .and_then(|menu| menu.container_id(host));
                        (closed, now, sent(&rx))
                    })
                    .unwrap();
                let (closed, now, packets) = in_turn;
                assert!(closed, "the player's own turn does not refuse the close");
                assert_eq!(now, Some(0), "the inventory menu is open at once");
                assert!(
                    packets
                        .iter()
                        .any(|p| matches!(p, C::ContainerClose(c) if c.container == window)),
                    "the client is told at the call: {packets:?}"
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert!(guest.menu.is_none());
                assert_eq!(guest.carried, None);
                assert!(
                    guest.inventory.iter().flatten().any(|s| *s == stone),
                    "the cursor is back in the player's own inventory"
                );
            });
        },
    );
}

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
