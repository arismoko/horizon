//! Block entities and the items that carry them
//! (`sunrise_client::block_entity_item`): the item Pick Block gives, with the
//! block entity's data on Ctrl in creative, and the data and components a
//! placed item gives its block entity.
use sunrise_core::nbt::{Compound, JavaString, Tag};
use sunrise_game_data::block_entity_nbt;
use sunrise_items::{item_data, items::ItemStack};
use sunrise_level::containers::BlockEntity;
use sunrise_sim::{block_entity_item, save};
use sunrise_world::{
    level::{Behaviours, classes::Class},
    registry::{BlockRegistry, BlockStateId},
};

/// The state to write and the state after the BlockItem's property tag, shared with prediction.
pub fn item_block_state(
    registry: &BlockRegistry,
    mut state: BlockStateId,
    stack: &ItemStack,
    game_master: bool,
) -> (BlockStateId, BlockStateId) {
    let Some(current) = registry.state(state) else {
        return (state, state);
    };
    let saved = stack.data().and_then(|d| d.other("minecraft:block_state"));
    let book = game_master
        && current.name == "minecraft:lectern"
        && block_entity_item::carries_book(stack);
    if !book && saved.is_none() {
        return (state, state);
    }
    let mut changed = current.clone();
    if book {
        // LecternBlock.getStateForPlacement applies this before the item tag.
        changed.properties.insert("has_book".into(), "true".into());
        state = registry.id(&changed).unwrap_or(state);
    }
    let Some(Tag::Compound(saved)) = saved else {
        return (state, state);
    };
    let mut tagged = state;
    for (key, value) in saved {
        let key = key.to_string_lossy();
        if let Tag::String(value) = value
            && let Some(property) = changed.properties.get_mut(&key)
        {
            let previous = std::mem::replace(property, value.to_string_lossy());
            if let Some(next) = registry.id(&changed) {
                tagged = next;
            } else {
                changed.properties.insert(key, previous);
            }
        }
    }
    (state, tagged)
}

/// The component patch of `stack`.
pub fn components(stack: &ItemStack) -> Compound {
    match item_data::component_snapshot(stack) {
        Some(Tag::Compound(c)) => c,
        _ => Compound::new(),
    }
}

/// `patch`'s components onto `stack`, over its own.
pub(in crate::game) fn apply_patch(stack: &mut ItemStack, patch: Compound) {
    if patch.is_empty() {
        return;
    }
    let mut all = components(stack);
    all.extend(patch);
    (stack.damage, stack.extra) = item_data::decode_components(Some(&Tag::Compound(all)));
}

/// `BlockState.getCloneItemStack`'s item: the block's own item, unless the
/// block overrides it. A block with no item of its own still has one to pick:
/// a stem's seeds, a crop's seed item, a vine's harvest.
pub(in crate::game) fn clone_item<'a>(name: &str, block_item: &'a str) -> &'a str {
    match name {
        // StemBlock and AttachedStemBlock: the seeds it grew from.
        "minecraft:pumpkin_stem" | "minecraft:attached_pumpkin_stem" => "minecraft:pumpkin_seeds",
        "minecraft:melon_stem" | "minecraft:attached_melon_stem" => "minecraft:melon_seeds",
        // CropBlock.getCloneItemStack: new ItemStack(getBaseSeedId()).
        "minecraft:wheat" => "minecraft:wheat_seeds",
        "minecraft:carrots" => "minecraft:carrot",
        "minecraft:potatoes" => "minecraft:potato",
        "minecraft:beetroots" => "minecraft:beetroot_seeds",
        "minecraft:torchflower_crop" => "minecraft:torchflower_seeds",
        // BambooSaplingBlock, SweetBerryBushBlock and the two cave vine
        // blocks: new ItemStack(their harvest item).
        "minecraft:bamboo_sapling" => "minecraft:bamboo",
        "minecraft:sweet_berry_bush" => "minecraft:sweet_berries",
        "minecraft:cave_vines" | "minecraft:cave_vines_plant" => "minecraft:glow_berries",
        // BigDripleafStemBlock: new ItemStack(Blocks.BIG_DRIPLEAF).
        "minecraft:big_dripleaf_stem" => "minecraft:big_dripleaf",
        // GrowingPlantBodyBlock: new ItemStack(getHeadBlock()), the block that
        // carries the item.
        "minecraft:kelp_plant" => "minecraft:kelp",
        "minecraft:twisting_vines_plant" => "minecraft:twisting_vines",
        "minecraft:weeping_vines_plant" => "minecraft:weeping_vines",
        // CandleCakeBlock: new ItemStack(Blocks.CAKE), for every colour and
        // the plain `minecraft:candle_cake`, whose own item is air.
        _ if name == "minecraft:candle_cake" || name.ends_with("_candle_cake") => "minecraft:cake",
        _ => block_item,
    }
}

pub(in crate::game) fn picked_item<'a>(
    behaviours: &'a Behaviours,
    state: BlockStateId,
    item: &'a str,
) -> Option<&'a str> {
    let plant = behaviours.data(state);
    let potted = plant
        .is(Class::FlowerPot)
        .then_some(plant.link)
        .flatten()
        .map(|block| &*behaviours.block_data(block).item)
        .filter(|item| *item != "minecraft:air");
    Some(potted.unwrap_or(item)).filter(|item| *item != "minecraft:air")
}

impl crate::game::CommonGame<'_> {
    /// Plain projection for the existing container-title consumers. The live
    /// `BaseContainerBlockEntity` name remains a Component, not rendered text.
    pub fn block_name_plain(&self, key: (i32, i32, i32)) -> Option<String> {
        self.world.block_names.get(&key).map(|name| match name {
            Tag::String(text) => text.to_string_lossy(),
            component => sunrise_game_data::component::name_text(component).plain(),
        })
    }

    /// The `CustomName` a block entity loads from `tag`
    /// (`BaseContainerBlockEntity.loadAdditional`): kept beside the model in
    /// `block_names`, and dropped when the tag has none. Banners and signs
    /// keep their name themselves. True when the name changed.
    pub fn load_block_name(
        &mut self,
        key: (i32, i32, i32),
        entity: &BlockEntity,
        tag: &Tag,
    ) -> bool {
        if matches!(entity, BlockEntity::Banner(_) | BlockEntity::Sign(_)) {
            return false;
        }
        let name = if matches!(entity, BlockEntity::Mod(_)) {
            save::text(tag.get("CustomName")).map(|name| Tag::String(name.as_str().into()))
        } else {
            tag.get("CustomName").and_then(block_entity_nbt::component)
        };
        if self.world.block_names.get(&key) == name.as_ref() {
            return false;
        }
        match name {
            Some(name) => self.world.block_names.insert(key, name),
            None => self.world.block_names.remove(&key),
        };
        true
    }
}

pub trait BlockItemsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The block entity at `key` as vanilla saves it, with its custom name.
    fn saved_block_entity(&self, key: (i32, i32, i32), for_save: bool) -> Option<Tag> {
        let entity = self.world.block_entities.get(&key)?;
        let block = &self
            .res
            .facts()
            .get(self.world.block(key.0, key.1, key.2))
            .name;
        let mut tag = if for_save {
            self.side_encode_live_block_entity(key, entity, block)
        } else {
            save::block_entity_encode::encode(key, entity, block, self.res.items())
        };
        if let (Some(name), Tag::Compound(map)) = (self.world.block_names.get(&key), &mut tag) {
            map.insert("CustomName".into(), name.clone());
        }
        if matches!(entity, BlockEntity::Mod(_)) {
            // The mod owns its serializer; the vanilla canonicalizer only knows
            // vanilla entity types and must not discard this instance's data.
            Some(tag)
        } else {
            block_entity_nbt::canonical(&tag)
        }
    }

    /// `BlockState.getCloneItemStack` of the block at `pos`, then, with
    /// `include_data`, `ServerGamePacketListenerImpl.addBlockDataToItem`.
    fn picked_stack(&self, pos: [i32; 3], include_data: bool) -> Option<ItemStack> {
        let mut stack = self.clone_stack(pos, include_data)?;
        if include_data {
            self.add_block_data(pos, &mut stack);
        }
        Some(stack)
    }

    /// `ServerGamePacketListenerImpl.addBlockDataToItem`: the block entity at
    /// `pos`, if any, onto `stack` (`BlockItem.setBlockEntityData`, then
    /// `ItemStack.applyComponents(collectComponents())`).
    fn add_block_data(&self, pos: [i32; 3], stack: &mut ItemStack) {
        if let Some(saved) = self.saved_block_entity((pos[0], pos[1], pos[2]), true) {
            apply_patch(stack, block_entity_item::pick(&saved));
        }
    }

    /// `BlockState.getCloneItemStack` of the block at `pos`: the item alone,
    /// with what the block itself puts on it; `include_data` as vanilla passes it.
    fn clone_stack(&self, pos: [i32; 3], include_data: bool) -> Option<ItemStack> {
        let key = (pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let facts = self.res.facts().get(state);
        // PistonHeadBlock.getCloneItemStack: the piston the head belongs to.
        let head = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name.as_str() == "minecraft:piston_head")
            .map(|s| match s.properties.get("type") {
                Some(t) if t == "sticky" => "minecraft:sticky_piston",
                _ => "minecraft:piston",
            });
        let item = clone_item(&facts.name, head.unwrap_or(&facts.pick_item));
        // FlowerPotBlock.getCloneItemStack: a potted plant picks as that plant
        // (`new ItemStack(potted)`), an empty pot as the pot itself.
        let item = picked_item(self.res.behaviours(), state, item)?;
        let mut stack = ItemStack::new(item, 1);
        let mut patch = Compound::new();
        // BeehiveBlock.getCloneItemStack: the honey level as the item's block state.
        if include_data
            && let Some(level) = self
                .pack
                .assets
                .registry
                .state(state)
                .filter(|s| matches!(s.name.as_str(), "minecraft:beehive" | "minecraft:bee_nest"))
                .and_then(|s| s.properties.get("honey_level"))
        {
            patch.insert(
                "minecraft:block_state".into(),
                Tag::compound([("honey_level", Tag::String(level.as_str().into()))]),
            );
        }
        // CopperGolemStatueBlock.getCloneItemStack: the statue's pose as the item's
        // block_state (`CopperGolemStatueBlockEntity.getItem`), data or not.
        if let Some(pose) = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name.ends_with("copper_golem_statue"))
            .and_then(|s| s.properties.get("copper_golem_pose"))
        {
            patch.insert(
                "minecraft:block_state".into(),
                Tag::compound([("copper_golem_pose", Tag::String(pose.as_str().into()))]),
            );
        }
        // LightBlock.getCloneItemStack, setLightOnStack: the light's level as
        // the item's block state, whether or not the block data is picked.
        if let Some(level) = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name.as_str() == "minecraft:light")
            .and_then(|s| s.properties.get("level"))
        {
            patch.insert(
                "minecraft:block_state".into(),
                Tag::compound([("level", Tag::String(level.as_str().into()))]),
            );
        }
        // AbstractBannerBlock.getCloneItemStack: the banner's own item
        // (BannerBlockEntity.getItem), data or not.
        let banner = matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Banner(_))
        );
        // DecoratedPotBlock.getCloneItemStack: createDecoratedPotInstance, the
        // sherds only.
        let pot = &*facts.name == "minecraft:decorated_pot";
        // CopperGolemStatueBlockEntity.getItem: collectComponents (the golem's
        // custom name), data or not.
        let statue = matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::CopperGolemStatue(_))
        );
        // A mod entity contributes only once, to the winning stack in add_block_data.
        let mod_entity = matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Mod(_))
        );
        if !mod_entity
            && (banner || pot || statue)
            && let Some(saved) = self.saved_block_entity(key, false)
        {
            let mut picked = block_entity_item::pick(&saved);
            if pot {
                picked.retain(|k, _| *k == JavaString::from("minecraft:pot_decorations"));
            }
            patch.extend(picked);
        }
        apply_patch(&mut stack, patch);
        Some(stack)
    }

    /// Shared `BlockItem.updateCustomBlockEntityTag` / `updateBlockEntityComponents`
    /// decode and install path; returns `Some(custom_data_loaded)` when the block
    /// entity was replaced.
    fn apply_placed_item_common(
        &mut self,
        pos: [i32; 3],
        stack: &ItemStack,
        game_master: bool,
    ) -> Option<bool> {
        let key = (pos[0], pos[1], pos[2]);
        self.pending.clear(pos);
        let saved = self.saved_block_entity(key, false)?;
        let (placed, custom_data_loaded) =
            block_entity_item::place_with_load_result(&saved, &components(stack), game_master);
        if placed == saved {
            return None;
        }
        let block = self.world.block(pos[0], pos[1], pos[2]);
        let name = &self.res.facts().get(block).name;
        let previous = self.world.block_entities.get(&key);
        let (_, mut entity) = save::decode_block_entity_replacing(&placed, name, previous)?;
        // The new crafter's state flag (CrafterBlock.newBlockEntity), which
        // its saved data has no part of.
        if let (BlockEntity::Crafter(new), Some(BlockEntity::Crafter(old))) =
            (&mut entity, previous)
        {
            new.state_triggered = old.state_triggered;
        }
        self.load_block_name(key, &entity, &placed);
        self.world.block_entities.insert(key, entity);
        Some(custom_data_loaded)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BlockItemsGameExt1<'__sg> for G {}

use crate::game::side_ops::blocks::BlocksSideGameExt as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{clone_item, picked_item};

    #[test]
    fn a_potted_plant_picks_its_plant_and_an_empty_pot_the_pot() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("block-behaviours.json").exists() {
            return;
        }
        let registry = sunrise_world::level::data::load_registry(&root).unwrap();
        let behaviours = sunrise_world::level::Behaviours::load(&registry, &root).unwrap();
        let pick = |block: &str, own: &'static str| {
            let state = registry.default_state(block).unwrap();
            picked_item(&behaviours, state, own).map(str::to_owned)
        };
        assert_eq!(
            pick("minecraft:potted_dandelion", "minecraft:air").as_deref(),
            Some("minecraft:dandelion")
        );
        assert_eq!(
            pick("minecraft:potted_oak_sapling", "minecraft:air").as_deref(),
            Some("minecraft:oak_sapling")
        );
        assert_eq!(
            pick("minecraft:flower_pot", "minecraft:flower_pot").as_deref(),
            Some("minecraft:flower_pot")
        );
        assert_eq!(
            pick("minecraft:stone", "minecraft:stone").as_deref(),
            Some("minecraft:stone")
        );
        assert_eq!(pick("minecraft:fire", "minecraft:air"), None);
    }

    /// `StemBlock.getCloneItemStack` and `AttachedStemBlock.getCloneItemStack`:
    /// pumpkin and melon stems, attached or not, pick the seed they grew from;
    /// a block without an override still picks its own item.
    #[test]
    fn stems_pick_their_seeds_and_blocks_their_item() {
        for (stem, seed) in [
            ("minecraft:pumpkin_stem", "minecraft:pumpkin_seeds"),
            ("minecraft:attached_pumpkin_stem", "minecraft:pumpkin_seeds"),
            ("minecraft:melon_stem", "minecraft:melon_seeds"),
            ("minecraft:attached_melon_stem", "minecraft:melon_seeds"),
        ] {
            assert_eq!(clone_item(stem, "minecraft:air"), seed, "{stem}");
        }
        assert_eq!(
            clone_item("minecraft:stone", "minecraft:stone"),
            "minecraft:stone"
        );
    }
    /// The blocks whose `getCloneItemStack` override returns something other
    /// than their own item: a crop's seed item, a sapling's bamboo, a cave
    /// vine's glow berries, a stem block's head. Each of them has no item of
    /// its own, so pick block gave nothing before; a block with an item is
    /// still untouched.
    #[test]
    fn blocks_without_an_item_pick_as_their_harvest() {
        for (block, item) in [
            ("minecraft:wheat", "minecraft:wheat_seeds"),
            ("minecraft:carrots", "minecraft:carrot"),
            ("minecraft:potatoes", "minecraft:potato"),
            ("minecraft:beetroots", "minecraft:beetroot_seeds"),
            ("minecraft:torchflower_crop", "minecraft:torchflower_seeds"),
            ("minecraft:bamboo_sapling", "minecraft:bamboo"),
            ("minecraft:sweet_berry_bush", "minecraft:sweet_berries"),
            ("minecraft:cave_vines", "minecraft:glow_berries"),
            ("minecraft:cave_vines_plant", "minecraft:glow_berries"),
            ("minecraft:big_dripleaf_stem", "minecraft:big_dripleaf"),
            ("minecraft:kelp_plant", "minecraft:kelp"),
            ("minecraft:twisting_vines_plant", "minecraft:twisting_vines"),
            ("minecraft:weeping_vines_plant", "minecraft:weeping_vines"),
            ("minecraft:candle_cake", "minecraft:cake"),
            ("minecraft:white_candle_cake", "minecraft:cake"),
            ("minecraft:light_blue_candle_cake", "minecraft:cake"),
        ] {
            assert_eq!(clone_item(block, "minecraft:air"), item, "{block}");
        }
        assert_eq!(
            clone_item("minecraft:sweet_berries", "minecraft:sweet_berries"),
            "minecraft:sweet_berries"
        );
        assert_eq!(
            clone_item("minecraft:cake", "minecraft:cake"),
            "minecraft:cake"
        );
    }
}
