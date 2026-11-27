use mod_api::ScreenshotImage;
use sunrise_game_data::text::Text;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// An F2 frame is read back; mods may change its image, cancel, rename it or replace its line.
    pub(in crate::game) fn native_screenshot(
        &mut self,
        image: &mut ScreenshotImage,
        file_name: &mut String,
        message: &mut Option<Text>,
        cancel: &mut bool,
    ) {
        self.fire_screenshot(image, file_name, message, cancel);
    }
}
