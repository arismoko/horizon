use sunrise_gameplay_common::game::lan_actions::guest_within_entity_range;

impl crate::game::ServerGame<'_> {
    /// `AbstractMountInventoryMenu.stillValid`: the mount is alive and the
    /// guest's eyes are within the entity interaction range (3, 5 in creative)
    /// plus 4 of its box (`Player.isWithinEntityInteractionRange(mount, 4.0)`).
    pub fn lan_mount_valid(&self, g: &sunrise_sim::lan::guest::Guest, mob: u32) -> bool {
        self.common
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .is_some_and(|m| m.alive() && guest_within_entity_range(g, &m.bounds(), 4.0))
    }
}
