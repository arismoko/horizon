use sunrise_sim::first_person_sim::Hand;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The pose of the arm holding `stack` in `hand` (`IClientItemExtensions.getArmPose`); set
    /// `pose` to replace what the item would give. Client-side.
    pub(in crate::game) fn native_get_arm_pose(
        &self,
        entity_id: u32,
        hand: Hand,
        stack: &mod_api::ItemStack,
        humanoid: &mod_api::HumanoidRenderState<'_>,
        pose: &mut Option<mod_api::ArmPose>,
    ) {
        self.fire_get_arm_pose(entity_id, hand, stack, humanoid, pose);
    }
}
