//! Creative tab generators and their ordered results, independent of the screen.
use crate::{
    item_data::{
        component_snapshot, components_tag, decode_components, encode_components, parse_stack,
    },
    items::{ItemStack, Items},
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU8,
    sync::Arc,
};
use sunrise_core::nbt::Tag;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Kind {
    Category,
    Hotbar,
    Inventory,
    Search,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Visibility {
    ParentAndSearchTabs,
    ParentTabOnly,
    SearchTabOnly,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Source {
    Stack {
        #[serde(deserialize_with = "stack")]
        snbt: ItemStack,
    },
    Paintings {
        placeable: bool,
    },
    Potions {
        item: String,
    },
    Enchantments {
        all_levels: bool,
    },
    Horns,
    #[serde(skip_deserializing)]
    SuspiciousStews,
    Banner {
        #[serde(deserialize_with = "stack")]
        snbt: ItemStack,
    },
}

fn stack<'de, D: serde::Deserializer<'de>>(de: D) -> Result<ItemStack, D::Error> {
    use serde::de::Error;
    let snbt = String::deserialize(de)?;
    parse_stack(&snbt)
        .map_err(D::Error::custom)?
        .filter(|stack| stack.count == 1)
        .ok_or_else(|| D::Error::custom("creative generator stacks must contain one item"))
}

/// `CreativeModeTabs`' key for an exported tab id (`itemGroup.*`); the
/// export's `title` is the English fallback.
pub fn item_group_key(id: &str) -> Option<&'static str> {
    Some(match id.trim_start_matches("minecraft:") {
        "building_blocks" => "itemGroup.buildingBlocks",
        "colored_blocks" => "itemGroup.coloredBlocks",
        "natural_blocks" => "itemGroup.natural",
        "functional_blocks" => "itemGroup.functional",
        "redstone_blocks" => "itemGroup.redstone",
        "hotbar" => "itemGroup.hotbar",
        "search" => "itemGroup.search",
        "tools_and_utilities" => "itemGroup.tools",
        "combat" => "itemGroup.combat",
        "food_and_drinks" => "itemGroup.foodAndDrink",
        "ingredients" => "itemGroup.ingredients",
        "spawn_eggs" => "itemGroup.spawnEggs",
        "op_blocks" => "itemGroup.op",
        "inventory" => "itemGroup.inventory",
        _ => return None,
    })
}
/// A mod tab's own `itemGroup.<mod>.<name>` key, for its id `<mod>:<name>`.
pub fn mod_item_group_key(id: &str) -> String {
    format!("itemGroup.{}", id.replace(':', "."))
}

pub fn parse_search_metadata_stack(row: &serde_json::Value) -> Result<ItemStack, String> {
    let snbt = row["stack"]
        .as_str()
        .ok_or("missing search metadata stack")?;
    let stack = parse_stack(snbt)?.ok_or("empty search metadata stack")?;
    Ok(stack)
}

#[derive(Deserialize)]
struct Row {
    visibility: Visibility,
    #[serde(flatten)]
    source: Source,
}

#[derive(Deserialize)]
pub struct TabRecipe {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    /// The English text of the tab's title: the export's, or a mod tab file's `title`.
    pub title: String,
    build: Vec<Row>,
    operator_build: Option<Vec<Row>>,
}

/// The immutable, prepared recipe. World-specific dynamic entries are never
/// taken from the export's old display or tooltip lists.
#[derive(Deserialize)]
pub struct Catalog {
    schema: u32,
    tabs: Vec<TabRecipe>,
    item_features: HashMap<String, Vec<String>>,
    potion_features: HashMap<String, Vec<String>>,
}

/// Ordered values from the world's holder provider. Potions are a built-in
/// registry, enchantments and paintings use holder order, horns use tag order,
/// and the banner generator only checks whether each named pattern exists.
#[derive(Clone, Default, Deserialize)]
pub struct Holders {
    pub potions: Vec<String>,
    pub enchantments: Vec<Enchantment>,
    pub paintings: Vec<Painting>,
    pub horns: Vec<String>,
    /// Text for the creative search projection, from the same provider as `horns`.
    #[serde(default)]
    pub instrument_descriptions: HashMap<String, serde_json::Value>,
    pub banner_patterns: HashSet<String>,
}

#[derive(Clone, Deserialize)]
pub struct Enchantment {
    pub id: String,
    /// The data codec accepts precisely `1..=255`.
    pub max_level: NonZeroU8,
    #[serde(default)]
    pub description: serde_json::Value,
}

#[derive(Clone, Deserialize)]
pub struct Painting {
    pub id: String,
    pub width: NonZeroU8,
    pub height: NonZeroU8,
    pub placeable: bool,
    #[serde(default)]
    pub title: Option<serde_json::Value>,
    #[serde(default)]
    pub author: Option<serde_json::Value>,
}

/// Independent insertion-ordered sets for a tab's grid and its contribution
/// to search. Every stack has count one.
#[derive(Default)]
pub struct TabContents {
    pub display: Vec<ItemStack>,
    pub search: Vec<ItemStack>,
}

#[derive(Clone, Copy)]
pub struct Parameters<'a> {
    pub features: &'a [String],
    pub operator: bool,
    pub holders: &'a Holders,
    pub items: &'a Items,
}

fn enabled(required: Option<&Vec<String>>, features: &[String]) -> bool {
    required.is_none_or(|required| required.iter().all(|flag| features.contains(flag)))
}

/// Shared item feature-flag table extracted from [`Catalog`] so servers can check
/// `ItemStack.isItemEnabled` without holding the client's creative screen.
#[derive(Clone, Default)]
pub struct ItemFeatureRequirements(Arc<HashMap<String, Vec<String>>>);

impl ItemFeatureRequirements {
    pub fn item_enabled(&self, stack: &ItemStack, features: &[String]) -> bool {
        enabled(self.0.get(&*stack.item), features)
    }
}

impl Catalog {
    /// `ItemStack.isItemEnabled`: the item's required feature flags from the prepared registry.
    pub fn item_enabled(&self, stack: &ItemStack, features: &[String]) -> bool {
        enabled(self.item_features.get(&*stack.item), features)
    }

    pub fn item_features(&self) -> ItemFeatureRequirements {
        ItemFeatureRequirements(Arc::new(self.item_features.clone()))
    }

    pub fn parse(json: &serde_json::Value) -> Result<Self, String> {
        let mut catalog = Self::deserialize(json).map_err(|e| e.to_string())?;
        if catalog.schema != 4 {
            return Err(format!(
                "unsupported creative inventory schema {}",
                catalog.schema
            ));
        }
        if let Some(food) = catalog
            .tabs
            .iter_mut()
            .find(|tab| tab.id == "minecraft:food_and_drinks")
        {
            // The prepared rows mark where vanilla called the generator; only
            // the position and visibility are kept, not the exported stacks.
            for rows in std::iter::once(&mut food.build).chain(&mut food.operator_build) {
                let mut inserted = false;
                rows.retain_mut(|row| {
                    if !matches!(&row.source, Source::Stack { snbt } if &*snbt.item == "minecraft:suspicious_stew") {
                        return true;
                    }
                    if inserted {
                        return false;
                    }
                    row.source = Source::SuspiciousStews;
                    inserted = true;
                    true
                });
            }
        }
        Ok(catalog)
    }

    pub fn tabs(&self) -> &[TabRecipe] {
        &self.tabs
    }

    /// The canonical display order of the global search tab in this build.
    pub fn search_display<'a>(&self, contents: &'a [Arc<TabContents>]) -> &'a [ItemStack] {
        self.tabs
            .iter()
            .zip(contents)
            .find(|(tab, _)| tab.kind == Kind::Search)
            .map_or(&[], |(_, list)| list.display.as_slice())
    }

    /// Rebuild in vanilla's two passes, retaining previous contents until a
    /// tab's turn. `edit` sees the partial build and the current tab's two
    /// generated lists before items disabled by flags are dropped. The
    /// returned snapshot is in registry order, not build order.
    pub fn build(
        &self,
        params: Parameters<'_>,
        previous: Option<&[Arc<TabContents>]>,
        mut edit: impl FnMut(&TabRecipe, &[Arc<TabContents>], &mut TabContents),
    ) -> Arc<[Arc<TabContents>]> {
        let empty = Arc::new(TabContents::default());
        let mut contents: Vec<_> = (0..self.tabs.len())
            .map(|i| {
                previous
                    .and_then(|tabs| tabs.get(i))
                    .unwrap_or(&empty)
                    .clone()
            })
            .collect();
        let mut paintings: Vec<_> = params.holders.paintings.iter().collect();
        paintings.sort_by_key(|p| {
            let width = u16::from(p.width.get());
            (width * u16::from(p.height.get()), width)
        });
        let admits = |stack: &ItemStack| self.item_enabled(stack, params.features);
        for category in [true, false] {
            for (i, tab) in self.tabs.iter().enumerate() {
                if category != (tab.kind == Kind::Category) {
                    continue;
                }
                let mut current = if tab.kind == Kind::Search {
                    let mut seen = HashSet::new();
                    let merged: Vec<_> = self
                        .tabs
                        .iter()
                        .zip(&contents)
                        .filter(|(other, _)| other.kind != Kind::Search)
                        .flat_map(|(_, other)| &other.search)
                        .filter(|stack| {
                            seen.insert((stack.item.clone(), component_snapshot(stack)))
                        })
                        .cloned()
                        .collect();
                    TabContents {
                        display: merged.clone(),
                        search: merged,
                    }
                } else {
                    self.generate(tab, params, &paintings)
                };
                edit(tab, &contents, &mut current);
                current.display.retain(admits);
                current.search.retain(admits);
                contents[i] = Arc::new(current);
            }
        }
        contents.into()
    }

    fn generate(
        &self,
        tab: &TabRecipe,
        params: Parameters<'_>,
        paintings: &[&Painting],
    ) -> TabContents {
        let mut current = TabContents::default();
        let mut display_seen = HashSet::new();
        let mut search_seen = HashSet::new();
        let rows = if params.operator {
            tab.operator_build.as_ref().unwrap_or(&tab.build)
        } else {
            &tab.build
        };
        for row in rows {
            let mut add = |mut stack: ItemStack| {
                let key = (
                    stack.item.clone(),
                    normalize_components(&mut stack, params.items),
                );
                let in_display = !matches!(row.visibility, Visibility::SearchTabOnly)
                    && display_seen.insert(key.clone());
                let in_search =
                    !matches!(row.visibility, Visibility::ParentTabOnly) && search_seen.insert(key);
                match (in_display, in_search) {
                    (true, true) => {
                        current.display.push(stack.clone());
                        current.search.push(stack);
                    }
                    (true, false) => current.display.push(stack),
                    (false, true) => current.search.push(stack),
                    (false, false) => {}
                }
            };
            let holders = params.holders;
            match &row.source {
                Source::Stack { snbt } => add(snbt.clone()),
                Source::Paintings { placeable } => {
                    for p in paintings.iter().filter(|p| p.placeable == *placeable) {
                        add(holder_stack(
                            "minecraft:painting",
                            "minecraft:painting/variant",
                            &p.id,
                        ));
                    }
                }
                Source::Potions { item } => {
                    for potion in &holders.potions {
                        if enabled(self.potion_features.get(potion), params.features) {
                            add(ItemStack::new(item, 1).with_potion(potion));
                        }
                    }
                }
                Source::Enchantments { all_levels } => {
                    for enchantment in &holders.enchantments {
                        let max = enchantment.max_level.get();
                        for level in (if *all_levels { 1 } else { max })..=max {
                            let mut book = ItemStack::new("minecraft:enchanted_book", 1);
                            book.enchant(&enchantment.id, u32::from(level));
                            add(book);
                        }
                    }
                }
                Source::SuspiciousStews => {
                    for item in &params.items.registry_order {
                        if let Some(effects) = params.items.suspicious_stew_effects(item) {
                            let mut stew = ItemStack::new("minecraft:suspicious_stew", 1);
                            stew.data_mut().stew_effects = effects.to_vec();
                            add(stew);
                        }
                    }
                }
                Source::Horns => {
                    for horn in &holders.horns {
                        add(holder_stack(
                            "minecraft:goat_horn",
                            "minecraft:instrument",
                            horn,
                        ));
                    }
                }
                Source::Banner { snbt } => {
                    let mut banner = snbt.clone();
                    if let Some(mut patch) = components_tag(banner.damage, banner.data())
                        && let Some(Tag::List(layers)) = patch.get_mut("minecraft:banner_patterns")
                    {
                        // Keep every other component, even with no remaining patterns.
                        layers.retain(|layer| {
                            matches!(layer.get("pattern"), Some(Tag::String(id))
                            if holders.banner_patterns.contains(&id.to_string_lossy()))
                        });
                        (banner.damage, banner.extra) = decode_components(Some(&patch));
                    }
                    add(banner);
                }
            }
        }
        current
    }
}

/// Use the item's prototype to elide redundant patches before comparing tab
/// stacks. Setting the default goat horn instrument, for example, leaves a
/// plain stack in vanilla, not a separate explicitly patched variant.
pub fn normalize_components(stack: &mut ItemStack, items: &Items) -> Option<Tag> {
    if stack.damage == 0 && stack.extra.is_none() {
        return None;
    }
    let patch = encode_components(stack, items);
    (stack.damage, stack.extra) = decode_components(patch.as_ref());
    patch
}

fn holder_stack(item: &str, component: &str, holder: &str) -> ItemStack {
    let mut stack = ItemStack::new(item, 1);
    stack
        .data_mut()
        .set_other(component, Some(Tag::String(holder.into())));
    stack
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn generators_use_the_supplied_holder_order_and_enabled_features() {
        let row = |source, visibility| {
            let mut row: serde_json::Value = source;
            row["visibility"] = json!(visibility);
            row
        };
        let both = "PARENT_AND_SEARCH_TABS";
        let data = json!({
            "schema": 4,
            "tabs": [{"id":"test:catalog", "title":"Catalog", "type":"CATEGORY", "build":[
                row(json!({"kind":"stack", "snbt":"{id:\"test:behind_flag\",count:1}"}), both),
                row(json!({"kind":"paintings", "placeable":true}), both),
                row(json!({"kind":"potions", "item":"minecraft:potion"}), both),
                row(json!({"kind":"enchantments", "all_levels":false}), "PARENT_TAB_ONLY"),
                row(json!({"kind":"enchantments", "all_levels":true}), "SEARCH_TAB_ONLY"),
                row(json!({"kind":"horns"}), both),
                row(json!({"kind":"banner","snbt":concat!(
                    "{id:\"minecraft:white_banner\",count:1,components:{",
                    "\"minecraft:banner_patterns\":[{pattern:\"test:present\",color:\"red\"},",
                    "{pattern:\"test:missing\",color:\"blue\"}]}}"
                )}), both),
            ]}],
            "item_features": {"test:behind_flag":["test:experimental"]},
            "potion_features": {"test:future":["test:experimental"]},
        });
        let catalog = Catalog::parse(&data).unwrap();
        let holders = serde_json::from_value(json!({
            "paintings": [
                {"id":"test:wide","width":2,"height":1,"placeable":true},
                {"id":"test:hidden","width":1,"height":1,"placeable":false},
                {"id":"test:tall","width":1,"height":2,"placeable":true},
                {"id":"test:same_size","width":1,"height":2,"placeable":true},
            ],
            "potions": ["test:future", "test:familiar"],
            "enchantments": [{"id":"test:charge","max_level":2}],
            "horns": ["test:z", "test:a"],
            "banner_patterns": ["test:present"],
        }))
        .unwrap();
        let run = |features: &[String]| {
            catalog.build(
                Parameters {
                    features,
                    operator: false,
                    holders: &holders,
                    items: &Items::default(),
                },
                None,
                |_, _, before_filter| {
                    assert_eq!(&*before_filter.display[0].item, "test:behind_flag")
                },
            )
        };
        let built = run(&[]);
        let tab = &built[0];
        let holder =
            |at: usize, key: &str| match tab.display[at].data().and_then(|data| data.other(key)) {
                Some(Tag::String(id)) => id.to_string_lossy(),
                other => panic!("expected {key} holder, got {other:?}"),
            };
        assert_eq!(holder(0, "minecraft:painting/variant"), "test:tall");
        assert_eq!(holder(1, "minecraft:painting/variant"), "test:same_size");
        assert_eq!(holder(2, "minecraft:painting/variant"), "test:wide");
        assert_eq!(tab.display[3].potion(), Some("test:familiar"));
        assert_eq!(tab.display[4].stored_enchantments()[0].1, 2);
        assert_eq!(tab.search[4].stored_enchantments()[0].1, 1);
        assert_eq!(tab.search[5].stored_enchantments()[0].1, 2);
        assert_eq!(holder(5, "minecraft:instrument"), "test:z");
        assert_eq!(holder(6, "minecraft:instrument"), "test:a");
        assert_eq!(
            tab.display[7].banner_layers(),
            &[crate::banner_layer::BannerLayer::new("test:present", "red")]
        );
        let enabled = run(&["test:experimental".into()]);
        assert_eq!(&*enabled[0].display[0].item, "test:behind_flag");
        assert_eq!(enabled[0].display[4].potion(), Some("test:future"));
    }

    #[test]
    fn search_reads_the_current_prefix_and_previous_suffix_in_registry_order() {
        let catalog = Catalog::parse(&json!({
            "schema":4,"item_features":{},"potion_features":{},"tabs":[
                {"id":"test:hotbar","title":"hotbar","type":"HOTBAR","build":[]},
                {"id":"test:search","title":"search","type":"SEARCH","build":[]},
                {"id":"test:category","title":"category","type":"CATEGORY","build":[]},
                {"id":"test:inventory","title":"inventory","type":"INVENTORY","build":[]}
            ]
        }))
        .unwrap();
        let stack = |id| ItemStack::new(id, 1);
        let prior = [
            Arc::new(TabContents::default()),
            Arc::new(TabContents::default()),
            Arc::new(TabContents::default()),
            Arc::new(TabContents {
                display: vec![],
                search: vec![stack("test:previous")],
            }),
        ];
        let mut events = Vec::new();
        let result = catalog.build(
            Parameters {
                features: &[],
                operator: false,
                holders: &Holders::default(),
                items: &Items::default(),
            },
            Some(&prior),
            |tab, partial, current| {
                events.push(tab.id.as_str().to_owned());
                match tab.kind {
                    Kind::Category => current
                        .search
                        .extend([stack("test:common"), stack("test:category")]),
                    Kind::Hotbar => current
                        .search
                        .extend([stack("test:hotbar"), stack("test:common")]),
                    Kind::Inventory => current.search.push(stack("test:next")),
                    Kind::Search => assert_eq!(partial[3].search, vec![stack("test:previous")]),
                }
            },
        );
        assert_eq!(
            events,
            [
                "test:category",
                "test:hotbar",
                "test:search",
                "test:inventory"
            ]
        );
        assert_eq!(
            result[1].display,
            vec![
                stack("test:hotbar"),
                stack("test:common"),
                stack("test:category"),
                stack("test:previous")
            ]
        );
        assert_eq!(result[1].search, result[1].display);
        assert_eq!(result[3].search, vec![stack("test:next")]);
    }
}
