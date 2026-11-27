#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::{Hooks, ModEntry, ServerHost, XpOrbTargetingEvent};
    use std::sync::{Arc, Mutex};
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::entities::ExperienceOrb;
    use {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };

    /// What the hook heard: the orb's id, the scan distance and the default
    /// player's UUID.
    type Heard = Arc<Mutex<Vec<(u32, f64, Option<u128>)>>>;

    /// Sends orb 900 to the far guest (UUID 124) and orb 901 to no one.
    struct Shepherd(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for Shepherd {
        fn xp_orb_targeting(&self, host: &mut dyn ServerHost, e: &mut XpOrbTargetingEvent<'_>) {
            let row = (e.orb.id(), e.scan_distance, e.following.map(|p| p.uuid()));
            self.0.lock().unwrap().push(row);
            match e.orb.id() {
                900 => *e.following = host.player(124),
                901 => *e.following = None,
                _ => {}
            }
        }
    }

    fn orb(id: u32, at: [f64; 3]) -> ExperienceOrb {
        let at = Vec3::new(at[0], at[1], at[2]);
        ExperienceOrb {
            size_answer: None,
            fire_ticks: 0,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id,
            position: at,
            previous: at,
            velocity: Vec3::new(0.0, 0.0, 0.0),
            value: 1,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            air: 300,
            health: 5,
            following: None,
        }
    }

    /// `XpOrbTargetingEvent`: each orb asks with the nearest guest as the
    /// default; a mod sends one orb to a guest out of range and stops another,
    /// and an orb it leaves alone keeps the default.
    #[test]
    #[ignore = "daily"]
    fn a_mod_chooses_whom_each_orb_follows() {
        let heard = Heard::default();
        let mods = vec![ModEntry::new("shepherd", Box::new(Shepherd(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
                for (conn, x) in [(1, 0.0), (2, 30.0)] {
                    let mut g = guest(conn);
                    g.loaded = true;
                    g.position = [x, 64.0, 0.0];
                    guests.push(g);
                }
                for id in [900, 901, 902] {
                    game.orbs.push(orb(id, [1.0, 64.0, 0.0]));
                }
                let followed = game.orb_followed();
                let half = game.server.lan.as_ref().unwrap().hub.guests[0].eye_height() / 2.0;
                let eye = |x: f64| [x, 64.0 + half, 0.0];
                let targets: Vec<_> = (0..3).map(|i| game.orb_target(i, &followed)).collect();
                assert_eq!(targets, [Some(eye(30.0)), None, Some(eye(0.0))]);
                let default = (8.0, Some(123));
                let heard = heard.lock().unwrap();
                assert_eq!(*heard, [900, 901, 902].map(|id| (id, default.0, default.1)));
            });
        });
    }
}
