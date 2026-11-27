//! The players' entity tick hooks on a server (#1142): each guest's `ServerPlayer.tick` is told,
//! a Pre that cancels it skips each of its parts for that guest alone (a batched one included),
//! and a player riding a pig is told around its own tick as the pig is around its own.

use mod_api::{EntityTickPreEvent, HostSide, ModEntry, Mods};
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_level::{chunk::Chunk, containers::BlockEntity};
use sunrise_sim::{lan::menu::Slots, mobs::Mob};

/// A listener's calls, in order: the phase and the entities told.
type Heard = Arc<Mutex<Vec<(&'static str, Vec<u32>)>>>;
type PostPositions = Arc<Mutex<Vec<(u32, [f64; 3])>>>;

/// Hears every entity tick; its Pre cancels `cancel`'s.
struct Players {
    cancel: Arc<Mutex<Option<u32>>>,
    heard: Heard,
    rider_immunity_at_pre: Option<i32>,
    post_positions: PostPositions,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for Players {
    fn entity_tick_pre(
        &self,
        host: HostSide<'_>,
        event: &mut EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        self.heard.lock().unwrap().push(("pre", vec![event.entity]));
        if let Some(id) = *self.cancel.lock().unwrap()
            && event.entity == id
        {
            if let (Some(immunity), HostSide::Server(host)) = (self.rider_immunity_at_pre, host) {
                assert_eq!(
                    host.entity(id).unwrap().invulnerable_time(host),
                    Some(immunity)
                );
            }
            return mod_api::Flow::Cancel;
        }
        mod_api::Flow::Continue
    }
    fn entity_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        self.heard
            .lock()
            .unwrap()
            .push(("post", vec![event.entity]));
        if let HostSide::Server(host) = host
            && let Some(pos) = host.entity(event.entity).and_then(|e| e.position(host))
        {
            self.post_positions
                .lock()
                .unwrap()
                .push((event.entity, pos));
        }
    }
}

/// Both guests (12 and 13) with a chest open 20 blocks away, past reach, and the listener's
/// cancel aimed at guest 12; returns their entity ids.
fn far_chests(game: &mut crate::game::ServerGame<'_>, cancel: &Mutex<Option<u32>>) -> [u32; 2] {
    game.common
        .world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let guests = &game.server.lan.as_ref().unwrap().hub.guests;
    let ids = [0, 1].map(|gi| guests[gi].entity_id as u32);
    let eyes = [0, 1].map(|gi| guests[gi].position);
    for (gi, conn) in [(0, 12), (1, 13)] {
        let eye = eyes[gi];
        let far = (eye[0] as i32, eye[1] as i32, eye[2] as i32 + 20 + gi as i32);
        game.common
            .world
            .block_entities
            .insert(far, BlockEntity::Chest(vec![None; 27]));
        let host = game.server.lan.as_mut().unwrap();
        host.hub.open_container(
            conn,
            "minecraft:generic_9x3",
            sunrise_core::nbt::Tag::String("test".into()),
            far,
            Slots::Block(far),
            vec![None; 27],
            None,
        );
    }
    *cancel.lock().unwrap() = Some(ids[0]);
    ids
}

fn open(game: &crate::game::ServerGame<'_>, gi: usize) -> bool {
    game.server.lan.as_ref().unwrap().hub.guests[gi]
        .menu
        .is_some()
}

/// A Pre that cancels a player's tick skips its `ServerPlayer.tick` for that player alone: the
/// container check, run for every guest at once, closes the other guest's far chest and leaves
/// its own open, and the warden spawn tracker of the world's owner (the canceled player) does
/// not count; the family hears one Pre and the Post of the player it let through.
#[test]
#[ignore = "daily"]
fn a_canceled_player_skips_its_server_player_tick_while_the_others_run_it() {
    let (heard, cancel) = (Heard::default(), Arc::new(Mutex::new(None)));
    let mods: Mods = vec![ModEntry::new(
        "player-ticks",
        Box::new(Players {
            cancel: cancel.clone(),
            heard: heard.clone(),
            rider_immunity_at_pre: None,
            post_positions: PostPositions::default(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let [first, second] = far_chests(game, &cancel);
            game.server.lan.as_mut().unwrap().owner = Some(crate::game::lan::Owner {
                uuid: 12,
                saved: None,
            });
            game.test_sculk_mut().tracker.ticks_since_last_warning = 5;
            game.tick_guests();
            game.tick_sculk_entities();
            assert!(
                open(game, 0),
                "the canceled player's container check did not run"
            );
            assert!(
                !open(game, 1),
                "the other player's container check closed its chest"
            );
            assert_eq!(
                game.test_sculk_mut().tracker.ticks_since_last_warning,
                5,
                "the canceled owner's warden spawn tracker did not count"
            );
            assert_eq!(
                *heard.lock().unwrap(),
                [
                    ("pre", vec![first]),
                    ("pre", vec![second]),
                    ("post", vec![second]),
                ]
            );
        });
    });
}

/// #1142's second test: a pig ridden by a player. The pig is bracketed individually around
/// its own tick in the mobs' pass and each player around its own in the players' pass;
/// canceling the player's Pre stops its tick (its far chest stays open) and its Post.
#[test]
#[ignore = "daily"]
fn a_player_riding_a_pig_is_told_its_own_tick_as_the_pig_is() {
    let (heard, cancel) = (Heard::default(), Arc::new(Mutex::new(None)));
    let post_positions = PostPositions::default();
    let mods: Mods = vec![ModEntry::new(
        "player-ticks",
        Box::new(Players {
            cancel: cancel.clone(),
            heard: heard.clone(),
            rider_immunity_at_pre: Some(7),
            post_positions: post_positions.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let [rider, other] = far_chests(game, &cancel);
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let at = game.server.lan.as_ref().unwrap().hub.guests[0].position;
            let mut pig = Mob::new(90, kind, Vec3::new(at[0] + 1.0, at[1], at[2]), 0.0);
            pig.no_ai = true;
            pig.no_gravity = true;
            pig.ground.on_ground = true;
            game.mobs.push(pig);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.vehicle = Some(90));
            game.as_guest(12, |game| {
                game.player.as_mut().unwrap().health.invulnerable_time = 8
            })
            .unwrap();
            game.tick_creature_entities().unwrap();
            game.tick_guests();
            assert!(
                open(game, 0),
                "the rider's canceled tick left its chest open"
            );
            assert!(!open(game, 1));
            assert_eq!(
                *heard.lock().unwrap(),
                [
                    ("pre", vec![90]),
                    ("post", vec![90]),
                    ("pre", vec![rider]),
                    ("pre", vec![other]),
                    ("post", vec![other]),
                ]
            );
            let seated = game.server.lan.as_ref().unwrap().hub.guests[0].position;
            assert_eq!(
                seated[0],
                at[0] + 1.0,
                "a canceled rider follows its moving seat"
            );

            *cancel.lock().unwrap() = None;
            heard.lock().unwrap().clear();
            game.mobs[0].position.x += 1.0;
            game.tick_guests();
            assert_eq!(
                *heard.lock().unwrap(),
                [
                    ("pre", vec![rider]),
                    ("post", vec![rider]),
                    ("pre", vec![other]),
                    ("post", vec![other]),
                ]
            );
            assert!(
                post_positions.lock().unwrap().contains(&(rider, seated)),
                "the rider's Post still sees the old seat position"
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].position[0],
                at[0] + 2.0
            );
        });
    });
}
