use crate::game::villager_client::ResolvedProfessions;

pub fn meta_hub(name: &str) -> sunrise_sim::lan::hub::Hub {
    profession_hub(name, false)
}
/// A hub with no players and empty registries, for `mob_meta`.
pub fn profession_hub(name: &str, vanilla_professions: bool) -> sunrise_sim::lan::hub::Hub {
    use sunrise_sim::lan::{Lan, NetConfig, chunks::ChunkEncoder, data, hub};
    let mut sync = data::SyncData::empty();
    for (registry, id) in [
        ("minecraft:villager_type", "minecraft:plains"),
        ("minecraft:villager_profession", "minecraft:none"),
    ] {
        sync.builtin.registries.insert(
            registry.into(),
            sunrise_protocol::registry::Ids::from_names(vec![id.into()]),
        );
    }
    if vanilla_professions {
        sync.builtin.registries.insert(
            "minecraft:villager_profession".into(),
            sunrise_protocol::registry::Ids::from_names(
                ResolvedProfessions::vanilla()
                    .entries()
                    .iter()
                    .map(|entry| {
                        if entry.id.contains(':') {
                            entry.id.clone()
                        } else {
                            format!("minecraft:{}", entry.id)
                        }
                    })
                    .collect(),
            ),
        );
    }
    let sync = std::sync::Arc::new(sync);
    let config = NetConfig {
        announce: false,
        listen: false,
        ..NetConfig::loopback(0, name.into(), 2)
    };
    let lan = Lan::start_with(sync.clone(), config).unwrap();
    let encoder = ChunkEncoder::new(
        [] as [(std::sync::Arc<str>, bool); 0],
        &sunrise_protocol::registry::Builtin::default(),
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
