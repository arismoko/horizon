use crate::game::client_side::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use crate::game::{ClientGame, client_side::remote_client::RemoteDimensionEntry};
use mod_api::{ClientBreakProgressEvent, ClientHost};
use std::sync::Arc;
use std::sync::Mutex;
use sunrise_gameplay_common::game::{
    Hit, pick_ray::PickRayGameExt1 as _, player_identity::PlayerIdentityGameExt1 as _,
};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// What a listener heard: the dimension, the breaker, the position and the progress.
type Heard = (String, i32, [i32; 3], i32);
static HEARD: Mutex<Vec<Heard>> = Mutex::new(Vec::new());

struct Cracks;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Cracks {
    fn client_break_progress(&self, _: &dyn ClientHost, event: &mut ClientBreakProgressEvent<'_>) {
        HEARD.lock().unwrap().push((
            event.dimension.to_owned(),
            event.breaker,
            [event.pos.x, event.pos.y, event.pos.z],
            event.progress,
        ));
    }
}

fn heard() -> Vec<Heard> {
    std::mem::take(&mut *HEARD.lock().unwrap())
}

fn crack(breaker: i32, pos: [i32; 3], progress: i32) -> Heard {
    ("minecraft:overworld".to_owned(), breaker, pos, progress)
}

/// The block the server cracks, and the two dirt blocks the player digs.
const STONE_AT: [i32; 3] = [0, 64, 0];
const DIRT_A: [i32; 3] = [1, 64, 0];
const DIRT_B: [i32; 3] = [2, 64, 0];

fn with_client(name: &str, test: impl FnOnce(&mut ClientGame<'_>)) {
    let module = module_path!().split_once("::").unwrap().1;
    let selector = format!("{module}::{name}");
    if run_fresh_cases(
        &selector,
        "SUNRISE_CLIENT_BREAK_PROGRESS_CHILD",
        &["cracks"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("cracks", Box::new(Cracks))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            // Play packets are suppressed until login and configuration finish.
            let (_peer, _writer) = connect_menu_test(game);
            let spawn = play::SpawnInfo {
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
            };
            game.enter_remote_dimension(
                &spawn,
                RemoteDimensionEntry::Login,
                sunrise_client_render::level_loading::Reason::Other,
            );
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.set_local_mode(sunrise_command::args::GameMode::Survival);
            let mut player = sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.5, 64.0, 3.5),
            )
            .unwrap();
            player.ground.on_ground = true;
            game.player = Some(player);
            game.inventory[0] = Some(ItemStack::new("minecraft:diamond_shovel", 1));
            game.selected = 0;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let state = |game: &ClientGame<'_>, name: &str| {
                game.pack.assets.registry.default_state(name).unwrap()
            };
            let (stone, dirt) = (
                state(game, "minecraft:stone"),
                state(game, "minecraft:dirt"),
            );
            for (at, block) in [(STONE_AT, stone), (DIRT_A, dirt), (DIRT_B, dirt)] {
                game.world.set(at[0], at[1], at[2], block).unwrap();
            }
            heard();
            test(game);
        });
    });
}

/// Points the player's pick at the top face of `pos`.
fn look_at(game: &mut ClientGame<'_>, pos: [i32; 3]) {
    game.set_test_forced_hit(Some(Hit {
        pos,
        face: [0, 1, 0],
        point: [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ],
        inside: false,
    }));
    game.refresh_client_pick(1.0);
    game.set_test_forced_hit(None);
}

/// Each crack packet is told as the server sent it, a repeated stage and the removal included.
#[test]
#[ignore = "daily"]
fn server_cracks_are_told_as_sent() {
    with_client("server_cracks_are_told_as_sent", |game| {
        let destruction = |game: &mut ClientGame<'_>, id: i32, progress: u8| {
            game.handle_packet(C::BlockDestruction(play::BlockDestruction {
                id,
                pos: STONE_AT,
                progress,
            }))
            .unwrap();
        };
        destruction(game, 31, 3);
        destruction(game, 31, 3);
        destruction(game, 31, 255);
        assert_eq!(
            heard(),
            [
                crack(31, STONE_AT, 3),
                crack(31, STONE_AT, 3),
                crack(31, STONE_AT, 255),
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_local_dig_tells_its_stage_every_tick_and_its_stop() {
    with_client(
        "the_local_dig_tells_its_stage_every_tick_and_its_stop",
        |game| {
            let own = game.own_entity_id();
            look_at(game, DIRT_A);
            game.remote_continue_attack(true);
            game.remote_continue_attack(true);
            game.remote_continue_attack(false);
            look_at(game, DIRT_B);
            for _ in 0..3 {
                game.remote_continue_attack(true);
            }
            assert_eq!(
                heard(),
                [
                    crack(own, DIRT_A, -1),
                    crack(own, DIRT_A, 5),
                    crack(own, DIRT_A, -1),
                    crack(own, DIRT_B, -1),
                    crack(own, DIRT_B, 5),
                    crack(own, DIRT_B, -1),
                ]
            );
        },
    );
}

fn connect_menu_test(
    game: &mut ClientGame<'_>,
) -> (
    sunrise_protocol::connection::ConnReader,
    sunrise_protocol::connection::ConnWriter,
) {
    connect_play_test(game, |_, _| {})
}

fn connect_play_test(
    game: &mut ClientGame<'_>,
    configure: impl FnOnce(
        &mut sunrise_protocol::connection::ConnReader,
        &mut sunrise_protocol::connection::ConnWriter,
    ),
) -> (
    sunrise_protocol::connection::ConnReader,
    sunrise_protocol::connection::ConnWriter,
) {
    use sunrise_protocol::connection::{Connection, Pipe};
    game.options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
    let (client, server) = Pipe::pair();
    game.connect_local(client, "MenuReadTest").unwrap();
    finish_play_test(game, Connection::local(server), configure)
}

fn finish_play_test(
    game: &mut ClientGame<'_>,
    server: sunrise_protocol::connection::Connection,
    configure: impl FnOnce(
        &mut sunrise_protocol::connection::ConnReader,
        &mut sunrise_protocol::connection::ConnWriter,
    ),
) -> (
    sunrise_protocol::connection::ConnReader,
    sunrise_protocol::connection::ConnWriter,
) {
    use sunrise_protocol::packets::{Empty, configuration as cfg, login};
    let (mut reader, mut writer) = server.split();
    reader
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    reader.recv().unwrap();
    reader.recv().unwrap();
    writer
        .send(
            &login::Clientbound::LoginFinished(login::LoginFinished {
                profile: login::GameProfile {
                    uuid: 42,
                    name: "MenuReadTest".into(),
                    properties: vec![],
                },
                session: 1,
            })
            .encode(),
        )
        .unwrap();
    let (id, body) = reader.recv().unwrap();
    assert!(matches!(
        login::Serverbound::decode(id, &body).unwrap(),
        login::Serverbound::LoginAcknowledged(_)
    ));
    reader.recv().unwrap();
    reader.recv().unwrap();
    configure(&mut reader, &mut writer);
    writer
        .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
        .unwrap();
    game.acknowledge_test_configuration();
    let (id, body) = reader.recv().unwrap();
    assert!(matches!(
        cfg::Serverbound::decode(id, &body).unwrap(),
        cfg::Serverbound::FinishConfiguration(_)
    ));
    (reader, writer)
}
