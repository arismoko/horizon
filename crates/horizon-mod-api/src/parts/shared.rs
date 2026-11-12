//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

pub(crate) fn insert_part<P: ?Sized>(
    table: &mut HashMap<Box<str>, Vec<Attached<P>>>,
    id: &str,
    part: Attached<P>,
    refused: &mut Vec<String>,
) -> bool {
    let list = table.entry(id.into()).or_default();
    if let Some(other) = list.iter().find(|other| other.type_name == part.type_name) {
        refused.push(format!(
            "{} is attached to {id} twice, by {} and {}",
            part.type_name, other.mod_id, part.mod_id
        ));
        return false;
    }
    list.push(part);
    true
}

thread_local! {
    // A reference to the registrar's canonical requests while Hooks::register is in flight.
    // The table is moved, not copied, into Installed when registration finishes.
    pub(crate) static REGISTERING_KEYS: RefCell<Option<Rc<RefCell<horizon_mod_runtime::registry_keys::Requests>>>> = const { RefCell::new(None) };
}

pub(crate) fn with_registering_keys<T>(
    read: impl FnOnce(Option<&horizon_mod_runtime::registry_keys::Requests>) -> T,
) -> T {
    REGISTERING_KEYS.with(|keys| match keys.borrow().as_ref() {
        Some(keys) => read(Some(&keys.borrow())),
        None => read(None),
    })
}

// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
/// The game's [`Registrar`]: every mod's registrations, checked as they come.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct PartRegistrar {
    pub(crate) biome_modifiers: Vec<BiomeModifierDeclaration>,
    pub(crate) datapack_registries: Vec<crate::DatapackRegistryDeclaration>,
    pub(crate) menu_recipe_books: HashMap<String, MenuBookDeclaration>,
    pub(crate) self_refusal: Option<String>,
    pub(crate) recipe_book_types: HashMap<String, String>,
    pub(crate) game_rule_categories: Vec<String>,
    pub(crate) game_rules: Vec<crate::GameRuleDecl>,
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    /// Parts registered so far, by every mod: the next part's `order`.
    pub(crate) registered: usize,
    pub(crate) parts: Parts,
    pub(crate) tags: TagParts,
    pub(crate) block_entity_types: Vec<BlockEntityTypeDecl>,
    pub(crate) block_entity_transforms: HashMap<String, Arc<dyn crate::BlockEntityTransform>>,
    pub(crate) entity_attributes: Vec<EntityAttributeDeclaration>,
    pub(crate) world_fluids: Vec<(String, crate::WorldFluid)>,
    pub(crate) entity_attribute_modifications: Vec<EntityAttributeModification>,
    pub(crate) attributes: Vec<AttributeDeclaration>,
    pub(crate) general_entity_types: Vec<GeneralEntityTypeDeclaration>,
    pub(crate) spawn_placements: Vec<SpawnPlacementDeclaration>,
    pub(crate) registry_keys: Rc<RefCell<horizon_mod_runtime::registry_keys::Requests>>,
    pub(crate) effects: OnceLock<horizon_mod_runtime::registry_keys::Effects>,
    pub(crate) controllers: Vec<ControllerInfo>,
    pub(crate) arm_poses: Vec<PoseInfo>,
    pub(crate) structure_types: Vec<StructureTypeInfo>,
    pub(crate) dimensions: Vec<DimensionInfo>,
    pub(crate) dimension_shape: fn(&crate::DimensionSettings) -> bool,
    pub(crate) chunk_generators: Vec<ChunkGeneratorInfo>,
    pub(crate) refused: Vec<String>,
}

impl PartRegistrar {
    pub(crate) fn new(mod_id: &'static str) -> Self {
        let _ = crate::__private::initialize();
        Self {
            menu_recipe_books: HashMap::new(),
            self_refusal: None,
            biome_modifiers: Vec::new(),
            datapack_registries: Vec::new(),
            recipe_book_types: HashMap::new(),
            game_rule_categories: Vec::new(),
            game_rules: Vec::new(),
            mod_id,
            mod_index: 0,
            registered: 0,
            parts: Parts::default(),
            tags: TagParts::default(),
            block_entity_types: Vec::new(),
            block_entity_transforms: HashMap::new(),
            entity_attributes: Vec::new(),
            world_fluids: Vec::new(),
            entity_attribute_modifications: Vec::new(),
            attributes: Vec::new(),
            general_entity_types: Vec::new(),
            spawn_placements: Vec::new(),
            registry_keys: Default::default(),
            effects: OnceLock::new(),
            controllers: Vec::new(),
            arm_poses: Vec::new(),
            structure_types: Vec::new(),
            dimensions: Vec::new(),
            dimension_shape: |_| false,
            chunk_generators: Vec::new(),
            refused: Vec::new(),
        }
    }
    /// `part`, of kind `kind`, for tag `tag`; none if it is refused.
    pub(crate) fn tagged<P: ?Sized>(
        &mut self,
        kind: &'static str,
        tag: &str,
        part: Arc<P>,
    ) -> Option<(Box<str>, Attached<P>)> {
        if tag.starts_with('#') || !namespaced(tag) {
            self.refused.push(format!(
                "{} attached {kind} to tag {tag:?}, which is not a namespaced tag name (write it without #)",
                self.mod_id
            ));
            return None;
        }
        self.registered += 1;
        Some((
            tag.into(),
            Attached {
                mod_id: self.mod_id,
                mod_index: self.mod_index,
                order: self.registered,
                type_name: kind,
                part,
            },
        ))
    }

    /// Attach `part`, of kind `kind`, to `id` in `table`; false if it is refused.
    pub(crate) fn attach<P: ?Sized>(
        &mut self,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<Attached<P>>>,
        kind: &'static str,
        id: &str,
        part: Arc<P>,
    ) -> bool {
        if !namespaced(id) {
            self.refused.push(format!(
                "{} attached {kind} to {id:?}, which is not a namespaced id",
                self.mod_id
            ));
            return false;
        }
        self.attach_known(table, kind, id, part)
    }

    pub(crate) fn attach_known<P: ?Sized>(
        &mut self,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<Attached<P>>>,
        kind: &'static str,
        id: &str,
        part: Arc<P>,
    ) -> bool {
        let part = Attached {
            mod_id: self.mod_id,
            mod_index: self.mod_index,
            order: self.registered + 1,
            type_name: kind,
            part,
        };
        let inserted = insert_part(table(&mut self.parts), id, part, &mut self.refused);
        if inserted {
            self.registered += 1;
        }
        inserted
    }
}

pub(crate) fn namespaced(id: &str) -> bool {
    id.split_once(':')
        .is_some_and(|(namespace, path)| !namespace.is_empty() && !path.is_empty())
}

/// Every mod's [`Hooks::register`](crate::Hooks::register) run against one registrar, in dispatch order.
pub(crate) fn register_all(
    mods: &[ModEntry],
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) -> PartRegistrar {
    let mut registrar = PartRegistrar::new(mods.first().map_or("horizon", |entry| {
        mod_vocab::__private::parts_installation::mod_entry_id(entry)
    }));
    registrar.dimension_shape = dimension_shape;
    struct RestoreKeys(Option<Rc<RefCell<horizon_mod_runtime::registry_keys::Requests>>>);
    impl Drop for RestoreKeys {
        fn drop(&mut self) {
            REGISTERING_KEYS.set(self.0.take());
        }
    }
    let _active = ActiveRegistration::enter();
    let _scope = RestoreKeys(REGISTERING_KEYS.replace(Some(registrar.registry_keys.clone())));
    for (index, hooks) in mod_vocab::__private::entered_registration(mods, "register").enumerate() {
        registrar.mod_id = mod_vocab::__private::parts_installation::mod_entry_id(&mods[index]);
        registrar.mod_index = index;
        let context = crate::RegistrationContext::new();
        let rollback = registrar.registration_checkpoint();
        hooks.register(&mut registrar, &context);
        let reason = mod_vocab::__private::take_registration_refusal(&context)
            .or_else(|| registrar.self_refusal.take());
        if let Some(reason) = reason {
            rollback(&mut registrar);
            registrar.refused.push(format!(
                "mod {} refused registration: {reason}",
                mod_vocab::__private::parts_installation::mod_entry_id(&mods[index])
            ));
        }
    }
    registrar.expand_block_entity_parts();
    registrar.refuse_unknown_dimension_generators();
    registrar
}

/// Builds the parts from every mod's [`Hooks::register`](crate::Hooks::register), in dispatch order. Once, at startup;
/// the error lists every part refused, naming its mods.
pub fn install_parts(
    mods: &[ModEntry],
    schema: &mod_vocab::__private::RuleSchema<'_>,
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) -> Result<(), Vec<String>> {
    install_registrar(register_all(mods, dimension_shape), schema)
}

pub(crate) fn install_registrar(
    mut registrar: PartRegistrar,
    schema: &mod_vocab::__private::RuleSchema<'_>,
) -> Result<(), Vec<String>> {
    let slots = schema.validate(&registrar.game_rule_categories, &registrar.game_rules);
    if let Err(errors) = &slots {
        registrar.refused.extend(errors.iter().cloned());
    }
    if !registrar.refused.is_empty() {
        return Err(registrar.refused);
    }
    let game_rule_slots = slots.expect("every schema refusal was collected");
    let installed = Installed {
        biome_modifiers: registrar.biome_modifiers,
        datapack_registries: registrar.datapack_registries,
        menu_recipe_books: registrar.menu_recipe_books,
        recipe_book_types: registrar.recipe_book_types,
        game_rule_slots,
        game_rules: registrar.game_rules,
        game_rule_categories: registrar.game_rule_categories,
        keys: registrar.parts,
        tags: registrar.tags,
        block_entity_types: registrar.block_entity_types,
        block_entity_transforms: registrar.block_entity_transforms,
        entity_attributes: registrar.entity_attributes,
        world_fluids: registrar.world_fluids,
        attributes: registrar.attributes,
        general_entity_types: registrar.general_entity_types,
        entity_attribute_modifications: registrar.entity_attribute_modifications,
        spawn_placements: registrar.spawn_placements,
        resolved_block_entity_types: OnceLock::new(),
        registry_keys: Rc::try_unwrap(registrar.registry_keys)
            .unwrap_or_else(|_| unreachable!("registration scope has ended"))
            .into_inner(),
        effects: registrar.effects,
        fluids: OnceLock::new(),
        resolved_items: OnceLock::new(),
        resolved_blocks: OnceLock::new(),
        particles: OnceLock::new(),
        controllers: registrar.controllers,
        arm_poses: registrar.arm_poses,
        structure_types: registrar.structure_types,
        dimensions: registrar.dimensions,
        chunk_generators: registrar.chunk_generators,
    };
    mod_vocab::__private::parts_installation::install(installed);

    Ok(())
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    use super::super::items::tests::Cell;
    use super::mark_game_thread;
    use super::*;
    use crate::parts::tests::*;
    use crate::*;

    use std::sync::Arc;

    /// Install fixture parts with the test rule schema.
    pub fn install_parts_fixture(mods: &[ModEntry]) -> Result<(), Vec<String>> {
        // This API-only fixture has no engine dimension codec. Dimension integration tests
        // use the game's installer, which supplies its real shape validator.
        install_parts(mods, &rule_schema(), |_| false)
    }
    /// A small independent vanilla-schema fixture for tests without a game.
    pub fn rule_schema() -> crate::__private::RuleSchema<'static> {
        crate::__private::RuleSchema {
            category: &|id| matches!(id, "minecraft:drops" | "minecraft:misc"),
            position: &|_| None,
            rule_count: 0,
        }
    }

    /// Register the game fixture's final block registry through the actual registrar, retaining
    /// its installed test mods and Parts. Production mods request only the entries they use.
    pub fn register_game_keys(
        ids: impl IntoIterator<Item = String>,
        schema: &mod_vocab::__private::RuleSchema<'_>,
        dimension_shape: fn(&crate::DimensionSettings) -> bool,
    ) {
        let mut registrar = super::register_all(crate::__private::mods(), dimension_shape);
        registrar.mod_id = "horizon";
        registrar.declare_fluid("definition_test");
        for id in ids {
            registrar.block_key(&id);
        }
        super::install_registrar(registrar, schema).unwrap();
        mod_vocab::__private::resolve_fluids(
            &[
                "minecraft:empty",
                "minecraft:water",
                "minecraft:flowing_water",
                "minecraft:lava",
                "minecraft:flowing_lava",
            ],
            // The engine's world fluids in vanilla registry order, as the game resolves them.
            &[
                "minecraft:empty",
                "minecraft:flowing_water",
                "minecraft:water",
                "minecraft:flowing_lava",
                "minecraft:lava",
            ],
        )
        .unwrap();
    }

    /// A previously requested block in the game fixture. Unknown names are a fixture error.
    pub fn block_key(id: &str) -> BlockKey {
        let canonical = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        mod_vocab::__private::parts_installation::requested_block_key(&canonical)
            .expect("fixture requested this block")
    }

    /// A registered effect exposed by the real load-owned mapping, also for unrequested vanilla entries.
    pub fn effect_key(id: &str) -> EffectKey {
        let canonical = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        mod_vocab::__private::parts_installation::resolved_effect_key(&canonical)
            .expect("fixture registered this effect")
    }

    #[cfg(test)]
    pub(crate) fn type_key(id: &str) -> BlockEntityTypeKey {
        mod_vocab::__private::parts_installation::requested_block_entity_type_key(id)
            .expect("fixture requested this type")
    }

    /// The level the test worlds are.
    pub(crate) fn here() -> LevelRef {
        LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::OVERWORLD)
    }

    /// The block at `pos` in [`here`], through `side`.
    pub(crate) fn on(pos: [i32; 3], side: Option<Direction>) -> BlockSide {
        mod_vocab::__private::parts_installation::block_side(here(), pos, side)
    }

    #[test]
    pub(crate) fn registry_keys_reject_unknown_unattached_requests_with_the_requesting_mod() {
        struct Unknown;
        #[mod_api::__subscriptions]
        impl Hooks for Unknown {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.block_key("absent:block");
                r.effect_key("absent:effect");
            }
        }
        install_parts(
            &[ModEntry::new("lookup", Box::new(Unknown))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        assert_eq!(
            mod_vocab::__private::resolve_block_keys(std::iter::empty(), |_| unreachable!())
                .unwrap_err(),
            ["mod lookup requested unknown block absent:block"]
        );
        assert_eq!(
            mod_vocab::__private::resolve_effects(std::iter::empty(), |_| None).unwrap_err(),
            ["mod lookup requested unknown effect absent:effect"]
        );
    }

    #[test]
    pub(crate) fn invalid_lookup_ids_refuse_registration_naming_the_requesting_mod() {
        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                for id in ["minecraft:", "Upper:chest", "test:has space"] {
                    r.block_entity_type_key(id);
                }
            }
        }
        let errors = install_parts(
            &[ModEntry::new("lookup", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(
            errors
                .iter()
                .all(|error| error.starts_with("mod lookup: invalid registry key"))
        );
    }
    #[cfg(test)]
    pub(crate) struct NoHost;
    #[cfg(test)]
    impl Host for NoHost {}
    #[cfg(test)]
    mod_api::impl_host_areas!(NoHost);
    #[cfg(test)]
    impl ServerHost for NoHost {}
    #[cfg(test)]
    mod_api::impl_server_host_areas!(NoHost);
    #[cfg(test)]
    impl ClientHost for NoHost {}
    #[cfg(test)]
    mod_api::impl_client_host_areas!(NoHost);

    #[test]
    pub(crate) fn a_host_without_a_backend_refuses_handle_calls() {
        // NoHost implements nothing, so Host::backend keeps its None default:
        // every handle answers its typed refusal, never a no-op success.
        let level = LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::OVERWORLD);
        assert!(!level.in_place(&NoHost));
        assert_eq!(
            level.entities_in(&NoHost, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            None
        );
    }

    /// Doubles every comparator reading, and attaches `Gauge` to `test:gauge`, `TagGauge` to
    /// tag `test:gauged` and `Press` to `test:press`.
    pub(crate) struct Doubler;
    #[mod_api::__subscriptions]
    impl Hooks for Doubler {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let block_key = r.block_key("test:gauge");
            r.block_part(block_key, Gauge);
            let block_key = r.block_key("test:connector");
            r.block_part(block_key, Connector);
            r.block_part_tag("test:gauged", TagGauge);
            let kind = r
                .block_entity_type("press_entity")
                .blocks(&["test:press"])
                .build();
            r.block_entity_part(&kind, Press).expect("native type");
            let hoard = r.block_entity_type("hoard").blocks(&["test:hoard"]).build();
            let granite = r.block_key("minecraft:granite");
            r.block_entity_part(&hoard, Hoard(granite))
                .expect("native type");
            // These handlers wrap the test hosts' own slots and tanks, as a mod replacing how a
            // block entity's are reached does: ahead of the game's.
            let pipe = r.block_entity_type_key("test:pipe");
            r.block_entity_capability_before_game(&ITEMS, &pipe, |_, at| Some(Box::new(Pipe(at))));
            let greedy = r.block_entity_type_key("test:greedy");
            r.block_entity_capability_before_game(&ITEMS, &greedy, |_, at| {
                Some(Box::new(Greedy(at)))
            });
            let item_overstates = r.block_entity_type_key("test:item_overstates");
            r.block_entity_capability_before_game(&ITEMS, &item_overstates, |_, _| {
                Some(Box::new(ItemOverstates))
            });
            r.entity_capability(&ENTITY_ITEMS, "test:robot", |_, _, ()| {
                Some(Box::new(Cargo))
            });
            // A battery reached from any side but below, as a mod's sided entity storage.
            r.entity_capability(
                &ENTITY_ENERGY,
                "minecraft:armor_stand",
                |_, entity, side| {
                    (side != Some(Direction::Down)).then(|| Box::new(EntityBattery(entity)) as _)
                },
            );
            r.entity_capability(&ENTITY_FLUIDS, "minecraft:armor_stand", |_, entity, _| {
                Some(Box::new(EntityTank(entity)))
            });
            r.item_capability_before_game(&ITEM_ITEMS, "test:pouch", |_, place| {
                Some(Box::new(Pouch(place.clone())))
            });
            r.item_capability(&ITEM_ENERGY, "test:cell", |_, place| {
                Some(Box::new(Cell(place.clone())))
            });
            r.item_capability_before_game(&ITEM_ITEMS, "test:bag", |_, place| {
                Some(Box::new(Bag(place.clone())))
            });
            r.item_capability_before_game(&ITEM_ITEMS, "test:case", |_, place| {
                Some(Box::new(Case(place.clone())))
            });
            r.place_provider("test:charms", Arc::new(Charms));
            r.place_provider("test:lying-place", Arc::new(deep_places::LyingPlace));
            r.item_capability_before_game(&ITEM_ITEMS, "test:outer", |_, place| {
                Some(Box::new(deep_places::Wrap(place.clone())))
            });
            // `test:ordered`: one provider that declines, two that answer, and one ahead of the
            // game that answers only at x = 9.
            let ordered = r.block_entity_type_key("test:ordered");
            r.block_entity_capability(&ITEMS, &ordered, |_, _| None);
            r.block_entity_capability(&ITEMS, &ordered, |_, _| Some(Box::new(Marked(1))));
            r.block_entity_capability(&ITEMS, &ordered, |_, _| Some(Box::new(Marked(2))));
            r.block_entity_capability_before_game(&ITEMS, &ordered, |_, at| {
                (at.pos[0] == 9).then(|| Box::new(Marked(3)) as _)
            });
            r.block_entity_capability_before_game(&ITEMS, &ordered, |_, at| {
                (at.pos[0] >= 9).then(|| Box::new(Marked(4)) as _)
            });
            // `test:ordered_item` the same, the one ahead of the game answering for two items.
            r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| None);
            r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
                Some(Box::new(Marked(1)))
            });
            r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
                Some(Box::new(Marked(2)))
            });
            r.item_capability_before_game(&ITEM_ITEMS, "test:ordered_item", |host, place| {
                let two = place.amount(host) == 2;
                two.then(|| Box::new(Marked(3)) as _)
            });
            for (id, lie) in [
                ("test:tank", None),
                ("test:gives_more", Some(Lie::GivesMore)),
                ("test:takes_more", Some(Lie::TakesMore)),
            ] {
                let kind = r.block_entity_type_key(id);
                r.block_entity_capability_before_game(&FLUIDS, &kind, move |_, at| {
                    Some(Box::new(ModTank { at, lie }))
                });
            }
            for (id, lie) in [
                ("test:battery_gives_more", Lie::GivesMore),
                ("test:battery_takes_more", Lie::TakesMore),
            ] {
                let kind = r.block_entity_type_key(id);
                r.block_entity_capability_before_game(&ENERGY, &kind, move |_, at| {
                    Some(Box::new(ModBattery { at, lie }))
                });
            }
            let chem_tank = r.block_entity_type_key("test:chem_tank");
            r.block_entity_capability(&CHEMICALS, &chem_tank, |_, at| Some(Box::new(ChemTank(at))));
        }
    }

    #[cfg(test)]
    /// State 1 is a `test:gauge`, state 2 a `test:other`.
    pub(crate) struct Level;
    #[cfg(test)]
    impl LevelHost for Level {
        fn block_state(&self, _: [i32; 3]) -> Option<crate::BlockState> {
            None
        }
        fn state_with_property(&self, _: BlockState, _: &str, _: &str) -> Option<BlockState> {
            None
        }
        fn state_block(&self, state: BlockState) -> Option<&str> {
            let metadata = crate::__private::block_definitions()?;
            let slot =
                mod_vocab::__private::parts_installation::block_key_slot(state.block()) as usize;
            if u32::from(state.ordinal()) >= metadata.definitions.get(slot)?.state_count() {
                return None;
            }
            metadata
                .names
                .entries
                .get(slot)
                .map(|entry| entry.id.as_str())
        }
    }

    pub(crate) struct FixtureContent;
    #[mod_api::__subscriptions]
    impl Hooks for FixtureContent {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            r.declare_fluid("potion");
            for id in CAPABILITY_TYPES {
                r.block_entity_type(id)
                    .blocks(&[&format!("test:{id}")])
                    .build();
            }
        }
        fn blocks(&self, _context: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
            event.blocks.extend(
                ["gauge", "connector", "press", "hoard"]
                    .into_iter()
                    .chain(CAPABILITY_TYPES.iter().copied())
                    .map(|id| mod_api::Block::new(format!("test:{id}"), "minecraft:stone")),
            );
        }
    }

    /// The test mods, their parts installed, on a thread that runs a game.
    pub fn game_thread() -> crate::Mods {
        let mods: crate::Mods = vec![
            ModEntry::new("doubler", Box::new(Doubler)),
            ModEntry::new("chemist", Box::new(Chemist)),
            ModEntry::new("test", Box::new(FixtureContent)),
        ]
        .into();
        crate::__private::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mod_vocab::__private::resolve_fluids(
            &[
                "minecraft:water",
                "minecraft:lava",
                "minecraft:flowing_water",
                "minecraft:flowing_lava",
                "minecraft:empty",
            ],
            &[],
        )
        .unwrap();
        mod_vocab::__private::resolve_block_keys(
            [
                "minecraft:air",
                "minecraft:granite",
                "test:gauge",
                "test:other",
                "test:connector",
                "test:press",
            ],
            |_| {
                horizon_mod_runtime::block_definitions::BlockDefinition::new(
                    &[&Default::default()],
                    0,
                )
                .map_err(|e| format!("{e:?}"))
            },
        )
        .unwrap();
        mark_game_thread();
        mods
    }

    /// How a mod tank misanswers: an extract takes and answers 100 mB more than asked
    /// (`GivesMore`), or an insert answers 100 mB more than went in (`TakesMore`).
    #[derive(Clone, Copy, PartialEq)]
    pub(crate) enum Lie {
        GivesMore,
        TakesMore,
    }
}
