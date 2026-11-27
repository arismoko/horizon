//! Data-driven mob variant and sound-variant registries (`VariantUtils`,
//! `SpawnPrioritySelectors`, `WolfVariant`, `WolfSoundVariant`, `CatVariant`,
//! `CatSoundVariant`, and the later families as they move onto `REGISTRY_KINDS`).
//!
//! Each family's frozen world registry is a [`VariantTable<T>`] whose entries
//! are sorted in vanilla's `ResourceManagerRegistryLoadTask` file order
//! (`(format!("{path}.json"), namespace)`), matching `lan::data::SyncData`'s
//! network ids. On a client, [`VariantTable::from_synced`] keeps the server's
//! exact sync order.
use super::finalize::FinalizeLevel;
use crate::lan::data::RegistryRef;
use serde_json::Value;
use std::{fs, path::Path};
use sunrise_core::{
    identifier::Identifier,
    nbt::{Compound, JavaString, Tag},
    random::RandomSource,
};
use sunrise_protocol::registry::Dynamic;

/// `SpawnCondition` (`minecraft:biome`, `minecraft:structure`, `minecraft:moon_brightness`).
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnCondition {
    /// `BiomeCheck`: canonical `"ns:id"` or `"#ns:tag"` entries.
    Biome(Vec<String>),
    /// `StructureCheck`: canonical `"ns:id"` or `"#ns:tag"` entries.
    Structure(Vec<String>),
    /// `MoonBrightnessCheck`: `MinMaxBounds.Doubles` over `MOON_BRIGHTNESS`.
    MoonBrightness { min: Option<f64>, max: Option<f64> },
}

/// `PriorityProvider.Selector<SpawnContext, SpawnCondition>`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnSelector {
    pub condition: Option<SpawnCondition>,
    pub priority: i32,
}

/// `PriorityProvider<SpawnContext, SpawnCondition>`.
pub trait HasSpawnConditions {
    fn spawn_conditions(&self) -> &[SpawnSelector];
}

/// Decode `SpawnPrioritySelectors.CODEC` (`spawn_conditions`).
pub fn decode_spawn_conditions(value: &Value) -> Result<Vec<SpawnSelector>, String> {
    let list = value
        .as_array()
        .ok_or("spawn_conditions: a list is required")?;
    let mut selectors = Vec::with_capacity(list.len());
    for (index, item) in list.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| format!("spawn_conditions[{index}]: not a map"))?;
        let priority = obj
            .get("priority")
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| format!("spawn_conditions[{index}].priority: an int is required"))?;
        let condition = match obj.get("condition") {
            None => None,
            Some(c) => Some(decode_spawn_condition(
                &format!("spawn_conditions[{index}].condition"),
                c,
            )?),
        };
        selectors.push(SpawnSelector {
            condition,
            priority,
        });
    }
    Ok(selectors)
}

fn decode_spawn_condition(field: &str, value: &Value) -> Result<SpawnCondition, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{field}: not a map"))?;
    let raw_type = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field}.type: a string is required"))?;
    let kind = Identifier::parse(raw_type)
        .map_err(|e| format!("{field}.type: invalid identifier {raw_type:?}: {e}"))?;
    match kind.as_str() {
        "minecraft:biome" => {
            let biomes = obj
                .get("biomes")
                .ok_or_else(|| format!("{field}.biomes is required"))?;
            Ok(SpawnCondition::Biome(decode_holder_set(
                &format!("{field}.biomes"),
                biomes,
            )?))
        }
        "minecraft:structure" => {
            let structures = obj
                .get("structures")
                .ok_or_else(|| format!("{field}.structures is required"))?;
            Ok(SpawnCondition::Structure(decode_holder_set(
                &format!("{field}.structures"),
                structures,
            )?))
        }
        "minecraft:moon_brightness" => {
            let range = obj
                .get("range")
                .ok_or_else(|| format!("{field}.range is required"))?;
            let (min, max) = decode_double_bounds(&format!("{field}.range"), range)?;
            Ok(SpawnCondition::MoonBrightness { min, max })
        }
        other => Err(format!("{field}.type: unknown spawn condition {other:?}")),
    }
}

/// `RegistryCodecs.holderSet`: `"#ns:tag"`, `"ns:id"`, or `["ns:id", ...]`.
fn decode_holder_set(field: &str, value: &Value) -> Result<Vec<String>, String> {
    match value {
        Value::String(raw) => {
            if let Some(tag) = raw.strip_prefix('#') {
                let id = Identifier::parse(tag)
                    .map_err(|e| format!("{field}: invalid tag {raw:?}: {e}"))?;
                Ok(vec![format!("#{}", id.as_str())])
            } else {
                let id = Identifier::parse(raw)
                    .map_err(|e| format!("{field}: invalid identifier {raw:?}: {e}"))?;
                Ok(vec![id.as_str().to_owned()])
            }
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let raw = item
                    .as_str()
                    .ok_or_else(|| format!("{field}[{i}]: a string is required"))?;
                let id = Identifier::parse(raw)
                    .map_err(|e| format!("{field}[{i}]: invalid identifier {raw:?}: {e}"))?;
                Ok(id.as_str().to_owned())
            })
            .collect(),
        _ => Err(format!("{field}: a string or list of strings is required")),
    }
}

/// `MinMaxBounds.Doubles.CODEC`: a finite number or `{ min?, max? }`.
fn decode_double_bounds(field: &str, value: &Value) -> Result<(Option<f64>, Option<f64>), String> {
    if let Some(exact) = value.as_f64() {
        if !exact.is_finite() {
            return Err(format!("{field}: must be finite"));
        }
        return Ok((Some(exact), Some(exact)));
    }
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{field}: a number or map is required"))?;
    let read = |key: &str| -> Result<Option<f64>, String> {
        match obj.get(key) {
            None => Ok(None),
            Some(v) => {
                let n = v
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| format!("{field}.{key}: a finite number is required"))?;
                Ok(Some(n))
            }
        }
    };
    let min = read("min")?;
    let max = read("max")?;
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err(format!(
            "{field}: min ({lo}) cannot be greater than max ({hi})"
        ));
    }
    Ok((min, max))
}

/// One frozen world registry of mob variants or sound variants, indexed by
/// network id (`RegistryRef<T>`).
#[derive(Clone, Debug)]
pub struct VariantTable<T> {
    entries: Vec<(String, T)>,
}

impl<T> Default for VariantTable<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T> VariantTable<T> {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a server-side world table in vanilla's `ResourceManagerRegistryLoadTask`
    /// file order (`(format!("{path}.json"), namespace)`), matching `SyncData`.
    pub fn from_entries(mut entries: Vec<((String, String), T)>) -> Self {
        entries.sort_by_cached_key(|((ns, path), _)| (format!("{path}.json"), ns.clone()));
        Self {
            entries: entries
                .into_iter()
                .map(|((ns, path), value)| (format!("{ns}:{path}"), value))
                .collect(),
        }
    }

    /// Build a client-side table preserving the server's exact sync order.
    pub fn from_synced(entries: Vec<(String, T)>) -> Self {
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, r: RegistryRef<T>) -> Option<&T> {
        self.entries.get(r.network_id() as usize).map(|(_, v)| v)
    }

    pub fn id(&self, r: RegistryRef<T>) -> Option<&str> {
        self.entries
            .get(r.network_id() as usize)
            .map(|(id, _)| id.as_str())
    }

    pub fn entry(&self, r: RegistryRef<T>) -> Option<(&str, &T)> {
        self.entries
            .get(r.network_id() as usize)
            .map(|(id, v)| (id.as_str(), v))
    }

    /// Look up an entry by canonical or default-namespace identifier.
    pub fn lookup(&self, id: &str) -> Option<RegistryRef<T>> {
        let canonical = Identifier::parse(id).ok()?;
        let target = canonical.as_str();
        self.entries
            .iter()
            .position(|(name, _)| name == target)
            .map(|i| RegistryRef::new(i as u32))
    }

    /// `VariantUtils.getDefaultOrAny`: `default_id` if present, else the first entry.
    pub fn default_or_any(&self, default_id: &str) -> Option<RegistryRef<T>> {
        self.lookup(default_id)
            .or_else(|| (!self.entries.is_empty()).then(|| RegistryRef::new(0)))
    }

    /// `Registry.getRandom`: any entry of the table.
    pub fn random(&self, random: &mut impl RandomSource) -> Option<RegistryRef<T>> {
        if self.entries.is_empty() {
            return None;
        }
        let i = random
            .next_int_bound(self.entries.len() as i32)
            .unwrap_or(0) as u32;
        Some(RegistryRef::new(i))
    }

    pub fn iter(&self) -> impl Iterator<Item = (RegistryRef<T>, &str, &T)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(i, (id, v))| (RegistryRef::new(i as u32), id.as_str(), v))
    }
}

impl<T: HasSpawnConditions> VariantTable<T> {
    /// `VariantUtils.selectVariantToSpawn` / `PriorityProvider.pick`: the entries
    /// of the highest priority whose condition holds, one of them at random.
    pub fn select<L: FinalizeLevel + ?Sized>(
        &self,
        level: &L,
        pos: [i32; 3],
        random: &mut impl RandomSource,
    ) -> Option<RegistryRef<T>> {
        let mut unpacked: Vec<(RegistryRef<T>, Option<&SpawnCondition>, i32)> = Vec::new();
        for (i, (_, entry)) in self.entries.iter().enumerate() {
            let r = RegistryRef::new(i as u32);
            for selector in entry.spawn_conditions() {
                unpacked.push((r, selector.condition.as_ref(), selector.priority));
            }
        }
        unpacked.sort_by_key(|(_, _, priority)| std::cmp::Reverse(*priority));
        let mut highest = i32::MIN;
        let mut selected = Vec::new();
        for (r, condition, priority) in unpacked {
            if priority < highest {
                continue;
            }
            let ok = match condition {
                None => true,
                Some(SpawnCondition::Biome(list)) => {
                    list.iter().any(|b| match b.strip_prefix('#') {
                        Some(tag) => level.biome_tag(tag, pos),
                        None => level.biome(pos) == *b,
                    })
                }
                Some(SpawnCondition::Structure(list)) => {
                    let here = level.structures_with_piece_at(pos);
                    list.iter().any(|s| match s.strip_prefix('#') {
                        Some(tag) => here.iter().any(|h| level.structure_tag(tag, h)),
                        None => here.iter().any(|h| h == s),
                    })
                }
                Some(SpawnCondition::MoonBrightness { min, max }) => {
                    let b = f64::from(super::spawning::MOON_BRIGHTNESS[level.moon_phase(pos) & 7]);
                    min.is_none_or(|m| b >= m) && max.is_none_or(|m| b <= m)
                }
            };
            if ok {
                highest = priority;
                selected.push(r);
            }
        }
        if selected.is_empty() {
            return None;
        }
        let pick = random.next_int_bound(selected.len() as i32).unwrap_or(0) as usize;
        Some(selected[pick])
    }
}

/// `WolfVariant.AssetInfo`: wild, tame and angry texture asset ids (canonical `ns:path`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WolfAssetInfo {
    pub wild: String,
    pub tame: String,
    pub angry: String,
}

impl WolfAssetInfo {
    fn decode(field: &str, value: &Value) -> Result<Self, String> {
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{field}: not a map"))?;
        let asset = |key: &str| -> Result<String, String> {
            let raw = obj
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{field}.{key}: a string is required"))?;
            let id = Identifier::parse(raw)
                .map_err(|e| format!("{field}.{key}: invalid identifier {raw:?}: {e}"))?;
            Ok(id.as_str().to_owned())
        };
        Ok(Self {
            wild: asset("wild")?,
            tame: asset("tame")?,
            angry: asset("angry")?,
        })
    }

    fn to_nbt(&self) -> Tag {
        Tag::compound([
            ("wild", Tag::String(JavaString::from(self.wild.as_str()))),
            ("tame", Tag::String(JavaString::from(self.tame.as_str()))),
            ("angry", Tag::String(JavaString::from(self.angry.as_str()))),
        ])
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        let read = |key: &str| match tag.get(key) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        Some(Self {
            wild: read("wild")?,
            tame: read("tame")?,
            angry: read("angry")?,
        })
    }
}

/// `WolfVariant`: adult and baby texture asset sets plus spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct WolfVariant {
    pub assets: WolfAssetInfo,
    pub baby_assets: WolfAssetInfo,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for WolfVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl WolfVariant {
    /// `WolfVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let assets =
            WolfAssetInfo::decode("assets", obj.get("assets").ok_or("assets is required")?)?;
        let baby_assets = WolfAssetInfo::decode(
            "baby_assets",
            obj.get("baby_assets").ok_or("baby_assets is required")?,
        )?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            assets,
            baby_assets,
            spawn_conditions,
        })
    }

    /// `WolfVariant.NETWORK_CODEC`: `assets` and `baby_assets` only.
    pub fn to_network_nbt(&self) -> Tag {
        Tag::compound([
            ("assets", self.assets.to_nbt()),
            ("baby_assets", self.baby_assets.to_nbt()),
        ])
    }

    /// Decode a client's synced `WolfVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        let assets = WolfAssetInfo::decode_nbt(tag.get("assets")?)?;
        let baby_assets = WolfAssetInfo::decode_nbt(tag.get("baby_assets")?)?;
        Some(Self {
            assets,
            baby_assets,
            spawn_conditions: Vec::new(),
        })
    }

    /// `Wolf.getTexture`: the atlas sprite key for this variant's age and mood.
    /// Default-namespace assets strip `"minecraft:"` to match Sunrise's atlas keys;
    /// non-default namespaces keep `"<ns>:<path>"`.
    pub fn texture(&self, baby: bool, tame: bool, angry: bool) -> &str {
        let info = if baby {
            &self.baby_assets
        } else {
            &self.assets
        };
        let asset = if tame {
            &info.tame
        } else if angry {
            &info.angry
        } else {
            &info.wild
        };
        asset.strip_prefix("minecraft:").unwrap_or(asset)
    }
}

/// `WolfSoundVariant.WolfSoundSet`: the seven sound event ids of an age set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WolfSoundSet {
    pub ambient: String,
    pub death: String,
    pub growl: String,
    pub hurt: String,
    pub pant: String,
    pub whine: String,
    pub step: String,
}

impl WolfSoundSet {
    fn decode_with(
        field: &str,
        value: &Value,
        decode_sound: &mut impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{field}: not a map"))?;
        let mut wire = Compound::new();
        let mut read = |key: &str| -> Result<String, String> {
            let path = format!("{field}.{key}");
            let raw = obj.get(key).ok_or_else(|| format!("{path} is required"))?;
            let (id, tag) = decode_sound(&path, raw)?;
            wire.insert(key.into(), tag);
            Ok(id)
        };
        let set = Self {
            ambient: read("ambient_sound")?,
            death: read("death_sound")?,
            growl: read("growl_sound")?,
            hurt: read("hurt_sound")?,
            pant: read("pant_sound")?,
            whine: read("whine_sound")?,
            step: read("step_sound")?,
        };
        Ok((set, Tag::Compound(wire)))
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        let read = |key: &str| match tag.get(key) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            Some(Tag::Compound(_)) => match tag.get(key)?.get("sound_id") {
                Some(Tag::String(s)) => Some(s.to_string_lossy()),
                _ => None,
            },
            _ => None,
        };
        Some(Self {
            ambient: read("ambient_sound")?,
            death: read("death_sound")?,
            growl: read("growl_sound")?,
            hurt: read("hurt_sound")?,
            pant: read("pant_sound")?,
            whine: read("whine_sound")?,
            step: read("step_sound")?,
        })
    }

    pub fn by_kind(&self, kind: &str) -> Option<&str> {
        let sound = match kind {
            "ambient" => &self.ambient,
            "death" => &self.death,
            "growl" => &self.growl,
            "hurt" => &self.hurt,
            "pant" => &self.pant,
            "whine" => &self.whine,
            "step" => &self.step,
            _ => return None,
        };
        (!sound.is_empty()).then_some(sound.as_str())
    }
}

/// `WolfSoundVariant`: adult and baby sound sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WolfSoundVariant {
    pub adult_sounds: WolfSoundSet,
    pub baby_sounds: WolfSoundSet,
}

impl WolfSoundVariant {
    /// `WolfSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
    pub fn decode_with(
        value: &Value,
        mut decode_sound: impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value.as_object().ok_or("not a map")?;
        let (adult_sounds, adult_tag) = WolfSoundSet::decode_with(
            "adult_sounds",
            obj.get("adult_sounds").ok_or("adult_sounds is required")?,
            &mut decode_sound,
        )?;
        let (baby_sounds, baby_tag) = WolfSoundSet::decode_with(
            "baby_sounds",
            obj.get("baby_sounds").ok_or("baby_sounds is required")?,
            &mut decode_sound,
        )?;
        let wire = Tag::compound([("adult_sounds", adult_tag), ("baby_sounds", baby_tag)]);
        Ok((
            Self {
                adult_sounds,
                baby_sounds,
            },
            wire,
        ))
    }

    /// Decode a client's synced `WolfSoundVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            adult_sounds: WolfSoundSet::decode_nbt(tag.get("adult_sounds")?)?,
            baby_sounds: WolfSoundSet::decode_nbt(tag.get("baby_sounds")?)?,
        })
    }

    /// `Wolf.getSoundSet`: the sound id for `kind` (`ambient`, `death`, `growl`,
    /// `hurt`, `pant`, `whine`, `step`) in the adult or baby set.
    pub fn sound(&self, baby: bool, kind: &str) -> Option<&str> {
        let set = if baby {
            &self.baby_sounds
        } else {
            &self.adult_sounds
        };
        set.by_kind(kind)
    }
}

/// `CatVariant`: adult and baby texture asset ids plus spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct CatVariant {
    pub asset_id: String,
    pub baby_asset_id: String,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for CatVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl CatVariant {
    /// `CatVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let asset_id = decode_asset_field(obj, "asset_id")?;
        let baby_asset_id = decode_asset_field(obj, "baby_asset_id")?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            asset_id,
            baby_asset_id,
            spawn_conditions,
        })
    }

    /// `CatVariant.NETWORK_CODEC`: `asset_id` and `baby_asset_id` only.
    pub fn to_network_nbt(&self) -> Tag {
        Tag::compound([
            (
                "asset_id",
                Tag::String(JavaString::from(self.asset_id.as_str())),
            ),
            (
                "baby_asset_id",
                Tag::String(JavaString::from(self.baby_asset_id.as_str())),
            ),
        ])
    }

    /// Decode a client's synced `CatVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            asset_id: read_nbt_string(tag, "asset_id")?,
            baby_asset_id: read_nbt_string(tag, "baby_asset_id")?,
            spawn_conditions: Vec::new(),
        })
    }

    /// `CatVariant.assetInfo`: the atlas sprite key for this variant's age.
    /// Default-namespace assets strip `"minecraft:"` to match Sunrise's atlas keys;
    /// non-default namespaces keep `"<ns>:<path>"`.
    pub fn texture(&self, baby: bool) -> &str {
        let asset = if baby {
            &self.baby_asset_id
        } else {
            &self.asset_id
        };
        strip_minecraft_prefix(asset)
    }
}

/// `CatSoundVariant.CatSoundSet`: the nine sound event ids of an age set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatSoundSet {
    pub ambient: String,
    pub stray_ambient: String,
    pub hiss: String,
    pub hurt: String,
    pub death: String,
    pub eat: String,
    pub beg_for_food: String,
    pub purr: String,
    pub purreow: String,
}

impl CatSoundSet {
    fn decode_with(
        field: &str,
        value: &Value,
        decode_sound: &mut impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{field}: not a map"))?;
        let mut wire = Compound::new();
        let mut read = |key: &str| -> Result<String, String> {
            let path = format!("{field}.{key}");
            let raw = obj.get(key).ok_or_else(|| format!("{path} is required"))?;
            let (id, tag) = decode_sound(&path, raw)?;
            wire.insert(key.into(), tag);
            Ok(id)
        };
        let set = Self {
            ambient: read("ambient_sound")?,
            stray_ambient: read("stray_ambient_sound")?,
            hiss: read("hiss_sound")?,
            hurt: read("hurt_sound")?,
            death: read("death_sound")?,
            eat: read("eat_sound")?,
            beg_for_food: read("beg_for_food_sound")?,
            purr: read("purr_sound")?,
            purreow: read("purreow_sound")?,
        };
        Ok((set, Tag::Compound(wire)))
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            ambient: read_nbt_sound(tag, "ambient_sound")?,
            stray_ambient: read_nbt_sound(tag, "stray_ambient_sound")?,
            hiss: read_nbt_sound(tag, "hiss_sound")?,
            hurt: read_nbt_sound(tag, "hurt_sound")?,
            death: read_nbt_sound(tag, "death_sound")?,
            eat: read_nbt_sound(tag, "eat_sound")?,
            beg_for_food: read_nbt_sound(tag, "beg_for_food_sound")?,
            purr: read_nbt_sound(tag, "purr_sound")?,
            purreow: read_nbt_sound(tag, "purreow_sound")?,
        })
    }

    pub fn by_kind(&self, kind: &str) -> Option<&str> {
        let sound = match kind {
            "ambient" => &self.ambient,
            "stray_ambient" => &self.stray_ambient,
            "hiss" => &self.hiss,
            "hurt" => &self.hurt,
            "death" => &self.death,
            "eat" => &self.eat,
            "beg_for_food" => &self.beg_for_food,
            "purr" => &self.purr,
            "purreow" => &self.purreow,
            _ => return None,
        };
        (!sound.is_empty()).then_some(sound.as_str())
    }
}

/// `CatSoundVariant`: adult and baby sound sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatSoundVariant {
    pub adult_sounds: CatSoundSet,
    pub baby_sounds: CatSoundSet,
}

impl CatSoundVariant {
    /// `CatSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
    pub fn decode_with(
        value: &Value,
        mut decode_sound: impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value.as_object().ok_or("not a map")?;
        let (adult_sounds, adult_tag) = CatSoundSet::decode_with(
            "adult_sounds",
            obj.get("adult_sounds").ok_or("adult_sounds is required")?,
            &mut decode_sound,
        )?;
        let (baby_sounds, baby_tag) = CatSoundSet::decode_with(
            "baby_sounds",
            obj.get("baby_sounds").ok_or("baby_sounds is required")?,
            &mut decode_sound,
        )?;
        let wire = Tag::compound([("adult_sounds", adult_tag), ("baby_sounds", baby_tag)]);
        Ok((
            Self {
                adult_sounds,
                baby_sounds,
            },
            wire,
        ))
    }

    /// Decode a client's synced `CatSoundVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            adult_sounds: CatSoundSet::decode_nbt(tag.get("adult_sounds")?)?,
            baby_sounds: CatSoundSet::decode_nbt(tag.get("baby_sounds")?)?,
        })
    }

    /// `Cat.getSoundSet`: the sound id for `kind` (`ambient`, `stray_ambient`,
    /// `hiss`, `hurt`, `death`, `eat`, `beg_for_food`, `purr`, `purreow`) in
    /// the adult or baby set.
    pub fn sound(&self, baby: bool, kind: &str) -> Option<&str> {
        let set = if baby {
            &self.baby_sounds
        } else {
            &self.adult_sounds
        };
        set.by_kind(kind)
    }
}

fn decode_asset_field(obj: &serde_json::Map<String, Value>, key: &str) -> Result<String, String> {
    let raw = obj
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key}: a string is required"))?;
    let id =
        Identifier::parse(raw).map_err(|e| format!("{key}: invalid identifier {raw:?}: {e}"))?;
    Ok(id.as_str().to_owned())
}

fn read_nbt_string(tag: &Tag, key: &str) -> Option<String> {
    match tag.get(key) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn read_nbt_sound(tag: &Tag, key: &str) -> Option<String> {
    match tag.get(key) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        Some(Tag::Compound(_)) => match tag.get(key)?.get("sound_id") {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        },
        _ => None,
    }
}

fn strip_minecraft_prefix(asset: &str) -> &str {
    asset.strip_prefix("minecraft:").unwrap_or(asset)
}

/// `PigVariant.ModelType`: `normal` or `cold`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PigModelType {
    #[default]
    Normal,
    Cold,
}

impl PigModelType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Cold => "cold",
        }
    }

    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "normal" => Ok(Self::Normal),
            "cold" => Ok(Self::Cold),
            other => Err(format!("model: unknown pig model type {other:?}")),
        }
    }

    fn decode(obj: &serde_json::Map<String, Value>) -> Result<Self, String> {
        match obj.get("model") {
            None => Ok(Self::Normal),
            Some(v) => Self::parse(v.as_str().ok_or("model: a string is required")?),
        }
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        match tag.get("model") {
            None => Some(Self::Normal),
            Some(Tag::String(s)) => Self::parse(&s.to_string_lossy()).ok(),
            _ => None,
        }
    }
}

/// `PigVariant`: model type, adult/baby texture asset ids, and spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct PigVariant {
    pub model: PigModelType,
    pub asset_id: String,
    pub baby_asset_id: String,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for PigVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl PigVariant {
    /// `PigVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let model = PigModelType::decode(obj)?;
        let asset_id = decode_asset_field(obj, "asset_id")?;
        let baby_asset_id = decode_asset_field(obj, "baby_asset_id")?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            model,
            asset_id,
            baby_asset_id,
            spawn_conditions,
        })
    }

    /// `PigVariant.NETWORK_CODEC`: `model`, `asset_id`, and `baby_asset_id`.
    pub fn to_network_nbt(&self) -> Tag {
        Tag::compound([
            ("model", Tag::String(JavaString::from(self.model.as_str()))),
            (
                "asset_id",
                Tag::String(JavaString::from(self.asset_id.as_str())),
            ),
            (
                "baby_asset_id",
                Tag::String(JavaString::from(self.baby_asset_id.as_str())),
            ),
        ])
    }

    /// Decode a client's synced `PigVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            model: PigModelType::decode_nbt(tag)?,
            asset_id: read_nbt_string(tag, "asset_id")?,
            baby_asset_id: read_nbt_string(tag, "baby_asset_id")?,
            spawn_conditions: Vec::new(),
        })
    }

    /// `PigRenderer.getTextureLocation`.
    pub fn texture(&self, baby: bool) -> &str {
        let asset = if baby {
            &self.baby_asset_id
        } else {
            &self.asset_id
        };
        strip_minecraft_prefix(asset)
    }

    /// `PigRenderer.bakeModels` / `submit`.
    pub fn model_key(&self, baby: bool) -> &'static str {
        if baby {
            "pig_baby#main"
        } else if self.model == PigModelType::Cold {
            "cold_pig#main"
        } else {
            "pig#main"
        }
    }
}

/// `PigSoundVariant.PigSoundSet`: the five sound event ids of an age set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PigSoundSet {
    pub ambient: String,
    pub hurt: String,
    pub death: String,
    pub step: String,
    pub eat: String,
}

impl PigSoundSet {
    fn decode_with(
        field: &str,
        value: &Value,
        decode_sound: &mut impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{field}: not a map"))?;
        let mut wire = Compound::new();
        let mut read = |key: &str| -> Result<String, String> {
            let path = format!("{field}.{key}");
            let raw = obj.get(key).ok_or_else(|| format!("{path} is required"))?;
            let (id, tag) = decode_sound(&path, raw)?;
            wire.insert(key.into(), tag);
            Ok(id)
        };
        let set = Self {
            ambient: read("ambient_sound")?,
            hurt: read("hurt_sound")?,
            death: read("death_sound")?,
            step: read("step_sound")?,
            eat: read("eat_sound")?,
        };
        Ok((set, Tag::Compound(wire)))
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            ambient: read_nbt_sound(tag, "ambient_sound")?,
            hurt: read_nbt_sound(tag, "hurt_sound")?,
            death: read_nbt_sound(tag, "death_sound")?,
            step: read_nbt_sound(tag, "step_sound")?,
            eat: read_nbt_sound(tag, "eat_sound")?,
        })
    }

    pub fn by_kind(&self, kind: &str) -> Option<&str> {
        let sound = match kind {
            "ambient" => &self.ambient,
            "hurt" => &self.hurt,
            "death" => &self.death,
            "step" => &self.step,
            "eat" => &self.eat,
            _ => return None,
        };
        (!sound.is_empty()).then_some(sound.as_str())
    }
}

/// `PigSoundVariant`: adult and baby sound sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PigSoundVariant {
    pub adult_sounds: PigSoundSet,
    pub baby_sounds: PigSoundSet,
}

impl PigSoundVariant {
    /// `PigSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
    pub fn decode_with(
        value: &Value,
        mut decode_sound: impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value.as_object().ok_or("not a map")?;
        let (adult_sounds, adult_tag) = PigSoundSet::decode_with(
            "adult_sounds",
            obj.get("adult_sounds").ok_or("adult_sounds is required")?,
            &mut decode_sound,
        )?;
        let (baby_sounds, baby_tag) = PigSoundSet::decode_with(
            "baby_sounds",
            obj.get("baby_sounds").ok_or("baby_sounds is required")?,
            &mut decode_sound,
        )?;
        let wire = Tag::compound([("adult_sounds", adult_tag), ("baby_sounds", baby_tag)]);
        Ok((
            Self {
                adult_sounds,
                baby_sounds,
            },
            wire,
        ))
    }

    /// Decode a client's synced `PigSoundVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            adult_sounds: PigSoundSet::decode_nbt(tag.get("adult_sounds")?)?,
            baby_sounds: PigSoundSet::decode_nbt(tag.get("baby_sounds")?)?,
        })
    }

    /// `Pig.getSoundSet`: the sound id for `kind` (`ambient`, `hurt`, `death`, `step`, `eat`).
    pub fn sound(&self, baby: bool, kind: &str) -> Option<&str> {
        let set = if baby {
            &self.baby_sounds
        } else {
            &self.adult_sounds
        };
        set.by_kind(kind)
    }
}

/// `CowVariant.ModelType`: `normal`, `cold`, or `warm`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CowModelType {
    #[default]
    Normal,
    Cold,
    Warm,
}

impl CowModelType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Cold => "cold",
            Self::Warm => "warm",
        }
    }

    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "normal" => Ok(Self::Normal),
            "cold" => Ok(Self::Cold),
            "warm" => Ok(Self::Warm),
            other => Err(format!("model: unknown cow model type {other:?}")),
        }
    }

    fn decode(obj: &serde_json::Map<String, Value>) -> Result<Self, String> {
        match obj.get("model") {
            None => Ok(Self::Normal),
            Some(v) => Self::parse(v.as_str().ok_or("model: a string is required")?),
        }
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        match tag.get("model") {
            None => Some(Self::Normal),
            Some(Tag::String(s)) => Self::parse(&s.to_string_lossy()).ok(),
            _ => None,
        }
    }
}

/// `CowVariant`: model type, adult/baby texture asset ids, and spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct CowVariant {
    pub model: CowModelType,
    pub asset_id: String,
    pub baby_asset_id: String,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for CowVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl CowVariant {
    /// `CowVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let model = CowModelType::decode(obj)?;
        let asset_id = decode_asset_field(obj, "asset_id")?;
        let baby_asset_id = decode_asset_field(obj, "baby_asset_id")?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            model,
            asset_id,
            baby_asset_id,
            spawn_conditions,
        })
    }

    /// `CowVariant.NETWORK_CODEC`: `model`, `asset_id`, and `baby_asset_id`.
    pub fn to_network_nbt(&self) -> Tag {
        Tag::compound([
            ("model", Tag::String(JavaString::from(self.model.as_str()))),
            (
                "asset_id",
                Tag::String(JavaString::from(self.asset_id.as_str())),
            ),
            (
                "baby_asset_id",
                Tag::String(JavaString::from(self.baby_asset_id.as_str())),
            ),
        ])
    }

    /// Decode a client's synced `CowVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            model: CowModelType::decode_nbt(tag)?,
            asset_id: read_nbt_string(tag, "asset_id")?,
            baby_asset_id: read_nbt_string(tag, "baby_asset_id")?,
            spawn_conditions: Vec::new(),
        })
    }

    /// `CowRenderer.getTextureLocation`.
    pub fn texture(&self, baby: bool) -> &str {
        let asset = if baby {
            &self.baby_asset_id
        } else {
            &self.asset_id
        };
        strip_minecraft_prefix(asset)
    }

    /// `CowRenderer.bakeModels` / `submit`.
    pub fn model_key(&self, baby: bool) -> &'static str {
        match (self.model, baby) {
            (CowModelType::Warm, false) => "warm_cow#main",
            (CowModelType::Warm, true) => "warm_cow_baby#main",
            (CowModelType::Cold, false) => "cold_cow#main",
            (CowModelType::Cold, true) => "cold_cow_baby#main",
            (CowModelType::Normal, false) => "cow#main",
            (CowModelType::Normal, true) => "cow_baby#main",
        }
    }
}

/// `CowSoundVariant`: flat set of four sound event ids (no adult/baby split in 26.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CowSoundVariant {
    pub ambient: String,
    pub hurt: String,
    pub death: String,
    pub step: String,
}

impl CowSoundVariant {
    /// `CowSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
    pub fn decode_with(
        value: &Value,
        mut decode_sound: impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value.as_object().ok_or("not a map")?;
        let mut wire = Compound::new();
        let mut read = |key: &str| -> Result<String, String> {
            let raw = obj.get(key).ok_or_else(|| format!("{key} is required"))?;
            let (id, tag) = decode_sound(key, raw)?;
            wire.insert(key.into(), tag);
            Ok(id)
        };
        let variant = Self {
            ambient: read("ambient_sound")?,
            hurt: read("hurt_sound")?,
            death: read("death_sound")?,
            step: read("step_sound")?,
        };
        Ok((variant, Tag::Compound(wire)))
    }

    /// Decode a client's synced `CowSoundVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            ambient: read_nbt_sound(tag, "ambient_sound")?,
            hurt: read_nbt_sound(tag, "hurt_sound")?,
            death: read_nbt_sound(tag, "death_sound")?,
            step: read_nbt_sound(tag, "step_sound")?,
        })
    }

    /// `Cow.getSoundSet`: the sound id for `kind` (`ambient`, `hurt`, `death`, `step`).
    pub fn sound(&self, kind: &str) -> Option<&str> {
        let sound = match kind {
            "ambient" => &self.ambient,
            "hurt" => &self.hurt,
            "death" => &self.death,
            "step" => &self.step,
            _ => return None,
        };
        (!sound.is_empty()).then_some(sound.as_str())
    }
}

/// `ChickenVariant.ModelType`: `normal` or `cold`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChickenModelType {
    #[default]
    Normal,
    Cold,
}

impl ChickenModelType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Cold => "cold",
        }
    }

    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "normal" => Ok(Self::Normal),
            "cold" => Ok(Self::Cold),
            other => Err(format!("model: unknown chicken model type {other:?}")),
        }
    }

    fn decode(obj: &serde_json::Map<String, Value>) -> Result<Self, String> {
        match obj.get("model") {
            None => Ok(Self::Normal),
            Some(v) => Self::parse(v.as_str().ok_or("model: a string is required")?),
        }
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        match tag.get("model") {
            None => Some(Self::Normal),
            Some(Tag::String(s)) => Self::parse(&s.to_string_lossy()).ok(),
            _ => None,
        }
    }
}

/// `ChickenVariant`: model type, adult/baby texture asset ids, and spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct ChickenVariant {
    pub model: ChickenModelType,
    pub asset_id: String,
    pub baby_asset_id: String,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for ChickenVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl ChickenVariant {
    /// `ChickenVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let model = ChickenModelType::decode(obj)?;
        let asset_id = decode_asset_field(obj, "asset_id")?;
        let baby_asset_id = decode_asset_field(obj, "baby_asset_id")?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            model,
            asset_id,
            baby_asset_id,
            spawn_conditions,
        })
    }

    /// `ChickenVariant.NETWORK_CODEC`: `model`, `asset_id`, and `baby_asset_id`.
    pub fn to_network_nbt(&self) -> Tag {
        Tag::compound([
            ("model", Tag::String(JavaString::from(self.model.as_str()))),
            (
                "asset_id",
                Tag::String(JavaString::from(self.asset_id.as_str())),
            ),
            (
                "baby_asset_id",
                Tag::String(JavaString::from(self.baby_asset_id.as_str())),
            ),
        ])
    }

    /// Decode a client's synced `ChickenVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            model: ChickenModelType::decode_nbt(tag)?,
            asset_id: read_nbt_string(tag, "asset_id")?,
            baby_asset_id: read_nbt_string(tag, "baby_asset_id")?,
            spawn_conditions: Vec::new(),
        })
    }

    /// `ChickenRenderer.getTextureLocation`.
    pub fn texture(&self, baby: bool) -> &str {
        let asset = if baby {
            &self.baby_asset_id
        } else {
            &self.asset_id
        };
        strip_minecraft_prefix(asset)
    }

    /// `ChickenRenderer.bakeModels` / `submit`.
    pub fn model_key(&self, baby: bool) -> &'static str {
        if baby {
            "chicken_baby#main"
        } else if self.model == ChickenModelType::Cold {
            "cold_chicken#main"
        } else {
            "chicken#main"
        }
    }
}

/// `ChickenSoundVariant.ChickenSoundSet`: the four sound event ids of an age set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChickenSoundSet {
    pub ambient: String,
    pub hurt: String,
    pub death: String,
    pub step: String,
}

impl ChickenSoundSet {
    fn decode_with(
        field: &str,
        value: &Value,
        decode_sound: &mut impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value
            .as_object()
            .ok_or_else(|| format!("{field}: not a map"))?;
        let mut wire = Compound::new();
        let mut read = |key: &str| -> Result<String, String> {
            let path = format!("{field}.{key}");
            let raw = obj.get(key).ok_or_else(|| format!("{path} is required"))?;
            let (id, tag) = decode_sound(&path, raw)?;
            wire.insert(key.into(), tag);
            Ok(id)
        };
        let set = Self {
            ambient: read("ambient_sound")?,
            hurt: read("hurt_sound")?,
            death: read("death_sound")?,
            step: read("step_sound")?,
        };
        Ok((set, Tag::Compound(wire)))
    }

    fn decode_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            ambient: read_nbt_sound(tag, "ambient_sound")?,
            hurt: read_nbt_sound(tag, "hurt_sound")?,
            death: read_nbt_sound(tag, "death_sound")?,
            step: read_nbt_sound(tag, "step_sound")?,
        })
    }

    pub fn by_kind(&self, kind: &str) -> Option<&str> {
        let sound = match kind {
            "ambient" => &self.ambient,
            "hurt" => &self.hurt,
            "death" => &self.death,
            "step" => &self.step,
            _ => return None,
        };
        (!sound.is_empty()).then_some(sound.as_str())
    }
}

/// `ChickenSoundVariant`: adult and baby sound sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChickenSoundVariant {
    pub adult_sounds: ChickenSoundSet,
    pub baby_sounds: ChickenSoundSet,
}

impl ChickenSoundVariant {
    /// `ChickenSoundVariant.DIRECT_CODEC` / `NETWORK_CODEC`.
    pub fn decode_with(
        value: &Value,
        mut decode_sound: impl FnMut(&str, &Value) -> Result<(String, Tag), String>,
    ) -> Result<(Self, Tag), String> {
        let obj = value.as_object().ok_or("not a map")?;
        let (adult_sounds, adult_tag) = ChickenSoundSet::decode_with(
            "adult_sounds",
            obj.get("adult_sounds").ok_or("adult_sounds is required")?,
            &mut decode_sound,
        )?;
        let (baby_sounds, baby_tag) = ChickenSoundSet::decode_with(
            "baby_sounds",
            obj.get("baby_sounds").ok_or("baby_sounds is required")?,
            &mut decode_sound,
        )?;
        let wire = Tag::compound([("adult_sounds", adult_tag), ("baby_sounds", baby_tag)]);
        Ok((
            Self {
                adult_sounds,
                baby_sounds,
            },
            wire,
        ))
    }

    /// Decode a client's synced `ChickenSoundVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            adult_sounds: ChickenSoundSet::decode_nbt(tag.get("adult_sounds")?)?,
            baby_sounds: ChickenSoundSet::decode_nbt(tag.get("baby_sounds")?)?,
        })
    }

    /// `Chicken.getSoundSet`: the sound id for `kind` (`ambient`, `hurt`, `death`, `step`).
    pub fn sound(&self, baby: bool, kind: &str) -> Option<&str> {
        let set = if baby {
            &self.baby_sounds
        } else {
            &self.adult_sounds
        };
        set.by_kind(kind)
    }
}

/// `FrogVariant`: texture asset id (`asset_id`) plus spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct FrogVariant {
    pub asset_id: String,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for FrogVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl FrogVariant {
    /// `FrogVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let asset_id = decode_asset_field(obj, "asset_id")?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            asset_id,
            spawn_conditions,
        })
    }

    /// `FrogVariant.NETWORK_CODEC`: `asset_id` only.
    pub fn to_network_nbt(&self) -> Tag {
        Tag::compound([(
            "asset_id",
            Tag::String(JavaString::from(self.asset_id.as_str())),
        )])
    }

    /// Decode a client's synced `FrogVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        Some(Self {
            asset_id: read_nbt_string(tag, "asset_id")?,
            spawn_conditions: Vec::new(),
        })
    }

    /// `FrogRenderer.extractRenderState`: the atlas sprite key for this variant.
    /// Default-namespace assets strip `"minecraft:"` to match Sunrise's atlas keys;
    /// non-default namespaces keep `"<ns>:<path>"`.
    pub fn texture(&self) -> &str {
        strip_minecraft_prefix(&self.asset_id)
    }
}

/// `ZombieNautilusVariant.ModelType`: normal (`zombie_nautilus#main`) or warm
/// (`zombie_nautilus_coral#main`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZombieNautilusModelType {
    #[default]
    Normal,
    Warm,
}

impl ZombieNautilusModelType {
    fn from_name(raw: &str) -> Option<Self> {
        match raw {
            "normal" => Some(Self::Normal),
            "warm" => Some(Self::Warm),
            _ => None,
        }
    }
}

/// `ZombieNautilusVariant`: `ModelAndTexture<ModelType>` plus spawn conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct ZombieNautilusVariant {
    pub model: ZombieNautilusModelType,
    pub asset_id: String,
    pub spawn_conditions: Vec<SpawnSelector>,
}

impl HasSpawnConditions for ZombieNautilusVariant {
    fn spawn_conditions(&self) -> &[SpawnSelector] {
        &self.spawn_conditions
    }
}

impl ZombieNautilusVariant {
    /// `ZombieNautilusVariant.DIRECT_CODEC`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let obj = value.as_object().ok_or("not a map")?;
        let model = match obj.get("model") {
            None => ZombieNautilusModelType::Normal,
            Some(v) => {
                let raw = v.as_str().ok_or("model: a string is required")?;
                ZombieNautilusModelType::from_name(raw)
                    .ok_or_else(|| format!("model: unknown zombie nautilus model {raw:?}"))?
            }
        };
        let asset_id = decode_asset_field(obj, "asset_id")?;
        let spawn_conditions = decode_spawn_conditions(
            obj.get("spawn_conditions")
                .ok_or("spawn_conditions is required")?,
        )?;
        Ok(Self {
            model,
            asset_id,
            spawn_conditions,
        })
    }

    /// `ZombieNautilusVariant.NETWORK_CODEC`: `model` (omitted when default `normal`)
    /// and `asset_id`.
    pub fn to_network_nbt(&self) -> Tag {
        let mut fields = Compound::new();
        if self.model == ZombieNautilusModelType::Warm {
            fields.insert("model".into(), Tag::String(JavaString::from("warm")));
        }
        fields.insert(
            "asset_id".into(),
            Tag::String(JavaString::from(self.asset_id.as_str())),
        );
        Tag::Compound(fields)
    }

    /// Decode a client's synced `ZombieNautilusVariant.NETWORK_CODEC` NBT entry.
    pub fn decode_network_nbt(tag: &Tag) -> Option<Self> {
        let model = match tag.get("model") {
            None => ZombieNautilusModelType::Normal,
            Some(Tag::String(s)) => ZombieNautilusModelType::from_name(&s.to_string_lossy())?,
            _ => return None,
        };
        let asset_id = read_nbt_string(tag, "asset_id")?;
        Some(Self {
            model,
            asset_id,
            spawn_conditions: Vec::new(),
        })
    }

    /// `ZombieNautilusRenderer.getTextureLocation`: the atlas sprite key for this variant.
    /// Default-namespace assets strip `"minecraft:"` to match Sunrise's atlas keys;
    /// non-default namespaces keep `"<ns>:<path>"`.
    pub fn texture(&self) -> &str {
        strip_minecraft_prefix(&self.asset_id)
    }

    /// `ZombieNautilusRenderer`: the model layer key for this variant's `ModelType`.
    pub fn model_key(&self) -> &'static str {
        match self.model {
            ZombieNautilusModelType::Normal => "zombie_nautilus#main",
            ZombieNautilusModelType::Warm => "zombie_nautilus_coral#main",
        }
    }
}

/// The frozen world variant tables converted onto `REGISTRY_KINDS`.
#[derive(Clone, Debug, Default)]
pub struct VariantTables {
    pub wolf: VariantTable<WolfVariant>,
    pub wolf_sound: VariantTable<WolfSoundVariant>,
    pub cat: VariantTable<CatVariant>,
    pub cat_sound: VariantTable<CatSoundVariant>,
    pub pig: VariantTable<PigVariant>,
    pub pig_sound: VariantTable<PigSoundVariant>,
    pub cow: VariantTable<CowVariant>,
    pub cow_sound: VariantTable<CowSoundVariant>,
    pub chicken: VariantTable<ChickenVariant>,
    pub chicken_sound: VariantTable<ChickenSoundVariant>,
    pub frog: VariantTable<FrogVariant>,
    pub zombie_nautilus: VariantTable<ZombieNautilusVariant>,
}

impl VariantTables {
    /// Whether no variant table has been populated yet.
    pub fn is_empty(&self) -> bool {
        self.wolf.is_empty()
            && self.wolf_sound.is_empty()
            && self.cat.is_empty()
            && self.cat_sound.is_empty()
            && self.pig.is_empty()
            && self.pig_sound.is_empty()
            && self.cow.is_empty()
            && self.cow_sound.is_empty()
            && self.chicken.is_empty()
            && self.chicken_sound.is_empty()
            && self.frog.is_empty()
            && self.zombie_nautilus.is_empty()
    }

    /// Load the prepared vanilla tables (`data/minecraft/<registry>/`) for worlds
    /// without packs, worldgen helpers and standalone probes.
    pub fn load_prepared(root: &Path) -> Self {
        let data = root.join("data/minecraft");
        Self {
            wolf: load_prepared_table(&data.join("wolf_variant"), WolfVariant::decode),
            wolf_sound: load_prepared_table(&data.join("wolf_sound_variant"), |v| {
                WolfSoundVariant::decode_with(v, decode_prepared_sound_holder).map(|(d, _)| d)
            }),
            cat: load_prepared_table(&data.join("cat_variant"), CatVariant::decode),
            cat_sound: load_prepared_table(&data.join("cat_sound_variant"), |v| {
                CatSoundVariant::decode_with(v, decode_prepared_sound_holder).map(|(d, _)| d)
            }),
            pig: load_prepared_table(&data.join("pig_variant"), PigVariant::decode),
            pig_sound: load_prepared_table(&data.join("pig_sound_variant"), |v| {
                PigSoundVariant::decode_with(v, decode_prepared_sound_holder).map(|(d, _)| d)
            }),
            cow: load_prepared_table(&data.join("cow_variant"), CowVariant::decode),
            cow_sound: load_prepared_table(&data.join("cow_sound_variant"), |v| {
                CowSoundVariant::decode_with(v, decode_prepared_sound_holder).map(|(d, _)| d)
            }),
            chicken: load_prepared_table(&data.join("chicken_variant"), ChickenVariant::decode),
            chicken_sound: load_prepared_table(&data.join("chicken_sound_variant"), |v| {
                ChickenSoundVariant::decode_with(v, decode_prepared_sound_holder).map(|(d, _)| d)
            }),
            frog: load_prepared_table(&data.join("frog_variant"), FrogVariant::decode),
            zombie_nautilus: load_prepared_table(
                &data.join("zombie_nautilus_variant"),
                ZombieNautilusVariant::decode,
            ),
        }
    }

    /// Build a client's variant tables from the server's synchronized `Dynamic`
    /// registries, keeping the server's exact network id order and resolving
    /// known-pack omitted entries from `fallback`. An entry that does not decode, or
    /// that has no data and is not in `fallback`, refuses the whole load
    /// (`RegistryDataLoader.load`).
    pub fn from_synced(registries: &Dynamic, fallback: &VariantTables) -> Result<Self, String> {
        Ok(Self {
            wolf: from_synced_table(
                registries,
                "minecraft:wolf_variant",
                &fallback.wolf,
                WolfVariant::decode_network_nbt,
            )?,
            wolf_sound: from_synced_table(
                registries,
                "minecraft:wolf_sound_variant",
                &fallback.wolf_sound,
                WolfSoundVariant::decode_network_nbt,
            )?,
            cat: from_synced_table(
                registries,
                "minecraft:cat_variant",
                &fallback.cat,
                CatVariant::decode_network_nbt,
            )?,
            cat_sound: from_synced_table(
                registries,
                "minecraft:cat_sound_variant",
                &fallback.cat_sound,
                CatSoundVariant::decode_network_nbt,
            )?,
            pig: from_synced_table(
                registries,
                "minecraft:pig_variant",
                &fallback.pig,
                PigVariant::decode_network_nbt,
            )?,
            pig_sound: from_synced_table(
                registries,
                "minecraft:pig_sound_variant",
                &fallback.pig_sound,
                PigSoundVariant::decode_network_nbt,
            )?,
            cow: from_synced_table(
                registries,
                "minecraft:cow_variant",
                &fallback.cow,
                CowVariant::decode_network_nbt,
            )?,
            cow_sound: from_synced_table(
                registries,
                "minecraft:cow_sound_variant",
                &fallback.cow_sound,
                CowSoundVariant::decode_network_nbt,
            )?,
            chicken: from_synced_table(
                registries,
                "minecraft:chicken_variant",
                &fallback.chicken,
                ChickenVariant::decode_network_nbt,
            )?,
            chicken_sound: from_synced_table(
                registries,
                "minecraft:chicken_sound_variant",
                &fallback.chicken_sound,
                ChickenSoundVariant::decode_network_nbt,
            )?,
            frog: from_synced_table(
                registries,
                "minecraft:frog_variant",
                &fallback.frog,
                FrogVariant::decode_network_nbt,
            )?,
            zombie_nautilus: from_synced_table(
                registries,
                "minecraft:zombie_nautilus_variant",
                &fallback.zombie_nautilus,
                ZombieNautilusVariant::decode_network_nbt,
            )?,
        })
    }
}

fn from_synced_table<T: Clone>(
    registries: &Dynamic,
    registry: &str,
    fallback: &VariantTable<T>,
    decode_nbt: impl Fn(&Tag) -> Option<T>,
) -> Result<VariantTable<T>, String> {
    let Some(entries) = registries.registries.get(registry) else {
        return Ok(VariantTable::default());
    };
    let entries = entries
        .iter()
        .map(|(id, data)| {
            let entry = match data {
                Some(data) => decode_nbt(data)
                    .ok_or_else(|| format!("{registry}/{id}: the entry does not decode"))?,
                None => fallback
                    .lookup(id)
                    .and_then(|r| fallback.get(r))
                    .cloned()
                    .ok_or_else(|| format!("{registry}/{id}: not in the client's known pack"))?,
            };
            Ok((id.clone(), entry))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(VariantTable::from_synced(entries))
}

fn load_prepared_table<T>(
    dir: &Path,
    decode: impl Fn(&Value) -> Result<T, String>,
) -> VariantTable<T> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.strip_suffix(".json"))
            .map(str::to_owned)
        else {
            continue;
        };
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let Ok(decoded) = decode(&value) {
            entries.push((("minecraft".to_owned(), name), decoded));
        }
    }
    VariantTable::from_entries(entries)
}

fn decode_prepared_sound_holder(field: &str, raw: &Value) -> Result<(String, Tag), String> {
    match raw {
        Value::String(s) => {
            let id = Identifier::parse(s)
                .map_err(|e| format!("{field}: invalid sound identifier {s:?}: {e}"))?;
            Ok((
                id.as_str().to_owned(),
                Tag::String(JavaString::from(id.as_str())),
            ))
        }
        Value::Object(map) => {
            let raw_id = map
                .get("sound_id")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{field}.sound_id: a string is required"))?;
            let id = Identifier::parse(raw_id)
                .map_err(|e| format!("{field}.sound_id: invalid identifier {raw_id:?}: {e}"))?;
            Ok((
                id.as_str().to_owned(),
                Tag::String(JavaString::from(id.as_str())),
            ))
        }
        _ => Err(format!("{field}: a sound event is required")),
    }
}

#[cfg(test)]
#[path = "wolf_variant_tests.rs"]
mod wolf_variant_tests;

#[cfg(test)]
#[path = "cat_variant_tests.rs"]
mod cat_variant_tests;

#[cfg(test)]
#[path = "farm_variant_tests.rs"]
mod farm_variant_tests;

#[cfg(test)]
#[path = "frog_variant_tests.rs"]
mod frog_variant_tests;

#[cfg(test)]
#[path = "zombie_nautilus_variant_tests.rs"]
mod zombie_nautilus_variant_tests;
