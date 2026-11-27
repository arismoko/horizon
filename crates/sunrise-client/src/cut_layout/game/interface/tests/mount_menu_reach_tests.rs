//! `AbstractMountInventoryMenu.stillValid` asks
//! `player.isWithinEntityInteractionRange(mount, 4.0)`, the player's eyes to
//! the mount's box against the entity interaction range plus 4: a donkey's
//! menu stays open at 6.5 blocks and closes past 7, not at the 4.5 a
//! centre-to-centre check gives. A server's guest is checked by
//! `Game::lan_mount_valid`, the window's own copy by `Game::tick_mount_menu`.
#[cfg(test)]
use crate::game::load_resources;
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_client::audio::Audio;
use sunrise_client::settings::Settings;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Options;
use sunrise_gameplay_common::game::Screen;
#[cfg(test)]
use sunrise_gameplay_common::game::execution_role;
use sunrise_gameplay_common::game::resource_load::Progress;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::{
    lan::{Joined, PlayerSender, guest::Guest},
    mobs::{Mob, Species},
    motion::PlayerMotion,
};

const DONKEY: u32 = 7;

/// A player at `(8.5, 64, 8.5)` with a tamed, chested donkey's menu open, its
/// box `gap` blocks from the eyes along +x.
fn game_with_open_donkey<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    gap: f64,
) -> sunrise_client_gameplay::game::ClientGame<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::Client;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        res,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root,
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    game.player =
        Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).expect("a player"));
    let kind = game
        .res
        .mob_types()
        .get("minecraft:donkey")
        .expect("a donkey type")
        .clone();
    let mut mob = Mob::new(DONKEY, kind, Vec3::new(8.5, 64.0, 8.5), 0.0);
    mob.extra.tame = true;
    mob.extra.chested = true;
    assert_eq!(mob.species(), Species::Donkey);
    game.mobs.push(mob);
    game.mobs[0].position.x = 8.5 + gap + f64::from(game.mobs[0].width()) / 2.0;
    game.open_mount_menu(0);
    assert_eq!(
        game.client.mount_menu.opened_mob(),
        Some(DONKEY),
        "the donkey's menu did not open"
    );
    game
}

/// `Player.isWithinEntityInteractionRange(mount, 4.0)`: 3 + 4 blocks from the
/// eyes to the box, so a mount moved 5 blocks and then 6.5 blocks away keeps
/// its menu open.
#[test]
#[ignore = "daily"]
fn a_mount_moved_five_blocks_away_keeps_its_menu_open() {
    let mut resources = None;
    let mut game = game_with_open_donkey(&mut resources, 5.0);
    game.tick_mount_menu();
    assert_eq!(
        game.client.mount_menu.opened_mob(),
        Some(DONKEY),
        "the menu closed 5 blocks from the donkey's box"
    );
    game.mobs[0].position.x += 1.5;
    game.tick_mount_menu();
    assert_eq!(
        game.client.mount_menu.opened_mob(),
        Some(DONKEY),
        "the menu closed 6.5 blocks from the donkey's box"
    );
    assert!(
        game.client.screen == Screen::Inventory,
        "the screen left the menu"
    );
}

/// Past the entity interaction range plus 4 (7 blocks from the box) the menu
/// closes.
#[test]
#[ignore = "daily"]
fn a_mount_past_seven_blocks_closes_its_menu() {
    let mut resources = None;
    let mut game = game_with_open_donkey(&mut resources, 7.5);
    game.tick_mount_menu();
    assert_eq!(
        game.client.mount_menu.opened_mob(),
        None,
        "the menu stayed open 7.5 blocks from the donkey's box"
    );
    assert!(
        game.client.screen == Screen::Playing,
        "the menu's screen stayed"
    );
}

/// A tamed, chested donkey at `(8.5, 64, 8.5)` and a guest whose eyes are
/// `gap` blocks east of its box (the box's east face is at 8.5 + its half
/// width, and the eyes sit inside its height).
fn guest_beside_a_donkey<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    gap: f64,
) -> (sunrise_gameplay::game::ServerGame<'a>, Guest) {
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    let kind = game
        .res
        .mob_types()
        .get("minecraft:donkey")
        .expect("a donkey type")
        .clone();
    let mut mob = Mob::new(DONKEY, kind, Vec3::new(8.5, 64.0, 8.5), 0.0);
    mob.extra.tame = true;
    mob.extra.chested = true;
    game.mobs.push(mob);
    let (tx, _rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 2,
            name: "ReachGuest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [8.5, 64.0, 8.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    guest.loaded = true;
    guest.position = [box_face(&game) + gap, 64.0, 8.5];
    (game, guest)
}

/// The east face of the donkey's box at `(8.5, 64, 8.5)`.
fn box_face(game: &sunrise_gameplay::game::ServerGame<'_>) -> f64 {
    8.5 + f64::from(game.mobs[0].width()) / 2.0
}

/// A server checks its guest's mount menu against the guest's own entity
/// interaction range (`Player.isWithinEntityInteractionRange(mount, 4.0)`):
/// 3 + 4 blocks in survival, 5 + 4 in creative.
#[test]
#[ignore = "daily"]
fn a_guests_mount_menu_holds_to_its_own_entity_interaction_range_plus_four() {
    let mut resources = None;
    let (game, mut guest) = guest_beside_a_donkey(&mut resources, 6.5);
    assert!(
        game.lan_mount_valid(&guest, DONKEY),
        "the menu closed 6.5 blocks from the donkey's box"
    );
    guest.position[0] = box_face(&game) + 7.5;
    assert!(
        !game.lan_mount_valid(&guest, DONKEY),
        "the menu stayed open 7.5 blocks from the donkey's box"
    );
    // Creative's entity interaction range is 5, so the menu holds to 9.
    guest.game_mode = 1;
    guest.position[0] = box_face(&game) + 8.0;
    assert!(
        game.lan_mount_valid(&guest, DONKEY),
        "a creative guest's menu closed 8 blocks from the donkey's box"
    );
    guest.position[0] = box_face(&game) + 9.5;
    assert!(
        !game.lan_mount_valid(&guest, DONKEY),
        "a creative guest's menu stayed open 9.5 blocks from the donkey's box"
    );
}
