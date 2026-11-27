//! Client-side furnace result-take prediction (`item_smelted` with `HostSide::Client`
//! and no client XP orb spawning).

use crate::game::{
    ClientGame,
    client_side::{
        native_entity_refs::tests::with_client_game, remote_client::RemoteDimensionEntry,
    },
};
use std::sync::{Arc, Mutex};
use sunrise_client_render::level_loading::Reason;
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_protocol::{
    connection::{ConnReader, ConnWriter, Connection, Pipe},
    item::Stack,
    packets::{
        Empty, configuration as cfg, login,
        play::{Clientbound, ContainerSetContent, OpenScreen, SpawnInfo},
    },
};
use sunrise_sim::{menu::Input, motion::PlayerMotion};

#[derive(Debug, PartialEq, Eq)]
struct SmeltedCall {
    is_client: bool,
    uuid: u128,
    entity_id: u32,
    item: Option<String>,
    count: u32,
    projected: Option<(Option<String>, u32)>,
    amount_removed: u32,
}

#[derive(Default)]
struct Recorded {
    item_smelted: Vec<SmeltedCall>,
}

struct FurnaceProbe {
    recorded: Arc<Mutex<Recorded>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for FurnaceProbe {
    fn item_smelted(&self, host: mod_api::HostSide<'_>, event: &mut mod_api::ItemSmeltedEvent<'_>) {
        let (is_client, item, projected) = match host {
            mod_api::HostSide::Server(host) => {
                let projected = event
                    .stack
                    .project()
                    .map(|stack| (stack.item().id(host).map(str::to_owned), stack.count()));
                (
                    false,
                    event.stack.item().id(host).map(str::to_owned),
                    projected,
                )
            }
            mod_api::HostSide::Client(host) => {
                let projected = event
                    .stack
                    .project()
                    .map(|stack| (stack.item().id(host).map(str::to_owned), stack.count()));
                (
                    true,
                    event.stack.item().id(host).map(str::to_owned),
                    projected,
                )
            }
        };
        self.recorded
            .lock()
            .unwrap()
            .item_smelted
            .push(SmeltedCall {
                is_client,
                uuid: event.player.uuid(),
                entity_id: event.player.entity().id(),
                item,
                count: event.stack.count(),
                projected,
                amount_removed: event.amount_removed,
            });
    }
}

fn with_probe(run: impl FnOnce(&Arc<Mutex<Recorded>>)) {
    let recorded = Arc::new(Mutex::new(Recorded::default()));
    let probe = FurnaceProbe {
        recorded: Arc::clone(&recorded),
    };
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("furnace_probe", Box::new(probe))].into(),
        || run(&recorded),
    );
}

fn connect_menu_test(game: &mut ClientGame<'_>) -> (ConnReader, ConnWriter) {
    game.common.options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
    let (client, server) = Pipe::pair();
    game.connect_local(client, "MenuReadTest").unwrap();
    let (mut reader, mut writer) = Connection::local(server).split();
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

#[test]
#[ignore = "daily"]
fn client_prediction_fires_item_smelted_with_client_host_and_no_xp() {
    with_probe(|recorded| {
        with_client_game(|game| {
            let (_reader, _writer) = connect_menu_test(game);
            let spawn = SpawnInfo {
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
            game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.common.player =
                Some(PlayerMotion::new(game.common.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());

            let furnace_menu = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:menu")
                .unwrap()
                .id("minecraft:furnace")
                .unwrap();
            let iron_ingot = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:item")
                .unwrap()
                .id("minecraft:iron_ingot")
                .unwrap();
            game.handle_packet(Clientbound::OpenScreen(OpenScreen {
                container: 7,
                menu: furnace_menu,
                title: Tag::String("Furnace".into()),
            }))
            .unwrap();

            let set_result = |game: &mut ClientGame<'_>, state: i32| {
                let mut items = vec![None; 39];
                items[2] = Some(Stack::new(iron_ingot, 2));
                game.handle_packet(Clientbound::ContainerSetContent(ContainerSetContent {
                    container: 7,
                    state,
                    items,
                    carried: None,
                }))
                .unwrap();
            };

            // Predicted pickup fires item_smelted with HostSide::Client and spawns no orbs.
            set_result(game, 1);
            assert!(game.remote_menu_ops(&[(Some(302), 0, Input::Pickup)]));
            {
                let mut seen = recorded.lock().unwrap();
                assert_eq!(seen.item_smelted.len(), 1);
                let call = &seen.item_smelted[0];
                assert!(call.is_client);
                assert_eq!(call.uuid, 42);
                assert_eq!(call.entity_id, 7);
                assert_eq!(call.item.as_deref(), Some("minecraft:iron_ingot"));
                assert_eq!(call.count, 2);
                assert_eq!(
                    call.projected,
                    Some((Some("minecraft:iron_ingot".to_owned()), 2))
                );
                assert_eq!(call.amount_removed, 2);
                seen.item_smelted.clear();
            }
            assert!(game.common.orbs.is_empty());

            // Predicted shift-click fires once with nonzero amount_removed and spawns no orbs.
            set_result(game, 2);
            assert!(game.remote_menu_ops(&[(Some(302), 0, Input::QuickMove)]));
            {
                let mut seen = recorded.lock().unwrap();
                assert_eq!(seen.item_smelted.len(), 1);
                assert!(seen.item_smelted[0].is_client);
                assert_eq!(seen.item_smelted[0].amount_removed, 2);
                assert_eq!(seen.item_smelted[0].count, 2);
                seen.item_smelted.clear();
            }
            assert!(game.common.orbs.is_empty());

            // Predicted hotbar swap has removeCount == 0: no item_smelted and no orbs.
            set_result(game, 3);
            assert!(game.remote_menu_ops(&[(Some(302), 1, Input::Swap)]));
            assert!(recorded.lock().unwrap().item_smelted.is_empty());
            assert!(game.common.orbs.is_empty());
        });
    });
}
