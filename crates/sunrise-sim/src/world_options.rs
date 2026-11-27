//! Portable Create World options, difficulty list, Superflat settings, and world-preset selection state.
use crate::{lang, save::valid_world_gen_dimensions as valid_dimensions};
use std::{collections::BTreeSet, path::Path};
use sunrise_core::nbt::Tag;
pub use sunrise_server_files::properties::DIFFICULTIES;

/// DimensionType.Y_SIZE: the most blocks a flat generator takes.
const Y_SIZE: i32 = 4064;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatLayer {
    /// A block id such as `minecraft:dirt`.
    pub block: String,
    /// How many blocks tall this layer is (0..=4064).
    pub height: i32,
}

/// FlatLevelGeneratorSettings: the layers (bottom to top), the biome and the
/// decorations CreateFlatWorldScreen edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatSettings {
    pub biome: String,
    pub features: bool,
    pub lakes: bool,
    pub layers: Vec<FlatLayer>,
    pub structure_overrides: Vec<String>,
}
impl Default for FlatSettings {
    fn default() -> Self {
        // worldgen/world_preset/flat.json's overworld.
        Self {
            biome: "minecraft:plains".into(),
            features: false,
            lakes: false,
            layers: vec![
                FlatLayer {
                    block: "minecraft:bedrock".into(),
                    height: 1,
                },
                FlatLayer {
                    block: "minecraft:dirt".into(),
                    height: 2,
                },
                FlatLayer {
                    block: "minecraft:grass_block".into(),
                    height: 1,
                },
            ],
            structure_overrides: vec!["minecraft:strongholds".into(), "minecraft:villages".into()],
        }
    }
}
impl FlatSettings {
    /// A saved world's flat generator settings, for Re-Create.
    pub fn from_world(dir: &Path) -> Option<Self> {
        let settings = crate::save::world_gen_settings(dir)?;
        let generator = settings
            .get("dimensions")?
            .get("minecraft:overworld")?
            .get("generator")?;
        if crate::save::text(generator.get("type"))?.trim_start_matches("minecraft:") != "flat" {
            return None;
        }
        Self::from_tag(generator.get("settings")?)
    }
    pub fn from_tag(tag: &Tag) -> Option<Self> {
        let Some(Tag::List(list)) = tag.get("layers") else {
            return None;
        };
        let layers: Vec<FlatLayer> = list
            .iter()
            .filter_map(|layer| {
                Some(FlatLayer {
                    block: crate::save::text(layer.get("block"))?,
                    height: crate::save::number(layer.get("height"))?.clamp(0, Y_SIZE as i64)
                        as i32,
                })
            })
            .collect();
        // `layers: []` is a valid void flat world; a list whose entries all
        // fail to parse is not.
        if layers.is_empty() && !list.is_empty() {
            return None;
        }
        let flag = |key: &str, default: bool| {
            crate::save::number(tag.get(key)).map_or(default, |value| value != 0)
        };
        let structure_overrides = match tag.get("structure_overrides") {
            Some(Tag::String(text)) => vec![text.to_string_lossy()],
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|value| crate::save::text(Some(value)))
                .collect(),
            _ => Vec::new(),
        };
        Some(Self {
            biome: crate::save::text(tag.get("biome")).unwrap_or_else(|| "minecraft:plains".into()),
            features: flag("features", false),
            lakes: flag("lakes", false),
            layers,
            structure_overrides,
        })
    }
    /// FlatLevelGeneratorSettings.CODEC, as world_gen_settings stores it.
    pub fn to_tag(&self) -> Tag {
        let string = |text: &str| Tag::String(text.into());
        Tag::compound([
            ("biome", string(&self.biome)),
            ("features", Tag::Byte(i8::from(self.features))),
            ("lakes", Tag::Byte(i8::from(self.lakes))),
            (
                "layers",
                Tag::List(
                    self.layers
                        .iter()
                        .map(|layer| {
                            Tag::compound([
                                ("block", string(&layer.block)),
                                ("height", Tag::Int(layer.height)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "structure_overrides",
                Tag::List(
                    self.structure_overrides
                        .iter()
                        .map(|id| string(id))
                        .collect(),
                ),
            ),
        ])
    }
    /// `CreateFlatWorldScreen.save`: the text the preset screen shows.
    pub fn to_share(&self) -> String {
        let layers = self
            .layers
            .iter()
            .map(|layer| {
                if layer.height == 1 {
                    layer.block.clone()
                } else {
                    format!("{}*{}", layer.height, layer.block)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("{layers};{}", self.biome)
    }
    /// `PresetFlatWorldScreen.fromString`: `layers;biome`, keeping the
    /// decorations the screen was opened with. `None` for a malformed list,
    /// as vanilla falls back to the preset's own settings.
    pub fn parse(
        text: &str,
        blocks: &BTreeSet<String>,
        biomes: &BTreeSet<String>,
        base: &Self,
    ) -> Option<Self> {
        let (layer_text, biome_text) = text.split_once(';').unwrap_or((text, ""));
        let mut layers = Vec::new();
        let mut total = 0;
        for part in layer_text.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (height, block) = match part.split_once('*') {
                Some((height, block)) => (height.trim().parse::<i32>().ok()?, block.trim()),
                None => (1, part),
            };
            let block = namespaced(block);
            if !blocks.is_empty() && !blocks.contains(&block) {
                return None;
            }
            let height = height.max(0).min(Y_SIZE - total);
            if height == 0 {
                continue;
            }
            total += height;
            layers.push(FlatLayer { block, height });
        }
        if layers.is_empty() {
            return None;
        }
        let mut biome = namespaced(biome_text.trim());
        if biome == "minecraft:" || biome.is_empty() {
            biome = "minecraft:plains".into();
        }
        if !biomes.is_empty() && !biomes.contains(&biome) {
            biome = "minecraft:plains".into();
        }
        Some(Self {
            biome,
            layers,
            ..base.clone()
        })
    }
}
/// `Identifier.parse`: bare names get the default namespace.
fn namespaced(id: &str) -> String {
    if id.is_empty() || id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// WorldCreationUiState beyond the name and seed.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldOptions {
    /// Selected data-pack preset; none selects `world_type`.
    pub custom_preset: Option<String>,
    /// Committed editor output, saved as the world's dimension stems.
    pub preset_dimensions: Option<sunrise_core::nbt::Tag>,
    /// Available enabled data-pack presets, refreshed when the pack selection changes.
    pub presets: std::collections::BTreeMap<String, sunrise_core::nbt::Tag>,
    /// The enabled `minecraft:normal` world-preset tag's selection order.
    pub preset_order: Vec<String>,
    /// The open tab: Game, World or More.
    pub tab: usize,
    /// Survival locked to Hard, one life (`SelectedGameMode.HARDCORE`).
    pub hardcore: bool,
    /// Index in `DIFFICULTIES`.
    pub difficulty: usize,
    /// Allow Cheats as toggled; unset follows the game mode.
    pub commands: Option<bool>,
    /// The world preset the world is created with.
    pub world_type: crate::save::WorldPreset,
    /// The biome the single-biome preset fixes (`CreateBuffetWorldScreen`'s
    /// choice), `minecraft:plains` until another is picked.
    pub single_biome: String,
    /// CreateFlatWorldScreen's layers and biome, kept with the world type.
    pub flat_settings: Option<FlatSettings>,
    pub structures: bool,
    pub bonus_chest: bool,
    /// `getGameRules`: the Game Rules screen's values, saved with the world.
    pub rules: crate::gamerules::GameRules,
    /// The Data Packs screen's selection, lowest priority first.
    pub data_packs: Vec<String>,
    /// The folder of packs for the new world (vanilla's `mcworld-`
    /// temporary folder), made when Data Packs first opens.
    pub pack_dir: Option<std::path::PathBuf>,
    /// Additional repository sources retained for this creation configuration.
    pub additional_data_packs: std::sync::Arc<[std::sync::Arc<crate::datapack::DataPack>]>,
    /// Alt is held (`CycleButton.DEFAULT_ALT_LIST_SELECTOR`): the world type
    /// button cycles the extended list, with Debug Mode.
    pub alt_down: bool,
}
impl Default for WorldOptions {
    fn default() -> Self {
        Self {
            custom_preset: None,
            preset_dimensions: None,
            presets: Default::default(),
            preset_order: crate::save::WorldPreset::ALL
                .iter()
                .map(|p| p.id().into())
                .collect(),
            tab: 0,
            hardcore: false,
            difficulty: 2,
            commands: None,
            world_type: crate::save::WorldPreset::Normal,
            single_biome: "minecraft:plains".into(),
            flat_settings: None,
            structures: true,
            bonus_chest: false,
            rules: crate::gamerules::GameRules::default(),
            data_packs: vec!["vanilla".into()],
            pack_dir: None,
            additional_data_packs: std::sync::Arc::from([]),
            alt_down: false,
        }
    }
}
impl WorldOptions {
    /// Refresh additional sources and insert required packs while preserving
    /// the chosen ids, including folder packs already copied to the new world.
    pub fn set_additional_data_packs(
        &mut self,
        packs: std::sync::Arc<[std::sync::Arc<crate::datapack::DataPack>]>,
    ) {
        let mut available: Vec<_> = packs
            .iter()
            .filter_map(|pack| pack.selection.as_ref())
            .map(crate::pack_meta::PackInfo::selectable)
            .collect();
        for id in &self.data_packs {
            if !available.iter().any(|pack| pack.id == *id) {
                available.push(crate::pack_meta::Selectable::optional(id, true));
            }
        }
        self.data_packs = crate::pack_meta::SelectionModel::new(available, &self.data_packs)
            .selected
            .into_iter()
            .rev()
            .map(|pack| pack.id)
            .collect();
        self.additional_data_packs = packs;
    }
    /// A fresh Create New World in the game folder (the working directory): the defaults, with
    /// the packs this game folder gives every new world.
    pub fn for_new_world() -> Self {
        let seeded = std::env::current_dir()
            .ok()
            .and_then(|dir| crate::datapack::seed_create_world(&dir));
        let mut options = Self::default();
        if let Some((dir, ids)) = seeded {
            options.pack_dir = Some(dir);
            options.data_packs.extend(ids);
        }
        options
    }
    /// Re-Create: the saved world's choices.
    pub fn of_world(
        level: &crate::save::Level,
        commands: bool,
        dir: &std::path::Path,
        registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    ) -> Self {
        let generation = crate::save::WorldGen::read(dir).unwrap_or_default();
        let (pack_dir, data_packs) = crate::datapack::for_recreate(dir);
        Self {
            hardcore: level.hardcore,
            difficulty: DIFFICULTIES
                .iter()
                .position(|d| *d == level.difficulty)
                .unwrap_or(2),
            commands: Some(commands),
            world_type: generation.preset,
            single_biome: fixed_biome(dir).unwrap_or_else(|| "minecraft:plains".into()),
            flat_settings: (generation.preset == crate::save::WorldPreset::Flat)
                .then(|| FlatSettings::from_world(dir))
                .flatten(),
            structures: generation.structures,
            bonus_chest: generation.bonus_chest,
            rules: crate::gamerules::GameRules::load_with_registry(
                dir,
                &level.enabled_features,
                registry,
            ),
            data_packs,
            pack_dir,
            ..Self::default()
        }
    }
    /// Re-Create's repository enables the original world's feature packs.
    pub fn restore_feature_packs(
        &mut self,
        resources: &std::path::Path,
        level: &crate::save::Level,
    ) {
        let packs = self.available_packs(resources);
        if let Some((enabled, disabled)) = &level.data_packs {
            let mut selected: Vec<_> = enabled
                .iter()
                .filter(|id| id.as_str() == "vanilla" || packs.iter().any(|p| &p.id == *id))
                .cloned()
                .collect();
            for id in &self.data_packs {
                if !selected.contains(id) && !disabled.contains(id) {
                    selected.push(id.clone());
                }
            }
            self.data_packs = selected;
        }
        let provided: Vec<_> = packs
            .iter()
            .filter(|p| self.data_packs.contains(&p.id))
            .flat_map(|p| p.features())
            .collect();
        let mut missing: Vec<_> = level
            .enabled_features
            .iter()
            .filter(|f| f.as_str() != "minecraft:vanilla" && !provided.contains(f))
            .cloned()
            .collect();
        for pack in packs.iter().filter(|p| !p.id.starts_with("file/")) {
            let required = pack.features();
            if required.iter().any(|f| missing.contains(f))
                && required.iter().all(|f| level.enabled_features.contains(f))
                && !self.data_packs.contains(&pack.id)
            {
                self.data_packs.push(pack.id.clone());
                missing.retain(|f| !required.contains(f));
            }
        }
    }
    pub fn available_packs(&self, resources: &std::path::Path) -> Vec<crate::datapack::DataPack> {
        let mut packs = crate::datapack::feature_packs(resources);
        packs.extend(
            self.additional_data_packs
                .iter()
                .map(|pack| (**pack).clone()),
        );
        if let Some(dir) = &self.pack_dir {
            packs.extend(crate::datapack::discover_folder(dir));
        }
        packs
    }
    /// WorldDataConfiguration's selected feature packs over vanilla's flags.
    pub fn enabled_features(&self, resources: &std::path::Path) -> Vec<String> {
        let mut features = vec!["minecraft:vanilla".to_owned()];
        features.extend(
            self.available_packs(resources)
                .iter()
                .filter(|pack| self.data_packs.contains(&pack.id))
                .flat_map(|pack| pack.features()),
        );
        features.sort();
        features.dedup();
        features
    }
    /// `getDifficulty`: Hardcore is always Hard.
    pub fn difficulty(&self) -> usize {
        if self.hardcore { 3 } else { self.difficulty }
    }
    /// `isAllowCommands`: never in Hardcore, else on for Creative until toggled.
    pub fn commands(&self, creative: bool) -> bool {
        !self.hardcore && self.commands.unwrap_or(creative)
    }
    /// `isBonusChest`: never in Hardcore.
    pub fn bonus_chest(&self) -> bool {
        !self.hardcore && self.bonus_chest
    }
    pub fn world_gen(&self) -> crate::save::WorldGen {
        crate::save::WorldGen {
            preset: self.world_type,
            structures: self.structures,
            bonus_chest: self.bonus_chest(),
        }
    }
    /// The biome the single-biome preset fixes, as
    /// `write_world_gen_settings_with` takes it; none for another preset,
    /// which has no fixed biome to honour.
    pub fn single_biome(&self) -> Option<&str> {
        (self.world_type == crate::save::WorldPreset::SingleBiomeSurface)
            .then_some(self.single_biome.as_str())
    }
    /// The custom layers, as `write_world_gen_settings_with` takes them.
    pub fn flat_settings(&self) -> Option<sunrise_core::nbt::Tag> {
        self.flat_settings.as_ref().map(FlatSettings::to_tag)
    }
    /// A press on one of the open tab's cycle buttons or switches.
    pub fn press(&mut self, id: u8, creative: &mut bool) {
        self.cycle(id, creative, 1);
    }
    /// A step on one of the open tab's cycle buttons or switches (mouse wheel or keyboard).
    pub fn cycle(&mut self, id: u8, creative: &mut bool, step: i32) {
        match id {
            // Survival, Hardcore, Creative.
            20 => {
                let modes = [(false, false), (false, true), (true, false)];
                let cur = (*creative, self.hardcore);
                let idx = modes.iter().position(|&m| m == cur).unwrap_or(0);
                let next_idx = ((idx as i32 + step).rem_euclid(modes.len() as i32)) as usize;
                (*creative, self.hardcore) = modes[next_idx];
            }
            23 => {
                let len = DIFFICULTIES.len() as i32;
                self.difficulty = ((self.difficulty as i32 + step).rem_euclid(len)) as usize;
            }
            24 => self.commands = Some(!self.commands(*creative)),
            25 => {
                self.cycle_preset(step);
                if self.world_type == crate::save::WorldPreset::Flat {
                    // setWorldType starts the generator from the flat preset.
                    self.flat_settings = Some(FlatSettings::default());
                }
                // ...and its dimensions replace the ones the preset before it
                // had, so a fixed biome the player chose starts over at the
                // single-biome preset's own.
                self.single_biome = "minecraft:plains".into();
            }
            27 => self.structures = !self.structures,
            28 => self.bonus_chest = !self.bonus_chest,
            _ => {}
        }
    }
}

/// The biome a saved world's fixed biome source names
/// (`CreateBuffetWorldScreen`'s initial selection).
fn fixed_biome(dir: &std::path::Path) -> Option<String> {
    let settings = crate::save::world_gen_settings(dir)?;
    let generator = settings
        .get("dimensions")?
        .get("minecraft:overworld")?
        .get("generator")?;
    crate::save::text(generator.get("biome_source")?.get("biome"))
}

pub fn preset_label(id: &str) -> String {
    lang::get(&format!("generator.{}", id.replace([':', '/'], ".")))
}

impl WorldOptions {
    /// Selected registry key, including data-pack presets outside the vanilla enum.
    pub fn preset_id(&self) -> &str {
        self.custom_preset
            .as_deref()
            .unwrap_or(self.world_type.id())
    }
    pub fn preset_label(&self) -> String {
        preset_label(self.preset_id())
    }
    /// The worldgen files of the selected packs, the mod data layer below them: what the Create
    /// World screen's preset lists read (`RegistryDataLoader` over the enabled packs).
    pub fn selected_overrides(
        &self,
        root: &Path,
        mod_data: Option<&crate::datapack::DataPack>,
    ) -> sunrise_world::worldgen::overrides::WorldgenOverrides {
        let packs = self.available_packs(root);
        let mut selected: Vec<_> = self
            .data_packs
            .iter()
            .filter_map(|id| packs.iter().find(|pack| pack.id == *id))
            .collect();
        if let Some(mod_data) = mod_data {
            selected.insert(0, mod_data);
        }
        let data = crate::datapack::DataPacks::new(&selected);
        crate::datapack::worldgen_overrides_over(
            &[],
            &data,
            &sunrise_world::conditions::LoadConditions::vanilla(),
        )
        .0
    }
    /// Reload Create World's selected pack resources, retaining a still-available choice.
    pub fn refresh_presets(&mut self, root: &Path, mod_data: Option<&crate::datapack::DataPack>) {
        use sunrise_world::worldgen::overrides::WORLD_PRESET_TAGS;
        let overrides = self.selected_overrides(root, mod_data);
        self.presets.clear();
        for (id, json) in overrides.entries("world_presets") {
            if !sunrise_core::level_id::valid_level_id(id) {
                continue;
            }
            let Ok(json) = serde_json::from_str::<serde_json::Value>(json) else {
                continue;
            };
            let dimensions = crate::save::json_tag(&json["dimensions"]);
            if valid_dimensions(&dimensions) {
                self.presets.insert(id.to_owned(), dimensions);
            }
        }
        let mut tags = sunrise_world::worldgen::tags::TagDefinitions::default();
        let defaults: Vec<_> = crate::save::WorldPreset::ALL
            .iter()
            .map(|p| p.id())
            .collect();
        tags.apply_json(
            "minecraft:normal",
            "vanilla",
            &serde_json::json!({"values": defaults}).to_string(),
        )
        .unwrap();
        for (id, pack, json) in overrides.tags(WORLD_PRESET_TAGS) {
            if let Err(error) = tags.apply_json(id, pack, json) {
                log::warn!("Invalid world preset tag {id} of {pack}: {error}");
            }
        }
        let all_presets = defaults
            .iter()
            .copied()
            .chain([crate::save::WorldPreset::Debug.id()])
            .map(str::to_owned)
            .chain(
                self.presets
                    .keys()
                    .filter(|id| crate::save::WorldPreset::of_id(id).is_none())
                    .cloned(),
            )
            .collect();
        self.preset_order = match tags.build(|id, _| {
            (crate::save::WorldPreset::of_id(id).is_some() || self.presets.contains_key(id))
                .then(|| id.to_owned())
        }) {
            Ok(mut built) => built
                .tags
                .remove("minecraft:normal")
                .filter(|ids| !ids.is_empty())
                .unwrap_or(all_presets),
            Err(error) => {
                log::warn!("Invalid world preset tags: {error}");
                all_presets
            }
        };
        // Re-Create retains Debug even when the normal tag hides it.
        if self.preset_id() != crate::save::WorldPreset::Debug.id()
            && !self.preset_order.iter().any(|id| id == self.preset_id())
        {
            self.custom_preset = self.preset_order.first().cloned();
            if let Some(preset) = self
                .custom_preset
                .as_deref()
                .and_then(crate::save::WorldPreset::of_id)
            {
                self.world_type = preset;
                self.custom_preset = None;
            }
            self.preset_dimensions = None;
        }
    }
    pub fn cycle_preset(&mut self, step: i32) {
        if self.alt_down {
            self.world_type = self.world_type.step(step, true);
            self.custom_preset = None;
            self.preset_dimensions = None;
            return;
        }
        if self.preset_order.is_empty() {
            return;
        }
        let id = self.cycled_preset(step).to_owned();
        self.custom_preset = if let Some(preset) = crate::save::WorldPreset::of_id(&id) {
            self.world_type = preset;
            None
        } else {
            Some(id)
        };
        self.preset_dimensions = None;
    }
    pub fn cycled_preset(&self, step: i32) -> &str {
        if self.preset_order.is_empty() {
            return self.preset_id();
        }
        let current = self
            .preset_order
            .iter()
            .position(|id| *id == self.preset_id())
            .unwrap_or(0);
        let next =
            ((current as i32 + step.signum()).rem_euclid(self.preset_order.len() as i32)) as usize;
        &self.preset_order[next]
    }
    /// Dimensions chosen by a data-pack preset or committed by its editor.
    pub fn dimensions(&self) -> Option<Tag> {
        self.preset_dimensions
            .clone()
            .or_else(|| self.presets.get(self.preset_id()).cloned())
    }
    pub fn editor_dimensions(&self) -> Option<Tag> {
        self.dimensions().or_else(|| {
            crate::save::new_world_gen_settings(
                0,
                self.world_gen(),
                self.flat_settings(),
                self.single_biome(),
            )
            .get("dimensions")
            .cloned()
        })
    }
    /// Write creation settings, including the custom editor's dimension stems.
    pub fn write_world_gen(&self, dir: &Path, seed: i64) -> crate::Result<()> {
        crate::save::write_world_gen_settings_with(
            dir,
            seed,
            self.world_gen(),
            self.flat_settings(),
            self.single_biome(),
            self.dimensions(),
        )
    }
}
