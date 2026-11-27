use mod_api::{Flow, Hooks, ModEntry, Mods, PlayerStatAwardEvent, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
use sunrise_sim::raid::{self, Action};

struct VictoryAwards(Arc<Mutex<Vec<u128>>>);

#[mod_api::__subscriptions]
impl Hooks for VictoryAwards {
    fn player_stat_award(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerStatAwardEvent<'_>,
    ) -> Flow {
        if event.stat.as_str() == "minecraft:raid_win" {
            self.0.lock().unwrap().push(event.player);
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn victory_rewards_players_in_hero_set_order() {
    let awards = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "victory_awards",
        Box::new(VictoryAwards(awards.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            let dim = game.dim.id();
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            // Hub order is first 14, then 13. The Java HashSet puts 13 first.
            for (conn, uuid) in [(12, 14), (13, 13)] {
                hub.update_guest(conn, |g| {
                    g.uuid = uuid;
                    g.loaded = true;
                    g.dimension = dim.to_owned();
                })
                .unwrap();
            }
            let mut raid = raid::Raid::new([8, 70, 8], 2, 0);
            for last in [14, 13, 0] {
                raid.add_hero([0, 0, 0, last]);
            }
            game.apply_raid_action(
                0,
                Action::Victory {
                    heroes: raid.heroes(),
                    amplifier: 1,
                },
            );
            assert_eq!(*awards.lock().unwrap(), [13, 14]);

            for guest in game.server.lan.as_ref().unwrap().hub.guests.iter() {
                let effect = guest.effects.get("hero_of_the_village").unwrap();
                assert_eq!((effect.duration, effect.amplifier), (48_000, 1));
                assert!(!effect.visible);
            }
        });
    });
}
