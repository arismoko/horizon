//! Block state ids and names, and item tags, for native mods:
//! registry queries with no level, so they answer anywhere a host is given.
use super::Game;
use horizon_client::command::{Reader, args::Registries, block};
use horizon_client::items::ItemStack;
use horizon_world::chunk::BlockState;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use mod_api::{
    BlockEntityTypeKey, BlockKey, BlockSoundType, BlockStateRange, EffectKey, FluidDefinition,
    FluidKey, FluidStateError, ItemKey, StateConstraint, StateMatchError, StateProperty,
};
use std::sync::{Arc, Mutex};

impl Game<'_> {
    /// The exported definition of a world fluid, or a tank-only identity.
    pub(super) fn fluid_state_definition(&self, fluid: FluidKey) -> Option<FluidDefinition> {
        let id = mod_api::__private::fluid_id(fluid)?;
        let Some(definition) = self.res.behaviours().fluid_definitions().get(id) else {
            return Some(FluidDefinition::TankOnly);
        };
        let default = super::native_world_read::api_fluid_state(definition.default)?;
        let properties = definition
            .properties
            .iter()
            .map(|(name, values)| StateProperty::new(name.clone(), values.clone()))
            .collect();
        Some(FluidDefinition::World {
            default,
            properties,
        })
    }

    /// Validate fluid constraints against the exported property definition.
    pub(super) fn fluid_state_validate(
        &self,
        fluid: FluidKey,
        constraints: &[StateConstraint],
    ) -> Result<(), FluidStateError> {
        let definition = mod_api::__private::fluid_id(fluid)
            .and_then(|id| self.res.behaviours().fluid_definitions().get(id))
            .ok_or(FluidStateError::NoWorldState)?;
        for (index, constraint) in constraints.iter().enumerate() {
            if constraints[..index]
                .iter()
                .any(|previous| previous.name == constraint.name)
            {
                return Err(FluidStateError::Duplicate(index as u32));
            }
            match_state_properties(
                &definition.properties,
                true,
                |_, _| true,
                std::slice::from_ref(constraint),
            )
            .map_err(|error| match error {
                StateMatchError::UnknownProperty(_) => {
                    FluidStateError::UnknownProperty(index as u32)
                }
                StateMatchError::InvalidValue(_) => FluidStateError::InvalidValue(index as u32),
                _ => unreachable!("StateMatchError: unhandled variant"),
            })?;
        }
        Ok(())
    }
    /// Resolve a registered fluid identifier.
    pub(super) fn fluid_key(&self, id: &str) -> Option<FluidKey> {
        mod_api::__private::fluid_key(id)
    }
    /// The canonical block id retained by the load-owned registration slot. An invalid slot
    /// is a boundary error, including when a wasm guest supplies it.
    pub(super) fn block_key_id(&self, key: BlockKey) -> String {
        mod_api::__private::block_key_id(key)
            .expect("invalid block key slot")
            .to_owned()
    }
    /// The canonical item id retained by the load-owned registration slot.
    pub(super) fn item_key_id(&self, key: ItemKey) -> String {
        mod_api::__private::item_key_id(key)
            .expect("invalid item key slot")
            .to_owned()
    }
    /// The canonical effect id retained by the load-owned registration slot.
    pub(super) fn effect_key_id(&self, key: EffectKey) -> String {
        mod_api::__private::effect_key_id(key)
            .expect("invalid effect key slot")
            .to_owned()
    }

    /// The canonical block entity type id retained by the load-owned slot.
    pub(super) fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> String {
        mod_api::__private::block_entity_type_name(key)
            .expect("invalid block entity type key slot")
            .to_owned()
    }

    /// The state `name` names, read as `/setblock` reads a block
    /// (`BlockStateParser.parseForBlock`): `minecraft:oak_stairs[facing=east]`,
    /// `stone` for `minecraft:stone`, the properties left out at their
    /// default. Its id is vanilla's `Block.getId`. None for an unknown block,
    /// a property it doesn't have or a value it can't take, or block NBT
    /// (`{...}`); and, as `/setblock`'s argument, for anything after the
    /// block.
    pub(super) fn block_state_id(&self, name: &str) -> Option<BlockStateId> {
        parse(&self.pack.block_registries, self.pack.assets.registry, name)
    }
    /// The state `state` as `/setblock` takes it back
    /// (`BlockStateParser.serialize`): its block, then each property in
    /// name order, `minecraft:oak_stairs[facing=east,half=bottom,...]`, or
    /// the block alone for one with no properties. None for an id no state
    /// has (`Block.stateById` gives air for one).
    pub(super) fn block_state_string(&self, state: BlockStateId) -> Option<String> {
        self.pack.assets.registry.state(state).map(serialize)
    }
    /// The default state of `block` (`minecraft:oak_stairs`, or `oak_stairs`):
    /// `Block.defaultBlockState`, the one a block takes when nothing else
    /// says. None for an unknown block.
    pub(super) fn block_state_default(&self, block: BlockKey) -> Option<BlockStateId> {
        Some(mod_api::__private::resolved_block(block)?.default)
    }
    /// `BlockState.canBeReplaced()`: whether placing a block where this state stands
    /// replaces it (air, grass, water, snow layers). None for an unknown state.
    pub(super) fn block_state_replaceable(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).replaceable)
    }
    /// `BlockBehaviour.Properties.destroyTime`, as `BlockState.getDestroySpeed` reads it:
    /// stone 1.5, obsidian 50, bedrock -1 (it can't be broken). None for an unknown state.
    pub(super) fn block_state_destroy_speed(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).hardness)
    }
    /// `BlockState.getSoundType()`: its volume, pitch and the sound events for breaking,
    /// stepping on, placing, hitting and falling on it. None for an unknown state.
    pub(super) fn block_state_sound(&self, state: BlockStateId) -> Option<BlockSoundType> {
        self.pack.assets.registry.state(state)?;
        let sounds = &self.res.facts().get(state).sounds;
        Some(BlockSoundType {
            volume: sounds.volume,
            pitch: sounds.pitch,
            break_sound: Arc::clone(&sounds.break_sound),
            step: Arc::clone(&sounds.step),
            place: Arc::clone(&sounds.place),
            hit: Arc::clone(&sounds.hit),
            fall: Arc::clone(&sounds.fall),
        })
    }
    /// `BlockState.hasBlockEntity()`. None for an unknown state.
    pub(super) fn block_state_has_block_entity(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).block_entity)
    }
    /// The states of `block` (`StateDefinition.getPossibleStates`), as the consecutive ids the
    /// registry numbers them with. None for an unknown block.
    pub(super) fn block_states_of(&self, block: BlockKey) -> Option<BlockStateRange> {
        Some(mod_api::__private::resolved_block(block)?.states)
    }

    /// The properties of a resolved block, copied for the mod as plain data.
    pub(super) fn block_state_definition(&self, block: BlockKey) -> Option<Vec<StateProperty>> {
        let resolved = mod_api::__private::resolved_block(block)?;
        let state = self.pack.assets.registry.state(resolved.default)?;
        let properties = self.pack.block_registries.blocks.get(&state.name)?;
        Some(
            properties
                .iter()
                .map(|(name, values)| StateProperty::new(name.clone(), values.clone()))
                .collect(),
        )
    }

    /// Match against the existing command registry definitions without allocating on the call.
    pub(super) fn block_state_matches(
        &self,
        state: BlockStateId,
        block: BlockKey,
        constraints: &[StateConstraint],
    ) -> Option<Result<bool, StateMatchError>> {
        let resolved = mod_api::__private::resolved_block(block)?;
        let registry = self.pack.assets.registry;
        let target = registry.state(resolved.default)?;
        let state = registry.state(state)?;
        let definition = self.pack.block_registries.blocks.get(&target.name)?;
        Some(match_state_properties(
            definition,
            state.name == target.name,
            |name, value| {
                state
                    .properties
                    .get(name)
                    .is_some_and(|actual| actual == value)
            },
            constraints,
        ))
    }

    /// `Item.getCraftingRemainder`: Horizon currently stores the remainder's
    /// item id, so the result is a single plain stack.
    pub(super) fn item_crafting_remainder(&self, stack: &ItemStack) -> Option<ItemStack> {
        let item = self.res.items().get(&stack.item)?;
        let remainder = item.crafting_remainder.as_deref()?;
        Some(ItemStack::new(remainder, 1))
    }

    /// `Block.asItem`: the block's item name, including air for a block with
    /// no item; None if the block name is unknown.
    pub(super) fn block_item(&self, block: BlockKey) -> Option<String> {
        let behaviours = self.res.behaviours();
        let id = mod_api::__private::resolved_block(block)?.block;
        Some(behaviours.block_data(id).item.to_string())
    }
}

pub(super) fn match_state_properties(
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
pub(super) fn block_registries(registry: &'static BlockRegistry) -> Arc<Registries> {
    static CACHE: Mutex<Vec<(usize, Arc<Registries>)>> = Mutex::new(Vec::new());
    let key = std::ptr::from_ref(registry) as usize;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, reg)) = cache.iter().find(|(cached, _)| *cached == key) {
        return reg.clone();
    }
    let reg = Arc::new(Registries {
        blocks: (*super::chat::block_properties(registry)).clone(),
        ..Registries::default()
    });
    cache.push((key, reg.clone()));
    reg
}

/// `BlockStateParser.parseForBlock` of the whole of `name`, NBT included, as
/// `/setblock` reads it; a state with NBT has no id.
pub(super) fn parse(
    reg: &Registries,
    registry: &BlockRegistry,
    name: &str,
) -> Option<BlockStateId> {
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
pub(super) fn serialize(state: &BlockState) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Air, a stair with two properties (its default `facing=north,half=bottom`)
    /// and wheat with an integer `age`, as `parse` reads them.
    fn blocks() -> (Arc<Registries>, &'static BlockRegistry) {
        let facts = br#"{"defaults":{"minecraft:air":0,"minecraft:oak_stairs":1,"minecraft:wheat":5},"states":[
            ["air",true,false,"","minecraft:air"],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"north","half":"bottom"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"north","half":"top"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"east","half":"bottom"}}],
            ["s",false,false,"",{"id":"minecraft:oak_stairs","properties":{"facing":"east","half":"top"}}],
            ["w",false,false,"",{"id":"minecraft:wheat","properties":{"age":"0"}}],
            ["w",false,false,"",{"id":"minecraft:wheat","properties":{"age":"1"}}]
        ]}"#;
        let registry = Box::leak(Box::new(
            BlockRegistry::from_facts_json(&facts[..]).unwrap(),
        ));
        (block_registries(registry), registry)
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn item_and_block_registry_facts_use_the_loaded_tables() {
        use super::super::api_values::TryIntoApi;
        use mod_api::Host;
        super::super::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let remainder = host
                .item_crafting_remainder(
                    &ItemStack::new("minecraft:water_bucket", 1)
                        .try_into_api()
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(
                remainder.item(),
                mod_api::__private::item_key("minecraft:bucket").unwrap()
            );
            assert_eq!(remainder.count(), 1);
            assert!(
                host.item_crafting_remainder(
                    &ItemStack::new("minecraft:stone", 1).try_into_api().unwrap()
                )
                .is_none()
            );
            assert_eq!(
                host.block_item(mod_api::__private::part_tests::block_key("minecraft:stone"))
                    .as_deref(),
                Some("minecraft:stone")
            );
            assert_eq!(
                host.block_item(mod_api::__private::part_tests::block_key("minecraft:water"))
                    .as_deref(),
                Some("minecraft:air")
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fluid_definitions_and_constraints_use_the_loaded_registry() {
        use mod_api::{Host, HostFluidStates};
        super::super::native_entity_refs::tests::with_game(|game| {
            let host: &dyn Host = game;
            let water = host.fluid_key("minecraft:water").unwrap();
            let flowing = host.fluid_key("minecraft:flowing_water").unwrap();
            let service = host.fluid_states();
            for phase in [
                mod_api::__private::ModPhase::Setup,
                mod_api::__private::ModPhase::Reloading,
            ] {
                let _scope = mod_api::__private::PhaseOwner::scope(phase);
                assert!(matches!(
                    service.definition(water),
                    Some(FluidDefinition::World { .. })
                ));
                assert_eq!(service.validate(water, &[]), Ok(()));
            }
            let tank = host.fluid_key("horizon:definition_test").unwrap();
            assert_eq!(service.definition(tank), Some(FluidDefinition::TankOnly));
            assert_eq!(
                service.validate(tank, &[]),
                Err(FluidStateError::NoWorldState)
            );
            for (id, amount, source, property_count) in [
                ("minecraft:empty", 0, false, 0),
                ("minecraft:water", 8, true, 1),
                ("minecraft:flowing_water", 1, false, 2),
                ("minecraft:lava", 8, true, 1),
                ("minecraft:flowing_lava", 1, false, 2),
            ] {
                let key = host.fluid_key(id).unwrap();
                let FluidDefinition::World {
                    default,
                    properties,
                } = service.definition(key).unwrap()
                else {
                    panic!("world definition")
                };
                assert_eq!(
                    (
                        default.fluid,
                        default.amount,
                        default.source,
                        default.falling
                    ),
                    (key, amount, source, amount != 0)
                );
                assert_eq!(properties.len(), property_count);
                assert_eq!(service.validate(key, &[]), Ok(()));
            }
            assert_eq!(
                service.validate(
                    flowing,
                    &[
                        StateConstraint::new("unknown", "true"),
                        StateConstraint::new("level", "1"),
                        StateConstraint::new("level", "1")
                    ]
                ),
                Err(FluidStateError::UnknownProperty(0))
            );
            let FluidDefinition::World {
                default,
                properties,
            } = service.definition(water).unwrap()
            else {
                panic!("world fluid")
            };
            assert!(default.is_source());
            assert!(properties.iter().any(|p| p.name == "falling"));
            assert_eq!(
                service.validate(water, &[StateConstraint::new("level", "8")]),
                Err(FluidStateError::UnknownProperty(0))
            );
            assert_eq!(
                service.validate(flowing, &[StateConstraint::new("level", "9")]),
                Err(FluidStateError::InvalidValue(0))
            );
            assert_eq!(
                service.validate(
                    flowing,
                    &[
                        StateConstraint::new("level", "8"),
                        StateConstraint::new("level", "8")
                    ]
                ),
                Err(FluidStateError::Duplicate(1))
            );
            assert!(default.matches(&[StateConstraint::new("falling", "true")]));
            assert!(!default.matches(&[StateConstraint::new("unknown", "true")]));
            let FluidDefinition::World {
                default: flowing_default,
                ..
            } = service.definition(flowing).unwrap()
            else {
                panic!("flowing fluid")
            };
            assert!(flowing_default.matches(&[StateConstraint::new("level", "1")]));
            assert!(!flowing_default.matches(&[StateConstraint::new("level", "8")]));
        });
    }

    #[test]
    fn block_property_constraints_validate_and_match_partial_states() {
        let (reg, registry) = blocks();
        let target = registry.state(BlockStateId(1)).unwrap();
        let definition = &reg.blocks["minecraft:oak_stairs"];
        let check = |id, constraints: &[StateConstraint]| {
            let state = registry.state(BlockStateId(id)).unwrap();
            match_state_properties(
                definition,
                target.name == state.name,
                |name, value| {
                    state
                        .properties
                        .get(name)
                        .is_some_and(|actual| actual == value)
                },
                constraints,
            )
        };
        let east = StateConstraint::new("facing", "east");
        assert_eq!(check(4, std::slice::from_ref(&east)), Ok(true));
        assert_eq!(check(1, std::slice::from_ref(&east)), Ok(false));
        assert_eq!(check(0, std::slice::from_ref(&east)), Ok(false));
        assert_eq!(
            check(4, &[StateConstraint::new("shape", "straight")]),
            Err(StateMatchError::UnknownProperty(0))
        );
        assert_eq!(
            check(4, &[StateConstraint::new("facing", "up")]),
            Err(StateMatchError::InvalidValue(0))
        );
        assert_eq!(check(4, &[]), Ok(true));
        assert_eq!(definition.len(), 2);
        assert!(
            definition
                .iter()
                .any(|(name, values)| name == "half" && values.contains(&"top".to_owned()))
        );
    }

    #[test]
    fn a_state_name_reads_as_setblock_reads_it_and_round_trips() {
        let (reg, registry) = blocks();
        let id = |name: &str| parse(&reg, registry, name);
        assert_eq!(id("minecraft:air"), Some(BlockStateId(0)));
        assert_eq!(id("oak_stairs"), Some(BlockStateId(1)));
        assert_eq!(
            id("oak_stairs[half=top,facing=east]"),
            Some(BlockStateId(4))
        );
        assert_eq!(
            id("minecraft:oak_stairs[facing=east]"),
            Some(BlockStateId(3))
        );
        // IntegerProperty.getValue: Integer.parseInt.
        assert_eq!(id("wheat[age=01]"), Some(BlockStateId(6)));
        for state in 0..7 {
            let text = serialize(registry.state(BlockStateId(state)).unwrap());
            assert_eq!(id(&text), Some(BlockStateId(state)), "{text}");
        }
        assert_eq!(
            serialize(registry.state(BlockStateId(4)).unwrap()),
            "minecraft:oak_stairs[facing=east,half=top]"
        );
        assert_eq!(
            serialize(registry.state(BlockStateId(0)).unwrap()),
            "minecraft:air"
        );
    }

    #[test]
    fn a_state_name_that_does_not_read_has_no_id() {
        let (reg, registry) = blocks();
        for name in [
            "minecraft:nothing",
            "oak_stairs[shape=straight]",
            "oak_stairs[facing=up]",
            "oak_stairs[facing=east,facing=north]",
            "wheat[age=2]",
            "oak_stairs{}",
            "oak_stairs extra",
            "#minecraft:stairs",
            "",
        ] {
            assert_eq!(parse(&reg, registry, name), None, "{name:?}");
        }
    }

    #[test]
    fn a_block_key_resolves_its_default_state() {
        use mod_api::__private::{ResolvedBlock, part_tests, resolve_blocks, resolved_block};
        let (_, registry) = blocks();
        mod_api::__private::install(Default::default());
        crate::game::api_values::register_game_keys([
            "minecraft:oak_stairs".to_owned(),
            "minecraft:wheat".to_owned(),
        ]);
        resolve_blocks(|id| {
            let default = registry.default_state(id)?;
            Some(ResolvedBlock {
                default,
                states: BlockStateRange::new(default, 1),
                block: horizon_world::level::states::BlockId(0),
            })
        })
        .unwrap();
        assert_eq!(
            resolved_block(part_tests::block_key("oak_stairs"))
                .unwrap()
                .default,
            BlockStateId(1)
        );
        assert_eq!(
            resolved_block(part_tests::block_key("minecraft:wheat"))
                .unwrap()
                .default,
            BlockStateId(5)
        );
        // A valid key has no mapping on a different owning thread. Raw unknown slots are
        // refused by the generated host boundary, not revalidated by typed operations.
        let key = part_tests::block_key("minecraft:wheat");
        assert!(
            std::thread::spawn(move || resolved_block(key))
                .join()
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod facts_tests {
    use mod_api::{BlockStateId, HostBlockStates};

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn block_state_facts_read_vanillas_tables_through_the_host() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let state = |name: &str| game.pack.assets.registry.default_state(name).unwrap();
            let (stone, obsidian, bedrock) = (
                state("minecraft:stone"),
                state("minecraft:obsidian"),
                state("minecraft:bedrock"),
            );
            let (air, grass, chest) = (
                state("minecraft:air"),
                state("minecraft:short_grass"),
                state("minecraft:chest"),
            );
            let stairs = state("minecraft:oak_stairs");
            let facts = game.block_states();
            // destroyTime: stone 1.5, obsidian 50, bedrock -1 (unbreakable).
            assert_eq!(facts.destroy_speed(stone), Some(1.5));
            assert_eq!(facts.destroy_speed(obsidian), Some(50.0));
            assert_eq!(facts.destroy_speed(bedrock), Some(-1.0));
            // canBeReplaced: air and grass, not stone.
            assert_eq!(facts.replaceable(air), Some(true));
            assert_eq!(facts.replaceable(grass), Some(true));
            assert_eq!(facts.replaceable(stone), Some(false));
            assert_eq!(facts.has_block_entity(chest), Some(true));
            assert_eq!(facts.has_block_entity(stone), Some(false));
            // SoundType.STONE: its break event, at volume and pitch 1.
            let sound = facts.sound(stone).unwrap();
            assert_eq!(&*sound.break_sound, "minecraft:block.stone.break");
            assert_eq!((sound.volume, sound.pitch), (1.0, 1.0));
            // Oak stairs: facing 4 × half 2 × shape 5 × waterlogged 2 = 80 states, its default
            // among them; stone has one.
            let range = facts
                .states_of(mod_api::__private::part_tests::block_key(
                    "minecraft:oak_stairs",
                ))
                .unwrap();
            assert_eq!(range.len(), 80);
            assert!(range.contains(stairs));
            assert!(range.iter().all(
                |s| game.pack.assets.registry.state(s).unwrap().name == "minecraft:oak_stairs"
            ));
            assert_eq!(
                facts
                    .states_of(mod_api::__private::part_tests::block_key("minecraft:stone"))
                    .map(|r| (r.first(), r.len())),
                Some((stone, 1))
            );
            // Unknown state ids: None.

            assert_eq!(facts.destroy_speed(BlockStateId(u32::MAX)), None);
            assert_eq!(facts.sound(BlockStateId(u32::MAX)), None);
        });
    }
}
