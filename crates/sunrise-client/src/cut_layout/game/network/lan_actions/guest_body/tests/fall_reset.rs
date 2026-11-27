//! `handlePlayerPositionChange`: a move that goes up ends the guest's fall.
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::{Empty, play, play::Serverbound as S};
use sunrise_sim::lan::guest::InboxItem;

pub(crate) const NAME: &str = "GuestBodyTest";
/// `fall_from_height` with no condition, then with a fall of at most 1 block, and of 5.
pub(crate) const CRITERIA: [&str; 3] = ["any", "short", "five"];

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::lan_actions_server::guest_body::tests::fall_reset::scene;

/// One server packet drain; put a client-tick marker between reports.
pub(crate) fn drain_moves(
    g: &mut sunrise_gameplay::game::ServerGame<'_>,
    reports: impl IntoIterator<Item = (f64, u8)>,
) {
    g.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |guest| {
            guest
                .inbox
                .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
            for (y, flags) in reports {
                let to = play::MovePlayerPos {
                    x: 12.5,
                    y,
                    z: 12.5,
                    flags,
                };
                guest
                    .inbox
                    .push_back(InboxItem::Packet(S::MovePlayerPos(to), None));
                guest
                    .inbox
                    .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
            }
        });
    g.tick_lan_packets();
}

/// One server tick in which the guest's client sent these airborne positions.
pub(crate) fn tick_with_moves(
    g: &mut sunrise_gameplay::game::ServerGame<'_>,
    heights: &[f64],
    before_level: (f64, Option<[f64; 3]>, [bool; 3]),
) {
    drain_moves(g, heights.iter().map(|&y| (y, 0)));
    let guest = &g.server.lan.as_ref().unwrap().hub.guests[0];
    assert_eq!(guest.moves.len(), heights.len(), "every move is accepted");
    assert_eq!(guest.position[1], *heights.last().unwrap());
    assert_eq!(fall(g), before_level, "state before the level tick");
    g.tick_lan(false);
}

/// The guest's fall distance, where its tracked fall began, and which criteria it has.
pub(crate) fn fall(
    g: &sunrise_gameplay::game::ServerGame<'_>,
) -> (f64, Option<[f64; 3]>, [bool; 3]) {
    let host = g.server.lan.as_ref().unwrap();
    let guest = &host.hub.guests[0];
    let done = CRITERIA.map(|name| {
        let i = g.adv.tree.find_id(&format!("fall:{name}")).unwrap();
        guest.advancements.done(&g.adv.tree.list[i])
    });
    (
        guest.motion.as_ref().unwrap().fall_distance,
        host.play.get(&1).unwrap().marks.fall,
        done,
    )
}

#[test]
#[ignore = "daily"]
fn fall_distance_handle_reads_the_guests_fall_and_landing() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::guest_body::tests::fall_reset::fall_distance_handle_reads_the_guests_fall_and_landing",
        || {
            let mut resources = None;
            let scratch = tempfile::tempdir().unwrap();
            let mut game = scene(&mut resources, scratch.path());
            let entity = game.player(1).unwrap().entity();
            drain_moves(&mut game, [(109.0, 0)]);
            assert_eq!(entity.fall_distance(&game), Some(1.0));
            game.tick_lan(false);
            drain_moves(&mut game, [(108.0, 0)]);
            assert_eq!(entity.fall_distance(&game), Some(2.0));
            game.tick_lan(false);
            drain_moves(&mut game, [(100.0, 1)]);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert_eq!(guest.position, [12.5, 100.0, 12.5]);
            assert!(guest.on_ground);
            assert_eq!(entity.fall_distance(&game), Some(0.0));
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_guest_move_that_goes_up_ends_its_fall() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::guest_body::tests::fall_reset::a_guest_move_that_goes_up_ends_its_fall",
        || {
            let mut resources = None;
            let scratch = tempfile::tempdir().unwrap();
            let mut g = scene(&mut resources, scratch.path());
            // One block down: a fall of 1, tracked from where the tick left the guest.
            tick_with_moves(&mut g, &[109.0], (1.0, None, [false; 3]));
            assert_eq!(fall(&g), (1.0, Some([12.5, 109.0, 12.5]), [false; 3]));
            // Half a block up: no fall left, and the fall of half a block is awarded.
            tick_with_moves(&mut g, &[109.5], (0.0, None, [true, true, false]));
            assert_eq!(fall(&g), (0.0, None, [true, true, false]));

            for name in ["any", "short"] {
                assert!(g.run_command(&format!("advancement revoke {NAME} only fall:{name}")));
            }
            tick_with_moves(&mut g, &[108.5], (1.0, None, [false; 3]));
            assert_eq!(fall(&g), (1.0, Some([12.5, 108.5, 12.5]), [false; 3]));
            // Three moves in one server tick: down to 104, up to 104.5, down to 103.5. The move up
            // ends the fall where it is, 4 blocks below its start, not at the tick's end, 5 below.
            // The last move starts a new fall of 1.
            tick_with_moves(
                &mut g,
                &[104.0, 104.5, 103.5],
                (1.0, None, [true, false, false]),
            );
            assert_eq!(
                fall(&g),
                (1.0, Some([12.5, 103.5, 12.5]), [true, false, false])
            );

            // A player that dies later in the tick was alive at its move up, so the fall still counts:
            // half a heart left, and a block at its head to suffocate in once it has moved.
            assert!(g.run_command(&format!("advancement revoke {NAME} only fall:any")));
            assert!(g.run_command("gamerule natural_health_regeneration false"));
            let health = g.server.lan.as_ref().unwrap().hub.guests[0].health;
            assert!(g.run_command(&format!("damage {NAME} {} minecraft:generic", health - 0.5)));
            // The damage clock runs out, so the next damage counts.
            for _ in 0..20 {
                g.tick_lan(false);
            }
            assert!(g.run_command("setblock 12 105 12 minecraft:stone"));
            tick_with_moves(&mut g, &[104.0], (0.0, None, [true, true, false]));
            let guest = &g.server.lan.as_ref().unwrap().hub.guests[0];
            assert!(
                guest.health <= 0.0,
                "the guest suffocated: {}",
                guest.health
            );
            // Half a block up from where the last fall began: any fall, and a short one.
            assert_eq!(fall(&g).2, [true, true, false]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn entering_water_ends_a_tracked_fall_before_the_next_move() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::guest_body::tests::fall_reset::entering_water_ends_a_tracked_fall_before_the_next_move",
        || {
            let mut resources = None;
            let scratch = tempfile::tempdir().unwrap();
            let mut g = scene(&mut resources, scratch.path());
            assert!(g.run_command("setblock 12 104 12 minecraft:water"));
            tick_with_moves(&mut g, &[105.0], (5.0, None, [false; 3]));
            assert_eq!(fall(&g).1, Some([12.5, 105.0, 12.5]));

            // Enter airborne water a half block down, then fall below it in the same batch.
            drain_moves(&mut g, [(104.5, 0), (101.5, 0)]);
            assert_eq!(
                g.server.lan.as_ref().unwrap().hub.guests[0].position[1],
                101.5
            );
            assert_eq!(fall(&g), (0.0, None, [true, true, false]));
        },
    );
}

pub(crate) fn lethal_landing(totem: bool) {
    let mut resources = None;
    let scratch = tempfile::tempdir().unwrap();
    let mut g = scene(&mut resources, scratch.path());
    tick_with_moves(&mut g, &[105.0], (5.0, None, [false; 3]));
    g.as_guest(1, |game| {
        game.player
            .as_mut()
            .unwrap()
            .set_health(0.5, false)
            .unwrap();
        game.inventory[0] = Some(ItemStack::new("minecraft:diamond", 1));
        if totem {
            game.offhand = Some(ItemStack::new("minecraft:totem_of_undying", 1));
        }
    })
    .unwrap();

    // Land, then report stepping up, with no world or connection tick in between.
    drain_moves(&mut g, [(100.0, 1), (100.5, 0)]);
    let guest = &g.server.lan.as_ref().unwrap().hub.guests[0];
    if totem {
        assert!(!guest.dead);
        assert_eq!(guest.health, 1.0);
        assert!(guest.offhand.is_none());
        assert_eq!(guest.position[1], 100.5);
        assert_eq!(fall(&g), (0.0, None, [true, false, true]));
    } else {
        assert!(guest.dead);
        assert_eq!(guest.position[1], 100.0);
        assert_eq!(guest.last_death.as_ref().unwrap().1, [12, 100, 12]);
        let drop = g
            .dropped
            .iter()
            .find(|item| item.stack.item.as_ref() == "minecraft:diamond")
            .unwrap();
        assert_eq!(
            drop.position.y,
            100.0 + guest.eye_height() - f64::from(0.3_f32)
        );
        assert_eq!(fall(&g), (0.0, None, [false; 3]));
    }
}

#[test]
#[ignore = "daily"]
fn a_lethal_landing_records_death_and_drops_before_the_next_move() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::guest_body::tests::fall_reset::a_lethal_landing_records_death_and_drops_before_the_next_move",
        || {
            lethal_landing(false);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_totem_saves_the_guest_before_its_tracked_fall_ends() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::guest_body::tests::fall_reset::a_totem_saves_the_guest_before_its_tracked_fall_ends",
        || {
            lethal_landing(true);
        },
    );
}

/// A mod's protector from outside the hands (#5751) saves a guest whose hands hold none, and the
/// game takes nothing from its inventory for it.
#[test]
#[ignore = "daily"]
fn a_mods_protector_from_outside_the_hands_saves_the_guest() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::guest_body::tests::fall_reset::a_mods_protector_from_outside_the_hands_saves_the_guest",
        || {
            /// A worn totem, lent once per ask; counts the asks.
            struct Charm(std::sync::Arc<std::sync::Mutex<u32>>);
            #[mod_api::__subscriptions]
            impl mod_api::Hooks for Charm {
                fn living_death_protection(
                    &self,
                    _: &mut dyn mod_api::HitHost,
                    event: &mut mod_api::LivingDeathProtectionEvent<'_>,
                ) {
                    use sunrise_gameplay_common::game::api_values::TryIntoApi;
                    *self.0.lock().unwrap() += 1;
                    let totem = ItemStack::new("minecraft:totem_of_undying", 1);
                    *event.protector = Some(totem.try_into_api().unwrap());
                }
            }
            let asks = std::sync::Arc::new(std::sync::Mutex::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "charm",
                Box::new(Charm(asks.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                let mut resources = None;
                let scratch = tempfile::tempdir().unwrap();
                let mut g = scene(&mut resources, scratch.path());
                tick_with_moves(&mut g, &[105.0], (5.0, None, [false; 3]));
                let stick = ItemStack::new("minecraft:stick", 2);
                g.as_guest(1, |game| {
                    game.player
                        .as_mut()
                        .unwrap()
                        .set_health(0.5, false)
                        .unwrap();
                    game.selected = 1;
                    game.inventory[1] = Some(stick.clone());
                })
                .unwrap();

                drain_moves(&mut g, [(100.0, 1)]);
                let guest = &g.server.lan.as_ref().unwrap().hub.guests[0];
                assert!(!guest.dead);
                assert_eq!(guest.health, 1.0);
                assert_eq!(guest.inventory[1].as_ref(), Some(&stick));
            });
            assert_eq!(*asks.lock().unwrap(), 1);
        },
    );
}

#[test]
#[ignore = "daily"]
fn lethal_landing_consumes_one_item_with_death_protection_from_the_selected_slot() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::lan_actions::guest_body::tests::fall_reset::lethal_landing_consumes_one_item_with_death_protection_from_the_selected_slot",
        || {
            let mut resources = None;
            let scratch = tempfile::tempdir().unwrap();
            let mut g = scene(&mut resources, scratch.path());
            tick_with_moves(&mut g, &[105.0], (5.0, None, [false; 3]));

            let mut item = ItemStack::new("minecraft:stick", 2);
            item.data_mut().set_other(
                "minecraft:death_protection",
                Some(sunrise_core::nbt::Tag::Compound(Default::default())),
            );
            g.as_guest(1, |game| {
                game.player
                    .as_mut()
                    .unwrap()
                    .set_health(0.5, false)
                    .unwrap();
                game.selected = 1;
                game.inventory[1] = Some(item.clone());
            })
            .unwrap();

            drain_moves(&mut g, [(100.0, 1)]);
            let guest = &g.server.lan.as_ref().unwrap().hub.guests[0];
            assert!(!guest.dead);
            assert_eq!(guest.health, 1.0);
            item.count = 1;
            assert_eq!(guest.inventory[1].as_ref(), Some(&item));
        },
    );
}

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
