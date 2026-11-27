#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Read-only client dispatch at the actual mid-setupAnim seam.
    pub(in crate::game) fn native_humanoid_setup_anim(
        &self,
        call: mod_api::__private::HumanoidSetupAnimCall<'_>,
    ) {
        self.fire_humanoid_setup_anim(call);
    }
}
