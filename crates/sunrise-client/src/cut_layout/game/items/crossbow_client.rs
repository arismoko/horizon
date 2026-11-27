#![cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sunrise_gameplay_common::game::crossbow_common::ARROW;

    /// `BowItem.releaseUsing` and `CrossbowItem.performShooting` play the shot
    /// with no player at the shooter's `getX/Y/Z`, the feet. At the eye, a
    /// source a few cm from the listener pans hard to one ear while strafing
    /// (#8448).
    #[test]
    #[ignore = "daily"]
    fn bow_and_crossbow_shots_sound_at_the_feet_not_the_eye() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let mut resources = None;
        let (mut game, feet) = game_with_player(&mut resources, &root);
        let eye = game.eye_position().unwrap();
        assert_ne!(eye, feet);

        let bow = ItemStack::new("minecraft:bow", 1);
        game.release_bow(&bow, 20);
        let shot = std::mem::take(&mut game.server.pending_sounds);
        let at: Vec<_> = shot
            .iter()
            .filter(|s| s.event == "minecraft:entity.arrow.shoot")
            .map(|s| s.position)
            .collect();
        assert_eq!(at, vec![feet]);

        game.start_audio_capture();
        let mut crossbow = ItemStack::new("minecraft:crossbow", 1);
        crossbow.data_mut().charged = vec![ItemStack::new(ARROW, 1)];
        *game.held_mut() = Some(crossbow);
        game.shoot_crossbow();
        let shot = std::mem::take(&mut game.server.pending_sounds);
        let at: Vec<_> = shot
            .iter()
            .filter(|s| s.event == "minecraft:item.crossbow.shoot")
            .map(|s| s.position)
            .collect();
        assert_eq!(at, vec![feet]);
    }

    /// A game on the prepared resources with a creative player.
    pub(crate) fn game_with_player<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        root: &std::path::Path,
    ) -> (sunrise_gameplay::game::ServerGame<'a>, [f64; 3]) {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            Vec::new(),
            None,
            root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.to_path_buf();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let res = resources.insert(loaded.resources);
        let mut game = sunrise_gameplay::game::ServerGame::new(
            res,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.creative = true;
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(10.5, 64.0, -3.25),
            )
            .unwrap(),
        );
        let p = game.player.as_ref().expect("a player").position;
        game.start_audio_capture();
        (game, [p.x, p.y, p.z])
    }
}
