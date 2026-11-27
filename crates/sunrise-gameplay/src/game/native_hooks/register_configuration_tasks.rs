impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn native_register_configuration_tasks(
        &mut self,
        connection: mod_api::ConfigurationConnection,
        player: u128,
        name: &str,
        registration: &mut mod_api::__private::ConfigurationTaskRegistration,
    ) {
        self.fire_register_configuration_tasks(connection, player, name, registration);
    }
}
