use crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::{logged_in, sent};
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use std::sync::Arc;
use sunrise_client_gameplay::game::remote::gamemode_actions_tests::add_entity;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::entities::ArmorStand;

type Heard = (bool, String, bool, bool, f32);
static HEARD: std::sync::Mutex<Vec<Heard>> = std::sync::Mutex::new(Vec::new());
/// Records each firing of `critical_hit`, then flips `critical` and sets
/// a large multiplier, which must reach nothing on this client.
struct Crits;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Crits {
    fn critical_hit(&self, host: mod_api::HostSide<'_>, event: &mut mod_api::CriticalHitEvent<'_>) {
        HEARD.lock().unwrap().push((
            matches!(host, mod_api::HostSide::Client(_)),
            event.target_kind.to_string(),
            event.vanilla_critical,
            event.critical,
            event.damage_multiplier,
        ));
        event.critical = !event.critical;
        event.damage_multiplier = 9.0;
    }
}

#[test]
#[ignore = "daily"]
fn the_attacking_client_asks_critical_hit_once_per_attack_on_a_living_target() {
    // The resource loader sets process-wide tables (the cauldron interactions): own process.
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::attack_target_tests::the_attacking_client_asks_critical_hit_once_per_attack_on_a_living_target",
        client_critical_hit_isolated,
    );
}

/// A charged hit while falling is vanilla's critical on a zombie and an
/// armor stand, standing it is not; a boat is no living entity, so it goes
/// unheard. What the listener writes leaves the one Attack packet as it is.
fn client_critical_hit_isolated() {
    let mods = vec![mod_api::ModEntry::new("crits", Box::new(Crits))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let at = Vec3::new(0.5, 65.0, 2.5);
            add_entity(game, 201, "minecraft:zombie", [at.x, at.y, at.z]);
            game.armor_stands.push(ArmorStand::new(202, at, 0.0));
            game.boats.push(Boat::new(203, Arc::from("oak"), at, 0.0));
            for (id, falling, heard) in [
                (201, true, Some(("minecraft:zombie", true))),
                (201, false, Some(("minecraft:zombie", false))),
                (202, true, Some(("minecraft:armor_stand", true))),
                (203, true, None),
            ] {
                game.client.hand_strength.ticker = 100;
                let p = game.player.as_mut().unwrap();
                p.ground.on_ground = !falling;
                p.fall_distance = if falling { 1.0 } else { 0.0 };
                sent(&mut reader);
                HEARD.lock().unwrap().clear();
                assert!(game.remote_attack_entity(id, 2.0), "entity {id}");
                let heard_now = std::mem::take(&mut *HEARD.lock().unwrap());
                let expected: Vec<Heard> = heard
                    .map(|(kind, crit)| {
                        let multiplier = if crit { 1.5 } else { 1.0 };
                        (true, kind.to_owned(), crit, crit, multiplier)
                    })
                    .into_iter()
                    .collect();
                assert_eq!(heard_now, expected, "entity {id}, falling {falling}");
                let packets = sent(&mut reader);
                assert!(
                    matches!(packets.as_slice(), [S::Attack(a)] if a.id == id),
                    "entity {id}: {packets:?}"
                );
            }
        });
    });
}
