//! Native item data component helpers.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_core::nbt::Tag;

/// Failure to access a native item component.
#[derive(Debug)]
pub enum NativeComponentError {
    UnknownType,
    InvalidValue(horizon_protocol::Error),
}
impl std::fmt::Display for NativeComponentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownType => f.write_str("unregistered component type"),
            Self::InvalidValue(error) => write!(f, "invalid component value: {error}"),
        }
    }
}
impl std::error::Error for NativeComponentError {}

impl Game<'_> {
    /// Item `item`'s default value of component `component`, in its prototype
    /// (`Item.components().get`): `minecraft:max_damage` on an iron sword is `250`, data
    /// packs' and native mods' changes included. Read from the item report's JSON, so a
    /// number is an `Int`, `Long` or `Double` and a boolean a `Byte`. None where the item
    /// hasn't the component, and for an unknown item.
    pub(super) fn item_default_component(
        &self,
        item: mod_api::ItemKey,
        component: &str,
    ) -> Option<Tag> {
        let item = mod_api::__private::item_key_id(item)?;
        let prototype = self.pack.item_renderer.models.prototypes.get(item)?;
        Some(horizon_client::scoreboard::json_to_tag(
            prototype.get(component)?,
        ))
    }
    /// Item `item`'s cooking-fuel burn time in ticks (`FuelValues.burnDuration` of its `cooking_fuel`
    /// component, the data pack's context int provider resolved), None where it is no fuel. API
    /// @API:item_components@.
    pub(super) fn item_burn_time(&self, item: &str) -> Option<i32> {
        let (time, _) = self.items().get(item)?.fuel.as_ref()?;
        Some(self.res.providers().int_value(time))
    }
    /// Item `item`'s chance to raise a composter's level (its `compostable` component's layers provider
    /// resolved), None where it is not compostable. API @API:item_components@.
    pub(super) fn item_compost_chance(&self, item: &str) -> Option<f32> {
        let layers = self.items().get(item)?.compostable.as_ref()?;
        Some(self.res.providers().layer_chance(layers))
    }
    /// The slot `stack` is equipped to (`Equippable.slot`) after its component patch, behind
    /// `ItemResource::equippable_slot`, through the opaque backend. API @API:item_components@.
    pub(in crate::game) fn item_equippable_slot(&self, stack: &ItemStack) -> Option<String> {
        horizon_client::item_sync::equippable_slot(stack, self.items())
    }
    /// `InstrumentItem.getUseDuration`: effective named or inline instrument; no instrument means zero.
    pub(in crate::game) fn native_item_instrument_duration(&self, stack: &ItemStack) -> i32 {
        if !stack.carries_component(self.items().get(&stack.item), "minecraft:instrument") {
            return 0;
        }
        if let Some(value) = stack
            .data()
            .and_then(|data| data.other("minecraft:instrument"))
        {
            return match value {
                Tag::String(id) => self
                    .items()
                    .instruments
                    .get(&id.to_string_lossy())
                    .map_or(0, |value| value.use_ticks()),
                _ => value
                    .get("use_duration")
                    .and_then(horizon_items::properties::component_float)
                    .map_or(0, horizon_items::items::instrument_use_ticks),
            };
        }
        let Some(value) = self
            .pack
            .item_renderer
            .models
            .prototypes
            .get(&*stack.item)
            .and_then(|item| item.get("minecraft:instrument"))
        else {
            return 0;
        };
        match value.as_str() {
            Some(id) => self
                .items()
                .instruments
                .get(id)
                .map_or(0, |value| value.use_ticks()),
            None => value
                .get("use_duration")
                .and_then(serde_json::Value::as_f64)
                .map_or(0, |seconds| {
                    horizon_items::items::instrument_use_ticks(seconds as f32)
                }),
        }
    }
    /// `ItemStack.getUseDuration`, behind ItemResource's read; gameplay owns the Part call.
    pub(in crate::game) fn item_resource_use_duration(&self, stack: &ItemStack) -> Option<i32> {
        self.items().get(&stack.item)?;
        Some(self.use_duration(stack))
    }
    /// `ItemStack.isBarVisible`, `getBarWidth`, `getBarColor`, before client-only hooks.
    pub(in crate::game) fn item_resource_bar(&self, stack: &ItemStack) -> Option<mod_api::ItemBar> {
        use super::api_values::TryIntoApi;
        let snapshot = stack.try_into_api().ok()?;
        let resource = mod_api::ItemResource::of(&snapshot);
        let component = |id| {
            resource.value(
                self,
                &mod_api::ComponentId::new(id).expect("built-in component"),
            )
        };
        if self.items().get(&stack.item)?.bundle {
            let contents = component("minecraft:bundle_contents")?;
            let patch = Tag::compound([("minecraft:bundle_contents", contents)]);
            let (damage, extra) = horizon_items::item_data::decode_components(Some(&patch));
            let effective = ItemStack {
                item: stack.item.clone(),
                count: 1,
                damage,
                extra,
            };
            let (width, color) =
                horizon_items::bundle::Contents::of(self.items(), &effective)?.bar()?;
            return Some(mod_api::ItemBar { width, color });
        }
        if !resource.damageable(self) {
            return None;
        }
        let max_damage = component("minecraft:max_damage")
            .as_ref()
            .and_then(horizon_items::properties::component_int)?;
        let damage = component("minecraft:damage")
            .as_ref()
            .and_then(horizon_items::properties::component_int)?;
        horizon_items::properties::damage_bar(damage, max_damage)
    }
    /// Whether enchantments `a` and `b` may share an item (`Enchantment.areCompatible`), by the
    /// world's enchantment definitions. API @API:item_components@.
    pub(in crate::game) fn enchantments_compatible(&self, a: &str, b: &str) -> bool {
        world_stations!(self).enchantments.compatible(a, b)
    }
    /// Whether an enchantment on `stack` has effect component `effect`, by the world's
    /// enchantment definitions (`EnchantmentHelper.has`), behind
    /// `ItemResource::has_enchantment_effect`, through the opaque backend. API
    /// @API:item_components@.
    pub(super) fn item_has_enchantment_effect(&self, stack: &ItemStack, effect: &str) -> bool {
        world_stations!(self)
            .enchantments
            .has_component(stack, effect)
    }
}

// Native mods use these helpers.
#[allow(dead_code)]
impl Game<'_> {
    /// Read a native component, including the item's default when not patched.
    pub(super) fn native_component<'a>(
        &'a self,
        stack: &'a ItemStack,
        id: &str,
    ) -> Result<Option<&'a Tag>, NativeComponentError> {
        if horizon_protocol::item::custom::id(id).is_none() {
            return Err(NativeComponentError::UnknownType);
        }
        if let Some(data) = stack.data() {
            if data.other(&format!("!{id}")).is_some() {
                return Ok(None);
            }
            if let Some(value) = data.other(id) {
                return Ok(Some(value));
            }
        }
        Ok(self
            .items()
            .get(&stack.item)
            .and_then(|item| item.custom_components.get(id)))
    }

    /// Set a registered native component after checking the network NBT limits.
    pub(super) fn native_set_component(
        &self,
        stack: &mut ItemStack,
        id: &str,
        value: Tag,
    ) -> Result<(), NativeComponentError> {
        if horizon_protocol::item::custom::id(id).is_none() {
            return Err(NativeComponentError::UnknownType);
        }
        horizon_protocol::item::custom::validate(&value)
            .map_err(NativeComponentError::InvalidValue)?;
        let default = self
            .items()
            .get(&stack.item)
            .and_then(|item| item.custom_components.get(id));
        let value = (default != Some(&value)).then_some(value);
        let removed = format!("!{id}");
        let data = stack.data_mut();
        data.set_other(&removed, None);
        data.set_other(id, value);
        stack.tidy();
        Ok(())
    }

    /// Remove a native component, including its item default.
    pub(super) fn native_remove_component(
        &self,
        stack: &mut ItemStack,
        id: &str,
    ) -> Result<(), NativeComponentError> {
        if horizon_protocol::item::custom::id(id).is_none() {
            return Err(NativeComponentError::UnknownType);
        }
        let default = self
            .items()
            .get(&stack.item)
            .is_some_and(|item| item.custom_components.contains_key(id));
        let removed = format!("!{id}");
        let data = stack.data_mut();
        data.set_other(id, None);
        data.set_other(&removed, default.then(|| Tag::Compound(Default::default())));
        stack.tidy();
        Ok(())
    }
}

#[cfg(test)]
mod equippable_slot_tests {
    use horizon_client::items::ItemStack;

    /// `Equippable.slot`: armour goes where it is worn, a plain item goes nowhere.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_equippable_slot_follows_the_equippable_component() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let slot = |item: &str| game.item_equippable_slot(&ItemStack::new(item, 1));
            assert_eq!(slot("minecraft:iron_helmet").as_deref(), Some("head"));
            assert_eq!(slot("minecraft:iron_boots").as_deref(), Some("feet"));
            assert_eq!(slot("minecraft:dirt"), None);
        });
    }
}

#[cfg(test)]
mod enchantment_conflict_tests {
    /// `Enchantment.areCompatible`: the same enchantment twice and the exclusive sets conflict,
    /// others and unknown ids do not.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn enchantments_compatible_follows_the_exclusive_sets() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            assert!(!game.enchantments_compatible("minecraft:sharpness", "minecraft:smite"));
            assert!(!game.enchantments_compatible("minecraft:sharpness", "minecraft:sharpness"));
            assert!(game.enchantments_compatible("minecraft:sharpness", "minecraft:unbreaking"));
            assert!(game.enchantments_compatible("minecraft:sharpness", "mod:unknown"));
        });
    }
}

#[cfg(test)]
mod item_resource_property_tests {
    fn engine(resource: &mod_api::ItemResource, count: u32) -> horizon_items::items::ItemStack {
        super::super::api_values::stack_value(&resource.to_stack(count).unwrap()).unwrap()
    }
    use serde_json::Value;
    mod support {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixture_support/item_resource_properties.rs"
        ));
    }
    /// All seven reads and their game consumers, with independently recorded vanilla outcomes.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn effective_resource_properties_match_vanilla() {
        let inputs = std::env::var("HORIZON_ITEM_PROPERTIES_CASES")
            .map(|path| std::fs::read_to_string(path).unwrap())
            .unwrap_or_else(|_| {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/item_resource_properties.jsonl"
                ))
                .into()
            });
        let expected = std::env::var("HORIZON_ITEM_PROPERTIES_EXPECTED")
            .map(|path| std::fs::read_to_string(path).unwrap())
            .unwrap_or_else(|_| {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/item_resource_properties_expected.jsonl"
                ))
                .into()
            });
        let rows = |text: &str| {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        };
        let (inputs, expected) = (rows(&inputs), rows(&expected));
        assert!(inputs.len() >= 41);
        assert_eq!(inputs.len(), expected.len());
        crate::game::native_entity_refs::tests::with_game(|game| {
            // Authoritative tag reads are available only while a world is owned.
            game.session_lock =
                Some(horizon_client::save::SessionLock::acquire(&game.save.root).unwrap());
            for (input, expected) in inputs.iter().zip(&expected) {
                assert_eq!(input["name"], expected["name"]);
                let support::Outcome {
                    item,
                    material,
                    values,
                } = support::evaluate(game, input);
                assert_eq!(&values, expected, "{}", input["name"]);
                let stack = engine(&item, 1);
                let definitions = &game.items().enchantments;
                let props = definitions.props(&stack.item);
                assert_eq!(
                    game.use_duration(&stack),
                    expected["use_duration"].as_i64().unwrap() as i32
                );
                assert_eq!(
                    horizon_client::stations::is_damageable(game.items(), &stack),
                    item.damageable(game)
                );
                assert_eq!(
                    stack.has_foil(props.and_then(|value| value.glint)),
                    item.foil(game)
                );
                assert_eq!(
                    horizon_client::item_tooltip::rarity(
                        &stack,
                        props.map_or("common", |value| value.rarity.as_str())
                    ),
                    item.rarity(game).as_str()
                );
                assert_eq!(
                    horizon_client::stations::is_valid_repair_item(
                        definitions,
                        game.items(),
                        &stack,
                        &engine(&material, 1)
                    ),
                    item.repairable(game, &material)
                );
            }
            game.session_lock = None;
        });
    }
    /// Changed defaults and a patched mod consumable exercise the real game consumers.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn item_resource_changed_defaults_and_consumption() {
        use horizon_client::items::ItemStack;
        use mod_api::{ComponentId, ItemResource, Tag};
        use serde_json::json;
        use std::{fs, path::Path};
        fn link_fixture_source(source: &Path, target: &Path) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(source, target).unwrap();
            #[cfg(not(unix))]
            {
                // Windows file hard links need no symlink privilege. Copy across volumes.
                if source.is_dir() {
                    fs::create_dir(target).unwrap();
                    for entry in fs::read_dir(source).unwrap() {
                        let entry = entry.unwrap();
                        link_fixture_source(&entry.path(), &target.join(entry.file_name()));
                    }
                } else if fs::hard_link(source, target).is_err() {
                    fs::copy(source, target).unwrap();
                }
            }
        }
        let resources = std::path::PathBuf::from(std::env::var("HORIZON_TEST_RESOURCES").unwrap());
        // A private default report goes through the owning resource load and all its metadata
        // readers, exactly as a registration-time change does. Shared resources stay read-only.
        let root =
            std::env::temp_dir().join(format!("horizon-property-defaults-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for entry in fs::read_dir(&resources).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() != "items.json" {
                link_fixture_source(&entry.path(), &root.join(entry.file_name()));
            }
        }
        let mut defaults: Value =
            serde_json::from_slice(&fs::read(resources.join("items.json")).unwrap()).unwrap();
        defaults["minecraft:iron_sword"]["minecraft:damage"] = json!(75);
        defaults["minecraft:iron_sword"]["minecraft:enchantment_glint_override"] = json!(false);
        defaults["horizon:consumable_fixture"] = defaults["minecraft:stick"].clone();
        defaults["minecraft:wooden_sword"]["minecraft:repairable"] = json!({"items":"#planks"});
        fs::write(
            root.join("items.json"),
            serde_json::to_vec(&defaults).unwrap(),
        )
        .unwrap();
        crate::game::native_entity_refs::tests::with_game_at(root.clone(), |game| {
            game.session_lock =
                Some(horizon_client::save::SessionLock::acquire(&game.save.root).unwrap());
            let wood = ItemResource::of(&super::super::api_values::test_snapshot(&ItemStack::new(
                "minecraft:wooden_sword",
                1,
            )));
            let plank = ItemResource::of(&super::super::api_values::test_snapshot(
                &ItemStack::new("minecraft:oak_planks", 1),
            ));
            assert!(wood.repairable(game, &plank));
            assert!(horizon_client::stations::is_valid_repair_item(
                &game.items().enchantments,
                game.items(),
                &engine(&wood, 1),
                &engine(&plank, 1)
            ));
            assert!(!horizon_items::enchant::Enchantments::item_in(
                game.items(),
                &["#planks".into()],
                "minecraft:iron_ingot"
            ));
            let stack = ItemStack::new("minecraft:iron_sword", 1);
            let resource = ItemResource::of(&super::super::api_values::test_snapshot(&stack));
            let bar = resource.bar(game).unwrap();
            // The independent oracle records the equivalent effective damage as a stack patch.
            let expected: Value = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/item_resource_properties_expected.jsonl"
            ))
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|row| row["name"] == "prototype-damage-equivalent")
            .unwrap();
            assert_eq!(
                serde_json::json!({"width":bar.width,"color":bar.color}),
                expected["bar"]
            );
            let info = game.items().get(&stack.item);
            assert_eq!(stack.effective_damage(info), 75);
            assert_eq!(
                horizon_client::hud_overlays::damage_bar(
                    stack.effective_damage(info),
                    stack.max_damage(info)
                ),
                Some((bar.width, bar.color))
            );
            let damage = ComponentId::new("minecraft:damage").unwrap();
            let zero = resource.with_component(game, &damage, Tag::Int(0)).unwrap();
            assert_eq!(zero.component(&damage), Some(Tag::Int(0)));
            assert_eq!(engine(&zero, 1).effective_damage(info), 0);
            assert!(zero.bar(game).is_none());
            let removed = resource.without_component(game, &damage);
            assert_eq!(engine(&removed, 1).effective_damage(info), 0);
            assert!(removed.bar(game).is_none());
            let enchanted = resource
                .with_component(
                    game,
                    &ComponentId::new("minecraft:enchantments").unwrap(),
                    Tag::compound([("minecraft:sharpness", Tag::Int(1))]),
                )
                .unwrap();
            let glint = game
                .items()
                .enchantments
                .props(&stack.item)
                .and_then(|p| p.glint);
            assert_eq!(glint, Some(false));
            assert!(!enchanted.foil(game));
            assert!(!engine(&enchanted, 1).has_foil(glint));
            let inherited = enchanted.without_component(
                game,
                &ComponentId::new("minecraft:enchantment_glint_override").unwrap(),
            );
            assert!(inherited.foil(game));
            assert!(engine(&inherited, 1).has_foil(glint));

            let base = ItemResource::of(&super::super::api_values::test_snapshot(&ItemStack::new(
                "horizon:consumable_fixture",
                2,
            )));
            let consumable = ComponentId::new("minecraft:consumable").unwrap();
            let patched = base
                .with_component(
                    game,
                    &consumable,
                    Tag::compound([
                        ("consume_seconds", Tag::Float(0.1_f32.to_bits())),
                        ("has_consume_particles", Tag::Byte(0)),
                    ]),
                )
                .unwrap();
            let stack = engine(&patched, 2);
            let info = game.items().get(&stack.item).unwrap().clone();
            assert_eq!(game.use_duration(&stack), 2);
            assert!(!game.mod_use(&stack));
            assert!(
                engine(&patched.without_component(game, &consumable), 1)
                    .consumable(Some(&info))
                    .is_none()
            );
            game.creative = false;
            game.right = true;
            game.screen = crate::game::Screen::Playing;
            game.inventory[game.selected] = Some(stack);
            assert!(game.start_eating(&info));
            assert!(game.using.is_some());
            game.tick_using();
            assert_eq!(game.held().unwrap().count, 2);
            game.tick_using();
            assert!(game.using.is_none());
            assert_eq!(
                game.held().unwrap().count,
                1,
                "completion must consume the patched mod item"
            );
            game.session_lock = None;
        });
        fs::remove_dir_all(root).unwrap();
    }
}
