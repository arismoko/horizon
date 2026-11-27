use crate::game::ServerGame;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::spill_common as spill;
use sunrise_items::items::ItemStack;
use sunrise_sim::entities::ItemEntity;

impl ServerGame<'_> {
    /// `new ItemEntity(..)`: the item at the end of `dropped`, not yet joined
    /// (`join_item_entity`). Returns its motion.
    pub fn push_item_entity(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: i32,
        random: Option<spill::ItemRandom>,
    ) -> [f64; 3] {
        let random = random.unwrap_or_else(|| spill::prepare_item_random(velocity));
        self.server.next_entity += 1;
        let p = Vec3::new(position[0], position[1], position[2]);
        self.common
            .persist
            .uuids
            .insert(self.server.next_entity, random.uuid);
        let (bob, yaw) = (random.bob, random.yaw);
        let velocity = velocity.unwrap_or(random.velocity);
        self.common.dropped.push(random.state.apply(ItemEntity {
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [yaw, 0.0],
            pickup_delay,
            random: random.own.into(),
            ..ItemEntity::new(
                self.server.next_entity,
                stack,
                p,
                Vec3::new(velocity[0], velocity[1], velocity[2]),
                bob,
            )
        }));
        self.wake_hoppers_near_dropped_item(self.server.next_entity);
        velocity
    }
}
