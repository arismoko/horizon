use std::collections::HashMap;

/// The same registries a vanilla client finishes configuring, in network ID order.
/// Known-pack entries have already been filled from local resources by configuration.
pub fn live_holders(
    builtin: &sunrise_protocol::registry::Builtin,
    dynamic: &sunrise_protocol::registry::Dynamic,
) -> sunrise_items::creative_catalog::Holders {
    use std::num::NonZeroU8;
    use sunrise_core::nbt::Tag;
    use sunrise_items::creative_catalog::{Enchantment, Holders, Painting};
    let size = |value: &Tag| {
        let n = match *value {
            Tag::Byte(n) => i64::from(n),
            Tag::Short(n) => i64::from(n),
            Tag::Int(n) => i64::from(n),
            Tag::Long(n) => n,
            _ => return None,
        };
        NonZeroU8::new(u8::try_from(n).ok()?)
    };
    let entries = |kind: &str| dynamic.registries.get(kind).into_iter().flatten();
    let text = |data: &Tag, key: &str| {
        data.get(key)
            .map(sunrise_game_data::scoreboard::tag_to_json)
    };
    let enchantments = entries("minecraft:enchantment")
        .filter_map(|(id, data)| {
            let data = data.as_ref()?;
            Some(Enchantment {
                id: id.clone(),
                max_level: size(data.get("max_level")?)?,
                description: text(data, "description").unwrap_or_default(),
            })
        })
        .collect();
    let placeable = dynamic
        .tag("minecraft:painting_variant", "minecraft:placeable")
        .unwrap_or_default();
    let paintings = entries("minecraft:painting_variant")
        .enumerate()
        .filter_map(|(i, (id, data))| {
            let data = data.as_ref()?;
            Some(Painting {
                id: id.clone(),
                width: size(data.get("width")?)?,
                height: size(data.get("height")?)?,
                placeable: placeable.contains(&(i as i32)),
                title: text(data, "title"),
                author: text(data, "author"),
            })
        })
        .collect();
    let horns = dynamic
        .tag("minecraft:instrument", "minecraft:goat_horns")
        .unwrap_or_default()
        .iter()
        .filter_map(|&id| dynamic.name("minecraft:instrument", id).map(str::to_owned))
        .collect();
    let instrument_descriptions = entries("minecraft:instrument")
        .filter_map(|(id, data)| Some((id.clone(), text(data.as_ref()?, "description")?)))
        .collect();
    let banner_patterns = entries("minecraft:banner_pattern")
        .map(|(id, _)| id.clone())
        .collect();
    let potions = builtin
        .get("minecraft:potion")
        .map_or_else(Vec::new, |registry| registry.names.clone());
    Holders {
        potions,
        enchantments,
        paintings,
        horns,
        instrument_descriptions,
        banner_patterns,
    }
}

/// Display names from the pinned en_us language file.
#[derive(Default)]
pub struct Names {
    map: HashMap<String, String>,
    item_names: HashMap<String, serde_json::Value>,
    /// The block registry, which says whether an item is a block's (`Item.Properties.useBlockDescriptionPrefix`).
    blocks: Option<&'static sunrise_world::registry::BlockRegistry>,
}
impl Names {
    pub fn load(root: &sunrise_assets::resource_source::ResourceSource) -> Self {
        let read = |namespace: &str| {
            root.read(&format!("assets/{namespace}/lang/en_us.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
                .unwrap_or_default()
        };
        let mut map = read("minecraft");
        // Content mods' names (docs/modpacks.md); none in a vanilla root.
        for namespace in root
            .namespaces("assets")
            .into_iter()
            .filter(|ns| ns != "minecraft")
        {
            map.extend(read(&namespace));
        }
        Self {
            map,
            item_names: HashMap::new(),
            blocks: None,
        }
    }
    /// Keep each item's default `minecraft:item_name` component from the items' prototypes:
    /// `Item.getDescriptionId` is its translation key, so the key is the item's own, never
    /// guessed from the language.
    pub fn with_item_names(
        mut self,
        prototypes: &std::collections::BTreeMap<
            String,
            std::collections::BTreeMap<String, serde_json::Value>,
        >,
        blocks: &'static sunrise_world::registry::BlockRegistry,
    ) -> Self {
        self.blocks = Some(blocks);
        self.item_names = prototypes
            .iter()
            .filter_map(|(id, components)| {
                Some((id.clone(), components.get("minecraft:item_name")?.clone()))
            })
            .collect();
        self
    }
    /// Whether `id` is the item of a block of the same id, which takes its block's `block.*`
    /// description id (a mod block item, whose prototype carries no `item_name`).
    pub fn is_block_item(&self, id: &str) -> bool {
        self.blocks
            .is_some_and(|blocks| blocks.default_state(id).is_some())
    }
    /// An item's default `minecraft:item_name` component, as its prototype has it.
    pub fn item_name(&self, id: &str) -> Option<&serde_json::Value> {
        self.item_names.get(id)
    }
    pub fn item(&self, id: &str) -> String {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        self.map
            .get(&format!("item.{namespace}.{path}"))
            .or_else(|| self.map.get(&format!("block.{namespace}.{path}")))
            .cloned()
            .unwrap_or_else(|| path.replace('_', " "))
    }
    /// An item's display name in the selected language: `item.*` then
    /// `block.*`, then the pinned English map (`Names::item`).
    pub fn display_item(&self, id: &str) -> String {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        if let Some(lang) = sunrise_game_data::lang::current() {
            for prefix in ["item", "block"] {
                if let Some(name) = lang.get(&format!("{prefix}.{namespace}.{path}")) {
                    return name.to_owned();
                }
            }
        }
        self.item(id)
    }
    /// A translation key's text in the selected language, else the pinned
    /// English map (`HoverName` and tooltip keys such as potion names).
    pub fn display_get(&self, key: &str) -> Option<String> {
        sunrise_game_data::lang::current()
            .and_then(|lang| lang.get(key).map(str::to_owned))
            .or_else(|| self.get(key).map(str::to_owned))
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
}
