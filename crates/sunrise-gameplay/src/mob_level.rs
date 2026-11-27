//! What mob code calls on `ServerLevel` and `Level`: the level's data as one borrow, and its services.

use sunrise_items::items::Items;
use sunrise_sim::entities::ItemEntity;

/// The level's data a mob family reads and writes, as one borrow of disjoint parts.
pub struct MobParts<'a> {
    pub mobs: &'a mut sunrise_sim::join_seq::JoinList<sunrise_sim::mobs::Mob>,
    pub dropped: &'a mut sunrise_sim::join_seq::JoinList<ItemEntity>,
    pub next_entity: u32,
    pub ticked: &'a sunrise_level::fast_hash::FxHashSet<u32>,
    pub items: &'a Items,
    pub facts: &'a sunrise_items::blocks::BlockFacts,
    pub world: &'a mut sunrise_level::world::World,
}

/// What mob code calls on `ServerLevel`, statically dispatched because it runs per mob per tick.
pub trait MobLevel {
    /// The level's data, borrowed for no longer than one statement run between services.
    fn parts(&mut self) -> MobParts<'_>;
    /// `GameRules.RULE_MOBGRIEFING` as `ServerLevel.getGameRules().getBoolean` reads it.
    fn mob_griefing(&self) -> bool;
    /// `EntityMobGriefingEvent`: `ServerLevel.getGameRules` with `rule` read, mods may change it.
    fn entity_grief(&self, entity: u32, rule: bool) -> bool;
    /// `new ItemEntity(level, x, y, z, stack)` added to the level; returns its own random's motion.
    fn spawn_item_own(
        &mut self,
        stack: sunrise_items::items::ItemStack,
        position: [f64; 3],
        pickup_delay: i32,
    ) -> [f64; 3];
    /// The item entity `ServerLevel.addFreshEntity` just added, unless a mod kept it out.
    fn just_spawned_item(&mut self) -> Option<&mut ItemEntity>;
    /// `Entity.getUUID` of a mob, made up and stored for a new one.
    fn mob_uuid(&mut self, id: u32) -> [i32; 4];
    /// `Level.playSound` at a position.
    fn play_sound(&mut self, event: &str, at: [f64; 3], volume: f32, pitch: f32);
    /// `LivingEntity.onItemPickup`: the thrower's advancement and the mod trigger.
    fn on_item_pickup(&mut self, mob_index: usize, item: &ItemEntity);
    /// `ServerLevel.getChunkSource().broadcast(ClientboundTakeItemEntityPacket)` via `Mob.take`.
    fn take(&mut self, collector: u32, item: ItemEntity);
    /// `Entity.discard` of dropped item `index`; mods hear it.
    fn discard_item(&mut self, index: usize) -> ItemEntity;
}
