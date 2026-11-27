//! A window joined to a server through memory, fed the server's packets as
//! it would receive them: what its native mods read of the entities and the
//! menu the server showed it.
#![cfg(feature = "client")]
use sunrise_protocol::packets::play::{self};

pub(crate) fn spawn(dimension: &str) -> play::SpawnInfo {
    play::SpawnInfo {
        dimension_type: 0,
        dimension: dimension.into(),
        seed: 1,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat: false,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

#[cfg(any())]
impl sunrise_client_gameplay::game::client_side::remote_client::session_state::Remote {
    /// Lists this client's own player with `latency`, as the server's player info update does.
    pub(crate) fn list_self(&mut self, latency: i32) {
        let mut me = sunrise_level::profile_lookup::TabPlayer::new(self.profile, "SessionReadTest");
        me.latency = latency;
        self.tab_players.insert(self.profile, me);
        self.tab_game_modes
            .entry(self.profile)
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicI32::new(0)));
    }
}

#[test]
#[ignore = "daily"]
fn client_base_tick_preserves_previous_eye_sample() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let _server = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(game);
            game.spawn_entity(15, "minecraft:zombie", [0.0; 3], [0.0; 3]);
            game.mobs
                .iter_mut()
                .find(|m| m.id == 15)
                .unwrap()
                .fluids
                .eye_in_water = true;
            game.tick_remote_entities(true);
            let m = game.mobs.iter_mut().find(|m| m.id == 15).unwrap();
            assert!(m.was_eye_in_water);
            assert!(!m.fluids.eye_in_water);
            game.tick_remote_entities(true);
            assert!(
                !game
                    .mobs
                    .iter()
                    .find(|m| m.id == 15)
                    .unwrap()
                    .was_eye_in_water
            );
        });
    });
}
