use super::*;

impl TryIntoApi for engine::Style {
    type Api = mod_api::text::TextStyle;
    type Error = TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        sunrise_sim::api_text::style_to_api(self)
    }
}
