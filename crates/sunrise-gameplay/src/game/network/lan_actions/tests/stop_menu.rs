pub struct StopMenu(pub Option<mod_api::GameMode>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for StopMenu {
    fn container_opened(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ContainerOpenedEvent<'_>,
    ) -> mod_api::Flow {
        if let Some(mode) = self.0 {
            let player = host.players().acting_player().unwrap();
            assert!(player.set_game_mode(host, mode));
        }
        mod_api::Flow::Cancel
    }
}
