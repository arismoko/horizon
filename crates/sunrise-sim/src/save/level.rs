use super::*;

/// Difficulty ids, in the cycle button's order.
pub use sunrise_server_files::properties::DIFFICULTIES;

#[derive(Clone, Debug, Default)]
pub struct Level {
    pub name: String,
    pub seed: i64,
    /// The generator selected for the session, including any replacement settings.
    #[doc(hidden)]
    pub generator: Option<WorldGenerator>,
    pub creative: bool,
    /// The game mode id (0 survival, 1 creative, 2 adventure, 3 spectator);
    /// `creative` wins when the two disagree.
    pub game_mode: i32,
    /// `previousPlayerGameType`: the mode the player left last (the F3+F4 switcher's
    /// return mode), when it has one.
    pub previous_game_mode: Option<i32>,
    pub time: i64,
    pub day_time: i64,
    /// The End's clock (`world_clocks.dat`'s `minecraft:the_end`); `None` in a world that has
    /// none yet, which starts it at the game time.
    pub end_clock: Option<i64>,
    /// The world spawn or a legacy bare Overworld spawn column. Personal
    /// points use `respawn_elsewhere` so their blocks are checked on death.
    pub spawn: Option<[f64; 3]>,
    /// A respawn point resolved through its block when the player respawns
    /// (a bed, a respawn anchor, a forced point in any dimension):
    /// dimension, block, yaw, pitch and `forced`.
    pub respawn_elsewhere: Option<(String, [i32; 3], f32, f32, bool)>,
    pub position: Option<[f64; 3]>,
    pub rotation: [f32; 2],
    pub health: f32,
    /// LivingEntity `AbsorptionAmount`: the golden hearts left.
    pub absorption: f32,
    pub food: i32,
    pub saturation: f32,
    /// (slot, item, count, damage): 0..36 inventory, 100..103 armor (feet..head), 150 offhand,
    /// 151 body and 152 saddle (`BODY_SLOT`).
    pub inventory: Vec<(u8, String, u32, u32)>,
    /// Extra item components of `inventory` entries, by slot.
    pub inventory_components: HashMap<u8, Box<crate::item_data::StackData>>,
    /// Player.XpSeed: the enchanting table seed.
    pub xp_seed: i32,
    /// ServerPlayer.seenCredits: the end poem has been shown.
    pub seen_credits: bool,
    /// ServerPlayer.postEffects: the `post_effect` chains `/posteffect` set.
    pub post_effects: Vec<String>,
    /// Player.lastDeathLocation: the dimension and block of the last death.
    pub last_death: Option<(String, [i32; 3])>,
    /// Player.active_effects: (id, duration, amplifier, ambient,
    /// show_particles, show_icon).
    pub active_effects: Vec<crate::effects::SavedEffect>,
    /// Player.EnderItems: the ender chest inventory (27 slots).
    pub ender_items: Vec<Option<crate::items::ItemStack>>,
    /// The `recipeBook` compound: open/filter settings and unlocked recipes.
    pub recipe_book: crate::recipe_book::Settings,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub xp_total: i32,
    /// `Player.getScore`, stored as `Score` independently of the experience fields.
    pub player_score: i32,
    pub weather: crate::world::Weather,
    /// Player.Dimension (minecraft:overworld / minecraft:the_nether).
    pub dimension: String,
    /// World spawn (`level.dat` `spawn`), distinct from a bed respawn point.
    pub world_spawn: Option<[i32; 3]>,
    /// The world spawn's yaw and pitch (`setworldspawn`'s rotation).
    pub world_spawn_angle: [f32; 2],
    /// The level the world spawn is in (`spawn.dimension`, `RespawnData.dimension`); None for
    /// the Overworld.
    pub world_spawn_dimension: Option<String>,
    pub difficulty: String,
    pub hardcore: bool,
    /// `difficulty_settings.locked` (LevelData.isDifficultyLocked).
    pub difficulty_locked: bool,
    /// `allowCommands`: read from `level.dat` or chosen on the Create New
    /// World screen; unset is written as off.
    pub allow_commands: Option<bool>,
    /// `DataPackConfig` (Create World's Data Packs and Experiments): the
    /// enabled and disabled pack ids. Unset keeps vanilla's defaults.
    pub data_packs: Option<(Vec<String>, Vec<String>)>,
    /// `WorldDataConfiguration`'s `enabled_features`: the effective feature flag ids.
    /// An omitted `level.dat` tag decodes as vanilla's default, `minecraft:vanilla`.
    pub enabled_features: FeatureFlagIds,
    /// The code mods (`fml.LoadingModList` beside `Data`), then the content mods (`sunrise.ContentModList`), the world was saved with; `None` without either list or when both are empty.
    pub saved_mods: Option<Vec<level_mods::ModRecord>>,
    /// Game rule values by name without namespace (`keep_inventory`, ...).
    pub game_rules: BTreeMap<String, i32>,
    /// Definitions used by this world session.
    pub rule_registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    pub player_uuid: Option<[i32; 4]>,
    pub selected_slot: i32,
    pub motion: [f64; 3],
    pub fall_distance: f64,
    pub on_ground: bool,
    /// `LivingEntity.addAdditionalSaveData`'s `FallFlying`: the player was
    /// gliding (shared flag 7), so the glide goes on after the load.
    pub fall_flying: bool,
    /// Current body's save projection. None before a live body supplies it;
    /// retained raw data stays intact until then.
    pub player_impulse_grace: Option<i32>,
    pub player_fall_particles: Option<bool>,
    pub food_exhaustion: f32,
    pub food_tick_timer: i32,
    /// Abilities.flying: whether creative/spectator flight is active.
    pub flying: bool,
    /// Abilities.flyingSpeed, when the file has one (vanilla's is 0.05).
    pub fly_speed: Option<f32>,
    /// The `abilities` compound of the player file (`Abilities.Packed`), as
    /// read and as a server player's stored abilities write it. When none,
    /// `player_tag` derives it from the game mode.
    pub abilities: Option<crate::lan::abilities::PlayerAbilities>,
    /// The vehicle the player rides, by UUID (`ServerPlayer.saveParentVehicle`'s
    /// "RootVehicle"), so the ride comes back on the next login.
    pub root_vehicle: Option<[i32; 4]>,
    /// The player compound as loaded, for fields Sunrise does not model
    /// (ender chest, effects, abilities, recipe book, attributes, ...).
    pub player_extra: Option<Tag>,
    /// The player was read from the file level.dat names (`singleplayer_uuid`,
    /// or its `.dat_old`): `PlayerList.loadPlayerData` gives the world's owner
    /// that file, not their own.
    pub singleplayer_file: bool,
}

impl Level {
    /// `PrimaryLevelData.setDifficulty`: the level data holds the world's
    /// difficulty, so the game and the file follow each other and a world
    /// that was set to hard is hard when it is opened again.
    pub fn set_difficulty(&mut self, difficulty: u8) {
        self.difficulty = DIFFICULTIES[difficulty as usize % 4].to_owned();
    }

    /// `PrimaryLevelData.getDifficulty`: the id of the level data's
    /// difficulty (peaceful 0 .. hard 3), normal when it is not one of them.
    pub fn difficulty_id(&self) -> u8 {
        DIFFICULTIES
            .iter()
            .position(|d| *d == self.difficulty)
            .map_or(2, |i| i as u8)
    }
    /// Initialize the offline single-player identity before saving or dropping items.
    pub fn ensure_player_uuid(&mut self) -> [i32; 4] {
        *self.player_uuid.get_or_insert_with(|| {
            let u = sunrise_protocol::crypto::offline_uuid("Player");
            [
                (u >> 96) as u32 as i32,
                (u >> 64) as u32 as i32,
                (u >> 32) as u32 as i32,
                u as u32 as i32,
            ]
        })
    }
    pub fn game_rule(&self, name: &str) -> i32 {
        self.game_rules
            .get(name)
            .copied()
            .or_else(|| {
                self.rule_registry
                    .rule(name)
                    .and_then(|rule| self.game_rules.get(&rule.name).copied())
            })
            .or_else(|| {
                self.rule_registry
                    .rule(name)
                    .map(|rule| match rule.default {
                        crate::gamerules::Value::Bool(value) => i32::from(value),
                        crate::gamerules::Value::Int(value) => value,
                    })
            })
            .unwrap_or(0)
    }
    /// The saved game mode id.
    pub fn mode(&self) -> i32 {
        match (self.creative, self.game_mode) {
            (true, _) => 1,
            (false, 1) => 0,
            (false, m) => m.clamp(0, 3),
        }
    }
    pub fn read(dir: &Path) -> Option<Self> {
        Self::read_with_registry(dir, Default::default())
    }
    /// Load a world using its session rule definitions.
    pub fn read_with_registry(
        dir: &Path,
        registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    ) -> Option<Self> {
        Self::read_root(dir, read_level_dat(dir)?, registry)
    }
    /// Select a generator for an existing dedicated world with unusable settings.
    pub fn recover_world_generator(
        &mut self,
        overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
        random_seed: impl FnOnce() -> i64,
    ) -> Result<()> {
        use sunrise_world::worldgen::overrides::DIMENSIONS;
        if self.generator.is_some() {
            return Ok(());
        }
        log::warn!(
            "Sunrise: world generator settings do not read; using a random world seed and datapack dimensions"
        );
        self.seed = random_seed();
        let mut data = new_world_gen_settings(self.seed, WorldGen::default(), None, None);
        let dimensions = data.get_mut("dimensions").expect("default dimension map");
        let Tag::Compound(stems) = dimensions else {
            unreachable!()
        };
        for (id, stem) in overrides.entries(DIMENSIONS) {
            stems.insert(
                id.into(),
                crate::scoreboard::json_to_tag(&serde_json::from_str(stem)?),
            );
        }
        if !valid_world_gen_dimensions(dimensions) {
            return Err("invalid datapack dimensions for generator recovery".into());
        }
        self.generator = Some(WorldGenerator {
            data,
            recovered: true,
        });
        Ok(())
    }
    /// `read` from level.dat alone, without the level.dat_old fallback: what
    /// `LevelStorageSource.readLevelSummary` reads for the world list.
    pub fn read_summary(dir: &Path) -> Option<Self> {
        Self::read_root(dir, read_dat(&dir.join("level.dat"))?, Default::default())
    }
    fn read_root(
        dir: &Path,
        root: Tag,
        registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    ) -> Option<Self> {
        let data = root.get("Data")?;
        let n = |t: Option<&Tag>| number(t).unwrap_or(0);
        let mut level = Self {
            rule_registry: registry,
            name: text(data.get("LevelName")).unwrap_or_else(|| "World".into()),
            creative: n(data.get("GameType")) == 1,
            game_mode: n(data.get("GameType")) as i32,
            time: n(data.get("Time")),
            health: 20.0,
            food: 20,
            saturation: 5.0,
            dimension: "minecraft:overworld".into(),
            difficulty: "normal".into(),
            ..Self::default()
        };
        // 26.3: spawn {pos: [I; x, y, z], dimension, yaw, pitch}; older: SpawnX/Y/Z.
        level.world_spawn = match data.get("spawn").and_then(|s| s.get("pos")) {
            Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
            _ => data.get("SpawnX").map(|_| {
                [
                    n(data.get("SpawnX")) as i32,
                    n(data.get("SpawnY")) as i32,
                    n(data.get("SpawnZ")) as i32,
                ]
            }),
        };
        let angle = |k: &str| match data.get("spawn").and_then(|s| s.get(k)) {
            Some(Tag::Float(b)) => f32::from_bits(*b),
            _ => 0.0,
        };
        level.world_spawn_angle = [angle("yaw"), angle("pitch")];
        level.world_spawn_dimension = text(data.get("spawn").and_then(|s| s.get("dimension")))
            .filter(|d| d != "minecraft:overworld");
        if let Some(d) = data.get("difficulty_settings") {
            level.difficulty = text(d.get("difficulty")).unwrap_or_else(|| "normal".into());
            level.hardcore = n(d.get("hardcore")) != 0;
            level.difficulty_locked = n(d.get("locked")) != 0;
        }
        level.allow_commands = Some(n(data.get("allowCommands")) != 0);
        // Identifier.CODEC resolves each name before FeatureFlags.REGISTRY looks it up.
        // WorldDataConfiguration.MAP_CODEC defaults the whole set on a decode error;
        // a valid empty set is not the default.
        if let Some(Tag::List(flags)) = data.get("enabled_features")
            && let Some(flags) = flags
                .iter()
                .map(|flag| mod_api::FeatureFlag::from_id(text(Some(flag))?.as_str()))
                .collect::<Option<FeatureFlagIds>>()
        {
            level.enabled_features = flags;
        }
        level.saved_mods = level_mods::saved(&root);
        if let Some(packs) = data.get("DataPacks") {
            let ids = |key| match packs.get(key) {
                Some(Tag::List(ids)) => ids.iter().filter_map(|id| text(Some(id))).collect(),
                _ => Vec::new(),
            };
            level.data_packs = Some((ids("Enabled"), ids("Disabled")));
        }
        let registry = level.rule_registry.clone();
        let mut rules: BTreeMap<String, i32> = registry
            .available_rules(false)
            .map(|rule| {
                (
                    rule.name.to_owned(),
                    match rule.default {
                        crate::gamerules::Value::Bool(value) => i32::from(value),
                        crate::gamerules::Value::Int(value) => value,
                    },
                )
            })
            .collect();
        let saved_rules = read_saved_data(dir, "game_rules");
        if let Some(Tag::Compound(map)) = &saved_rules {
            for (k, v) in map {
                let name = k.to_string_lossy();
                let name = if name.contains(':') {
                    name
                } else {
                    format!("minecraft:{name}")
                };
                if let Some(v) = number(Some(v)) {
                    rules.insert(name, v as i32);
                }
            }
        }
        // GameRules(enabledFeatures, map) removes disabled experimental rules
        // and supplies their defaults when enabled but absent from saved data.
        let registry = level.rule_registry.clone();
        let minecart_rule = registry.rule("max_minecart_speed").expect("vanilla rule");
        if level
            .enabled_features
            .iter()
            .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS)
        {
            let crate::gamerules::Value::Int(default) = minecart_rule.default else {
                unreachable!()
            };
            let value = rules
                .entry(minecart_rule.name.to_owned())
                .or_insert(default);
            if !(minecart_rule.min..=minecart_rule.max).contains(value) {
                *value = default;
            }
        } else {
            rules.remove(&minecart_rule.name);
        }
        level.game_rules = rules;
        (level.generator, level.seed) = WorldGenerator::read(dir, Some(data));
        // Day time: the overworld clock (26.3) or Data.DayTime.
        level.day_time = read_saved_data(dir, "world_clocks")
            .and_then(|c| {
                number(
                    c.get("minecraft:overworld")
                        .and_then(|o| o.get("total_ticks")),
                )
            })
            .or_else(|| number(data.get("DayTime")))
            .unwrap_or(level.time);
        level.end_clock = read_saved_data(dir, "world_clocks").and_then(|c| {
            number(
                c.get("minecraft:the_end")
                    .and_then(|o| o.get("total_ticks")),
            )
        });
        let weather = read_saved_data(dir, "weather");
        let w = |new: &str, old: &str| {
            number(weather.as_ref().and_then(|w| w.get(new)))
                .or_else(|| number(data.get(old)))
                .unwrap_or(0)
        };
        let raining = w("raining", "raining") != 0;
        let thundering = w("thundering", "thundering") != 0;
        level.weather = crate::world::Weather {
            clear_time: w("clear_weather_time", "clearWeatherTime") as i32,
            rain_time: w("rain_time", "rainTime") as i32,
            thunder_time: w("thunder_time", "thunderTime") as i32,
            raining,
            thundering,
            rain_level: if raining { (1.0, 1.0) } else { (0.0, 0.0) },
            thunder_level: if thundering { (1.0, 1.0) } else { (0.0, 0.0) },
        };
        // The player: players/data/<singleplayer_uuid>.dat, or the legacy Data.Player. With no
        // `singleplayer_uuid` (a world from a server, a Realms download) the owner loads their
        // own file, not another player's (`PlayerList.loadPlayerData`): `load_owner` does.
        let uuid = uuid_from_tag(data.get("singleplayer_uuid"));
        let player_dir = dir.join("players").join("data");
        // PlayerDataStorage.load: the player's file, else its .dat_old.
        let owner = uuid.and_then(|u| {
            let name = uuid_text(u);
            read_dat(&player_dir.join(format!("{name}.dat")))
                .or_else(|| read_dat(&player_dir.join(format!("{name}.dat_old"))))
        });
        level.singleplayer_file = owner.is_some();
        let mut player = owner.or_else(|| data.get("Player").cloned());
        if let Some(player) = &mut player {
            horizon_names::adopt_in_player(player);
        }
        level.player_uuid = uuid_from_tag(player.as_ref().and_then(|p| p.get("UUID"))).or(uuid);
        let spawn = level
            .world_spawn
            .map(|s| [f64::from(s[0]), f64::from(s[1]), f64::from(s[2])]);
        level.spawn = spawn;
        if let Some(p) = &player {
            level.read_player(p);
        }
        level.player_extra = player;
        Some(level)
    }
    pub fn read_player(&mut self, p: &Tag) {
        self.player_impulse_grace = None;
        self.player_fall_particles = None;
        let n = |k: &str| number(p.get(k));
        let f = |k: &str| float(p.get(k));
        self.position = read_doubles::<3>(p.get("Pos"));
        if let Some(r) = read_doubles::<2>(p.get("Rotation")) {
            self.rotation = [r[0] as f32, r[1] as f32];
        }
        self.motion = read_doubles::<3>(p.get("Motion")).unwrap_or_default();
        self.health = f("Health").map_or(20.0, |v| v as f32);
        self.absorption = f("AbsorptionAmount").map_or(0.0, |v| v as f32);
        self.food = n("foodLevel").map_or(20, |v| v as i32);
        self.saturation = f("foodSaturationLevel").map_or(5.0, |v| v as f32);
        self.food_exhaustion = f("foodExhaustionLevel").unwrap_or(0.0) as f32;
        self.food_tick_timer = n("foodTickTimer").unwrap_or(0) as i32;
        self.xp_level = n("XpLevel").unwrap_or(0) as i32;
        self.xp_progress = f("XpP").unwrap_or(0.0) as f32;
        self.xp_total = n("XpTotal").unwrap_or(0) as i32;
        self.player_score = n("Score").unwrap_or(0) as i32;
        self.selected_slot = n("SelectedItemSlot").unwrap_or(0) as i32;
        self.fall_distance = f("fall_distance").unwrap_or(0.0);
        self.on_ground = n("OnGround").unwrap_or(0) != 0;
        // LivingEntity.readAdditionalSaveData: `getBooleanOr("FallFlying",
        // false)` sets shared flag 7; any numeric tag counts, as a byte.
        self.fall_flying = n("FallFlying").unwrap_or(0) != 0;
        self.seen_credits = n("seenCredits").unwrap_or(0) != 0;
        self.post_effects = match p.get("post_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|t| match t {
                    Tag::String(s) => Some(s.to_string_lossy()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        self.last_death = p.get("LastDeathLocation").and_then(|d| {
            let Some(Tag::IntArray(pos)) = d.get("pos") else {
                return None;
            };
            Some((
                text(d.get("dimension"))?,
                [*pos.first()?, *pos.get(1)?, *pos.get(2)?],
            ))
        });
        if let Some(mode) = n("playerGameType") {
            self.creative = mode == 1;
            self.game_mode = mode as i32;
        }
        self.previous_game_mode = n("previousPlayerGameType").map(|m| m as i32);
        self.flying = p
            .get("abilities")
            .and_then(|a| number(a.get("flying")))
            .is_some_and(|v| v != 0);
        self.fly_speed = p
            .get("abilities")
            .and_then(|a| float(a.get("flySpeed")))
            .map(|v| v as f32);
        self.abilities = p.get("abilities").map(|a| {
            let flag = |key: &str, default: bool| number(a.get(key)).map_or(default, |v| v != 0);
            let speed = |key: &str, default: f32| float(a.get(key)).map_or(default, |v| v as f32);
            crate::lan::abilities::PlayerAbilities {
                invulnerable: flag("invulnerable", false),
                flying: flag("flying", false),
                may_fly: flag("mayfly", false),
                instabuild: flag("instabuild", false),
                may_build: flag("mayBuild", true),
                flying_speed: speed("flySpeed", 0.05),
                walking_speed: speed("walkSpeed", 0.1),
            }
        });
        if let Some(d) = text(p.get("Dimension")) {
            self.dimension = d;
        }
        if let Some(respawn) = respawn::read(p) {
            self.respawn_elsewhere = Some(respawn);
        }
        let mut inventory = Vec::new();
        let mut inventory_components = HashMap::new();
        if let Some(Tag::List(items)) = p.get("Inventory") {
            for it in items {
                let slot = number(it.get("Slot")).unwrap_or(0) as i8 as u8;
                if let Some(stack) = item_from_nbt(it) {
                    let slot = if slot as i8 == -106 { 150 } else { slot };
                    if let Some(extra) = &stack.extra {
                        inventory_components.insert(slot, extra.clone());
                    }
                    // Legacy armor slots 100..103 and offhand -106 (150).
                    let slot = if slot as i8 == -106 { 150 } else { slot };
                    inventory.push((slot, stack.item.to_string(), stack.count, stack.damage));
                }
            }
        }
        if let Some(eq) = p.get("equipment") {
            let eq = sunrise_items::equipment::Equipment::read(eq);
            for (ordinal, slot) in PLAYER_EQUIPMENT_SLOTS {
                if let Some(stack) = eq.get(ordinal) {
                    inventory.retain(|(s, ..)| *s != slot);
                    if let Some(extra) = &stack.extra {
                        inventory_components.insert(slot, extra.clone());
                    }
                    inventory.push((slot, stack.item.to_string(), stack.count, stack.damage));
                }
            }
        }
        self.inventory = inventory;
        self.ender_items = decode_items(p.get("EnderItems"), 27);
        self.recipe_book = crate::recipe_book::Settings::read(p.get("recipeBook"));
        self.inventory_components = inventory_components;
        self.xp_seed = n("XpSeed").unwrap_or(0) as i32;
        self.active_effects = match p.get("active_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| Some(read_effect_details(e, &text(e.get("id"))?)))
                .collect(),
            _ => Vec::new(),
        };
        // ServerPlayer.saveParentVehicle: the mount the player rides, by
        // UUID; the vehicle tree stays in the raw compound until it is back.
        self.root_vehicle = p
            .get("RootVehicle")
            .and_then(|r| uuid_from_tag(r.get("Attach")));
    }
    /// The player compound: loaded data with current state written over it.
    /// `LivingEntity.addAdditionalSaveData` / `ServerPlayer.addAdditionalSaveData`:
    /// the live fall fields and the player's empty brain are saved too.
    pub fn player_tag(&self, items: &crate::items::Items) -> Tag {
        let mut p = compound_or_empty(self.player_extra.clone());
        // Keys rewritten from scratch.
        if let Tag::Compound(map) = &mut p {
            for key in [
                "Inventory",
                "EnderItems",
                "equipment",
                "active_effects",
                "post_effects",
                "Pos",
                "respawn",
                "recipeBook",
                "LastDeathLocation",
                "previousPlayerGameType",
            ] {
                map.remove(&JavaString::from(key));
            }
        }
        let mut inventory = Vec::new();
        let mut equipment = sunrise_items::equipment::Equipment::default();
        for (slot, id, count, damage) in &self.inventory {
            let extra = self.inventory_components.get(slot).cloned();
            let stack = crate::items::ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra,
            };
            match *slot {
                0..=35 => {
                    let mut entries = vec![("Slot", Tag::Byte(*slot as i8))];
                    entries.extend(encode_item(&stack, items));
                    inventory.push(Tag::compound(entries));
                }
                _ => {
                    if let Some((ordinal, _)) = PLAYER_EQUIPMENT_SLOTS
                        .iter()
                        .find(|(_, saved)| saved == slot)
                    {
                        equipment.set(*ordinal, Some(stack));
                    }
                }
            }
        }
        // The stored abilities, else GameType.updatePlayerAbilities.
        let mode = self.mode();
        let (creative, spectator) = (mode == 1, mode == 3);
        let mut abilities = compound_or_empty(p.get("abilities").cloned());
        if let Some(a) = self.abilities {
            for (k, v) in [
                ("invulnerable", a.invulnerable),
                ("flying", a.flying),
                ("mayfly", a.may_fly),
                ("instabuild", a.instabuild),
                ("mayBuild", a.may_build),
            ] {
                put(&mut abilities, k, Tag::Byte(i8::from(v)));
            }
            put(
                &mut abilities,
                "flySpeed",
                Tag::Float(a.flying_speed.to_bits()),
            );
            put(
                &mut abilities,
                "walkSpeed",
                Tag::Float(a.walking_speed.to_bits()),
            );
        } else {
            for (k, v) in [
                ("invulnerable", creative || spectator),
                ("mayfly", creative || spectator),
                ("instabuild", creative),
                ("mayBuild", mode < 2),
            ] {
                put(&mut abilities, k, Tag::Byte(i8::from(v)));
            }
            if spectator {
                put(&mut abilities, "flying", Tag::Byte(1));
            } else if creative {
                put(&mut abilities, "flying", Tag::Byte(i8::from(self.flying)));
            } else {
                put(&mut abilities, "flying", Tag::Byte(0));
            }
        }
        if let Tag::Compound(map) = &mut abilities {
            map.entry("mayBuild".into()).or_insert(Tag::Byte(1));
            if let Some(speed) = self.fly_speed {
                map.insert("flySpeed".into(), Tag::Float(speed.to_bits()));
            }
            map.entry("flySpeed".into())
                .or_insert(Tag::Float(0.05f32.to_bits()));
            map.entry("walkSpeed".into())
                .or_insert(Tag::Float(0.1f32.to_bits()));
            map.entry("flying".into()).or_insert(Tag::Byte(0));
        }
        let impulse_grace = self
            .player_impulse_grace
            .map(Tag::Int)
            .or_else(|| p.get("current_impulse_context_reset_grace_time").cloned())
            .unwrap_or(Tag::Int(0));
        let fall_particles = self
            .player_fall_particles
            .map(|v| Tag::Byte(i8::from(v)))
            .or_else(|| p.get("spawn_extra_particles_on_fall").cloned())
            .unwrap_or(Tag::Byte(0));
        let uuid = self.player_uuid.unwrap_or([0, 0, 0, 0]);
        let mut entries: Vec<(&str, Tag)> = vec![
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("UUID", uuid_tag(uuid)),
            ("Rotation", floats(&self.rotation)),
            ("Motion", doubles(&self.motion)),
            ("OnGround", Tag::Byte(i8::from(self.on_ground))),
            ("fall_distance", Tag::Double(self.fall_distance.to_bits())),
            // LivingEntity.addAdditionalSaveData: `putBoolean("FallFlying",
            // isFallFlying())`, always written.
            ("FallFlying", Tag::Byte(i8::from(self.fall_flying))),
            ("Health", Tag::Float(self.health.to_bits())),
            // Player inherits LivingEntity.makeBrain: a new empty Brain;
            // Brain.Packed's codec stores its empty memories map.
            (
                "Brain",
                Tag::compound([("memories", Tag::Compound(Default::default()))]),
            ),
            ("current_impulse_context_reset_grace_time", impulse_grace),
            ("spawn_extra_particles_on_fall", fall_particles),
            ("AbsorptionAmount", Tag::Float(self.absorption.to_bits())),
            ("foodLevel", Tag::Int(self.food)),
            ("foodSaturationLevel", Tag::Float(self.saturation.to_bits())),
            (
                "foodExhaustionLevel",
                Tag::Float(self.food_exhaustion.to_bits()),
            ),
            ("foodTickTimer", Tag::Int(self.food_tick_timer)),
            ("XpLevel", Tag::Int(self.xp_level)),
            ("XpP", Tag::Float(self.xp_progress.to_bits())),
            ("XpTotal", Tag::Int(self.xp_total)),
            ("Score", Tag::Int(self.player_score)),
            ("SelectedItemSlot", Tag::Int(self.selected_slot)),
            ("XpSeed", Tag::Int(self.xp_seed)),
            (
                "active_effects",
                Tag::List(
                    self.active_effects
                        .iter()
                        .map(|e| {
                            let mut effect = vec![("id", string(&e.id))];
                            effect.extend(effect_details(e));
                            Tag::compound(effect)
                        })
                        .collect(),
                ),
            ),
            ("playerGameType", Tag::Int(self.mode())),
            ("Inventory", Tag::List(inventory)),
            ("EnderItems", encode_items(&self.ender_items, items)),
            ("recipeBook", self.recipe_book.write()),
            ("abilities", abilities),
            (
                "Dimension",
                string(if self.dimension.is_empty() {
                    "minecraft:overworld"
                } else {
                    &self.dimension
                }),
            ),
        ];
        // LivingEntity.addAdditionalSaveData omits empty equipment and an empty
        // effect list. PlayerEquipment.isEmpty also counts the selected hotbar
        // item, which the codec never writes: holding one saves `equipment: {}`.
        let holding = self
            .inventory
            .iter()
            .any(|(slot, _, count, _)| i32::from(*slot) == self.selected_slot && *count > 0);
        match equipment.write(items) {
            Some(tag) => entries.push(("equipment", tag)),
            None if holding => entries.push(("equipment", Tag::Compound(Default::default()))),
            None => {}
        }
        if self.active_effects.is_empty() {
            entries.retain(|(key, _)| *key != "active_effects");
        }
        if let Some(pos) = self.position {
            entries.push(("Pos", doubles(&pos)));
        }
        entries.push(("seenCredits", Tag::Byte(i8::from(self.seen_credits))));
        // `output.store` writes the list even when it is empty.
        entries.push((
            "post_effects",
            Tag::List(self.post_effects.iter().map(|e| string(e)).collect()),
        ));
        if let Some((dimension, pos)) = &self.last_death {
            entries.push((
                "LastDeathLocation",
                Tag::compound([
                    ("dimension", string(dimension)),
                    ("pos", Tag::IntArray(pos.to_vec())),
                ]),
            ));
        }
        // A respawn point other than the world spawn: an anchor elsewhere, or a bed.
        if let Some(saved) = &self.respawn_elsewhere {
            entries.push(("respawn", respawn::write(saved)));
        } else if let Some(s) = self.spawn {
            let block = [
                s[0].floor() as i32,
                s[1].floor() as i32,
                s[2].floor() as i32,
            ];
            // A point of the player's own in the Overworld other than a bed
            // (`respawn_elsewhere`): `spawnpoint`'s, forced.
            if Some(block) != self.world_spawn {
                entries.push((
                    "respawn",
                    Tag::compound([
                        ("dimension", string("minecraft:overworld")),
                        ("pos", Tag::IntArray(block.to_vec())),
                        ("yaw", Tag::Float(0f32.to_bits())),
                        ("pitch", Tag::Float(0f32.to_bits())),
                        ("forced", Tag::Byte(1)),
                    ]),
                ));
            }
        }
        // ServerPlayer.storeGameTypes: storeNullable, so absent without one.
        if let Some(mode) = self.previous_game_mode {
            entries.push(("previousPlayerGameType", Tag::Int(mode)));
        }
        for (key, value) in entries {
            put(&mut p, key, value);
        }
        if let Tag::Compound(map) = &mut p {
            for (key, value) in [
                ("Air", Tag::Short(300)),
                ("Fire", Tag::Short(-20)),
                ("Invulnerable", Tag::Byte(0)),
                ("PortalCooldown", Tag::Int(0)),
                ("SleepTimer", Tag::Short(0)),
                ("XpSeed", Tag::Int(0)),
                ("seenCredits", Tag::Byte(0)),
                ("HurtTime", Tag::Short(0)),
                ("DeathTime", Tag::Short(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        // ServerPlayer.saveParentVehicle: the savers put the whole
        // RootVehicle in the compound; a mount known by UUID alone (older
        // Sunrise files) is still named.
        if let Some(vehicle) = self.root_vehicle
            && p.get("RootVehicle").is_none()
        {
            put(
                &mut p,
                "RootVehicle",
                Tag::compound([("Attach", uuid_tag(vehicle))]),
            );
        }
        p
    }
    /// Write every file; a world without a player UUID gets a random one.
    pub fn write(&mut self, dir: &Path, items: &crate::items::Items) -> Result<()> {
        self.write_internal(dir, Some(items), false, false)
    }

    /// Dedicated servers save connected players separately.
    pub fn write_without_player(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, None, false, false)
    }

    /// Write world metadata without saving the owner. The first `level.dat` names a
    /// known profile, even before login. Later writes preserve the existing profile
    /// reference and any legacy `Data.Player`.
    pub fn write_keeping_player(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, None, false, true)
    }

    /// The temporary world a Realms upload packs
    /// (`RealmsCreateWorldFlow.createTemporaryWorldFolder`): level.dat as
    /// `WorldData.createTag(null)` writes it — no `singleplayer_uuid` — with the world
    /// generation and game rules, and no player file or weather/clocks the server makes
    /// on its own. The caller writes the data packs and removes the folder.
    pub fn write_upload(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, None, true, false)
    }

    /// The player's file alone, `players/data/<singleplayer_uuid>.dat`
    /// (`PlayerDataStorage.save`): the one level.dat names.
    pub fn write_player(&mut self, dir: &Path, items: &crate::items::Items) -> Result<()> {
        let uuid = self.ensure_player_uuid();
        let player_file = dir
            .join("players")
            .join("data")
            .join(format!("{}.dat", uuid_text(uuid)));
        back_up_unreadable_player(&player_file);
        write_dat(&player_file, self.player_tag(items))
    }

    fn write_internal(
        &mut self,
        dir: &Path,
        items: Option<&crate::items::Items>,
        upload: bool,
        keep_legacy_player: bool,
    ) -> Result<()> {
        fs::create_dir_all(dir)?;
        restore_level_dat(dir);
        let existing = read_dat(&dir.join("level.dat"));
        // `saveAllChunks` passes the single-player profile independently of `PlayerList`.
        // For an existing world with no placed owner, keep its on-disk reference (even
        // when absent); `self.player_uuid` can name a different profile in that case.
        let uuid = if upload {
            None
        } else if keep_legacy_player {
            self.player_uuid.filter(|_| existing.is_none())
        } else {
            Some(self.ensure_player_uuid())
        };
        let mut root = compound_or_empty(existing);
        let legacy = root
            .get("Data")
            .is_some_and(|d| d.get("Player").is_some() || d.get("WorldGenSettings").is_some());
        let mut data = compound_or_empty(root.get("Data").cloned());
        let kept_player = keep_legacy_player
            .then(|| data.get("Player").cloned())
            .flatten();
        if let Tag::Compound(map) = &mut data {
            // Pre-26.3 keys now stored elsewhere (a legacy Sunrise level.dat).
            for key in [
                "Player",
                "WorldGenSettings",
                "DayTime",
                "SpawnX",
                "SpawnY",
                "SpawnZ",
                "SpawnAngle",
                "clearWeatherTime",
                "rainTime",
                "thunderTime",
                "raining",
                "thundering",
                "Difficulty",
                "hardcore",
                "GameRules",
                "DimensionData",
                "WanderingTraderSpawnChance",
                "WanderingTraderSpawnDelay",
                "WanderingTraderId",
                "BorderCenterX",
                "BorderCenterZ",
                "BorderSize",
                "BorderSafeZone",
                "BorderWarningBlocks",
                "BorderWarningTime",
                "BorderSizeLerpTime",
                "BorderSizeLerpTarget",
                "BorderDamagePerBlock",
                "DifficultyLocked",
                "ScheduledEvents",
                "CustomBossEvents",
                "DragonFight",
            ] {
                map.remove(&JavaString::from(key));
            }
        }
        // A new world's first spawn point is its world spawn.
        if self.world_spawn.is_none() {
            self.world_spawn = self.spawn.map(|s| {
                [
                    s[0].floor() as i32,
                    s[1].floor() as i32,
                    s[2].floor() as i32,
                ]
            });
        }
        let spawn = self.world_spawn.unwrap_or([0, 64, 0]);
        let mut history: Vec<Tag> = match data.get("version_history") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        if history.last() != Some(&Tag::Int(DATA_VERSION)) {
            history.push(Tag::Int(DATA_VERSION));
        }
        let mut brands: Vec<Tag> = match data.get("ServerBrands") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        if !brands.contains(&string("sunrise")) {
            brands.push(string("sunrise"));
        }
        let mut difficulty = compound_or_empty(data.get("difficulty_settings").cloned());
        put(
            &mut difficulty,
            "difficulty",
            string(if self.difficulty.is_empty() {
                "normal"
            } else {
                &self.difficulty
            }),
        );
        put(
            &mut difficulty,
            "hardcore",
            Tag::Byte(i8::from(self.hardcore)),
        );
        put(
            &mut difficulty,
            "locked",
            Tag::Byte(i8::from(self.difficulty_locked)),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        for (key, value) in [
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "Version",
                Tag::compound([
                    ("Name", string(VERSION_NAME)),
                    ("Id", Tag::Int(DATA_VERSION)),
                    ("Snapshot", Tag::Byte(0)),
                    ("Series", string("main")),
                ]),
            ),
            ("version_history", Tag::List(history)),
            ("version", Tag::Int(19133)),
            ("LevelName", string(&self.name)),
            ("GameType", Tag::Int(self.mode())),
            ("Time", Tag::Long(self.time)),
            ("LastPlayed", Tag::Long(now)),
            (
                "spawn",
                Tag::compound([
                    ("pos", Tag::IntArray(spawn.to_vec())),
                    (
                        "dimension",
                        string(
                            self.world_spawn_dimension
                                .as_deref()
                                .unwrap_or("minecraft:overworld"),
                        ),
                    ),
                    ("yaw", Tag::Float(self.world_spawn_angle[0].to_bits())),
                    ("pitch", Tag::Float(self.world_spawn_angle[1].to_bits())),
                ]),
            ),
            ("difficulty_settings", difficulty),
            ("initialized", Tag::Byte(1)),
            ("ServerBrands", Tag::List(brands)),
        ] {
            put(&mut data, key, value);
        }
        level_mods::write(&mut root);
        if let Some(uuid) = uuid {
            put(&mut data, "singleplayer_uuid", uuid_tag(uuid));
        }
        if let Tag::Compound(map) = &mut data {
            map.entry("allowCommands".into()).or_insert(Tag::Byte(0));
            if let Some(allow) = self.allow_commands {
                map.insert("allowCommands".into(), Tag::Byte(i8::from(allow)));
            }
            map.entry("WasModded".into()).or_insert(Tag::Byte(0));
            // WorldDataConfiguration omits the default, even when the retained Data had a tag.
            if *self.enabled_features == VANILLA_FEATURE_FLAGS {
                map.remove(&JavaString::from("enabled_features"));
            } else {
                map.insert(
                    "enabled_features".into(),
                    Tag::List(self.enabled_features.iter().map(|s| string(s)).collect()),
                );
            }
            map.entry("DataPacks".into()).or_insert_with(|| {
                let (enabled, disabled) = self.data_packs.clone().unwrap_or_else(|| {
                    (
                        vec!["vanilla".into()],
                        [
                            "minecart_improvements",
                            "redstone_experiments",
                            "trade_rebalance",
                        ]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    )
                });
                let list = |ids: Vec<String>| Tag::List(ids.iter().map(|s| string(s)).collect());
                Tag::compound([("Enabled", list(enabled)), ("Disabled", list(disabled))])
            });
        }
        // Saved data: weather, clocks, and (when missing or legacy) world
        // generation settings and game rules. An upload's temporary world carries only
        // what the server needs before its first tick.
        if !upload {
            write_saved_data(
                dir,
                "weather",
                Tag::compound([
                    ("clear_weather_time", Tag::Int(self.weather.clear_time)),
                    ("rain_time", Tag::Int(self.weather.rain_time)),
                    ("thunder_time", Tag::Int(self.weather.thunder_time)),
                    ("raining", Tag::Byte(i8::from(self.weather.raining))),
                    ("thundering", Tag::Byte(i8::from(self.weather.thundering))),
                ]),
            )?;
            let mut clocks = compound_or_empty(read_saved_data(dir, "world_clocks"));
            let mut overworld = compound_or_empty(clocks.get("minecraft:overworld").cloned());
            put(&mut overworld, "total_ticks", Tag::Long(self.day_time));
            put(&mut clocks, "minecraft:overworld", overworld);
            let mut end = compound_or_empty(clocks.get("minecraft:the_end").cloned());
            put(
                &mut end,
                "total_ticks",
                Tag::Long(self.end_clock.unwrap_or(self.time)),
            );
            put(&mut clocks, "minecraft:the_end", end);
            write_saved_data(dir, "world_clocks", clocks)?;
        }
        if let Some(generator) = self.generator.as_mut().filter(|g| g.recovered) {
            write_saved_data(dir, "world_gen_settings", generator.data.clone())?;
            generator.recovered = false;
        } else if legacy || !saved_data_path(dir, "world_gen_settings").exists() {
            write_world_gen_settings(dir, self.seed, WorldGen::default())?;
        }
        if !saved_data_path(dir, "game_rules").exists() || !self.game_rules.is_empty() {
            // Preserve other namespaces even when their mod is not currently loaded.
            let mut rules = match read_saved_data(dir, "game_rules") {
                Some(Tag::Compound(entries)) => Tag::Compound(
                    entries
                        .into_iter()
                        .filter(|(key, _)| {
                            key.to_string().is_ok_and(|s| {
                                s.split_once(':').is_some_and(|(ns, _)| ns != "minecraft")
                            })
                        })
                        .collect(),
                ),
                _ => Tag::Compound(Compound::new()),
            };
            let registry = &self.rule_registry;
            for rule in registry.available_rules(
                self.enabled_features
                    .iter()
                    .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS),
            ) {
                let tag = match rule.default {
                    crate::gamerules::Value::Bool(default) => Tag::Byte(i8::from(
                        self.game_rules
                            .get(&rule.name)
                            .copied()
                            .unwrap_or(i32::from(default))
                            != 0,
                    )),
                    crate::gamerules::Value::Int(default) => {
                        Tag::Int(self.game_rules.get(&rule.name).copied().unwrap_or(default))
                    }
                };
                put(&mut rules, &rule.name.clone(), tag);
            }
            if self
                .enabled_features
                .iter()
                .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS)
            {
                let rule = registry.rule("max_minecart_speed").expect("vanilla rule");
                let crate::gamerules::Value::Int(default) = rule.default else {
                    unreachable!()
                };
                let value = self
                    .game_rules
                    .get(&rule.name)
                    .copied()
                    .filter(|v| (rule.min..=rule.max).contains(v))
                    .unwrap_or(default);
                put(&mut rules, &rule.name.clone(), Tag::Int(value));
            }
            write_saved_data(dir, "game_rules", rules)?;
        }
        if let Some(items) = items {
            self.write_player(dir, items)?;
        }
        if let Some(player) = kept_player {
            put(&mut data, "Player", player);
        }
        put(&mut root, "Data", data);
        write_dat(&dir.join("level.dat"), root)?;
        Ok(())
    }
}
