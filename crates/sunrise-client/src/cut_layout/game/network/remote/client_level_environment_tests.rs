use std::sync::Arc;
use sunrise_level::environment::Sample;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};
use {
    crate::game::server_side::native_entity_refs::tests::with_uncached_client_game as with_client_game,
    sunrise_client_gameplay::game::packs::stage_pack, sunrise_gameplay_common::game::Dim,
    sunrise_gameplay_common::game::Progress,
};

/// The prepared resources' dimension type `name`, as a data pack writes it, with its
/// `minecraft:visual/cloud_height` set to `height`.
pub(in crate::game) fn with_height(name: &str, height: f64) -> serde_json::Value {
    let resources = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let path = format!("data/minecraft/dimension_type/{name}.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(resources.join(path)).unwrap()).unwrap();
    json["attributes"]["minecraft:visual/cloud_height"] = serde_json::json!(height);
    json
}

/// The server's dimension types: each vanilla one with its cloud height raised (the Overworld's
/// to 250, the Nether's to 300, the End's to 400), and a mod's type with the Overworld's 250.
pub(in crate::game) fn server_types() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("minecraft:overworld", with_height("overworld", 250.0)),
        ("minecraft:the_nether", with_height("the_nether", 300.0)),
        ("minecraft:the_end", with_height("the_end", 400.0)),
        ("test:high_sky", with_height("overworld", 250.0)),
    ]
}

pub(in crate::game) fn spawn(dimension_type: i32, dimension: &str) -> play::SpawnInfo {
    play::SpawnInfo {
        dimension_type,
        dimension: dimension.into(),
        seed: 0,
        game_type: 1,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

/// The registry data a server sends in its configuration: `types` as its dimension types (ids
/// in order).
pub(in crate::game) fn send_types(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    types: &[(&str, serde_json::Value)],
) {
    let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
    registries.registries.insert(
        "minecraft:dimension_type".into(),
        types
            .iter()
            .map(|(name, json)| {
                (
                    (*name).into(),
                    Some(sunrise_game_data::scoreboard::json_to_tag(json)),
                )
            })
            .collect(),
    );
    game.set_registries(Arc::new(registries)).unwrap();
}

/// The `Login` packet a server sends, into `entered`.
pub(in crate::game) fn login(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    entered: play::SpawnInfo,
) {
    game.handle_packet(C::Login(play::Login {
        player_id: 7,
        hardcore: false,
        levels: vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ],
        max_players: 1,
        chunk_radius: 2,
        simulation_distance: 2,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: entered,
        online_mode: false,
        enforces_secure_chat: false,
    }))
    .unwrap();
}

/// Connects to a server whose dimension types are [`server_types`] and joins `entered`.
pub(in crate::game) fn join(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    entered: play::SpawnInfo,
) -> Pipe {
    let (client, server) = Pipe::pair();
    game.connect_local(client, "SyncedEnvironmentTest").unwrap();
    send_types(game, &server_types());
    login(game, entered);
    server
}

pub(in crate::game) fn respawn(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    spawn: play::SpawnInfo,
) {
    game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
        .unwrap();
}

/// A resource pack in a directory of its own whose Polytone dimension modifiers add a
/// post-process layer over each vanilla dimension's cloud height, each its own amount: the
/// Overworld's 10, the Nether's 20, the End's 30.
pub(in crate::game) struct CloudPack(pub(in crate::game) std::path::PathBuf);

impl CloudPack {
    pub(in crate::game) fn new(case: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "client-level-environment-{case}-{}",
            std::process::id()
        ));
        let modifiers = dir.join("assets/minecraft/polytone/dimension_modifiers");
        std::fs::create_dir_all(&modifiers).unwrap();
        std::fs::write(
            dir.join("pack.mcmeta"),
            br#"{"pack": {"description": "cloud heights", "min_format": 97, "max_format": 97}}"#,
        )
        .unwrap();
        for (dimension, add) in [("overworld", 10), ("the_nether", 20), ("the_end", 30)] {
            std::fs::write(
                modifiers.join(format!("{dimension}.json")),
                format!(
                    r#"{{"attributes_modifiers": {{"post_process": {{"visual/cloud_height": "v + {add}"}}}}}}"#
                ),
            )
            .unwrap();
        }
        Self(dir)
    }

    pub(in crate::game) fn pack(&self) -> sunrise_client::packs::Pack {
        sunrise_client::packs::pack_for(
            "server/cloud_heights".into(),
            sunrise_game_data::text::Text::literal("cloud heights"),
            sunrise_client::packs::Source::Dir(self.0.clone()),
            sunrise_client::packs::RESOURCE_FORMAT,
        )
        .unwrap()
    }
}

impl Drop for CloudPack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The client's resources reloaded with `server` as the server's packs, through the reload's
/// own staging and its render-thread apply.
pub(in crate::game) fn reload(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    server: &[sunrise_client::packs::Pack],
) {
    let staged = stage_pack(
        &game.vanilla_root(),
        &[],
        server,
        &game.mods,
        game.register_client_reload_listeners(
            &game
                .capture_candidate_settings(&game.options.resources)
                .unwrap(),
        ),
        &[],
        &game.pack.assets,
        &game.res,
        game.client_res(),
        &game.client.client_pack.context,
        false,
        false,
        &Progress::new(),
    )
    .unwrap();
    game.apply_staged_pack(staged).unwrap();
}

/// The frame's sample at the camera: the dimension's value, as no biome sets the height.
pub(in crate::game) fn drawn(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
) -> sunrise_level::environment::Sky {
    game.view_environment().sample(&Sample::default())
}

/// Gameplay's sample at the camera.
pub(in crate::game) fn gameplay(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
) -> sunrise_level::environment::Sky {
    game.dim_environment().sample(&Sample::default())
}

/// The joined level's gameplay and frame environments.
pub(in crate::game) type Built = (
    Arc<sunrise_level::environment::Environment>,
    Option<Arc<sunrise_level::environment::Environment>>,
);

pub(in crate::game) fn built(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> Built {
    let remote = game.client.remote.as_ref().unwrap();
    (
        remote.level_environment.clone().unwrap(),
        remote.level_pack_environment.clone(),
    )
}

/// A joined level's environment attributes start from the dimension type its server synced, as
/// `EnvironmentAttributeSystem.addDimensionLayer` takes `level.dimensionType()`'s attributes, and
/// the packs' Polytone modifiers of that dimension apply over them: a data pack's Overworld cloud
/// height reaches the frame on a client whose own resources say 192.33, the Nether and the End
/// take their own pack modifiers and not the Overworld's, and a resource reload rebuilds them.
#[test]
#[ignore = "daily"]
fn a_joined_levels_synced_cloud_height_reaches_the_frame_under_its_own_pack_layers() {
    with_client_game(|game| {
        let _server = join(game, spawn(0, "minecraft:overworld"));
        // The client's own Overworld says 192.33; the server's type says 250.
        assert_eq!(
            game.res
                .environment()
                .sample(&Sample::default())
                .cloud_height,
            192.33
        );
        assert_eq!(drawn(game).cloud_height, 250.0);
        assert_eq!(gameplay(game).cloud_height, 250.0);
        // A parked Nether keeps the client's Nether (sky light level 4), not the joined
        // Overworld's type.
        assert_eq!(game.sky_darken_in(&game.world, Dim::Nether), 11);

        // A server's resource pack arrives: the reload applies its Overworld modifier over the
        // server's type, 250 + 10, while gameplay keeps the type without the packs.
        let pack = CloudPack::new("joined");
        reload(game, &[pack.pack()]);
        assert!(game.client.client_pack.environments.is_some());
        assert_eq!(drawn(game).cloud_height, 260.0);
        assert_eq!(gameplay(game).cloud_height, 250.0);
        let (_, overworld_frame) = built(game);

        // The Nether's synchronized type goes over the packs' Nether stack: its own modifier,
        // 300 + 20, and not the Overworld's (300 + 10).
        respawn(game, spawn(1, "minecraft:the_nether"));
        assert_eq!(drawn(game).cloud_height, 320.0);
        assert_eq!(gameplay(game).cloud_height, 300.0);
        // Its base is the client's Nether, not its Overworld: the sky colour is the Nether's.
        let nether = game.res.nether_environment().sample(&Sample::default());
        let overworld_sky = game.res.environment().sample(&Sample::default()).sky_color;
        assert_ne!(nether.sky_color, overworld_sky);
        assert_eq!(drawn(game).sky_color, nether.sky_color);
        assert_eq!(gameplay(game).sky_color, nether.sky_color);
        assert_eq!(game.sky_darken_in(&game.world, Dim::Nether), 11);

        // The End's goes over the packs' End stack: 400 + 30, not 400 + 10.
        respawn(game, spawn(2, "minecraft:the_end"));
        assert_eq!(drawn(game).cloud_height, 430.0);
        assert_eq!(gameplay(game).cloud_height, 400.0);

        // Back in the Overworld, nothing changed: the environment the reload built is drawn
        // again, not built anew.
        respawn(game, spawn(0, "minecraft:overworld"));
        let (overworld_gameplay, frame) = built(game);
        assert!(Arc::ptr_eq(
            frame.as_ref().unwrap(),
            overworld_frame.as_ref().unwrap()
        ));
        assert_eq!(drawn(game).cloud_height, 260.0);

        // The pack is dropped: the reload builds the frame's environment again, the server's
        // 250 without the modifier; gameplay's, which has no pack layers, is kept.
        reload(game, &[]);
        assert!(game.client.client_pack.environments.is_none());
        assert_eq!(drawn(game).cloud_height, 250.0);
        let (gameplay_after, frame) = built(game);
        assert!(frame.is_none());
        assert!(Arc::ptr_eq(&gameplay_after, &overworld_gameplay));
    });
}

/// A built-in level whose synchronized type is a mod's has no pack stack of its type: it draws
/// the type as the server sent it, not the packs' Overworld.
#[test]
#[ignore = "daily"]
fn an_overworld_of_a_mods_type_draws_the_type_without_the_packs_overworld() {
    with_client_game(|game| {
        let pack = CloudPack::new("mod-type");
        reload(game, &[pack.pack()]);
        assert!(game.client.client_pack.environments.is_some());
        let _server = join(game, spawn(3, "minecraft:overworld"));
        assert_eq!(drawn(game).cloud_height, 250.0);
        assert_eq!(gameplay(game).cloud_height, 250.0);
        assert!(built(game).1.is_none());
    });
}

/// Entering a level whose type, JSON and packs are what an earlier entry built from reuses the
/// built environments; a type whose JSON changed (a proxy's next server, through a new
/// configuration and `Login`) is built again from the new JSON.
#[test]
#[ignore = "daily"]
fn an_unchanged_type_reuses_its_environments_and_a_changed_one_is_built_again() {
    with_client_game(|game| {
        let pack = CloudPack::new("reuse");
        reload(game, &[pack.pack()]);
        let _server = join(game, spawn(0, "minecraft:overworld"));
        let (own, frame) = built(game);
        // A death respawn into the same level.
        respawn(game, spawn(0, "minecraft:overworld"));
        let (again_own, again_frame) = built(game);
        assert!(Arc::ptr_eq(&own, &again_own));
        assert!(Arc::ptr_eq(
            frame.as_ref().unwrap(),
            again_frame.as_ref().unwrap()
        ));

        // The next server's Overworld says 270: the same type name with other JSON is built
        // from that JSON, not taken from what 250 built.
        let mut types = server_types();
        types[0].1 = with_height("overworld", 270.0);
        send_types(game, &types);
        login(game, spawn(0, "minecraft:overworld"));
        assert_eq!(drawn(game).cloud_height, 280.0);
        assert_eq!(gameplay(game).cloud_height, 270.0);
        let (changed_own, changed_frame) = built(game);
        assert!(!Arc::ptr_eq(&own, &changed_own));
        assert!(!Arc::ptr_eq(
            frame.as_ref().unwrap(),
            changed_frame.as_ref().unwrap()
        ));
    });
}

use sunrise_gameplay_common::game::end_common::EndGameExt as _;
