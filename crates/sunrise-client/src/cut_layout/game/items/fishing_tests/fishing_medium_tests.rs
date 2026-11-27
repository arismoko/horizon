//! `fishing_medium` through the game's own bobber: no listener and a None answer leave the
//! bobber falling; an answer floats it at its surface, tells the caster, and the catch rolls
//! its checked table; the client keeps only the surface; the first mod in order wins.
#![cfg(feature = "client")]
use sunrise_core::geometry::Vec3;
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay::game::server_side::fishing::WARNED_TABLES;
use sunrise_gameplay_common::game::fishing_common::{Hook, State};
use sunrise_items::items::ItemStack;
// Included as fishing's child so fixtures can construct its private hook state.
use std::sync::{Arc, Mutex};
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

const JUNK: &str = "minecraft:gameplay/fishing/junk";
/// The bobber's caster stands here; a medium's surface is 4 blocks under their feet.
const FEET: [f64; 3] = [0.5, 64.0, 0.5];
const FISH: [&str; 4] = [
    "minecraft:cod",
    "minecraft:salmon",
    "minecraft:pufferfish",
    "minecraft:tropical_fish",
];

/// What a probe mod answers, and what it was told.
#[derive(Default)]
struct Probe {
    /// None answers no medium; otherwise the surface is the caster's feet less 4.
    table: Option<Option<String>>,
    lure_ticks: Option<[i32; 2]>,
    asks: usize,
    told: Option<(
        mod_api::LevelRef,
        u32,
        Option<mod_api::PlayerRef>,
        Option<[f64; 3]>,
    )>,
}
struct Medium(Arc<Mutex<Probe>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Medium {
    fn fishing_medium(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::FishingMediumEvent,
    ) -> Option<mod_api::FishingMedium> {
        let mut probe = self.0.lock().unwrap();
        probe.asks += 1;
        probe.told = Some((
            event.level,
            event.bobber.id(),
            event.owner,
            event.owner_position,
        ));
        let table = probe.table.clone()?;
        Some(mod_api::FishingMedium {
            surface: event.owner_position.unwrap_or(FEET)[1] - 4.0,
            loot_table: table,
            lure_ticks: probe.lure_ticks,
        })
    }
}
fn probe(table: Option<Option<&str>>) -> Arc<Mutex<Probe>> {
    Arc::new(Mutex::new(Probe {
        table: table.map(|t| t.map(str::to_owned)),
        lure_ticks: Some([20, 60]),
        ..Default::default()
    }))
}
fn mods(probes: &[(&'static str, &Arc<Mutex<Probe>>)]) -> mod_api::Mods {
    probes
        .iter()
        .map(|(id, probe)| mod_api::ModEntry::new(id, Box::new(Medium(Arc::clone(probe)))))
        .collect::<Vec<_>>()
        .into()
}

/// A server's caster (UUID 7, entity 20) at `FEET` over an empty chunk of air, rod in hand,
/// with the server's loot tables loaded.
fn caster(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    game.player =
        Some(PlayerMotion::new(game.motion, Vec3::new(FEET[0], FEET[1], FEET[2])).unwrap());
    game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:fishing_rod", 1));
    game.server.lan_acting_uuid = Some(7);
    game.server.lan_acting_entity = Some(20);
    let loot = sunrise_world::loot::LootData::load(
        &game.options.resources,
        &Default::default(),
        game.res.item_defaults(),
    )
    .unwrap();
    game.server.server_loot.0.set(Some(Arc::new(loot)));
}
/// Cast a bobber at (5.5, 66, 0.5), over the air, and tick it `ticks` times: its id.
fn cast(game: &mut sunrise_gameplay::game::ServerGame<'_>, ticks: usize) -> u32 {
    let id = game.take_entity_id();
    let mut bobber = Hook::remote(id, Vec3::new(5.5, 66.0, 0.5), [0; 4]);
    bobber.random = LegacyRandom::new(1);
    game.install_fishing_hook(bobber);
    for _ in 0..ticks {
        game.tick_fishing();
    }
    id
}
fn bobber_y(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> f64 {
    game.hook.as_mut().expect("the bobber stays out").position.y
}

#[test]
#[ignore = "daily"]
fn with_no_listener_the_bobber_falls() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_game(|game| {
            caster(game);
            cast(game, 40);
            assert!(bobber_y(game) < 58.0, "fell to {}", bobber_y(game));
            assert_eq!(game.hook_status().unwrap()["state"], "Flying");
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_mods_medium_floats_the_bobber_and_its_catch_rolls_the_checked_table() {
    if run_fresh_cases(
        "cut_layout::game::server_side::fishing::fishing_medium_tests::a_mods_medium_floats_the_bobber_and_its_catch_rolls_the_checked_table",
        "SUNRISE_FISHING_MEDIUM_CASE",
        &["float-and-catch"],
    ) {
        return;
    }
    let medium = probe(None);
    // FishingHook.catchingFish waits at least 100 ticks before the approach starts. Keep that
    // vanilla wait for the float check; a bite deliberately plunges the bobber below the surface.
    medium.lock().unwrap().lure_ticks = None;
    mod_platform::testing::with_mods(mods(&[("medium", &medium)]), || {
        with_game(|game| {
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().into();
            caster(game);

            // Answered None: asked, and the bobber falls as vanilla's.
            cast(game, 40);
            assert!(bobber_y(game) < 58.0, "fell to {}", bobber_y(game));
            assert!(medium.lock().unwrap().asks > 0);

            // A medium: the bobber floats at its surface, 60, and the caster is told.
            medium.lock().unwrap().table = Some(Some(JUNK.into()));
            let id = cast(game, 100);
            assert_eq!(game.hook_status().unwrap()["state"], "Bobbing");
            assert_eq!(game.hook.as_mut().unwrap().nibble, 0);
            let y = bobber_y(game);
            assert!((59.0..61.0).contains(&y), "floats at {y}");
            assert_eq!(game.hook_status().unwrap()["loot_table"], JUNK);
            let told = medium.lock().unwrap().told;
            assert_eq!(
                told,
                Some((
                    game.level("minecraft:overworld").unwrap(),
                    id,
                    game.player(7),
                    Some(FEET),
                ))
            );
            assert!(told.unwrap().2.is_some(), "the caster is a player handle");

            // Its catch rolls the junk table, which has no fish (vanilla's mostly does).
            let rod = ItemStack::new("minecraft:fishing_rod", 1);
            let mut hook = game.hook.extract().unwrap();
            let before = game.dropped.len();
            for _ in 0..6 {
                hook.nibble = 1;
                game.retrieve_hook_extracted(&rod, &mut hook);
            }
            game.hook.put_back(hook);
            let caught: Vec<String> = game.dropped[before..]
                .iter()
                .map(|item| item.stack.item.to_string())
                .collect();
            assert!(!caught.is_empty());
            assert!(
                caught.iter().all(|item| !FISH.contains(&item.as_str())),
                "{caught:?}"
            );

            // A hooked bobber is not asked.
            let asks = medium.lock().unwrap().asks;
            game.hook.as_mut().unwrap().state = State::Hooked(12345);
            game.tick_fishing();
            assert_eq!(medium.lock().unwrap().asks, asks);
            game.hook.discard();

            // Answered None later, the bobber no longer keeps the old table.
            cast(game, 100);
            medium.lock().unwrap().table = None;
            game.tick_fishing();
            assert!(game.hook_status().unwrap()["loot_table"].is_null());
            game.hook.discard();

            // An id that names no table, or is not an identifier, rolls vanilla's table and
            // warns once for that mod and id; the medium still holds the bobber.
            for id in ["minecraft:no_such_table", "Not An Id!"] {
                medium.lock().unwrap().table = Some(Some(id.into()));
                cast(game, 100);
                assert_eq!(game.hook_status().unwrap()["state"], "Bobbing", "{id}");
                assert!(game.hook_status().unwrap()["loot_table"].is_null(), "{id}");
                let warned = WARNED_TABLES.lock().unwrap();
                assert_eq!(
                    warned
                        .iter()
                        .filter(|(m, t)| *m == "medium" && t == id)
                        .count(),
                    1,
                    "{id}"
                );
                drop(warned);
                game.hook.discard();
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn the_first_mod_in_order_to_answer_wins() {
    let first = probe(Some(Some(JUNK)));
    let second = probe(Some(Some("minecraft:gameplay/fishing/fish")));
    mod_platform::testing::with_mods(mods(&[("first", &first), ("second", &second)]), || {
        with_game(|game| {
            caster(game);
            let at = Vec3::new(5.5, 59.5, 0.5);
            let feet = Some(Vec3::new(FEET[0], FEET[1], FEET[2]));
            let answer = game.hook_medium(1, at, feet, true).unwrap();
            assert_eq!(answer.mod_id, "first");
            assert_eq!(answer.answer.loot_table.as_deref(), Some(JUNK));
            assert_eq!(second.lock().unwrap().asks, 0, "the first answer ends it");
            first.lock().unwrap().table = None;
            let answer = game.hook_medium(1, at, feet, true).unwrap();
            assert_eq!(answer.mod_id, "second");
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_client_keeps_only_the_surface_and_draws_the_bobber_floating() {
    let medium = probe(Some(Some(JUNK)));
    mod_platform::testing::with_mods(mods(&[("medium", &medium)]), || {
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "Fisher").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            let feet = Vec3::new(FEET[0], FEET[1], FEET[2]);
            let answer = game
                .hook_medium(1, Vec3::new(5.5, 59.5, 0.5), Some(feet), true)
                .unwrap();
            assert_eq!(answer.answer.surface, 60.0);
            assert_eq!(answer.answer.loot_table, None);
            assert_eq!(answer.answer.lure_ticks, None);
            assert!(game.local_player().is_some());
            assert_eq!(medium.lock().unwrap().told.unwrap().2, game.local_player());

            let mut state = (
                Vec3::new(5.5, 62.0, 0.5),
                Vec3::ZERO,
                State::Flying,
                Default::default(),
            );
            let mut random = LegacyRandom::new(7);
            for tick in 0..100 {
                state = game.client_hook_step(1, Some(feet), true, state, &mut random);
                if tick > 60 {
                    assert!((59.0..61.0).contains(&state.0.y), "client at {}", state.0.y);
                }
            }
        });
    });
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
