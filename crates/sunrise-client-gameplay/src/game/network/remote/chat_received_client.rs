use super::*;
pub(in crate::game::client_side) use sunrise_gameplay_common::game::remote::chat_received::player_chat_info;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// NeoForge's `ClientChatReceivedEvent` (`ClientHooks.onClientChat`,
    /// `onClientPlayerChat` and `onClientSystemChat`): mods may replace
    /// `line` or cancel it. False when cancelled.
    pub(in crate::game::client_side) fn chat_received(
        &mut self,
        system: bool,
        overlay: bool,
        sender: u128,
        chat_type: Option<i32>,
        player: Option<&mod_api::PlayerChatInfo>,
        line: &mut sunrise_game_data::text::Text,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientChatReceived) {
            return true;
        }
        let registries = self
            .client
            .remote
            .as_ref()
            .map(|r| Arc::clone(&r.registries));
        let name = chat_type
            .zip(registries.as_deref())
            .and_then(|(id, registries)| registries.name("minecraft:chat_type", id));
        // A line the API's text form cannot hold is not told, as before.
        let Ok(mut message) = sunrise_sim::api_text::to_api(line) else {
            return true;
        };
        let mut cancel = false;
        self.native_client_chat_received(mod_api::__private::ClientChatReceivedCall {
            system,
            overlay,
            sender,
            chat_type: name,
            player_message: player,
            message: &mut message,
            cancel: &mut cancel,
        });
        if let Ok(text) = sunrise_sim::api_text::to_engine(&message) {
            *line = text;
        }
        !cancel
    }
}
