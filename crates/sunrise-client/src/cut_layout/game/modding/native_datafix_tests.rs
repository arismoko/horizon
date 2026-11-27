//! Standalone upgrades through a mod callback on each logical side.
#![cfg(feature = "client")]
use mod_api::{
    Compound, Hooks, HostSide, LevelTickPreEvent, LogicalSide, ModEntry, NbtDataType,
    NbtUpgradeError, Tag,
};
use std::sync::{Arc, Mutex};
use sunrise_core::nbt::parse_snbt;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

fn compound(text: &str) -> Compound {
    match parse_snbt(text).unwrap().0 {
        Tag::Compound(c) => c,
        other => panic!("expected a compound, got {other:?}"),
    }
}

struct Upgrade(Arc<Mutex<Vec<LogicalSide>>>);

#[mod_api::__subscriptions]
impl Hooks for Upgrade {
    fn level_tick_pre(&self, host: HostSide<'_>, _: &mut LevelTickPreEvent) {
        let side = match host {
            HostSide::Client(host) => {
                let wool = compound(
                    r#"{id:"minecraft:wool",Damage:14s,Count:3b,tag:{display:{Name:"Quest reward"}}}"#,
                );
                assert_eq!(
                    host.upgrade_nbt(NbtDataType::ItemStack, 1343, wool),
                    Ok(compound(
                        r#"{id:"minecraft:red_wool",count:3,components:{"minecraft:custom_name":{text:"Quest reward"}}}"#
                    )),
                );
                LogicalSide::Client
            }
            HostSide::Server(host) => {
                // DataVersion 1900: Profession 1 with Career 2 is a cartographer.
                let villager =
                    compound(r#"{id:"minecraft:villager",Profession:1,Career:2,CareerLevel:1}"#);
                let upgraded = Tag::Compound(
                    host.upgrade_nbt(NbtDataType::Entity, 1900, villager)
                        .unwrap(),
                );
                assert_eq!(
                    upgraded
                        .get("VillagerData")
                        .and_then(|data| data.get("profession")),
                    Some(&Tag::String("minecraft:cartographer".into())),
                );
                let future = sunrise_world::datafix::CURRENT + 1;
                assert_eq!(
                    host.upgrade_nbt(
                        NbtDataType::BlockEntity,
                        future,
                        compound(r#"{id:"minecraft:sign"}"#)
                    ),
                    Err(NbtUpgradeError::FutureVersion(future)),
                );
                LogicalSide::Server
            }
        };
        // A failed assertion unwinds inside the dispatcher, so record the side only on success.
        self.0.lock().unwrap().push(side);
    }
}

#[test]
#[ignore = "daily"]
fn a_mods_host_upgrades_old_data_and_refuses_by_name() {
    const CHILD: &str = "SUNRISE_NATIVE_DATAFIX_CHILD";
    if run_fresh_cases(
        "cut_layout::game::native_datafix_tests::a_mods_host_upgrades_old_data_and_refuses_by_name",
        CHILD,
        &["client", "server"],
    ) {
        return;
    }
    // The game fixture installs registries. Keep each role in its own process.
    sunrise_world::datafix::set_resources(&fixture_root());
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "nbt_upgrade",
        Box::new(Upgrade(observed.clone())),
    )]
    .into();
    let expected = match std::env::var(CHILD).unwrap().as_str() {
        "client" => LogicalSide::Client,
        "server" => LogicalSide::Server,
        case => panic!("unexpected datafix fixture case: {case}"),
    };
    mod_platform::testing::with_mods(mods, || {
        match expected {
            LogicalSide::Client => with_client_game(|game| {
                use sunrise_client::level_loading::Reason;
                use sunrise_protocol::{connection::Pipe, packets::play::SpawnInfo};
                use {
                    sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry,
                    sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle,
                };

                game.native_lifecycle = NativeLifecycle::default();
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "DatafixTest").unwrap();
                let spawn = SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 1,
                    previous_game_type: None,
                    debug: false,
                    flat: true,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
                game.client.remote.as_mut().unwrap().logged_in = true;
                game.tick_remote_level().unwrap();
            }),
            LogicalSide::Server => with_game(|game| {
                let level = game.level("minecraft:overworld").unwrap();
                game.native_level_tick_pre(level);
            }),
        }
        assert_eq!(*observed.lock().unwrap(), [expected]);
    });
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
