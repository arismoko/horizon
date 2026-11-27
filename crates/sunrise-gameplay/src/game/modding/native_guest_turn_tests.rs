use crate::game::server_side::lan::LanSettings;
use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicUsize},
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::{
    execution_role::ExecutionRole, native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
};
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

pub fn with_guests(test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    with_guests_at(super::native_entity_refs::tests::fixture_root(), test);
}

/// Runs the guest fixture with the origin and its neighboring columns loaded and ticking.
pub fn with_resident_guests(test: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    with_guests(|game| {
        super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            sunrise_core::position::ChunkPos::new(0, 0),
        );
        test(game);
    });
}

pub fn with_guests_at(
    root: std::path::PathBuf,
    test: impl FnOnce(&mut crate::game::ServerGame<'_>),
) {
    with_guests_for_role(root, ExecutionRole::IntegratedServer, test);
}

pub fn with_guests_for_role(
    root: std::path::PathBuf,
    role: ExecutionRole,
    test: impl FnOnce(&mut crate::game::ServerGame<'_>),
) {
    super::native_entity_refs::tests::with_game_role(root, role, |game| {
        open_fixture_hub(game);
        add_fixture_guests(game);
        test(game);
    });
}

pub fn open_fixture_hub(game: &mut crate::game::ServerGame<'_>) {
    let role = game.options.execution_role;
    let settings = LanSettings {
        port: 0,
        guest_commands: false,
        force_game_mode: false,
    };
    let net = NetConfig {
        listen: false,
        announce: false,
        lod_distance: 0,
        ..NetConfig::lan(0, String::new(), 2)
    };
    let config = HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: game.dim.id().to_owned(),
        allow_flight: true,
        seed: 0,
        max_players: 2,
        integrated: role == ExecutionRole::IntegratedServer,
        levels: game.server.level_shapes.clone(),
        level_seeds: game.server.levels.0.runtime.seeds(),
    };
    game.lan_host(settings, net, config).unwrap();
}

pub fn add_fixture_guests(game: &mut crate::game::ServerGame<'_>) {
    for id in [12, 13] {
        // These guests bypass login's entity ID allocation; reserve their IDs before spawning.
        game.server.next_entity = game.server.next_entity.max(id);
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: id,
                uuid: u128::from(id),
                name: format!("guest{id}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            id as i32,
            [3.0, 64.0, 5.0],
            10,
        );
        guest.placed = true;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    }
}

/// Add a fixture menu to both the hub's protocol registry and the mod API's menu keys.
pub fn register_test_menu(game: &mut crate::game::ServerGame<'_>, id: &str) -> mod_api::MenuKey {
    use sunrise_protocol::registry::Ids;
    use sunrise_sim::lan::data::SyncData;

    let current = game.server.lan.as_ref().unwrap().hub.sync_data();
    let mut sync = SyncData::empty();
    sync.builtin = current.builtin.clone();
    sync.dynamic = current.dynamic.clone();
    sync.recipes = current.recipes.clone();
    sync.tags = current.tags.clone();
    let mut names = sync.builtin.get("minecraft:menu").unwrap().names.clone();
    names.push(id.to_owned());
    mod_api::__private::resolve_menus(names.iter().map(String::as_str)).unwrap();
    sync.builtin
        .registries
        .insert("minecraft:menu".into(), Ids::from_names(names));
    game.with_advancement_hub(|hub, owners| hub.reload_resources(Arc::new(sync), owners))
        .unwrap();
    mod_api::__private::menu_keys()
        .into_iter()
        .find(|key| mod_api::__private::menu_key_id(*key).as_deref() == Some(id))
        .unwrap()
}

/// The UUIDs of the players in the level, checked against the full reader.
pub fn level_uuids(game: &crate::game::ServerGame<'_>) -> Vec<u128> {
    let uuids: Vec<u128> = game.level_player_uuids(game.dim.id()).collect();
    let players: Vec<u128> = game.level_players().iter().map(|p| p.uuid).collect();
    assert_eq!(uuids, players, "both enumerations select the same players");
    uuids
}

/// The host in place, with a body and UUID 99.
pub fn place_host(game: &mut crate::game::ServerGame<'_>) {
    let mut body =
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
    body.entity_id = 99;
    game.player = Some(body);
    game.level.player_uuid = Some([0, 0, 0, 99]);
}

/// Put the follow-up blow outside the victim's damage cooldown.
pub fn expire_guest_damage_cooldown(game: &mut crate::game::ServerGame<'_>, conn: u32) {
    game.as_guest(conn, |game| {
        let health = &mut game.player.as_mut().unwrap().health;
        while health.hurt.cooldown > 0 {
            health.advance_damage_cooldown();
        }
    })
    .unwrap();
}

/// An armor stand summoned at (1.5, 70, 1.5) in the level in place, which `with_guests` puts
/// its two players beside, at (3, 64, 5).
pub fn summon_stand(game: &mut crate::game::ServerGame<'_>) -> mod_api::EntityRef {
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    let (sunrise_core::nbt::Tag::Compound(tag), _) =
        sunrise_core::nbt::parse_snbt("{NoGravity:1b}").unwrap()
    else {
        unreachable!()
    };
    let level = game.level(game.dim.id()).unwrap();
    game.level_summon(level, "minecraft:armor_stand", [1.5, 70.0, 1.5], tag)
        .unwrap()
        .unwrap()
}

/// Whether anything the server sent the acting player since `before` is a chat line.
pub fn told_since(game: &crate::game::ServerGame<'_>, before: usize) -> bool {
    use sunrise_protocol::packets::play::Clientbound as C;
    game.server.guest_packets[before..]
        .iter()
        .any(|p| matches!(p, C::SystemChat(_)))
}

pub fn with_command_guests(name: &str, body: impl FnOnce(&mut crate::game::ServerGame<'_>)) {
    sunrise_gameplay_common::game::in_own_process(
        &format!("cut_layout::game::server_side::native_guest_turn_tests::{name}"),
        || {
            with_guests(body);
        },
    );
}
