//! The host's build metadata agrees with the chunk writer, before any world is open.
#![cfg(feature = "client")]
use mod_api::{ClientHost, Host, ServerHost};
use std::sync::Arc;
use sunrise_core::nbt::{Limits, Tag};
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_level::chunk::Chunk;
use sunrise_sim::save::{LevelShape, SaveDir};
use sunrise_world::region::RegionFile;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

#[test]
#[ignore = "daily"]
fn host_data_version_matches_a_freshly_saved_chunk() {
    with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        let save = SaveDir::new(
            scratch.path().to_path_buf(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        assert_eq!(
            save.save(
                &[Arc::new(Chunk::empty(0, 0))],
                game.pack.assets.registry,
                game.res.biomes(),
            )
            .unwrap(),
            1
        );
        // Read the actual region record without the save loader's datafixes.
        let saved = RegionFile::open(scratch.path().join("region/r.0.0.mca"))
            .unwrap()
            .read(ChunkPos::new(0, 0))
            .unwrap()
            .expect("saved chunk")
            .nbt(Limits::saved())
            .unwrap();
        let Some(Tag::Int(version)) = saved.tag.get("DataVersion") else {
            panic!("saved chunk must have an integer DataVersion");
        };
        assert_eq!(*version, sunrise_core::version::DATA_VERSION);

        for role in [
            ExecutionRole::IntegratedServer,
            ExecutionRole::DedicatedServer,
        ] {
            game.options.execution_role = role;
            let server: &dyn ServerHost = game;
            let host: &dyn Host = server;
            assert_eq!(host.server_data_version(), *version);
            assert_eq!(host.server_data_version(), *version);
        }
        with_client_game(|client_game| {
            let client: &dyn ClientHost = client_game;
            let host: &dyn Host = client;
            assert_eq!(host.server_data_version(), *version);
            assert_eq!(host.server_data_version(), *version);
        });
    });
}
