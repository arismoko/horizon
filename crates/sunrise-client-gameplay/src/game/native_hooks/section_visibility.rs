#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A section in the player's level is about to mesh (`section_visibility`): each listener
    /// may return one answer from the raw section and its border. Client world thread only.
    pub(in crate::game) fn native_section_visibility(
        &self,
        call: mod_api::__private::SectionVisibilityCall<'_>,
    ) {
        self.fire_section_visibility(call);
    }
}
