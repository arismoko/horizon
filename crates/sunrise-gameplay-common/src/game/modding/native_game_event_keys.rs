//! Startup game-event registry reads.
impl crate::game::CommonGame<'_> {
    pub fn game_event_key(&self, id: &str) -> Option<mod_api::GameEventKey> {
        mod_api::__private::game_event_key(id)
    }
    pub fn game_event_key_id(
        &self,
        key: mod_api::GameEventKey,
    ) -> Result<mod_api::RegistryName, mod_api::GameEventKeyError> {
        mod_api::__private::game_event_key_id(key).ok_or(mod_api::GameEventKeyError::InvalidKey)
    }
}
