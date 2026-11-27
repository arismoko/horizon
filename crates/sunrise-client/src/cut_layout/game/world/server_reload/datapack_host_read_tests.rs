#![cfg(feature = "client")]
use super::*;
use mod_api::Host;
use mod_api::{
    DatapackEntry, DatapackField, DatapackFieldKind, DatapackReadError as E, DatapackRegistrySync,
    LevelRef, Registrar, RegistrationContext, WorldEpoch,
};
use std::ops::ControlFlow;
use sunrise_client_gameplay::game::client_side::{
    native_client_session::tests::logged_client, remote_client::session_state::RemoteDimensionEntry,
};
use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;

struct SyncedRegistryMod;
#[mod_api::__subscriptions]
impl Hooks for SyncedRegistryMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for (name, sync) in [
            ("spell", DatapackRegistrySync::Required),
            ("server_only", DatapackRegistrySync::ServerOnly),
        ] {
            r.declare_datapack_registry(
                name,
                sync,
                vec![DatapackField::required(
                    "power",
                    DatapackFieldKind::Int { min: 0, max: 100 },
                )],
            )
            .unwrap();
        }
    }
}
fn assert_spell_ids(host: &dyn Host, level: LevelRef, expected: &[&str]) {
    let mut ids = Vec::new();
    assert_eq!(
        level.visit_datapack_entries(host, "registry_test:spell", &mut |id| {
            ids.push(id.to_owned());
            ControlFlow::Continue(())
        }),
        Ok(ControlFlow::Continue(()))
    );
    assert_eq!(ids, expected);
}

#[test]
#[ignore = "daily"]
fn native_datapack_host_reads_enforce_level_session_and_generation() {
    if run_fresh_cases(
        "cut_layout::game::server_side::server_reload::tests::datapack_publication_tests::datapack_host_read_tests::native_datapack_host_reads_enforce_level_session_and_generation",
        "SUNRISE_DATAPACK_HOST_READ_CHILD",
        &["host-reads"],
    ) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(SyncedRegistryMod))].into(),
        || {
            // A Cell: the server and the client closure both read it while the visitor is live.
            let visited_refusal = std::cell::Cell::new(false);
            let mut unexpected_visit = |_: &DatapackEntry| visited_refusal.set(true);
            let mut unexpected_id = |_: &str| -> ControlFlow<()> { panic!("unexpected entry ID") };
            let mut server_registries = None;
            with_game(|game| {
                game.load_world(request(scratch.path())).unwrap();
                let candidate = |power: i64| {
                    mod_api::DatapackCandidate::build(
                        mod_api::__private::datapack_registry_declarations(),
                        &[
                            (
                                "data/registry_test/registry_test/spell/fire.json".into(),
                                vec![(
                                    "pack".to_owned(),
                                    format!(r#"{{"power":{power}}}"#).into_bytes(),
                                )],
                            ),
                            (
                                "data/addon/registry_test/spell/ice.json".into(),
                                vec![("pack".to_owned(), br#"{"power":3}"#.to_vec())],
                            ),
                        ],
                        &mut mod_api::__private::NoVanillaValueDecoder,
                    )
                    .unwrap()
                };
                game.mods.datapack_registries = Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::from_candidate(candidate(7)),
                ));
                let level = game.level(game.dim.id()).unwrap();
                assert_spell_ids(game, level, &["addon:ice", "registry_test:fire"]);
                assert_eq!(
                    level.visit_datapack_entries(game, "registry_test:spell", &mut |id| {
                        assert_eq!(id, "addon:ice");
                        ControlFlow::Break(())
                    }),
                    Ok(ControlFlow::Break(()))
                );
                let published = game
                    .mods
                    .datapack_registries
                    .as_ref()
                    .unwrap()
                    .get("registry_test:spell", "registry_test:fire")
                    .unwrap();
                let mut visits = 0;
                level
                    .datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut |entry| {
                            assert!(std::ptr::eq(entry, published));
                            assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(7))]);
                            visits += 1;
                        },
                    )
                    .unwrap();
                assert_eq!(visits, 1);
                assert_eq!(
                    level.visit_datapack_entries(game, "missing:registry", &mut unexpected_id),
                    Err(E::UnknownRegistry)
                );
                assert_eq!(
                    level.datapack_read(
                        game,
                        "missing:registry",
                        "addon:ice",
                        &mut unexpected_visit,
                    ),
                    Err(E::UnknownRegistry)
                );
                assert_eq!(
                    level.datapack_read(
                        game,
                        "registry_test:spell",
                        "addon:missing",
                        &mut unexpected_visit,
                    ),
                    Err(E::MissingEntry)
                );
                for refused in [
                    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                    LevelRef::with_generation(
                        level.epoch(),
                        level.dimension(),
                        level.generation() + 1,
                    ),
                    LevelRef::new(
                        level.epoch(),
                        mod_api::__private::dimension_key("registry_test:unopened"),
                    ),
                ] {
                    assert_eq!(
                        refused.visit_datapack_entries(
                            game,
                            "registry_test:spell",
                            &mut unexpected_id
                        ),
                        Err(E::UnknownRegistry)
                    );
                    assert_eq!(
                        refused.datapack_read(
                            game,
                            "registry_test:spell",
                            "registry_test:fire",
                            &mut unexpected_visit,
                        ),
                        Err(E::UnknownRegistry)
                    );
                }

                let parked = game.level("minecraft:the_nether").unwrap();
                assert_ne!(parked.dimension(), level.dimension());
                assert_spell_ids(game, parked, &["addon:ice", "registry_test:fire"]);
                let mut parked_visits = 0;
                parked
                    .datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut |entry| {
                            assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(7))]);
                            parked_visits += 1;
                        },
                    )
                    .unwrap();
                assert_eq!(parked_visits, 1);
                game.mods.datapack_registries = Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::from_candidate(candidate(9)),
                ));
                let mut reloaded_visits = 0;
                level
                    .datapack_read(
                        game,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut |entry| {
                            assert_eq!(entry.values, [("power".to_owned(), DatapackValue::Int(9))]);
                            reloaded_visits += 1;
                        },
                    )
                    .unwrap();
                assert_eq!(reloaded_visits, 1);
                server_registries.clone_from(&game.mods.datapack_registries);
            });
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|client| {
                client.mods.datapack_registries = server_registries;
                let declarations = mod_api::__private::datapack_registry_declarations();
                let remote = mod_api::DatapackCandidate::build(
                    declarations,
                    &[(
                        "data/registry_test/registry_test/spell/fire.json".into(),
                        vec![("pack".to_owned(), br#"{"power":19}"#.to_vec())],
                    )],
                    &mut mod_api::__private::NoVanillaValueDecoder,
                )
                .unwrap();
                sunrise_sim::datapack_net::serve(Some(std::sync::Arc::new(
                    mod_api::DatapackRegistrySet::from_candidate(remote),
                )));
                let offer = sunrise_sim::datapack_net::offer()
                    .unwrap()
                    .expect("synchronized registry offer required");
                let sunrise_sim::datapack_net::Step::Send(bytes) =
                    sunrise_sim::datapack_net::server_step(true, Some(&offer))
                else {
                    panic!("snapshot required");
                };
                let _server = logged_client(client);
                client.enter_remote_dimension(
                    &sunrise_protocol::packets::play::SpawnInfo {
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
                    },
                    RemoteDimensionEntry::Login,
                    sunrise_client_render::level_loading::Reason::Other,
                );
                let receiver = &mut client.client.remote.as_mut().unwrap().datapacks;
                for chunk in sunrise_sim::datapack_net::chunks(&bytes).unwrap() {
                    receiver.accept_chunk(&chunk).unwrap();
                }
                let level = client.level("minecraft:overworld").unwrap();
                assert_eq!(
                    level.visit_datapack_entries(
                        client,
                        "registry_test:server_only",
                        &mut unexpected_id
                    ),
                    Ok(ControlFlow::Continue(())),
                    "an omitted declared registry is an empty level listing"
                );
                let mut visits = 0;
                level
                    .datapack_read(
                        client,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut |entry| {
                            assert_eq!(entry.get("power"), Some(&DatapackValue::Int(19)));
                            visits += 1;
                        },
                    )
                    .unwrap();
                assert_eq!(visits, 1);
                client.client.remote.as_mut().unwrap().datapacks = Default::default();
                assert_eq!(
                    level.visit_datapack_entries(client, "registry_test:spell", &mut unexpected_id),
                    Err(E::UnknownRegistry),
                    "the next enumeration sees that the publication is gone"
                );
                assert_eq!(
                    level.datapack_read(
                        client,
                        "registry_test:spell",
                        "registry_test:fire",
                        &mut unexpected_visit,
                    ),
                    Err(E::UnknownRegistry)
                );
                assert!(!visited_refusal.get());
            });
        },
    );
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
