use std::sync::{Arc, Mutex};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

/// Halves the player's visibility, and logs what it was asked.
struct Halve(Arc<Mutex<Vec<(u32, Option<u32>, f64)>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Halve {
    fn living_visibility(&self, _: &dyn mod_api::Host, event: &mut mod_api::LivingVisibilityEvent) {
        let looking = event.looking.map(|e| e.id());
        self.0
            .lock()
            .unwrap()
            .push((event.entity.entity().id(), looking, event.visibility));
        event.visibility *= 0.5;
    }
}

/// `LivingVisibilityEvent` for a guest a mob targets: the mod gets the
/// guest, the mob and vanilla's percent, and its answer is the percent.
#[test]
#[ignore = "daily"]
fn a_mod_halves_a_guests_visibility() {
    let heard = Arc::default();
    let mods = vec![mod_api::ModEntry::new(
        "halve",
        Box::new(Halve(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            let mut seen = guest(5);
            seen.loaded = true;
            game.server.lan.as_mut().unwrap().hub.guests.push(seen);
            let view = game.lan_guest_views()[0].1.clone();
            let world = game.mob_world_base(&[]);
            assert!(world.mob_hooks.is_some(), "asked only while a mod listens");
            assert_eq!(world.player_visibility(&view, 900, 0.8), 0.4);
            assert_eq!(*heard.lock().unwrap(), [(24, Some(900), 0.8)]);
        });
    });
}

struct NamePlayer(Arc<Mutex<Vec<Option<u128>>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for NamePlayer {
    fn living_visibility(
        &self,
        host: &dyn mod_api::Host,
        event: &mut mod_api::LivingVisibilityEvent,
    ) {
        let player = event.entity.player(host).map(|p| p.uuid());
        self.0.lock().unwrap().push(player);
    }
}

/// A `living_visibility` listener names the guest it is asked about by its
/// UUID, under the hook's read-only host (#5944), and an entity that is no player as none;
/// then the host player, acting and in place, by its UUID.
#[test]
#[ignore = "daily"]
fn a_mod_names_the_guest_whose_visibility_it_is_asked() {
    let heard = Arc::default();
    let mods = vec![mod_api::ModEntry::new(
        "name_player",
        Box::new(NamePlayer(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            let mut seen = guest(5);
            seen.loaded = true;
            let uuid = seen.uuid;
            game.server.lan.as_mut().unwrap().hub.guests.push(seen);
            let view = game.lan_guest_views()[0].1.clone();
            let world = game.mob_world_base(&[]);
            world.player_visibility(&view, 900, 0.8);
            world
                .mob_hooks
                .unwrap()
                .living_visibility_mob(900, 901, 1.0);
            assert_eq!(*heard.lock().unwrap(), [Some(uuid), None]);
            assert_eq!(uuid, 127, "guest 5's fixture UUID");
            // The host player, first acting, then in place with no turn running.
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let host = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.scaled_visibility(|epoch| mod_api::EntityRef::new(epoch, 20), 900, 1.0);
                heard.lock().unwrap().pop().unwrap()
            };
            assert_eq!(host(game), Some(123), "the acting host player");
            // In place, the host is found as `host.player` finds it: while the world is served.
            (game.server.lan_acting_uuid, game.server.lan_acting_entity) = (None, None);
            game.server.serving =
                Some(sunrise_gameplay::game::server_side::integrated::Serving::fixture(0));
            assert_eq!(host(game), Some(123), "the host player in place");
        });
    });
}
