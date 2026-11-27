#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    /// A flying rocket's vertices carry its own id for frame generation's and a temporal
    /// upscaler's motion vectors (`Frame::entity_motion`), so the motion matched between
    /// two frames is its flight, not only the camera's.
    #[test]
    #[ignore = "daily"]
    fn a_flying_rocket_opens_a_motion_run_of_its_own() {
        use crate::settings::Settings;
        use sunrise_client_lib::audio::Audio;
        use sunrise_core::geometry::Vec3;
        use {
            crate::game::load_resources, sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::Progress,
        };

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
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
        let res = loaded.resources;
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            &res,
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
        let camera = glam::DVec3::new(0.5, 64.0, 0.5);
        let start = Vec3::new(0.5, 64.0, 4.5);
        game.rockets
            .push(sunrise_gameplay_common::game::fireworks::Rocket::remote(
                77,
                start,
                Vec3::ZERO,
            ));
        let id = sunrise_gameplay_common::game::MOTION_OBJECT | 77;
        // One frame's motion runs: the rocket's vertices open a run under its own id, and
        // a camera-only run (id 0) closes it so nothing drawn after takes its motion.
        game.client.entity_motion.borrow_mut().clear();
        let mut out = Vec::new();
        game.rocket_geometry(camera, 1.0, &mut out, 0..game.rockets.len());
        let marks = game.client.entity_motion.borrow().clone();
        let at = marks
            .iter()
            .position(|m| m.1 == id)
            .expect("the rocket's run");
        assert_eq!(
            marks[at].0, 0,
            "the run starts at the rocket's first vertex"
        );
        assert_eq!(marks.last(), Some(&(out.len() as u32, 0)));
    }

    pub(crate) fn assert_client_silent_rocket_burst_without_blast_or_twinkle(
        explosion: sunrise_client::particles::FireworkExplosion,
    ) {
        use sunrise_client::particles::ParticleLevel;
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play::{Clientbound as C, EntityEvent};
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5))
                    .unwrap(),
            );
            let (quiet_at, loud_at) = ([6.5, 201.5, 10.5], [12.5, 201.5, 10.5]);
            let mut quiet = sunrise_gameplay_common::game::fireworks::Rocket::remote(
                101,
                Vec3::new(quiet_at[0], quiet_at[1], quiet_at[2]),
                Vec3::ZERO,
            );
            quiet.silent = true;
            quiet.explosions = vec![explosion.clone()].into();
            let mut loud = sunrise_gameplay_common::game::fireworks::Rocket::remote(
                102,
                Vec3::new(loud_at[0], loud_at[1], loud_at[2]),
                Vec3::ZERO,
            );
            loud.explosions = vec![explosion].into();
            game.rockets.push(quiet);
            game.rockets.push(loud);
            for id in [101, 102] {
                game.apply_packet(C::EntityEvent(EntityEvent {
                    id,
                    event: sunrise_gameplay_common::game::fireworks::FIREWORK_EXPLODE_EVENT,
                }))
                .unwrap();
            }
            game.client.particles.0.sounds.clear();
            for _ in 0..20 {
                let (common, client) =
                    sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                let level = ParticleLevel {
                    world: &common.world,
                    facts: common.res.facts(),
                    collision: common.collision,
                    player: None,
                    cached_chunk: std::cell::Cell::new(None),
                };
                client.particles.0.tick(&level);
            }
            let bursts: Vec<(&str, [f64; 3])> = game
                .client
                .particles
                .0
                .sounds
                .iter()
                .filter(|s| s.0.starts_with("minecraft:entity.firework_rocket."))
                .map(|s| (s.0, s.1))
                .collect();
            assert_eq!(
                bursts,
                vec![
                    ("minecraft:entity.firework_rocket.blast", loud_at),
                    ("minecraft:entity.firework_rocket.twinkle", loud_at),
                ],
                "only the loud rocket bangs and twinkles, both at its own position"
            );
            assert!(
                !game.client.particles.0.is_empty(),
                "the silent rocket still drew its sparks and flash"
            );
        });
    }
}
