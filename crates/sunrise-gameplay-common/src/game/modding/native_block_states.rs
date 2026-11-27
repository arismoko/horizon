//! Block state ids and names, and item tags, for native mods:
//! registry queries with no level, so they answer anywhere a host is given.

pub use mod_api::{
    BlockEntityTypeKey, BlockKey, EffectKey, EnchantmentKey, EntityTypeKey, FluidKey, FluidState,
    ItemKey, MenuKey, RegistryName, StateConstraint, StateMatchError,
};
pub use std::sync::{Arc, Mutex};
pub use sunrise_command::{Reader, args::Registries, block};
pub use sunrise_items::items::ItemStack;
use sunrise_world::chunk::BlockState;
use sunrise_world::fluid::{FluidKind, FluidState as EngineFluidState};
pub use sunrise_world::registry::{BlockRegistry, BlockStateId};

/// The engine's fluid state for an exported one, or none for a state no fluid can be in: no
/// world form, an amount above 8, a zero amount for anything but the empty fluid, or a source
/// that isn't full.
pub(super) fn engine_fluid_state(state: FluidState) -> Option<EngineFluidState> {
    let kind = crate::game::native_world_read_common::engine_fluid_kind(state.fluid)?;
    let engine = EngineFluidState::new(kind, state.amount, state.falling);
    let consistent = match kind {
        FluidKind::Empty => state.amount == 0,
        _ if engine.is_source() => state.amount == 8,
        _ => (1..=8).contains(&state.amount),
    };
    consistent.then_some(engine)
}

impl crate::game::CommonGame<'_> {
    /// Resolve a registered fluid identifier.
    pub fn fluid_key(&self, id: &str) -> Option<FluidKey> {
        mod_api::__private::fluid_key(id)
    }
    /// Borrows the canonical block id if the name is available.
    pub fn block_key_id(&self, key: BlockKey) -> Option<&str> {
        mod_api::__private::block_key_id(key)
    }
    /// The canonical item id retained by the load-owned registration slot.
    pub fn item_key_id(&self, key: ItemKey) -> Option<&str> {
        mod_api::__private::item_key_id(key)
    }
    /// The canonical effect id retained by the load-owned registration slot.
    pub fn effect_key_id(&self, key: EffectKey) -> Option<&str> {
        mod_api::__private::effect_key_id(key)
    }

    /// The canonical fluid id retained by the completed registry.
    pub fn fluid_key_id(&self, key: FluidKey) -> Option<&str> {
        mod_api::__private::fluid_id(key)
    }
    /// The key of a block in the completed registry.
    pub fn block_key(&self, id: &str) -> Option<BlockKey> {
        mod_api::__private::block_key(id)
    }
    /// The key of an effect in the completed registry.
    pub fn effect_key(&self, id: &str) -> Option<EffectKey> {
        mod_api::__private::effect_key(id)
    }
    /// The canonical block entity type id retained by the load-owned slot.
    pub fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> Option<&str> {
        mod_api::__private::block_entity_type_name(key)
    }
    /// Every block key in registry order, mod blocks after vanilla's.
    pub fn block_keys(&self) -> Vec<BlockKey> {
        mod_api::__private::block_keys()
    }
    /// Every item key in registry order, mod items after vanilla's.
    pub fn item_keys(&self) -> Vec<ItemKey> {
        mod_api::__private::item_keys()
    }
    /// Every entity-type key in registry order.
    pub fn entity_type_keys(&self) -> Vec<EntityTypeKey> {
        mod_api::__private::entity_type_keys()
    }
    /// Every menu-type key in registry order.
    pub fn menu_keys(&self) -> Vec<MenuKey> {
        mod_api::__private::menu_keys()
    }
    /// Every enchantment key in registry order.
    pub fn enchantment_keys(&self) -> Vec<EnchantmentKey> {
        mod_api::__private::enchantment_keys()
    }
    /// The canonical entity type id retained by the load-owned slot.
    pub fn entity_type_key_id(&self, key: EntityTypeKey) -> Option<RegistryName> {
        mod_api::__private::entity_type_key_id(key)
    }
    /// The canonical menu type id retained by the load-owned slot.
    pub fn menu_key_id(&self, key: MenuKey) -> Option<RegistryName> {
        mod_api::__private::menu_key_id(key)
    }
    /// The canonical enchantment id retained by the load-owned slot.
    pub fn enchantment_key_id(&self, key: EnchantmentKey) -> Option<RegistryName> {
        mod_api::__private::enchantment_key_id(key)
    }

    /// The state `name` names, read as `/setblock` reads a block
    /// (`BlockStateParser.parseForBlock`): `minecraft:oak_stairs[facing=east]`,
    /// `stone` for `minecraft:stone`, the properties left out at their
    /// default. Its id is vanilla's `Block.getId`. None for an unknown block,
    /// a property it doesn't have or a value it can't take, or block NBT
    /// (`{...}`); and, as `/setblock`'s argument, for anything after the
    /// block.
    pub fn block_state_id(&self, name: &str) -> Option<BlockStateId> {
        parse(&self.pack.block_registries, self.pack.assets.registry, name)
    }
    /// The state `state` as `/setblock` takes it back
    /// (`BlockStateParser.serialize`): its block, then each property in
    /// name order, `minecraft:oak_stairs[facing=east,half=bottom,...]`, or
    /// the block alone for one with no properties. None for an id no state
    /// has (`Block.stateById` gives air for one).
    pub fn block_state_string(&self, state: BlockStateId) -> Option<String> {
        self.pack.assets.registry.state(state).map(serialize)
    }
    /// The default state of `block` (`minecraft:oak_stairs`, or `oak_stairs`):
    /// `Block.defaultBlockState`, the one a block takes when nothing else
    /// says. None for an unknown block.
    pub fn block_state_default(&self, block: BlockKey) -> Option<BlockStateId> {
        super::api_values::default_state(block)
    }

    /// `Item.getCraftingRemainder`: Sunrise currently stores the remainder's
    /// item id, so the result is a single plain stack.
    pub fn item_crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        sunrise_sim::crafting::CraftingRemainder::crafting_remainder(self.res.items(), stack)
    }

    /// `Block.asItem`: the block's item name, including air for a block with
    /// no item; None if the block name is unknown.
    pub fn block_item(&self, block: BlockKey) -> Option<mod_api::ItemKey> {
        let behaviours = self.res.behaviours();
        let id = super::api_values::behavior_block(block, behaviours)?;
        mod_api::__private::item_key(&behaviours.block_data(id).item)
    }

    /// `BlockItem.getBlock`: the block `item` places. The behaviour table
    /// keeps one binding per item that places a block (`wheat_seeds` to
    /// `wheat`), and the state table names it.
    pub fn item_block(&self, item: ItemKey) -> Option<BlockKey> {
        let behaviours = self.res.behaviours();
        let id = mod_api::__private::item_key_id(item)?;
        let block = behaviours.item_binding(id)?.block;
        mod_api::__private::block_key(&behaviours.table.info(block).name)
    }
}

pub trait NativeBlockStatesGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        let mut remainder = self.item_crafting_remainder(stack);
        if mod_api::__private::item_listens(mod_api::HookId::CraftingRemainder, &stack.item) {
            self.side_crafting_remainder_hook(stack, &mut remainder);
        }
        remainder.filter(|value| value.count > 0)
    }

    #[allow(dead_code)]
    fn craft_grid_remainders(&self, recipe: &str) -> Vec<Option<ItemStack>> {
        self.recipes().remainders(
            &self.loop_context,
            recipe,
            &self.craft_grid[..self.craft_size * self.craft_size],
            self.craft_size,
            &|s| NativeBlockStatesGameExt1::crafting_remainder(self, s),
        )
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeBlockStatesGameExt1<'__sg> for G {}

pub fn match_state_properties(
    definition: &[(String, Vec<String>)],
    same_type: bool,
    has_value: impl Fn(&str, &str) -> bool,
    constraints: &[StateConstraint],
) -> Result<bool, StateMatchError> {
    let mut matches = same_type;
    for (index, constraint) in constraints.iter().enumerate() {
        let Some((_, values)) = definition.iter().find(|(name, _)| name == &constraint.name) else {
            return Err(StateMatchError::UnknownProperty(index as u32));
        };
        if !values.contains(&constraint.value) {
            return Err(StateMatchError::InvalidValue(index as u32));
        }
        matches &= has_value(&constraint.name, &constraint.value);
    }
    Ok(matches)
}

/// The command registries' block table of `registry` alone, built once: what
/// the parser checks properties against, whether a world is open or not.
pub fn block_registries(registry: &BlockRegistry) -> Arc<Registries> {
    static CACHE: Mutex<Vec<(usize, Arc<Registries>)>> = Mutex::new(Vec::new());
    // Derived from the registry's data alone, so equal data across installations shares one.
    let key = registry.payload_identity();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, reg)) = cache.iter().find(|(cached, _)| *cached == key) {
        return reg.clone();
    }
    let reg = Arc::new(Registries {
        blocks: (*super::chat_common::block_properties(registry)).clone(),
        ..Registries::default()
    });
    cache.push((key, reg.clone()));
    reg
}

/// `BlockStateParser.parseForBlock` of the whole of `name`, NBT included, as
/// `/setblock` reads it; a state with NBT has no id.
pub fn parse(reg: &Registries, registry: &BlockRegistry, name: &str) -> Option<BlockStateId> {
    let mut reader = Reader::new(name);
    let input = block::parse(&mut reader, reg, false).ok()?;
    if reader.can_read() || input.nbt.is_some() {
        return None;
    }
    let state = BlockState {
        name: input.id,
        properties: input.properties.into_iter().collect(),
    };
    registry.resolve(&state).ok()
}

/// `BlockStateParser.serialize`: the block, then `[key=value,...]` in the
/// state definition's (name) order.
pub fn serialize(state: &BlockState) -> String {
    if state.properties.is_empty() {
        return state.name.clone();
    }
    let properties: Vec<String> = state
        .properties
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    format!("{}[{}]", state.name, properties.join(","))
}

/// `Util.makeDescriptionId`: `<kind>.<namespace>.<path>`, `/` in the path as `.`.
pub fn description_id(kind: &str, id: &str) -> Option<String> {
    let (namespace, path) = id.split_once(':')?;
    Some(format!("{kind}.{namespace}.{}", path.replace('/', ".")))
}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

use crate::game::tables::GameItems as _;
