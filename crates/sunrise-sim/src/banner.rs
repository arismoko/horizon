//! Banner patterns: the `banner_pattern` registry and its tags, the
//! `banner_patterns` layers carried by banner items, shields and placed
//! banners, and the loom rules of `LoomMenu` (selectable patterns, the
//! six-layer limit and the result stack).
use crate::items::{ItemStack, Items};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};
use sunrise_core::nbt::Tag;
use sunrise_level::dyes::{DYES, diffuse};

pub use sunrise_items::banner_layer::{BannerLayer, layers_from_nbt, layers_to_nbt};

/// Layers a loom may stack on a banner (`LoomMenu`'s `hasMaxPatterns`).
pub const MAX_LAYERS: usize = 6;

/// One `banner_pattern` registry entry.
#[derive(Clone, Debug)]
pub struct Pattern {
    pub id: Arc<str>,
    /// `asset_id` path, the texture under `entity/banner` and `entity/shield`.
    pub asset: String,
    pub translation_key: String,
}

impl Pattern {
    /// `BannerPattern.DIRECT_CODEC`.
    pub fn decode(id: &str, value: &Value) -> Result<Self, String> {
        let asset = value["asset_id"].as_str().ok_or("asset_id is required")?;
        let asset = sunrise_core::identifier::Identifier::parse(asset)
            .map_err(|error| format!("asset_id: {error}"))?
            .to_string();
        let translation_key = value["translation_key"]
            .as_str()
            .ok_or("translation_key is required")?;
        Ok(Self {
            id: id.into(),
            asset: asset.trim_start_matches("minecraft:").to_owned(),
            translation_key: translation_key.to_owned(),
        })
    }
}

/// The pattern registry, its tags and the item components the loom reads.
#[derive(Default)]
pub struct Banners {
    patterns: Vec<Pattern>,
    /// `#minecraft:no_item_required`, in tag order.
    no_item_required: Vec<Arc<str>>,
    /// Item -> `dye` component.
    dyes: HashMap<Arc<str>, Arc<str>>,
    /// Item -> `provides_banner_patterns`, in tag order.
    provided: HashMap<Arc<str>, Vec<Arc<str>>>,
    /// Banner-pattern tag id -> its members, for a component that names a tag.
    pattern_tags: HashMap<String, Vec<Arc<str>>>,
}

/// `Identifier.parse`: an id or tag without a namespace is `minecraft`'s.
fn namespaced(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// The game's own `banner_pattern` entry `id`, read from the prepared root.
fn prepared_pattern(root: &Path, id: &str) -> Option<Pattern> {
    let stem = id.strip_prefix("minecraft:")?;
    let bytes = fs::read(root.join(format!("data/minecraft/banner_pattern/{stem}.json"))).ok()?;
    Pattern::decode(id, &serde_json::from_slice(&bytes).ok()?).ok()
}

impl Banners {
    /// The prepared root's registry and tags, for a game with no world open.
    pub fn load(root: &Path, defaults: &sunrise_world::item_defaults::ItemDefaults) -> Self {
        let mut stems: Vec<_> = fs::read_dir(root.join("data/minecraft/banner_pattern"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .filter_map(|p| Some(p.file_stem()?.to_str()?.to_owned()))
            .collect();
        stems.sort();
        let patterns: Vec<Pattern> = stems
            .iter()
            .filter_map(|stem| prepared_pattern(root, &format!("minecraft:{stem}")))
            .collect();
        let tags = sunrise_world::tag_layers::load(
            root,
            "banner_pattern",
            &sunrise_world::loot::PackOverlay::NONE,
            |id| patterns.iter().any(|p| &*p.id == id),
        )
        .unwrap_or_default();
        Self::new(patterns, tags, defaults)
    }
    /// The world's `banner_pattern` registry (the game's entries its packs
    /// keep and the packs' own winners) and its bound, layered tags.
    pub fn stage(
        root: &Path,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
        packed: &crate::lan::data::PackRegistryDefinitions,
        tags: impl IntoIterator<Item = (String, Vec<String>)>,
    ) -> Self {
        let patterns = packed
            .entries()
            .filter(|(registry, _, admitted)| *admitted && *registry == "minecraft:banner_pattern")
            .filter_map(|(_, id, _)| {
                packed
                    .banner_patterns()
                    .find(|pattern| *pattern.id == *id)
                    .cloned()
                    .or_else(|| prepared_pattern(root, &id))
            })
            .collect();
        Self::new(patterns, tags, defaults)
    }
    fn new(
        patterns: Vec<Pattern>,
        tags: impl IntoIterator<Item = (String, Vec<String>)>,
        defaults: &sunrise_world::item_defaults::ItemDefaults,
    ) -> Self {
        let pattern_tags: HashMap<String, Vec<Arc<str>>> = tags
            .into_iter()
            .map(|(tag, members)| (tag, members.iter().map(|m| Arc::from(&**m)).collect()))
            .collect();
        let mut banners = Self {
            patterns,
            no_item_required: pattern_tags
                .get("minecraft:no_item_required")
                .cloned()
                .unwrap_or_default(),
            pattern_tags,
            ..Self::default()
        };
        for (item, c) in defaults.entries() {
            let id: Arc<str> = Arc::from(item.as_str());
            if let Some(dye) = c["minecraft:dye"].as_str() {
                banners.dyes.insert(id.clone(), Arc::from(dye));
            }
            match &c["minecraft:provides_banner_patterns"] {
                Value::String(tag) if tag.starts_with('#') => {
                    let members = banners.provides_one(tag);
                    banners.provided.insert(id, members);
                }
                Value::String(single) => {
                    banners
                        .provided
                        .insert(id, vec![Arc::from(single.as_str())]);
                }
                _ => {}
            }
        }
        banners
    }
    pub fn pattern(&self, id: &str) -> Option<&Pattern> {
        self.patterns.iter().find(|p| &*p.id == id)
    }
    /// `Sheets.getBannerSprite` texture name of a pattern.
    pub fn banner_texture(&self, id: &str) -> String {
        match self.pattern(id) {
            Some(p) => sprite("entity/banner", &p.asset),
            None => format!("entity/banner/{}", id.trim_start_matches("minecraft:")),
        }
    }
    /// `Sheets.getShieldSprite` texture name of a pattern.
    pub fn shield_texture(&self, id: &str) -> String {
        match self.pattern(id) {
            Some(p) => sprite("entity/shield", &p.asset),
            None => format!("entity/shield/{}", id.trim_start_matches("minecraft:")),
        }
    }
    /// `BannerPatternLayers.Layer.description` translation key.
    pub fn layer_key(&self, layer: &BannerLayer) -> String {
        let base = self.pattern(&layer.pattern).map_or_else(
            || {
                format!(
                    "block.minecraft.banner.{}",
                    layer.pattern.trim_start_matches("minecraft:")
                )
            },
            |p| p.translation_key.clone(),
        );
        format!("{base}.{}", layer.color)
    }
    /// `LoomMenu.isDyeItem`: the dye colour of a loom dye.
    pub fn loom_dye(&self, items: &Items, stack: &ItemStack) -> Option<Arc<str>> {
        if !items
            .item_tags
            .get("minecraft:loom_dyes")
            .is_some_and(|t| t.contains(&*stack.item))
        {
            return None;
        }
        let data = stack.data();
        if data.is_some_and(|d| d.other("!minecraft:dye").is_some()) {
            return None;
        }
        if let Some(Tag::String(color)) = data.and_then(|d| d.other("minecraft:dye")) {
            return Some(Arc::from(
                color.to_string_lossy().trim_start_matches("minecraft:"),
            ));
        }
        self.dyes.get(&stack.item).cloned()
    }
    /// `LoomMenu.isPatternItem`: the item tag and a present
    /// `provides_banner_patterns` component (the stack's, else the item's).
    pub fn is_pattern_item(&self, items: &Items, stack: &ItemStack) -> bool {
        if !items
            .item_tags
            .get("minecraft:loom_patterns")
            .is_some_and(|t| t.contains(&*stack.item))
        {
            return false;
        }
        let data = stack.data();
        if data.is_some_and(|d| d.other("!minecraft:provides_banner_patterns").is_some()) {
            return false;
        }
        data.is_some_and(|d| d.other("minecraft:provides_banner_patterns").is_some())
            || self.provided.contains_key(&stack.item)
    }
    /// `LoomMenu.getSelectablePatterns`.
    pub fn selectable(&self, pattern_item: Option<&ItemStack>) -> Vec<Arc<str>> {
        match pattern_item {
            None => self.no_item_required.clone(),
            Some(stack) => {
                let data = stack.data();
                if data.is_some_and(|d| d.other("!minecraft:provides_banner_patterns").is_some()) {
                    return Vec::new();
                }
                match data.and_then(|d| d.other("minecraft:provides_banner_patterns")) {
                    Some(tag) => self.provides(tag),
                    None => self.provided.get(&stack.item).cloned().unwrap_or_default(),
                }
            }
        }
    }
    /// A `provides_banner_patterns` value: a pattern id, a tag, or a list of
    /// either (`HolderSet`'s saved forms).
    fn provides(&self, tag: &Tag) -> Vec<Arc<str>> {
        match tag {
            Tag::String(value) => self.provides_one(&value.to_string_lossy()),
            Tag::List(values) => values
                .iter()
                .flat_map(|value| match value {
                    Tag::String(value) => self.provides_one(&value.to_string_lossy()),
                    _ => Vec::new(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    fn provides_one(&self, value: &str) -> Vec<Arc<str>> {
        match value.strip_prefix('#') {
            Some(tag) => self
                .pattern_tags
                .get(&namespaced(tag))
                .cloned()
                .unwrap_or_default(),
            None => vec![Arc::from(namespaced(value))],
        }
    }
}

/// `Sheets.BANNER_MAPPER`/`SHIELD_MAPPER`: an asset under `prefix`, in its own
/// namespace when it names one (`minecraft`'s is a bare path here).
fn sprite(prefix: &str, asset: &str) -> String {
    match asset.split_once(':') {
        Some((namespace, path)) => format!("{namespace}:{prefix}/{path}"),
        None => format!("{prefix}/{asset}"),
    }
}

/// `BannerItem`: a banner's dye colour from its item id.
pub fn banner_color(item: &str) -> Option<&'static str> {
    let name = item.strip_prefix("minecraft:")?.strip_suffix("_banner")?;
    DYES.iter().map(|d| d.0).find(|d| *d == name)
}
/// `DyeColor.getTextureDiffuseColor` (RGB) by dye name, as the resource
/// packs draw it (`pack_palette`): banner layers, banner and shield items and the
/// loom follow a pack's dye colours as the banner base does.
pub fn dye_rgb(name: &str) -> u32 {
    DYES.iter()
        .position(|d| d.0 == name)
        .map_or(0xFFFFFF, diffuse)
}
/// The ominous banner's component patch hides its pattern tooltip.
pub fn tooltip_layers(stack: &ItemStack) -> &[BannerLayer] {
    if stack.data().is_some_and(|d| d.ominous_banner) {
        &[]
    } else {
        stack.banner_layers()
    }
}
/// `LoomMenu.setupResultSlot`: one banner with the pattern added in the dye colour.
pub fn loom_result(banner: &ItemStack, dye: &str, pattern: &str) -> ItemStack {
    let mut result = ItemStack {
        count: 1,
        ..banner.clone()
    };
    result
        .data_mut()
        .banner_layers_mut()
        .push(BannerLayer::new(pattern, dye));
    result.tidy();
    result
}
/// Whether the banner already carries the loom's limit.
pub fn has_max_layers(banner: &ItemStack) -> bool {
    banner.banner_layers().len() >= MAX_LAYERS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banners() -> Banners {
        let mut b = Banners {
            no_item_required: vec![Arc::from("minecraft:stripe_top")],
            ..Default::default()
        };
        b.provided.insert(
            Arc::from("minecraft:flower_banner_pattern"),
            vec![Arc::from("minecraft:flower")],
        );
        b
    }
    #[test]
    fn selectable_follow_the_pattern_item() {
        let b = banners();
        assert_eq!(b.selectable(None).len(), 1);
        let item = ItemStack::new("minecraft:flower_banner_pattern", 1);
        assert_eq!(&*b.selectable(Some(&item))[0], "minecraft:flower");
    }
    #[test]
    fn prepared_dye_component() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let defaults = sunrise_world::item_defaults::shared(&root).unwrap();
        let items =
            Items::tags_only(&root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap();
        let b = Banners::load(&root, &defaults);
        let mut dye = ItemStack::new("minecraft:white_dye", 1);
        assert_eq!(&*b.loom_dye(&items, &dye).unwrap(), "white");
        dye.data_mut()
            .set_other("minecraft:dye", Some(Tag::String("red".into())));
        assert_eq!(&*b.loom_dye(&items, &dye).unwrap(), "red");
        dye.data_mut().set_other(
            "!minecraft:dye",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert!(b.loom_dye(&items, &dye).is_none());
    }
    #[test]
    fn prepared_pattern_component_presence() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let defaults = sunrise_world::item_defaults::shared(&root).unwrap();
        let items =
            Items::tags_only(&root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap();
        let b = Banners::load(&root, &defaults);
        let mut pattern = ItemStack::new("minecraft:globe_banner_pattern", 1);
        assert!(b.is_pattern_item(&items, &pattern));
        pattern.data_mut().set_other(
            "!minecraft:provides_banner_patterns",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert!(!b.is_pattern_item(&items, &pattern));
    }
    #[test]
    fn prepared_pattern_component_selection() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let defaults = sunrise_world::item_defaults::shared(&root).unwrap();
        let b = Banners::load(&root, &defaults);
        let mut pattern = ItemStack::new("minecraft:globe_banner_pattern", 1);
        assert_eq!(&*b.selectable(Some(&pattern))[0], "minecraft:globe");
        pattern.data_mut().set_other(
            "minecraft:provides_banner_patterns",
            Some(Tag::List(vec![Tag::String("minecraft:creeper".into())])),
        );
        assert_eq!(&*b.selectable(Some(&pattern))[0], "minecraft:creeper");
        // One id without its namespace, or a tag, as `HolderSet`'s codec reads them.
        pattern.data_mut().set_other(
            "minecraft:provides_banner_patterns",
            Some(Tag::String("creeper".into())),
        );
        assert_eq!(
            b.selectable(Some(&pattern)),
            [Arc::from("minecraft:creeper")]
        );
        pattern.data_mut().set_other(
            "minecraft:provides_banner_patterns",
            Some(Tag::String("#pattern_item/globe".into())),
        );
        assert_eq!(b.selectable(Some(&pattern)), [Arc::from("minecraft:globe")]);
        pattern.data_mut().set_other(
            "!minecraft:provides_banner_patterns",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert!(b.selectable(Some(&pattern)).is_empty());
    }
    #[test]
    fn banner_colour_comes_from_the_item() {
        assert_eq!(
            banner_color("minecraft:light_gray_banner"),
            Some("light_gray")
        );
        assert_eq!(banner_color("minecraft:white_banner"), Some("white"));
        assert_eq!(banner_color("minecraft:white_wool"), None);
    }
    #[test]
    fn loom_adds_a_layer_up_to_the_limit() {
        let mut banner = ItemStack::new("minecraft:white_banner", 3);
        for i in 0..MAX_LAYERS {
            assert!(!has_max_layers(&banner));
            banner = loom_result(&banner, "red", "minecraft:cross");
            assert_eq!(banner.count, 1);
            assert_eq!(banner.banner_layers().len(), i + 1);
        }
        assert!(has_max_layers(&banner));
    }
    #[test]
    fn ominous_banner_hides_tooltip_layers_but_keeps_its_patterns() {
        let mut banner = ItemStack::new("minecraft:white_banner", 1);
        banner.data_mut().ominous_banner = true;
        (banner.damage, banner.extra) = crate::item_data::decode_components(
            crate::item_data::encode_components(&banner, &Default::default()).as_ref(),
        );
        assert_eq!(
            banner.banner_layers().len(),
            crate::item_data::OMINOUS_LAYERS.len()
        );
        assert!(tooltip_layers(&banner).is_empty());
        banner.data_mut().custom_name =
            Some(sunrise_core::nbt::Tag::String("Captain's Standard".into()));
        assert!(tooltip_layers(&banner).is_empty());
        banner
            .data_mut()
            .set_other("minecraft:tooltip_display", None);
        assert_eq!(tooltip_layers(&banner), banner.banner_layers());
        assert!(!tooltip_layers(&banner).is_empty());
    }

    #[test]
    fn layers_round_trip_through_nbt() {
        let l = vec![
            BannerLayer::new("minecraft:flower", "red"),
            BannerLayer::new("minecraft:border", "black"),
        ];
        assert_eq!(layers_from_nbt(Some(&layers_to_nbt(&l))), l);
    }
}
