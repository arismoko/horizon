//! Exercise both owning player tick routes, not the standalone swim oracle.
#![cfg(feature = "client")]
use mod_api::{CanStartSwimmingEvent, EquipmentSlot, Hooks, Host, LogicalSide, SwimmingCheck};
use std::sync::Arc;
use std::sync::Mutex;
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_gameplay_common::game::native_hooks::can_start_swimming::SprintGameExt;
use sunrise_gameplay_common::game::remote::{Chunk, ItemStack, Keys, PlayerMotion, Vec3, play};
use winit::keyboard::KeyCode;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

#[derive(Clone, Default)]
struct SwimRules(Arc<Mutex<Vec<(LogicalSide, SwimmingCheck, u32)>>>);

#[mod_api::__subscriptions]
impl Hooks for SwimRules {
    fn can_start_swimming(
        &self,
        host: &dyn Host,
        event: &mut CanStartSwimmingEvent,
    ) -> Option<bool> {
        self.0
            .lock()
            .unwrap()
            .push((event.side, event.check, event.player.entity().id()));
        let feet = host.living(event.player.entity())?.with_equipment(
            host,
            EquipmentSlot::Feet,
            |feet| feet.item(),
        )?;
        let item = mod_api::__private::item_key_id(feet)?;
        match (event.check, item) {
            (SwimmingCheck::Entry, "minecraft:leather_boots")
            | (SwimmingCheck::ShallowWater, "minecraft:golden_boots") => Some(false),
            (SwimmingCheck::Entry, "minecraft:iron_boots") => Some(true),
            _ => None,
        }
    }
}

struct MutatesQuery;

#[mod_api::__subscriptions]
impl Hooks for MutatesQuery {
    fn can_start_swimming(&self, _: &dyn Host, event: &mut CanStartSwimmingEvent) -> Option<bool> {
        event.check = SwimmingCheck::ShallowWater;
        None
    }
}

fn water_column(game: &mut sunrise_gameplay_common::game::CommonGame<'_>) {
    let water = game
        .pack
        .assets
        .registry
        .default_state("minecraft:water")
        .unwrap();
    for y in 64..=66 {
        game.world.set(4, y, 4, water).unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn local_swim_eligibility_replaces_entry_and_the_shallow_sprint_query() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::swim_start_tests::local_swim_eligibility_replaces_entry_and_the_shallow_sprint_query",
        || {
            let rules = SwimRules::default();
            let mods = vec![
                mod_api::ModEntry::new("rewrite-query", Box::new(MutatesQuery)),
                mod_api::ModEntry::new("swim", Box::new(rules.clone())),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_client_game(|game| {
                    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(client, "swim-client").unwrap();
                    game.enter_remote_dimension(
                        &play::SpawnInfo {
                            dimension_type: 0,
                            dimension: "minecraft:overworld".into(),
                            seed: 0,
                            game_type: 0,
                            previous_game_type: None,
                            debug: false,
                            flat: true,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        },
                        RemoteDimensionEntry::Login,
                        sunrise_client::level_loading::Reason::Other,
                    );
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.logged_in = true;
                    remote.loaded_sent = true;
                    remote.entity_id = 42;
                    remote.profile = 42;
                    remote.level_load.finish();
                    game.client
                        .streaming
                        .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
                    water_column(game);
                    let mut player =
                        PlayerMotion::new(game.motion, Vec3::new(4.5, 64.5, 4.5)).unwrap();
                    player.entity_id = 42;
                    game.player = Some(player);
                    game.bind_local_player_residency();
                    let tick = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                        game.tick_local_player(Vec::new(), 7, None).unwrap();
                    };
                    // Seed the previous eye sample without asking the entry predicate.
                    tick(game);
                    assert!(rules.0.lock().unwrap().is_empty());
                    game.insert_device_key(KeyCode::KeyW);
                    game.armor[3] = Some(ItemStack::new("minecraft:leather_boots", 1));
                    game.player.as_mut().unwrap().set_sprinting(true).unwrap();
                    tick(game);
                    let p = game.player.as_ref().unwrap();
                    assert!(p.is_under_water());
                    assert!(p.sprinting, "forward input keeps the existing sprint");
                    assert!(!p.swimming, "the equipment rule refused swim entry");
                    assert_eq!(
                        std::mem::take(&mut *rules.0.lock().unwrap()),
                        [
                            (LogicalSide::Client, SwimmingCheck::Entry, 42),
                            (LogicalSide::Client, SwimmingCheck::ShallowWater, 42),
                        ]
                    );

                    game.armor[3] = Some(ItemStack::new("minecraft:golden_boots", 1));
                    game.player.as_mut().unwrap().set_sprinting(false).unwrap();
                    game.player_sprint_tick(
                        Keys {
                            forward: true,
                            sprint: true,
                            ..Keys::default()
                        },
                        7,
                        None,
                    )
                    .unwrap();
                    assert!(!game.player.as_ref().unwrap().sprinting);
                    assert_eq!(
                        std::mem::take(&mut *rules.0.lock().unwrap()),
                        [(LogicalSide::Client, SwimmingCheck::ShallowWater, 42),]
                    );

                    game.armor[3] = Some(ItemStack::new("minecraft:iron_boots", 1));
                    let p = game.player.as_mut().unwrap();
                    p.position = Vec3::new(10.5, 64.5, 10.5);
                    p.velocity = Vec3::ZERO;
                    p.set_sprinting(true).unwrap();
                    tick(game);
                    assert!(game.player.as_ref().unwrap().swimming);
                    assert_eq!(
                        std::mem::take(&mut *rules.0.lock().unwrap()),
                        [(LogicalSide::Client, SwimmingCheck::Entry, 42),]
                    );

                    // A newly mounted sprinter is asked before the boat's sprint
                    // decision stops sprinting. An affirmative answer cannot make
                    // a passenger swim, and the next tick doesn't ask again.
                    game.clear_device_keys();
                    let p = game.player.as_mut().unwrap();
                    p.swimming = false;
                    p.set_sprinting(true).unwrap();
                    game.boats
                        .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                            71,
                            "oak".into(),
                            Vec3::new(10.5, 64.5, 10.5),
                            0.0,
                        ));
                    game.riding = Some(71);
                    tick(game);
                    let p = game.player.as_ref().unwrap();
                    assert!(!p.swimming && !p.sprinting);
                    assert_eq!(
                        std::mem::take(&mut *rules.0.lock().unwrap()),
                        [(LogicalSide::Client, SwimmingCheck::Entry, 42)]
                    );
                    tick(game);
                    assert!(rules.0.lock().unwrap().is_empty());
                });
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn swim_entry_reads_each_authoritative_guests_own_equipment() {
    use {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::swim_start_tests::swim_entry_reads_each_authoritative_guests_own_equipment",
        || {
            let rules = SwimRules::default();
            let mods = vec![mod_api::ModEntry::new("swim", Box::new(rules.clone()))].into();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    open_to_lan(game);
                    game.server
                        .streaming
                        .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
                    water_column(game);
                    let mut ids = Vec::new();
                    for (conn, position, boots) in [
                        (1, [4.5, 64.5, 4.5], "minecraft:leather_boots"),
                        (2, [10.5, 64.5, 10.5], "minecraft:iron_boots"),
                    ] {
                        let mut body = guest(conn);
                        body.loaded = true;
                        body.position = position;
                        body.motion = Some(Box::new(
                            PlayerMotion::new(
                                game.motion,
                                Vec3::new(position[0], position[1], position[2]),
                            )
                            .unwrap(),
                        ));
                        body.armor[3] = Some(ItemStack::new(boots, 1));
                        ids.push(body.entity_id as u32);
                        game.server.lan.as_mut().unwrap().hub.guests.push(body);
                        game.guest_server_tick(conn, true);
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .update_guest(conn, |g| g.sprinting = true)
                            .unwrap();
                    }
                    for (conn, swims) in [(1, false), (2, true)] {
                        game.guest_server_tick(conn, true);
                        let body = game.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                        assert_eq!(body.motion.as_ref().unwrap().swimming, swims);
                        assert!(body.sprinting);
                    }
                    assert_eq!(
                        std::mem::take(&mut *rules.0.lock().unwrap()),
                        [
                            (LogicalSide::Server, SwimmingCheck::Entry, ids[0]),
                            (LogicalSide::Server, SwimmingCheck::Entry, ids[1]),
                        ]
                    );
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(1, |g| g.armor[3] = None)
                        .unwrap();
                    game.guest_server_tick(1, true);
                    let body = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                    assert!(body.motion.as_ref().unwrap().swimming);
                });
            });
        },
    );
}

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
