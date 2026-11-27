#![cfg(feature = "client")]
#![cfg(test)]
//! Prepared-resource proof of the API request, server queue and client model resolution.
use {crate::game::*, sunrise_gameplay_common::game::ItemStack};
use {
    sunrise_gameplay_common::game::api_values::TryIntoApi,
    sunrise_gameplay_common::game::api_values::TryIntoEngine,
};

#[test]
#[ignore = "daily"]
fn stack_particle_crosses_server_packet_and_resolves_apple() {
    let mut transfer = None;
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        // The game fixture resolves the particle providers, as content_root does.
        let stack = ItemStack::new("minecraft:apple", 1);
        let api_stack = stack.try_into_api().unwrap();
        let key = mod_api::__private::particle_named_key("minecraft:item").unwrap();
        let options = mod_api::ParticleOptions::item_stack(key, api_stack.clone());
        let level = game.level(game.dim.id()).unwrap();
        assert!(level.send_particles(game, &options, None, [0.0, 64.0, 0.0], [0.2; 3], 0.0, 5));
        let queued = game.server.pending_particles.pop().unwrap();
        let mut sync = sunrise_sim::lan::data::SyncData::empty();
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        sync.builtin = sunrise_protocol::registry::Builtin::load(
            &sunrise_protocol::registry::Builtin::find(&root).unwrap(),
        )
        .unwrap();
        let sync = std::sync::Arc::new(sync);
        let (hub, _, _) = sunrise_gameplay::game::server_side::lan_particles::tests::test_hub(
            "Item particle",
            vec![],
            Some(sync.clone()),
        );
        let particle = hub.server_particle(&queued.particle).unwrap();
        use sunrise_protocol::packets::play::{self, Clientbound};
        let packet = Clientbound::LevelParticles(play::LevelParticles {
            particle,
            override_limiter: false,
            always_show: false,
            pos: queued.at,
            dist: queued.spread.map(|v| v as f32),
            max_speed: queued.speed.map(|v| v as f32),
            count: queued.count,
            randomization: 0,
        })
        .encode();
        let mut oversized = api_stack;
        oversized.set_count(99).unwrap();
        let bad = mod_api::ParticleOptions::item_stack(key, oversized);
        assert!(bad.try_into_engine(game.items()).is_err());
        assert!(!level.send_particles(game, &bad, None, [0.0; 3], [0.0; 3], 0.0, 1));
        assert!(game.server.pending_particles.is_empty());
        transfer = Some((packet, sync));
    });
    let (packet, sync) = transfer.unwrap();
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        use sunrise_protocol::packets::play::Clientbound;
        let mut reader = sunrise_protocol::buf::Reader::new(&packet);
        let id = reader.varint().unwrap();
        let Clientbound::LevelParticles(decoded) = Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("not a particle packet");
        };
        assert_eq!(decoded.count, 5);
        let particle = decoded.particle;
        let sunrise_protocol::entity_data::ParticleData::Item(network_stack) =
            particle.data.unwrap()
        else {
            panic!("not an item particle")
        };
        assert_eq!(
            sync.builtin
                .get("minecraft:item")
                .unwrap()
                .name(network_stack.item),
            Some("minecraft:apple")
        );
        let received =
            sunrise_sim::item_sync::from_network(&network_stack, &sync.builtin, &sync.dynamic);
        assert_eq!(&*received.item, "minecraft:apple");
        assert_eq!(
            game.item_particle_options(&received).unwrap().item,
            Some(vec!["item/apple".into()])
        );
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
