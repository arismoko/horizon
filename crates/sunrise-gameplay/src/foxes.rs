//! Foxes and dolphins: what they take from the ground and where they go for it, on `MobLevel`.
use crate::mob_level::MobLevel;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::item_areas::ItemAreas;
use sunrise_sim::mobs::{
    Species, dolphin,
    equip::Slot,
    finalize::{self, set_drop_chance, set_item_stack},
    fox,
};

/// `Fox.ALLOWED_ITEMS` and the looting block of `Mob.aiStep` for foxes, once a tick.
pub fn tick_foxes(level: &mut impl MobLevel) {
    let griefing = level.mob_griefing();
    let mut near = ItemAreas::default();
    for i in 0..level.parts().mobs.len() {
        let (id, reach, can_loot) = {
            let p = level.parts();
            let m = &p.mobs[i];
            if m.species() != Species::Fox || !m.alive() || m.removed || !p.ticked.contains(&m.id) {
                continue;
            }
            // `Mob.aiStep`: the items in the box grown by (1, 0, 1), in order.
            (
                m.id,
                m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0)),
                m.can_pick_up_loot(),
            )
        };
        if can_loot && level.entity_grief(id, griefing) && {
            let p = level.parts();
            near.may_meet(p.dropped, p.next_entity, reach)
        } {
            let mut j = 0;
            while j < level.parts().dropped.len() {
                let p = level.parts();
                let e = &p.dropped[j];
                if e.pickup_delay > 0 || e.stack.count == 0 || !reach.intersects(e.bounds()) {
                    j += 1;
                    continue;
                }
                let item_food = fox_food(p.items, &e.stack.item);
                let held_food = p.mobs[i]
                    .extra
                    .main_hand_id()
                    .is_some_and(|held| fox_food(p.items, held));
                if !fox::can_hold_item(&p.mobs[i], item_food, held_food) {
                    j += 1;
                    continue;
                }
                // The entity is discarded: the next one is at `j` now.
                fox_pick_up(level, i, j);
            }
        }
        // `FoxSearchForItemsGoal`: the first allowed item within 8 blocks.
        let p = level.parts();
        let area = p.mobs[i].bounds().inflate(Vec3::new(8.0, 8.0, 8.0));
        let at = if near.may_meet(p.dropped, p.next_entity, area) {
            p.dropped
                .iter()
                .find(|e| e.pickup_delay == 0 && e.health > 0 && area.intersects(e.bounds()))
                .map(|e| [e.position.x, e.position.y, e.position.z])
        } else {
            None
        };
        fox::set_item_at(&mut p.mobs[i], at);
    }
}

/// `Dolphin.pickUpItem` in `Mob.aiStep`'s looting block, and the item
/// `Dolphin.MoveToItemGoal` swims to (`Dolphin.ALLOWED_ITEMS`: no pickup delay, alive, in water).
pub fn tick_dolphins(level: &mut impl MobLevel) {
    let griefing = level.mob_griefing();
    let mut near = ItemAreas::default();
    for i in 0..level.parts().mobs.len() {
        let (id, reach, can_loot) = {
            let p = level.parts();
            let m = &p.mobs[i];
            if m.species() != Species::Dolphin
                || !m.alive()
                || m.removed
                || !p.ticked.contains(&m.id)
            {
                continue;
            }
            // `Mob.aiStep`: the items in the box grown by (1, 0, 1); `Dolphin.pickUpItem`
            // takes the whole stack when the mouth is empty (`Mob.canHoldItem` is true).
            (
                m.id,
                m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0)),
                m.can_pick_up_loot(),
            )
        };
        if can_loot && level.entity_grief(id, griefing) && {
            let p = level.parts();
            near.may_meet(p.dropped, p.next_entity, reach)
        } {
            let mut j = 0;
            while j < level.parts().dropped.len() {
                let p = level.parts();
                let e = &p.dropped[j];
                if e.pickup_delay > 0
                    || e.stack.count == 0
                    || !reach.intersects(e.bounds())
                    || p.mobs[i].extra.main_hand_id().is_some()
                {
                    j += 1;
                    continue;
                }
                let before = p.dropped[j].clone();
                level.on_item_pickup(i, &before);
                let m = &mut level.parts().mobs[i];
                set_item_stack(m, Slot::MainHand.index(), before.stack.clone());
                set_drop_chance(m, Slot::MainHand.index(), 2.0);
                let id = m.id;
                level.take(id, before);
                level.discard_item(j);
            }
        }
        let p = level.parts();
        let area = p.mobs[i].bounds().inflate(Vec3::new(8.0, 8.0, 8.0));
        let at = if near.may_meet(p.dropped, p.next_entity, area) {
            p.dropped
                .iter()
                .find(|e| {
                    e.pickup_delay == 0
                        && e.health > 0
                        && area.intersects(e.bounds())
                        && p.facts
                            .get(p.world.block(
                                e.position.x.floor() as i32,
                                e.position.y.floor() as i32,
                                e.position.z.floor() as i32,
                            ))
                            .fluid
                            == sunrise_items::blocks::Fluid::Water
                })
                .map(|e| [e.position.x, e.position.y, e.position.z])
        } else {
            None
        };
        dolphin::set_item_at(&mut p.mobs[i], at);
    }
}

/// `Fox.isConsumableFood`.
fn fox_food(items: &sunrise_items::items::Items, item: &str) -> bool {
    items
        .get(item)
        .is_some_and(|i| i.food.is_some() && i.consumable.is_some())
}

/// `Fox.pickUpItem`: the extra items drop where it stands, what it held is spat out
/// ahead of it, and it holds one of the item.
fn fox_pick_up(level: &mut impl MobLevel, fox_index: usize, item_index: usize) {
    let (before, at) = {
        let p = level.parts();
        let m = &p.mobs[fox_index];
        (
            p.dropped[item_index].clone(),
            [m.position.x, m.position.y, m.position.z],
        )
    };
    let mut stack = before.stack.clone();
    let count = stack.count;
    // `dropItemStack(itemStack.split(count - 1))`: a default ItemEntity at its feet.
    if count > 1 {
        let mut rest = stack.clone();
        rest.count = count - 1;
        stack.count = 1;
        level.spawn_item_own(rest, at, 10);
    }
    // `spitOutItem(getItemBySlot(MAINHAND))`.
    let held = finalize::item_stack(&level.parts().mobs[fox_index], Slot::MainHand.index());
    if let Some(held) = held {
        let look = level.parts().mobs[fox_index].view_vector();
        let from = [at[0] + look[0], at[1] + 1.0, at[2] + look[2]];
        level.spawn_item_own(held, from, 40);
        // A listener on the spawn may have put another mob in this slot: its UUID tags the item.
        let fox_id = level.parts().mobs[fox_index].id;
        let id = level.mob_uuid(fox_id);
        if let Some(thrown) = level.just_spawned_item() {
            thrown.thrower = Some(id);
        }
        level.play_sound("minecraft:entity.fox.spit", at, 1.0, 1.0);
    }
    // `onItemPickup`, then `setItemSlot(MAINHAND, split(1))`, `setGuaranteedDrop`.
    level.on_item_pickup(fox_index, &before);
    let m = &mut level.parts().mobs[fox_index];
    set_item_stack(m, Slot::MainHand.index(), stack);
    set_drop_chance(m, Slot::MainHand.index(), 2.0);
    fox::took_item(m);
    let id = m.id;
    // `take(entity, count)` and `entity.discard()`.
    level.take(id, before);
    level.discard_item(item_index);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mob_level::MobParts;
    use std::sync::Arc;
    use sunrise_items::{
        blocks::BlockFacts,
        items::{ItemStack, Items},
    };
    use sunrise_level::{fast_hash::FxHashSet, world::World};
    use sunrise_sim::{
        entities::ItemEntity,
        join_seq::JoinList,
        mobs::{Anim, Brain, Mob, MobType},
    };
    use sunrise_world::registry::BlockStateId;

    fn kind(id: &str, species: Species) -> Arc<MobType> {
        Arc::new(MobType {
            type_key: Default::default(),
            id: id.into(),
            species,
            width: 0.6,
            height: 0.7,
            eye_height: 0.4,
            category: "creature".into(),
            max_health: 10.0,
            speed: 0.3,
            flying_speed: 0.02,
            attack_damage: 2.0,
            attack_knockback: 0.0,
            follow_range: 16.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Default::default(),
            item_stack_limits: Default::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            brain: Brain::Passive,
            anim: Anim::Rabbit,
            sounds: [""; 3],
            sound_overrides: Default::default(),
            food: Arc::default(),
            tempt: Arc::default(),
            passenger_attachments: Vec::new(),
        })
    }

    fn dropped(stack: ItemStack) -> ItemEntity {
        let at = Vec3::new(0.5, 64.0, 0.5);
        ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 5,
            thrower: None,
            target: None,
            stack,
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            ground: sunrise_world::ground_support::GroundState {
                on_ground: true,
                ..Default::default()
            },
            vertical_collision: false,
            age: 0,
            pickup_delay: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }

    /// A level where adding the spat-out item lets a listener move the fox away and put a cow in its slot.
    struct ListenerLevel {
        mobs: JoinList<Mob>,
        dropped: JoinList<ItemEntity>,
        ticked: FxHashSet<u32>,
        items: Items,
        facts: BlockFacts,
        world: World,
        uuids_asked: Vec<u32>,
    }
    impl MobLevel for ListenerLevel {
        fn parts(&mut self) -> MobParts<'_> {
            MobParts {
                mobs: &mut self.mobs,
                dropped: &mut self.dropped,
                next_entity: 10,
                ticked: &self.ticked,
                items: &self.items,
                facts: &self.facts,
                world: &mut self.world,
            }
        }
        fn mob_griefing(&self) -> bool {
            true
        }
        fn entity_grief(&self, _entity: u32, rule: bool) -> bool {
            rule
        }
        fn spawn_item_own(
            &mut self,
            _stack: ItemStack,
            _at: [f64; 3],
            pickup_delay: i32,
        ) -> [f64; 3] {
            if pickup_delay == 40 {
                self.mobs[0] = Mob::new(9, kind("minecraft:cow", Species::Cow), Vec3::ZERO, 0.0);
            }
            [0.0; 3]
        }
        fn just_spawned_item(&mut self) -> Option<&mut ItemEntity> {
            None
        }
        fn mob_uuid(&mut self, id: u32) -> [i32; 4] {
            self.uuids_asked.push(id);
            [id as i32, 0, 0, 0]
        }
        fn play_sound(&mut self, _event: &str, _at: [f64; 3], _volume: f32, _pitch: f32) {}
        fn on_item_pickup(&mut self, _mob_index: usize, _item: &ItemEntity) {}
        fn take(&mut self, _collector: u32, _item: ItemEntity) {}
        fn discard_item(&mut self, index: usize) -> ItemEntity {
            self.dropped.remove(index)
        }
    }

    /// After a spawn listener replaced the fox's slot, the spat item asks the UUID of the mob now there.
    #[test]
    fn the_spit_asks_the_uuid_of_the_mob_in_the_slot_after_the_spawn() {
        let mut fox = Mob::new(
            1,
            kind("minecraft:fox", Species::Fox),
            Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        set_item_stack(
            &mut fox,
            Slot::MainHand.index(),
            ItemStack::new("minecraft:stick", 1),
        );
        let mut level = ListenerLevel {
            mobs: [fox].into_iter().collect(),
            dropped: [dropped(ItemStack::new("minecraft:sweet_berries", 1))]
                .into_iter()
                .collect(),
            ticked: FxHashSet::default(),
            items: Items::empty(),
            facts: BlockFacts {
                states: vec![],
                air: BlockStateId(0),
                woods: Default::default(),
                sign_texts: Default::default(),
            },
            world: World::new(BlockStateId(0), 0),
            uuids_asked: Vec::new(),
        };
        fox_pick_up(&mut level, 0, 0);
        assert_eq!(level.uuids_asked, vec![9]);
        assert!(level.dropped.is_empty(), "the berries were taken");
    }
}
