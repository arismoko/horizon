use super::*;

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn text_width(&self, text: &mod_api::Text) -> i32 {
        let engine = sunrise_sim::api_text::to_engine_inprocess(text);
        self.client.gui.atlas.font.width_text(&engine) as i32
    }

    pub(in crate::game) fn language_right_to_left(&self) -> bool {
        sunrise_client::lang::bidirectional()
    }

    pub(in crate::game) fn text_split(
        &self,
        text: &mod_api::Text,
        width: i32,
    ) -> Vec<mod_api::Text> {
        let local = sunrise_sim::api_text::to_local_engine(text);
        self.client
            .gui
            .atlas
            .font
            .split_formatted(&local.engine().resolved(), width as f32)
            .iter()
            .map(|line| local.text_to_api(line))
            .collect()
    }
}
