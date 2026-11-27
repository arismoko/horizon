#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use crate::game::{Audio, Options, Progress, Settings, load_resources};

    use std::sync::Arc;

    use sunrise_core::geometry::Vec3;

    use sunrise_level::chunk::Chunk;

    use sunrise_sim::mobs::Mob;

    /// A game on the prepared resources with an empty chunk at (0, 0), the
    /// window's player (the camera the dust needs) and one breeze standing
    /// with its feet at `feet`.
    pub(crate) fn game_with_breeze<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        feet: Vec3,
    ) -> sunrise_client_gameplay::game::ClientGame<'a> {
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
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        let res = &*resources.insert(loaded.resources);
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
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        // Eight blocks south of the breeze: inside the 32 blocks the dust needs.
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 66.0, 16.5))
                .unwrap(),
        );
        sunrise_client_gameplay::game::client_side::breezes_client::tests::set_test_particle_camera(
            &mut game,
            [8.5, 66.0, 16.5],
        );
        let kind = game.res.mob_types()["minecraft:breeze"].clone();
        game.mobs.push(Mob::new(13001, kind, feet, 0.0));
        game
    }
}
