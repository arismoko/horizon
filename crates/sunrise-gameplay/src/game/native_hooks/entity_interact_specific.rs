use sunrise_items::items::ItemStack;

impl crate::game::ServerGame<'_> {
    /// a right-click on an entity with the point clicked (`native_entity_interact_specific`).
    pub fn native_entity_interact_specific(
        &mut self,
        entity: i32,
        kind: &str,
        stack: Option<&ItemStack>,
        location: [f64; 3],
        claimed: &mut bool,
    ) {
        self.fire_entity_interact_specific(entity, kind, stack, location, claimed);
    }
}
