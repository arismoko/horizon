#![cfg(test)]

#[cfg(test)]
pub(crate) mod conduit_target_tests {
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};
    use {
        crate::game::server_side::block_entity_anims::conduit_target_tests::game,
        sunrise_gameplay::game::server_side::block_entity_anims::conduit_target_tests::attack_conduit_for_test,
        sunrise_gameplay::game::server_side::block_entity_anims::conduit_target_tests::summon,
        sunrise_gameplay_common::game::server_side::block_entity_anims::conduit_target_tests::CONDUIT,
    };
    use {
        crate::game::{Audio, Settings, load_resources},
        sunrise_gameplay_common::game::Options,
        sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role,
    };

    fn game_with_client(
        resources: &mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
        role: execution_role::ExecutionRole,
    ) -> sunrise_client_gameplay::game::ClientGame<'_> {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            role,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = role;
        let res = resources.insert(loaded.resources);
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            res,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game
    }

    /// A window of one: a client game with no conduit state of its own, and a
    /// conduit block whose block entity arrives from the server.
    fn client(
        resources: &mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_client_gameplay::game::ClientGame<'_> {
        let mut game = game_with_client(resources, execution_role::ExecutionRole::Client);
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Window").unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:conduit")
            .expect("the conduit's block state");
        game.world
            .set(CONDUIT[0], CONDUIT[1], CONDUIT[2], state)
            .unwrap();
        // The block entity a chunk's block entities build (`from_name`).
        game.world.block_entities.insert(
            (CONDUIT[0], CONDUIT[1], CONDUIT[2]),
            BlockEntity::Conduit(None),
        );
        game
    }

    /// `updateAndAttackTarget` puts the target in the block entity's `Target`
    /// and sends it (`sendBlockUpdated`); the window loads that tag, finds the
    /// mob and `animationTick` draws one nautilus particle at its eye. A tag
    /// without a `Target` takes the mob away again.
    #[test]
    #[ignore = "daily"]
    fn a_conduits_target_reaches_the_client_as_a_particle() {
        let mut server_resources = None;
        let mut server = game(&mut server_resources);
        let drowned = summon(&mut server, "drowned");
        server.world.block_log = Some(Vec::new());
        attack_conduit_for_test(&mut server, CONDUIT, true);
        let uuid = server.persist.uuids[&drowned];
        let key = (CONDUIT[0], CONDUIT[1], CONDUIT[2]);
        // What `sendBlockUpdated` puts on the wire.
        let tag = sunrise_sim::lan::chunks::entity_tag(
            &server.world.block_entities[&key],
            false,
            &Default::default(),
        )
        .expect("a conduit has an update tag");
        assert_eq!(tag.get("Target"), Some(&Tag::IntArray(uuid.to_vec())));
        assert!(
            server.world.block_log.as_ref().unwrap().contains(&key),
            "the new target is sent"
        );
        // The window: no `conduit_servers` of its own, the same mob, and the
        // server's tag through `handleBlockEntityData`.
        let mut client_resources = None;
        let mut client = client(&mut client_resources);
        let watched = 77_u32;
        client.spawn_entity(
            watched as i32,
            "minecraft:drowned",
            [4.5, 100.0, 8.5],
            [0.0; 3],
        );
        client.persist.uuids.insert(watched, uuid);
        let kind = client
            .client
            .remote
            .as_ref()
            .and_then(|r| r.builtin.get("minecraft:block_entity_type"))
            .and_then(|ids| ids.id("minecraft:conduit"))
            .expect("the conduit's block entity type");
        client.remote_block_entity_data(
            CONDUIT,
            kind,
            mod_api::BlockEntityDataSource::UpdatePacket,
            &tag,
        );
        let target = client
            .test_conduit_target(CONDUIT)
            .expect("the window sees the hunted mob");
        let (position, eye_y) = {
            let m = client.mobs.iter().find(|m| m.id == watched).unwrap();
            (m.position, m.eye_y())
        };
        assert_eq!(target.eye_y, eye_y);
        let mut anims = sunrise_client::block_entity_anim::Conduit::default();
        let particles = anims.client_tick(
            CONDUIT,
            1,
            |_| true,
            |_| false,
            Some(target),
            &mut client.world.random,
        );
        // One particle, at the eye: a frame's own particles need a frame block.
        assert_eq!(particles.len(), 1);
        assert_eq!(particles[0].0, [position.x, eye_y, position.z]);
        // The conduit stopped hunting: its empty tag drops the target.
        client.remote_block_entity_data(
            CONDUIT,
            kind,
            mod_api::BlockEntityDataSource::UpdatePacket,
            &Tag::Compound(Default::default()),
        );
        assert!(
            client.test_conduit_target(CONDUIT).is_none(),
            "the target is dropped"
        );
    }
}
