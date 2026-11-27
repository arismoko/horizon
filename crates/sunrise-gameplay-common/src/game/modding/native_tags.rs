//! One membership implementation for full-game and in-write tag services.
use mod_api::{BlockKey, DamageTypeKey, FluidKey, Identifier, ItemKey, TagKey, TagMembers};
use sunrise_items::items::{Items, TagRegistry};
use sunrise_world::loot::LootTags;

pub fn is_in_tag(items: Option<&Items>, item: ItemKey, tag: TagKey<ItemKey>) -> Option<bool> {
    let items = items.filter(|items| items.tags_loaded(TagRegistry::Item))?;
    let item = mod_api::__private::item_key_id(item)?;
    mod_api::__private::with_tag_name(tag.untyped(), |name| {
        Some(
            items
                .item_tags
                .get(name?)
                .is_some_and(|members| members.contains(item)),
        )
    })
}
/// `DamageSource.is(TagKey)` over the damage type registry and tags `items` was bound to at
/// its last reload. The tag's name is read on the call, so a key made after that reload resolves.
pub fn damage_type_in_tag(
    items: Option<&Items>,
    damage_type: &str,
    tag: TagKey<DamageTypeKey>,
) -> Option<bool> {
    let items = items?;
    mod_api::__private::with_tag_name(tag.untyped(), |name| {
        items.damage_type_in_tag(damage_type, name?)
    })
}
pub fn item_tags(tags: &dyn LootTags, item: ItemKey) -> Option<Vec<Identifier>> {
    let item = mod_api::__private::item_key_id(item)?;
    tags.item_tag_names()?
        .matching(|name| tags.contains("item", name, item))
}

pub fn item_tag_names(tags: &dyn LootTags) -> Option<Vec<Identifier>> {
    tags.item_tag_names()?.matching(|_| true)
}

pub fn block_in_tag(items: Option<&Items>, block: BlockKey, tag: TagKey<BlockKey>) -> Option<bool> {
    let items = items.filter(|items| items.tags_loaded(TagRegistry::Block))?;
    let block = mod_api::__private::block_key_id(block)?;
    mod_api::__private::with_tag_name(tag.untyped(), |name| Some(items.block_in_tag(name?, block)))
}

pub fn fluid_in_tag(items: Option<&Items>, fluid: FluidKey, tag: TagKey<FluidKey>) -> Option<bool> {
    let items = items.filter(|items| items.tags_loaded(TagRegistry::Fluid))?;
    let fluid = mod_api::__private::fluid_id(fluid)?;
    mod_api::__private::with_tag_name(tag.untyped(), |name| Some(items.fluid_in_tag(name?, fluid)))
}

pub fn snapshot_members<K, T>(
    items: Option<&Items>,
    registry: TagRegistry,
    tag: TagKey<T>,
    key: impl FnMut(&str) -> Option<K>,
) -> TagMembers<K> {
    let Some(items) = items.filter(|items| items.tags_loaded(registry)) else {
        return TagMembers::Unavailable;
    };
    members(tag, |name| items.tag_members(registry, name), key)
}

/// Map complete ordered membership from either a host snapshot or a loot table.
pub(super) fn members<K, T, I: IntoIterator>(
    tag: TagKey<T>,
    read: impl FnOnce(&str) -> Option<I>,
    mut key: impl FnMut(&str) -> Option<K>,
) -> TagMembers<K>
where
    I::Item: AsRef<str>,
{
    mod_api::__private::with_tag_name(tag.untyped(), |name| {
        let Some(name) = name else {
            return TagMembers::Unavailable;
        };
        let Some(values) = read(name) else {
            return TagMembers::Missing;
        };
        values
            .into_iter()
            .map(|id| key(id.as_ref()))
            .collect::<Option<Vec<_>>>()
            .map_or(TagMembers::Unavailable, TagMembers::Members)
    })
}

// A restricted host supplies the same authoritative item tables but cannot borrow Game.
// Only that choice differs; key resolution, ordered reads and membership stay above.
#[macro_export]
macro_rules! impl_tags {
    ($ty:ty, $this:ident, $items:expr) => {
        $crate::game::native_tags::impl_tags!(
            $ty,
            $this,
            $items,
            None::<&mod_api::__private::BiomeKeys>;
            generics()
        );
    };
    ($ty:ty, $this:ident, $items:expr, $biome_keys:expr) => {
        $crate::game::native_tags::impl_tags!(
            $ty,
            $this,
            $items,
            $biome_keys;
            generics()
        );
    };
    ($ty:ty, $this:ident, $items:expr; generics($($g:tt)*)) => {
        $crate::game::native_tags::impl_tags!(
            $ty,
            $this,
            $items,
            None::<&mod_api::__private::BiomeKeys>;
            generics($($g)*)
        );
    };
    ($ty:ty, $this:ident, $items:expr, $biome_keys:expr; generics($($g:tt)*)) => {
        impl<$($g)*> mod_api::Tags for $ty {
            fn key(&self, id: &mod_api::Identifier) -> Option<mod_api::TagKey> {
                mod_api::__private::tag_key(id)
            }
            fn is_in_tag(
                &self,
                item: mod_api::ItemKey,
                tag: mod_api::TagKey<mod_api::ItemKey>,
            ) -> Option<bool> {
                let $this = self;
                $crate::game::native_tags::is_in_tag($items, item, tag)
            }
            fn damage_type_in_tag(
                &self,
                damage_type: &str,
                tag: mod_api::TagKey<mod_api::DamageTypeKey>,
            ) -> Option<bool> {
                let $this = self;
                $crate::game::native_tags::damage_type_in_tag($items, damage_type, tag)
            }
            fn item_tags(&self, item: mod_api::ItemKey) -> Option<Vec<mod_api::Identifier>> {
                let $this = self;
                let items: Option<&sunrise_items::items::Items> = $items;
                $crate::game::native_tags::item_tags(items?, item)
            }
            fn item_tag_names(&self) -> Option<Vec<mod_api::Identifier>> {
                let $this = self;
                let items: Option<&sunrise_items::items::Items> = $items;
                $crate::game::native_tags::item_tag_names(items?)
            }
            fn block_in_tag(
                &self,
                block: mod_api::BlockKey,
                tag: mod_api::TagKey<mod_api::BlockKey>,
            ) -> Option<bool> {
                let $this = self;
                $crate::game::native_tags::block_in_tag($items, block, tag)
            }
            fn fluid_in_tag(
                &self,
                fluid: mod_api::FluidKey,
                tag: mod_api::TagKey<mod_api::FluidKey>,
            ) -> Option<bool> {
                let $this = self;
                $crate::game::native_tags::fluid_in_tag($items, fluid, tag)
            }
            fn items(
                &self,
                tag: mod_api::TagKey<mod_api::ItemKey>,
            ) -> mod_api::TagMembers<mod_api::ItemKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Item,
                    tag,
                    mod_api::__private::item_key,
                )
            }
            fn fluids(
                &self,
                tag: mod_api::TagKey<mod_api::FluidKey>,
            ) -> mod_api::TagMembers<mod_api::FluidKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Fluid,
                    tag,
                    mod_api::__private::fluid_key,
                )
            }
            fn blocks(
                &self,
                tag: mod_api::TagKey<mod_api::BlockKey>,
            ) -> mod_api::TagMembers<mod_api::BlockKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Block,
                    tag,
                    mod_api::__private::block_key,
                )
            }
            fn entity_types(
                &self,
                tag: mod_api::TagKey<mod_api::EntityTypeKey>,
            ) -> mod_api::TagMembers<mod_api::EntityTypeKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::EntityType,
                    tag,
                    mod_api::__private::resolved_entity_type_key,
                )
            }
            fn biomes(
                &self,
                tag: mod_api::TagKey<mod_api::BiomeKey>,
            ) -> mod_api::TagMembers<mod_api::BiomeKey> {
                let $this = self;
                let Some(biome_keys): Option<&mod_api::__private::BiomeKeys> = $biome_keys else {
                    return mod_api::TagMembers::Unavailable;
                };
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Biome,
                    tag,
                    |id| biome_keys.view(id).ok().map(|view| view.key().clone()),
                )
            }
            fn enchantments(
                &self,
                tag: mod_api::TagKey<mod_api::EnchantmentKey>,
            ) -> mod_api::TagMembers<mod_api::EnchantmentKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Enchantment,
                    tag,
                    mod_api::__private::enchantment_key,
                )
            }
            fn structures(
                &self,
                tag: mod_api::TagKey<mod_api::StructureKey>,
            ) -> mod_api::TagMembers<mod_api::StructureKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Structure,
                    tag,
                    |id| {
                        mod_api::Identifier::parse(id)
                            .ok()
                            .map(mod_api::StructureKey)
                    },
                )
            }
            fn potions(
                &self,
                tag: mod_api::TagKey<mod_api::PotionKey>,
            ) -> mod_api::TagMembers<mod_api::PotionKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Potion,
                    tag,
                    |id| mod_api::Identifier::parse(id).ok().map(mod_api::PotionKey),
                )
            }
            fn damage_types(
                &self,
                tag: mod_api::TagKey<mod_api::DamageTypeKey>,
            ) -> mod_api::TagMembers<mod_api::Identifier> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::DamageType,
                    tag,
                    |id| mod_api::Identifier::parse(id).ok(),
                )
            }
            fn world_clocks(
                &self,
                tag: mod_api::TagKey<mod_api::WorldClockKey>,
            ) -> mod_api::TagMembers<mod_api::WorldClockKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::WorldClock,
                    tag,
                    |id| {
                        mod_api::Identifier::parse(id)
                            .ok()
                            .map(mod_api::WorldClockKey)
                    },
                )
            }
        }
    };
}
pub use crate::impl_tags;

#[cfg(any(test, feature = "test-support"))]
pub const PACK_BINDING_CASES: [(&str, &[u8], bool); 3] = [
    (
        "optional_only",
        br#"{"values":[{"id":"binding:not_registered","required":false}]}"#,
        true,
    ),
    (
        "broken_parent",
        br##"{"values":["minecraft:stick","#binding:absent"]}"##,
        false,
    ),
    (
        "broken_nested",
        br##"{"values":["#binding:absent"]}"##,
        false,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::Tags;

    struct TagView<'a>(Option<&'a Items>);
    impl_tags!(TagView<'_>, view, view.0);

    #[test]
    fn registration_resolves_the_same_tag_key_without_loaded_membership() {
        use mod_api::{Hooks, Identifier, ModEntry, Registrar, RegistrationContext};
        use std::sync::{Arc, OnceLock};

        struct ResolveTag(Arc<OnceLock<TagKey>>);
        #[mod_api::__subscriptions]
        impl Hooks for ResolveTag {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let _ = self.0.set(r.tag_key("registrar_tag_lookup"));
            }
        }

        let found = Arc::new(OnceLock::new());
        crate::game::api_values::install_parts(&[ModEntry::new(
            "tag_reader",
            Box::new(ResolveTag(found.clone())),
        )])
        .unwrap();
        struct UnloadedTags;
        impl_tags!(UnloadedTags, _view, None);
        let name = Identifier::parse("minecraft:registrar_tag_lookup").unwrap();
        assert_eq!(*found.get().unwrap(), UnloadedTags.key(&name).unwrap());
    }

    #[test]
    fn block_members_preserve_authoritative_order_reload_and_atomic_refusal() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_block_keys(
            ["minecraft:air", "minecraft:stone", "minecraft:birch_log"],
            |_| {
                let properties = std::collections::BTreeMap::new();
                sunrise_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                    .map_err(|error| format!("{error:?}"))
            },
        )
        .unwrap();
        let stone = mod_api::__private::block_key("minecraft:stone").unwrap();
        let birch = mod_api::__private::block_key("minecraft:birch_log").unwrap();
        let stone_state = mod_api::__private::block_state_from_ordinal(stone, 0);
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:blocks").unwrap())
            .unwrap()
            .typed();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("test:empty").unwrap())
                .unwrap()
                .typed();
        let mut table = Items::for_tests(Default::default());
        assert_eq!(TagView(Some(&table)).blocks(tag), TagMembers::Unavailable);
        assert_eq!(stone.is_in_tag(&TagView(Some(&table)), tag), None);
        table.set_tags(
            TagRegistry::Block,
            [
                (
                    "test:blocks".into(),
                    vec!["minecraft:stone".into(), "minecraft:birch_log".into()],
                ),
                ("test:empty".into(), vec![]),
            ]
            .into(),
        );
        let view = TagView(Some(&table));
        assert_eq!(view.blocks(tag), TagMembers::Members(vec![stone, birch]));
        assert_eq!(view.blocks(missing), TagMembers::Members(vec![]));
        assert_eq!(stone.is_in_tag(&view, tag), Some(true));
        assert_eq!(stone_state.is_in_tag(&view, tag), Some(true));
        assert_eq!(stone.is_in_tag(&view, missing), Some(false));
        assert_eq!(TagView(None).blocks(tag), TagMembers::Unavailable);
        assert_eq!(stone.is_in_tag(&TagView(None), tag), None);
        table.set_tags(
            TagRegistry::Block,
            [("test:blocks".into(), vec!["minecraft:birch_log".into()])].into(),
        );
        let view = TagView(Some(&table));
        assert_eq!(view.blocks(tag), TagMembers::Members(vec![birch]));
        assert_eq!(view.blocks(missing), TagMembers::Missing);
        assert_eq!(stone.is_in_tag(&view, tag), Some(false));
        assert_eq!(stone_state.is_in_tag(&view, tag), Some(false));
        assert_eq!(birch.is_in_tag(&view, tag), Some(true));
        table.block_tag_order.insert(
            "test:blocks".into(),
            vec!["minecraft:stone".into(), "test:unloaded".into()],
        );
        assert_eq!(
            TagView(Some(&table)).blocks(tag),
            TagMembers::Unavailable,
            "no partial member list on unresolved block mapping"
        );
    }

    #[test]
    fn fluid_members_distinguish_missing_empty_and_unavailable() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_fluids(
            &["minecraft:water", "minecraft:lava"],
            &sunrise_world::fluid::FluidKind::ALL.map(|kind| kind.name()),
        )
        .unwrap();
        let water = mod_api::__private::fluid_key("minecraft:water").unwrap();
        let water_state = mod_api::__private::fluid_state(water, 8, false, true);
        let tag =
            |name| mod_api::__private::tag_key(&mod_api::Identifier::parse(name).unwrap()).unwrap();
        let empty = tag("test:empty_fluid");
        let absent = tag("test:absent_fluid");
        let mut table = Items::empty();
        table.set_tags(TagRegistry::Block, Default::default());
        assert_eq!(
            TagView(Some(&table)).fluids(empty.typed()),
            TagMembers::Unavailable
        );
        assert_eq!(water.is_in_tag(&TagView(Some(&table)), empty.typed()), None);
        table.set_tags(
            TagRegistry::Fluid,
            [("test:empty_fluid".into(), vec![])].into(),
        );
        let view = TagView(Some(&table));
        assert_eq!(view.fluids(empty.typed()), TagMembers::Members(vec![]));
        assert_eq!(view.fluids(absent.typed()), TagMembers::Missing);
        assert_eq!(view.items(absent.typed()), TagMembers::Unavailable);
        assert_eq!(water.is_in_tag(&view, empty.typed()), Some(false));
        assert_eq!(water.is_in_tag(&view, absent.typed()), Some(false));
        assert_eq!(water.is_in_tag(&TagView(None), empty.typed()), None);
        table.set_tags(
            TagRegistry::Fluid,
            [("test:empty_fluid".into(), vec!["minecraft:water".into()])].into(),
        );
        let view = TagView(Some(&table));
        assert_eq!(water.is_in_tag(&view, empty.typed()), Some(true));
        assert_eq!(water_state.is_in_tag(&view, empty.typed()), Some(true));
        table.set_tags(
            TagRegistry::Fluid,
            [("test:empty_fluid".into(), vec!["minecraft:lava".into()])].into(),
        );
        let view = TagView(Some(&table));
        assert_eq!(water.is_in_tag(&view, empty.typed()), Some(false));
        assert_eq!(water_state.is_in_tag(&view, empty.typed()), Some(false));
    }

    #[test]
    fn empty_stacks_refuse_before_membership_and_air_is_an_explicit_key() {
        use crate::game::api_values::{StackReadError, TryIntoApi};
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:air", "minecraft:stone"]).unwrap();
        let air = mod_api::__private::item_key("minecraft:air").unwrap();
        let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("x").unwrap())
            .unwrap()
            .typed();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("minecraft:y").unwrap())
                .unwrap()
                .typed();
        let mut table = Items::for_tests(
            [(
                "minecraft:x".to_owned(),
                ["minecraft:air".to_owned()].into(),
            )]
            .into(),
        );
        let empty = sunrise_items::items::ItemStack::new("minecraft:stone", 0);
        assert_eq!(empty.try_into_api(), Err(StackReadError::Empty));
        assert_eq!(is_in_tag(Some(&table), air, tag), Some(true));
        assert_eq!(is_in_tag(Some(&table), stone, tag), Some(false));
        assert_eq!(is_in_tag(Some(&table), air, missing), Some(false));
        table.item_tag_order.insert(
            "minecraft:x".into(),
            vec!["minecraft:air".into(), "missing:unloaded".into()],
        );
        assert_eq!(
            TagView(Some(&table)).items(tag),
            TagMembers::Unavailable,
            "unresolved members must not produce a partial list"
        );
    }
    /// A damage catalog of `in_fire` and `arrow`, with `tags` as its damage type tag files.
    #[cfg(any())]
    fn bind_damage(table: &mut Items, tags: serde_json::Value) {
        let types: Vec<_> = ["in_fire", "arrow"]
            .into_iter()
            .map(|name| {
                serde_json::json!({
                    "id": format!("minecraft:{name}"), "message_id": name, "exhaustion": "3dcccccd",
                    "scaling": "never", "effects": "hurt", "death_message_type": "default"
                })
            })
            .collect();
        let catalog = sunrise_world::damage_source::Catalog::from_json(
            &serde_json::json!({"data_version": sunrise_world::DATA_VERSION, "types": types}),
            &tags,
        )
        .unwrap();
        std::sync::Arc::make_mut(&mut table.enchantments).bind_damage_catalog(&catalog);
        table.bind_component_damage_catalog(&catalog);
    }

    /// Runs in a fresh child process: its keys intern tag names in the process-wide table.
    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn damage_type_membership_follows_the_last_bound_catalog() {
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::native_tags::tests::damage_type_membership_follows_the_last_bound_catalog",
            "SUNRISE_TEST_DAMAGE_TYPE_CATALOG",
            &["catalog"],
        ) {
            return;
        }
        let tag = |name| {
            mod_api::__private::tag_key(&mod_api::Identifier::parse(name).unwrap())
                .unwrap()
                .typed()
        };
        let fire = tag("minecraft:is_fire");
        let mut table = Items::empty();
        let view = TagView(Some(&table));
        assert_eq!(view.damage_type_in_tag("minecraft:in_fire", fire), None);
        assert_eq!(
            TagView(None).damage_type_in_tag("minecraft:in_fire", fire),
            None
        );
        bind_damage(
            &mut table,
            serde_json::json!({
                "minecraft:is_fire": r#"{"values":["minecraft:in_fire"]}"#,
                "minecraft:bypasses_armor": r#"{"values":[]}"#,
            }),
        );
        let view = TagView(Some(&table));
        assert_eq!(
            view.damage_type_in_tag("minecraft:in_fire", fire),
            Some(true)
        );
        assert_eq!(
            view.damage_type_in_tag("minecraft:arrow", fire),
            Some(false)
        );
        assert_eq!(view.damage_type_in_tag("test:unknown", fire), None);
        // A key first made after the catalog was bound resolves on its first read.
        let armor = tag("minecraft:bypasses_armor");
        let absent = tag("test:damage_absent");
        assert_eq!(
            view.damage_type_in_tag("minecraft:arrow", armor),
            Some(false)
        );
        assert_eq!(
            view.damage_type_in_tag("minecraft:arrow", absent),
            Some(false)
        );
        bind_damage(
            &mut table,
            serde_json::json!({
                "minecraft:is_fire": r#"{"values":[]}"#,
                "minecraft:bypasses_armor": r#"{"values":["minecraft:arrow"]}"#,
            }),
        );
        let view = TagView(Some(&table));
        assert_eq!(
            view.damage_type_in_tag("minecraft:in_fire", fire),
            Some(false)
        );
        assert_eq!(
            view.damage_type_in_tag("minecraft:arrow", armor),
            Some(true)
        );
    }

    #[test]
    fn keys_survive_membership_reload_and_reads_distinguish_unavailable_from_unknown() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:stick", "minecraft:stone"]).unwrap();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:reload").unwrap())
            .unwrap()
            .typed();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("test:missing").unwrap())
                .unwrap()
                .typed();
        let mut table = Items::empty();
        assert_eq!(TagView(Some(&table)).items(tag), TagMembers::Unavailable);
        table.set_tags(
            TagRegistry::Item,
            [
                (
                    "test:reload".into(),
                    vec!["minecraft:stone".into(), "minecraft:stick".into()],
                ),
                ("test:missing".into(), Vec::new()),
            ]
            .into(),
        );
        assert_eq!(is_in_tag(Some(&table), stick, tag), Some(true));
        assert_eq!(
            TagView(Some(&table)).items(tag),
            TagMembers::Members(vec![stone, stick])
        );
        assert_eq!(
            TagView(Some(&table)).items(missing),
            TagMembers::Members(vec![])
        );
        table.set_tags(
            TagRegistry::Item,
            [("test:reload".into(), vec!["minecraft:stone".into()])].into(),
        );
        assert_eq!(is_in_tag(Some(&table), stick, tag), Some(false));
        assert_eq!(
            TagView(Some(&table)).items(tag),
            TagMembers::Members(vec![stone])
        );
        assert_eq!(is_in_tag(Some(&table), stone, missing), Some(false));
        assert_eq!(TagView(Some(&table)).items(missing), TagMembers::Missing);
        assert_eq!(is_in_tag(None, stone, tag), None);
        assert_eq!(TagView(None).items(tag), TagMembers::Unavailable);
    }
}
