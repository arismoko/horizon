use super::Game;

impl Game<'_> {
    /// Collect the mod configuration tasks for a joining profile.
    pub(in crate::game) fn native_register_configuration_tasks(
        &mut self,
        player: u128,
        name: &str,
        brand: &str,
        tasks: &mut Vec<mod_api::ConfigurationTask>,
    ) {
        self.fire_register_configuration_tasks(player, name, brand, tasks);
    }
}
