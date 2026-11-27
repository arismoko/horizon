//! Common `AttachmentHost` impl and shared level-weather helpers across server and client roles.

use mod_api::AttachmentAccess;

pub fn weather_for_level(
    shape: sunrise_level::chunk::LevelShape,
    is_end: bool,
    levels: (f32, f32),
) -> (f32, f32) {
    if shape.can_have_weather(is_end) {
        levels
    } else {
        (0.0, 0.0)
    }
}

pub trait GameAttachmentHost<'g>: crate::game::SideGame<'g> {
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)
    }

    fn validate_attachment_key(
        &self,
        write: &mod_api::AttachmentStoreKey<'_>,
    ) -> Result<(), mod_api::AttachmentError> {
        mod_api::__private::validate_attachment_key(write)
    }

    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        self.side_attachment_map(holder, access)
    }
}

use crate::game::side_ops::modapi::ModapiSideGameExt as _;

impl<'g, G: crate::game::SideGame<'g>> GameAttachmentHost<'g> for G {}
