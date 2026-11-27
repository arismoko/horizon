use super::*;

impl crate::game::ClientGame<'_> {
    /// The opener position for the currently shown mod menu, if known.
    pub fn screen_pos(&self) -> Option<[i32; 3]> {
        self.shown_native_screen()?.menu.pos
    }
    /// Applies the `sunrise:menu_extra` payload sent before `OpenScreen`, or disconnects on error.
    pub(in crate::game) fn receive_menu_extra(
        &mut self,
        data: &[u8],
    ) -> sunrise_gameplay_common::game::Result<()> {
        match sunrise_protocol::menu_extra::decode(data) {
            Ok(extra) => {
                if let Some(r) = self.client.remote.as_mut() {
                    r.menu_extra = Some(extra);
                }
                Ok(())
            }
            Err(e) => {
                log::error!("Sunrise: malformed menu extra payload: {e}");
                let reason =
                    Text::translate("disconnect.genericReason", &[Text::literal(e.to_string())]);
                self.leave_server(Some(reason))
            }
        }
    }
}
