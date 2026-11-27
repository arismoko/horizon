use crate::items::EntityTypeTags;
use sunrise_world::{
    collision_context::{CollisionContext, CollisionEntity, EntityContextKind},
    entity_obstruction::EntityIdentity,
};

impl super::Mob {
    /// The live mob context for `Level.findSupportingBlock`.
    pub fn support_collision(
        &self,
        tags: Option<&EntityTypeTags>,
    ) -> (
        CollisionContext<'_>,
        crate::collisions::PowderSnowSubject<'_>,
    ) {
        let subject = self.collision_subject(tags);
        let context = CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(u64::from(self.id)),
                kind: EntityContextKind::Living,
                y: self.position.y,
                descending: false,
                main_hand: subject.main_hand,
            },
            false,
        );
        (context, subject)
    }

    pub(super) fn collision_subject(
        &self,
        tags: Option<&EntityTypeTags>,
    ) -> crate::collisions::PowderSnowSubject<'_> {
        use sunrise_world::collision_context::CollisionItem;
        crate::collisions::PowderSnowSubject {
            fall_distance: self.fall_distance,
            walkable_type: tags.is_some_and(|tags| {
                tags.has_bit(
                    self.kind.flags.entity_tag_slot,
                    EntityTypeTags::POWDER_SNOW_WALKABLE,
                )
            }),
            leather_boots: self
                .extra
                .slot_stack(sunrise_items::equipment::FEET)
                .is_some_and(|stack| {
                    stack.count > 0 && stack.item.as_ref() == "minecraft:leather_boots"
                }),
            main_hand: Some(
                self.extra
                    .main_hand_stack()
                    .map_or(CollisionItem::EMPTY, |stack| CollisionItem {
                        item: &stack.item,
                        count: stack.count.min(i32::MAX as u32) as i32,
                    }),
            ),
            lava_walker: self.kind.flags.lava_walker,
            ..Default::default()
        }
    }
}
