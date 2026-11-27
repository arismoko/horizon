//! Adapt the current loot evaluator's tag reads to the existing API resource.
use mod_api::{
    BiomeKey, BlockKey, DamageTypeKey, EnchantmentKey, EntityTypeKey, FluidKey, Identifier,
    ItemKey, PotionKey, StructureKey, TagKey, TagMembers, Tags, WorldClockKey,
};

pub(crate) struct LootTags<'a>(pub &'a dyn sunrise_world::loot::LootTags);

impl Tags for LootTags<'_> {
    fn key(&self, id: &Identifier) -> Option<TagKey> {
        mod_api::__private::tag_key(id)
    }
    fn is_in_tag(&self, item: ItemKey, tag: TagKey<ItemKey>) -> Option<bool> {
        let item = mod_api::__private::item_key_id(item)?;
        mod_api::__private::with_tag_name(tag.untyped(), |tag| {
            Some(self.0.contains("item", tag?, item))
        })
    }
    fn damage_type_in_tag(&self, damage_type: &str, tag: TagKey<DamageTypeKey>) -> Option<bool> {
        mod_api::__private::with_tag_name(tag.untyped(), |tag| {
            self.0.damage_type_in_tag(damage_type, tag?)
        })
    }
    fn item_tags(&self, item: ItemKey) -> Option<Vec<Identifier>> {
        super::native_tags::item_tags(self.0, item)
    }
    fn item_tag_names(&self) -> Option<Vec<Identifier>> {
        super::native_tags::item_tag_names(self.0)
    }
    fn block_in_tag(&self, block: BlockKey, tag: TagKey<BlockKey>) -> Option<bool> {
        let block = mod_api::__private::block_key_id(block)?;
        mod_api::__private::with_tag_name(tag.untyped(), |tag| {
            Some(self.0.contains("block", tag?, block))
        })
    }
    fn fluid_in_tag(&self, fluid: FluidKey, tag: TagKey<FluidKey>) -> Option<bool> {
        let fluid = mod_api::__private::fluid_id(fluid)?;
        mod_api::__private::with_tag_name(tag.untyped(), |tag| {
            Some(self.0.contains("fluid", tag?, fluid))
        })
    }
    fn items(&self, tag: TagKey<ItemKey>) -> TagMembers<ItemKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("item", name),
            mod_api::__private::item_key,
        )
    }
    fn blocks(&self, tag: TagKey<BlockKey>) -> TagMembers<BlockKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("block", name),
            mod_api::__private::block_key,
        )
    }
    fn fluids(&self, tag: TagKey<FluidKey>) -> TagMembers<FluidKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("fluid", name),
            mod_api::__private::fluid_key,
        )
    }
    fn entity_types(&self, tag: TagKey<EntityTypeKey>) -> TagMembers<EntityTypeKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("entity_type", name),
            mod_api::__private::resolved_entity_type_key,
        )
    }
    fn biomes(&self, _tag: TagKey<BiomeKey>) -> TagMembers<BiomeKey> {
        TagMembers::Unavailable
    }
    fn enchantments(&self, tag: TagKey<EnchantmentKey>) -> TagMembers<EnchantmentKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("enchantment", name),
            mod_api::__private::enchantment_key,
        )
    }
    fn structures(&self, tag: TagKey<StructureKey>) -> TagMembers<StructureKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("worldgen/structure", name),
            |id| Identifier::parse(id).ok().map(StructureKey),
        )
    }
    fn potions(&self, tag: TagKey<PotionKey>) -> TagMembers<PotionKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("potion", name),
            |id| Identifier::parse(id).ok().map(PotionKey),
        )
    }
    fn damage_types(&self, tag: TagKey<DamageTypeKey>) -> TagMembers<Identifier> {
        super::native_tags::members(
            tag,
            |name| self.0.members("damage_type", name),
            |id| Identifier::parse(id).ok(),
        )
    }
    fn world_clocks(&self, tag: TagKey<WorldClockKey>) -> TagMembers<WorldClockKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("world_clock", name),
            |id| Identifier::parse(id).ok().map(WorldClockKey),
        )
    }
}
