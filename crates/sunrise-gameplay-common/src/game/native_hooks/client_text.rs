use super::*;
impl crate::game::CommonGame<'_> {
    pub fn text_string(&self, text: &mod_api::Text) -> String {
        sunrise_sim::api_text::to_engine_inprocess(text).plain()
    }
}
