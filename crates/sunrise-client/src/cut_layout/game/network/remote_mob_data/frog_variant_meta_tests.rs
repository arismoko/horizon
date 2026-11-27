//! `Frog.DATA_VARIANT_ID` (entry 18) carries the synchronized `frog_variant`
//! registry's network id: what the host sends is what the client stores.
use sunrise_client_gameplay::game::remote_mob_data_test_support::{
    apply_registry_data, fixture_mob,
};
use sunrise_gameplay_common::game::lan_mob_metadata::mob_meta;
use sunrise_protocol::entity_data::{Value, serializer};
use sunrise_protocol::packets::configuration::{RegistryData, RegistryEntry};
use sunrise_protocol::registry::Builtin;
use sunrise_sim::lan::data::{RegistryRef, SyncData};

fn meta_hub_with(sync: SyncData) -> sunrise_sim::lan::hub::Hub {
    use sunrise_sim::lan::{Lan, NetConfig, chunks::ChunkEncoder, hub};
    let sync = std::sync::Arc::new(sync);
    let config = NetConfig {
        announce: false,
        listen: false,
        ..NetConfig::loopback(0, "FrogVariantMetaTest".into(), 2)
    };
    let lan = Lan::start_with(sync.clone(), config).unwrap();
    let encoder = ChunkEncoder::new(
        [] as [(std::sync::Arc<str>, bool); 0],
        &Builtin::default(),
        &[],
        &[],
        Default::default(),
    );
    let hub_config = hub::HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 10,
        simulation_distance: 10,
        dimension: "minecraft:overworld".into(),
        allow_flight: false,
        seed: 0,
        max_players: 20,
        integrated: false,
        levels: Default::default(),
        level_seeds: Default::default(),
    };
    hub::Hub::new(
        lan,
        sync,
        encoder,
        hub_config,
        std::sync::Arc::new(
            sunrise_gameplay_common::game::test_server_load()
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
        ),
    )
}

fn sent(m: &sunrise_sim::mobs::Mob, hub: &sunrise_sim::lan::hub::Hub) -> Vec<(u8, i32, Value)> {
    let (mut meta, mut defaults) = (Vec::new(), Vec::new());
    mob_meta(m, hub, None, 0, &mut meta, &mut defaults);
    let default = defaults.iter().find(|(index, _, _)| *index == 18);
    assert_eq!(
        default,
        Some(&(18, serializer::FROG_VARIANT, Value::Id(0))),
        "the default is minecraft:temperate's id"
    );
    meta.into_iter()
        .filter(|(index, _, _)| *index == 18)
        .collect()
}

#[test]
fn a_pack_frog_variant_reaches_the_client_by_network_id() {
    let mut sync = SyncData::empty();
    sync.dynamic.add(RegistryData {
        registry: "minecraft:frog_variant".to_owned(),
        entries: ["minecraft:temperate", "test:glow"]
            .into_iter()
            .map(|id| RegistryEntry {
                id: id.to_owned(),
                data: None,
            })
            .collect(),
    });
    let hub = meta_hub_with(sync);

    let mut host = fixture_mob("minecraft:frog");
    host.extra.frog_variant = Some(RegistryRef::new(1));
    let meta = sent(&host, &hub);
    assert_eq!(meta, vec![(18, serializer::FROG_VARIANT, Value::Id(1))]);

    let mut client = fixture_mob("minecraft:frog");
    apply_registry_data(&mut client, &meta, 0);
    assert_eq!(client.extra.frog_variant, Some(RegistryRef::new(1)));

    // The default variant is not sent: the client keeps its own default.
    host.extra.frog_variant = Some(RegistryRef::new(0));
    assert!(sent(&host, &hub).is_empty());
}
