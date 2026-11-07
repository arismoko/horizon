//! A level's world settings through a `LevelRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): the seed,
//! difficulty and game rules on the server, and weather on both sides.
//! Stale handles return `None`.
use super::{Dim, Game};
use horizon_client::command::vanilla::DIFFICULTIES;
use horizon_client::world::World;
use horizon_core::position::BlockPos;
use horizon_core::random::LegacyRandom;
use horizon_world::registry::BlockStateId;
use mod_api::{BoolGameRuleKey, FeatureFlags, GameRuleValue, LevelDimensionShape, LevelRef};
use std::sync::Arc;
use {horizon_core::dimension::Dimension as LevelDimension, horizon_world::level::LevelAccess};

fn weather_for_level(
    shape: horizon_client::chunk::LevelShape,
    is_end: bool,
    levels: (f32, f32),
) -> (f32, f32) {
    if shape.sky_light && !shape.has_ceiling && !is_end {
        levels
    } else {
        (0.0, 0.0)
    }
}

fn weather_for_world(world: &World, is_end: bool) -> (f32, f32) {
    weather_for_level(world.shape, is_end, world.weather.levels(1.0))
}

/// `Level.isRaining` and `isThundering` from the rain and thunder levels.
fn weather_flags((rain, thunder): (f32, f32)) -> (bool, bool) {
    (rain > 0.2, thunder > 0.9)
}

/// A read-only view of one level's column for `Level.isRainingAt`, so a level parked
/// while the server ticks another answers like the one in place.
struct RainView<'a> {
    world: &'a World,
    facts: &'a horizon_client::blocks::BlockFacts,
    biomes: &'a horizon_client::biome::Biomes,
    dimension: LevelDimension,
    raining: bool,
    /// `LevelAccess::random` asks for a generator; the read draws none.
    random: LegacyRandom,
}

impl LevelAccess for RainView<'_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world.block(pos.x, pos.y, pos.z)
    }
    fn set_raw(&mut self, _pos: BlockPos, _state: BlockStateId) -> Option<BlockStateId> {
        None
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.loaded(pos.x, pos.z)
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.world.game_time
    }
    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }
    fn max_y(&self) -> i32 {
        self.world.shape.max_y()
    }
    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4)
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |s| self.facts.is_air(s))
    }
    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        let chunk = self.world.chunks.get(&(pos.x >> 4, pos.z >> 4))?;
        Some(&self.biomes.get(chunk.biome(pos.x, pos.y, pos.z)).name)
    }
    fn is_raining(&self) -> bool {
        self.raining
    }
    fn dimension(&self) -> LevelDimension {
        self.dimension
    }
}

impl Game<'_> {
    /// The enabled feature flags as this side knows them: a client's are the server's, sent at
    /// join; a server's are its world's `enabled_features`.
    pub(super) fn enabled_feature_ids(&self) -> &Arc<[String]> {
        self.remote
            .as_ref()
            .map_or(&self.level.enabled_features, |remote| &remote.features)
    }
    /// Dimension shape of a loaded server level, including parked levels.
    pub(super) fn level_dimension_shape(&self, level: LevelRef) -> Option<LevelDimensionShape> {
        let shape = self.native_level_world(level)?.shape;
        Some(LevelDimensionShape::new(
            shape.min_y,
            shape.height,
            shape.logical_height,
            shape.sky_light,
            shape.has_ceiling,
        ))
    }

    /// The world's seed (`WorldOptions.seed`), as `/seed` shows it.
    pub(super) fn level_seed(&self, level: LevelRef) -> Option<i64> {
        self.native_level_world(level)?;
        Some(self.level.seed)
    }
    /// The difficulty (`Level.getDifficulty`): `peaceful`, `easy`, `normal`
    /// or `hard`, as `/difficulty` names it.
    pub(super) fn level_difficulty(&self, level: LevelRef) -> Option<&'static str> {
        self.native_level_world(level)?;
        DIFFICULTIES.get(usize::from(self.ui.difficulty)).copied()
    }
    /// Read a registered boolean game rule.
    pub(super) fn level_game_rule_bool(
        &self,
        level: LevelRef,
        key: BoolGameRuleKey,
    ) -> Option<bool> {
        self.native_level_world(level)?;
        self.ui
            .rules
            .bool_slot(mod_api::__private::game_rule_bool_slot(key)?)
    }
    /// Game rule `rule` (`minecraft:keep_inventory` or `keep_inventory`), as
    /// `/gamerule` queries it: None for an unknown rule.
    pub(super) fn level_game_rule(&self, level: LevelRef, rule: &str) -> Option<GameRuleValue> {
        self.native_level_world(level)?;
        self.ui.rules.get(rule)
    }
    /// Every game rule the world has (`GameRules`) with its current value, by `minecraft:` id
    /// (`max_minecart_speed` only with minecart improvements): None for a stale handle and on
    /// a client.
    pub(super) fn level_game_rules(&self, level: LevelRef) -> Option<Vec<(String, GameRuleValue)>> {
        self.native_level_world(level)?;
        let rules = &self.ui.rules;
        Some(
            rules
                .available_rules()
                .filter_map(|r| Some((r.name.clone(), rules.get(&r.name)?)))
                .collect(),
        )
    }
    /// The rain and thunder levels, each 0.0 to 1.0 (`Level.getRainLevel`,
    /// `getThunderLevel` at partial tick 1.0): both 0 in a level with no weather.
    /// Reads any open server level or the client's current synchronized level;
    /// None for a stale handle, an unavailable level or a client not logged in.
    pub(super) fn level_weather(&self, level: LevelRef) -> Option<(f32, f32)> {
        let world = self.native_level_read_world(level)?;
        Some(weather_for_world(
            world,
            level.dimension() == Dim::End.key(),
        ))
    }
    /// Whether it rains (`Level.isRaining`: a rain level above 0.2).
    pub(super) fn level_is_raining(&self, level: LevelRef) -> Option<bool> {
        let world = self.native_level_world(level)?;
        Some(
            weather_flags(weather_for_world(
                world,
                level.dimension() == Dim::End.key(),
            ))
            .0,
        )
    }
    /// `Level.isRainingAt(pos)` in the level a handle names, the one in place or a parked one:
    /// it rains there and `pos` is under open sky in a biome that rains. None for a stale
    /// handle, on a client, and where `pos` is not loaded or outside the world's height; a
    /// read never loads a chunk.
    pub(super) fn level_is_raining_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        let world = self.native_level_world(level)?;
        if !world.is_loaded_at(pos[0], pos[1], pos[2]) {
            return None;
        }
        let view = RainView {
            world,
            facts: self.res.facts(),
            biomes: world_biomes!(self),
            dimension: Dim::from_id(level.dimension_id()).level(),
            raining: self.level_is_raining(level)?,
            random: LegacyRandom::new(0),
        };
        Some(
            self.res
                .behaviours()
                .is_raining_at(&view, BlockPos::new(pos[0], pos[1], pos[2])),
        )
    }
    /// `LevelReader.enabledFeatures` of the level a handle names. Every level of a world shares
    /// the world's flags. None for a stale handle and on a client.
    pub(super) fn level_enabled_features(&self, level: LevelRef) -> Option<FeatureFlags> {
        self.native_level_world(level)?;
        Some(FeatureFlags::new(self.enabled_feature_ids().clone()))
    }
    /// Whether it thunders (`Level.isThundering`: a thunder level above 0.9).
    pub(super) fn level_is_thundering(&self, level: LevelRef) -> Option<bool> {
        let world = self.native_level_world(level)?;
        Some(
            weather_flags(weather_for_world(
                world,
                level.dimension() == Dim::End.key(),
            ))
            .1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::chunk::Chunk;
    use horizon_client::gamerules;
    use mod_api::testing::with_mods;
    use mod_api::{
        FeatureFlags, GameRuleChangedEvent, GameRuleValue, Hooks, LevelHost, ModEntry, ServerHost,
    };
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn dimension_shape_reads_current_parked_and_stale_levels() {
        use super::super::Dim;
        use horizon_client::chunk::LevelShape;
        use mod_api::{Host, LevelDimensionShape, NativeAuthority};

        with_game(|game| {
            game.world.shape = LevelShape::new(-32, 320, 256, true, false).unwrap();
            let overworld = game.level("minecraft:overworld").unwrap();
            let expected = LevelDimensionShape::new(-32, 320, 256, true, false);
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), Some(expected));
            game.enter_level(Dim::Nether).unwrap();
            game.world.shape = LevelShape::new(0, 256, 128, false, true).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), Some(expected));
            assert_eq!(
                nether.dimension_shape(host),
                Some(LevelDimensionShape::new(0, 256, 128, false, true))
            );
            game.native_lifecycle
                .begin_open(NativeAuthority::IntegratedServer);
            let host: &dyn Host = game;
            assert_eq!(overworld.dimension_shape(host), None);
            assert_eq!(nether.dimension_shape(host), None);
        });
    }

    /// A chunk of air with `biome` in every cell: full sky light everywhere.
    fn chunk_in(game: &super::Game<'_>, x: i32, biome: &str) -> Arc<Chunk> {
        let id = game.res.biomes().id(biome);
        let mut chunk = Chunk::empty(x, 0);
        for qy in 0..chunk.sections.len() as i32 * 4 {
            for (qx, qz) in (0..4).flat_map(|qx| (0..4).map(move |qz| (qx, qz))) {
                chunk.set_biome_cell(qx, qy, qz, id);
            }
        }
        Arc::new(chunk)
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn rain_at_a_position_checks_sky_roof_and_biome() {
        with_game(|game| {
            game.install_live_features();
            for (x, biome) in [
                (0, "minecraft:plains"),
                (1, "minecraft:desert"),
                (2, "minecraft:snowy_plains"),
            ] {
                let chunk = chunk_in(game, x, biome);
                game.world.chunks.insert((x, 0), chunk);
            }
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(3, 100, 3, stone).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let chunks = game.world.chunks.len();
            let plains = [1, 70, 1];
            let roofed = [3, 70, 3];
            let desert = [17, 70, 1];
            let snowy = [33, 70, 1];
            let unloaded = [16 * 9, 70, 1];

            game.world.weather.rain_level = (1.0, 1.0);
            for (pos, rains) in [
                (plains, true),
                (roofed, false),
                (desert, false),
                (snowy, false),
            ] {
                assert_eq!(game.level_is_raining_at(level, pos), Some(rains), "{pos:?}");
            }
            assert_eq!(game.level_is_raining_at(level, unloaded), None);
            assert_eq!(game.level_is_raining_at(level, [1, 4000, 1]), None);
            game.with_level(|_, host, _| {
                assert!(host.is_raining_at(plains));
                assert!(!host.is_raining_at(roofed));
                assert!(!host.is_raining_at(desert));
                assert!(!host.is_raining_at(snowy));
                assert!(!host.is_raining_at(unloaded));
            });

            game.world.weather.rain_level = (0.0, 0.0);
            assert_eq!(game.level_is_raining_at(level, plains), Some(false));
            game.with_level(|_, host, _| assert!(!host.is_raining_at(plains)));
            assert_eq!(game.world.chunks.len(), chunks, "nothing was loaded");
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn enabled_features_default_to_vanilla_and_follow_the_world() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let flags = game.level_enabled_features(level).unwrap();
            assert!(flags.contains("minecraft:vanilla"));
            assert!(flags.contains("vanilla"));
            assert!(!flags.contains("minecraft:redstone_experiments"));
            game.level.enabled_features = [
                "minecraft:vanilla".to_string(),
                "minecraft:redstone_experiments".to_string(),
            ]
            .into();
            let flags = game.level_enabled_features(level).unwrap();
            assert!(flags.contains("minecraft:redstone_experiments"));
            assert_eq!(flags.iter().count(), 2);
            assert_eq!(
                FeatureFlags::new(Arc::default()).iter().collect::<Vec<_>>(),
                ["minecraft:vanilla"]
            );
            let ids: Vec<String> = flags.iter().map(str::to_owned).collect();
            game.with_level(|_, host, _| {
                assert_eq!(host.enabled_features().iter().collect::<Vec<_>>(), ids);
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn native_mod_reads_registered_bool_rule_by_key() {
        use std::sync::OnceLock;
        static KEY: OnceLock<mod_api::BoolGameRuleKey> = OnceLock::new();
        struct RuleMod;
        #[mod_api::__subscriptions]
        impl Hooks for RuleMod {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                registrar.game_rule_category("machines").unwrap();
                let key = registrar
                    .game_rule_bool("enabled", "example:machines", true)
                    .unwrap();
                let _ = KEY.set(key);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("example", Box::new(RuleMod))].into();
        with_mods(mods, || {
            with_game(|game| {
                let registry = Arc::new(crate::game::api_values::game_rule_registry());
                game.level.rule_registry = registry.clone();
                game.ui.rules = gamerules::GameRules::with_registry(registry, &[]);
                let level = game.level("minecraft:overworld").unwrap();
                assert_eq!(level.game_rule_bool(game, *KEY.get().unwrap()), Some(true));
            })
        });
    }

    /// What a `game_rule_changed` listener read of the whole rule set: the changed rule, whether a
    /// player caused it, and `keep_inventory`, `random_tick_speed` and the rule count it saw.
    type RuleRead = (
        String,
        bool,
        Option<GameRuleValue>,
        Option<GameRuleValue>,
        usize,
    );
    struct RuleReader(Arc<Mutex<Vec<RuleRead>>>);
    #[mod_api::__subscriptions]
    impl Hooks for RuleReader {
        fn game_rule_changed(
            &self,
            host: &mut dyn ServerHost,
            event: &mut GameRuleChangedEvent<'_>,
        ) {
            let rules = host
                .level("minecraft:overworld")
                .and_then(|level| level.game_rules(host))
                .unwrap_or_default();
            let value = |id: &str| rules.iter().find(|(rule, _)| rule == id).map(|(_, v)| *v);
            self.0.lock().unwrap().push((
                event.rule.to_owned(),
                event.actor.is_some(),
                value("minecraft:keep_inventory"),
                value("minecraft:random_tick_speed"),
                rules.len(),
            ));
        }
    }

    /// A listener of a change no player made reads every server rule through its host: the new
    /// value and the others as they stand.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_game_rule_listener_reads_the_whole_rule_set() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "rule-reader",
            Box::new(RuleReader(seen.clone())),
        )]
        .into();
        with_mods(mods, || {
            with_game(|game| {
                assert!(
                    game.ui
                        .rules
                        .set("random_tick_speed", GameRuleValue::Int(7))
                );
                let keep_inventory = gamerules::rule_registry()
                    .rule("keep_inventory")
                    .unwrap()
                    .clone();
                game.set_game_rule(&keep_inventory, GameRuleValue::Bool(true), None);
            });
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [(
                "minecraft:keep_inventory".to_owned(),
                false,
                Some(GameRuleValue::Bool(true)),
                Some(GameRuleValue::Int(7)),
                // Vanilla 26.3 `GameRules` has 59, `max_minecart_speed` behind minecart improvements.
                58,
            )]
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn weather_read_resolves_levels_and_refuses_stale_or_unavailable_handles() {
        use super::super::execution_role::ExecutionRole;
        use mod_api::{LevelRef, NativeWorldEpoch};

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            game.world.weather.rain_level = (0.0, 0.5);
            game.world.weather.thunder_level = (0.0, 0.75);
            assert_eq!(level.weather(game), Some((0.5, 0.375)));
            let stale = LevelRef::new(
                NativeWorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            assert_eq!(stale.weather(game), None);
            let absent = LevelRef::new(
                level.epoch(),
                mod_api::__private::dimension_key("missing:level"),
            );
            assert_eq!(absent.weather(game), None);
            // The current world's dimension type, not its id alone, gates weather.
            game.world.shape.has_ceiling = true;
            assert_eq!(level.weather(game), Some((0.0, 0.0)));
            game.options.execution_role = ExecutionRole::Client;
            assert_eq!(level.weather(game), None, "no logged-in remote level");
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "WeatherTest").unwrap();
            assert_eq!(level.weather(game), None, "login has not completed");
            game.remote.as_mut().unwrap().logged_in = true;
            game.world.weather.rain_level = (0.0, 0.5);
            game.world.weather.thunder_level = (0.0, 0.75);
            assert_eq!(level.weather(game), Some((0.5, 0.375)));
            let other = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(
                other.weather(game),
                None,
                "client shows only its current level"
            );
            assert_eq!(stale.weather(game), None);
            // Only the weather pair is a both-side read; the server queries keep their guard.
            assert_eq!(game.level_is_raining(level), None);
            assert_eq!(game.level_is_thundering(level), None);
        });
    }

    #[test]
    fn weather_requires_skylight_no_ceiling_and_not_end() {
        use horizon_client::chunk::LevelShape;
        let rain = (1.0, 0.5);
        assert_eq!(
            super::weather_for_level(LevelShape::OVERWORLD, false, rain),
            rain
        );
        assert_eq!(
            super::weather_for_level(LevelShape::NETHER, false, rain),
            (0.0, 0.0)
        );
        assert_eq!(
            super::weather_for_level(LevelShape::END, true, rain),
            (0.0, 0.0)
        );
        let mut skyless = LevelShape::OVERWORLD;
        skyless.sky_light = false;
        assert_eq!(super::weather_for_level(skyless, false, rain), (0.0, 0.0));
        assert_eq!(
            super::weather_for_level(LevelShape::OVERWORLD.with_ceiling(true), false, rain),
            (0.0, 0.0)
        );
    }

    #[test]
    fn rain_and_thunder_start_above_vanillas_thresholds() {
        assert_eq!(super::weather_flags((0.2, 0.0)), (false, false));
        assert_eq!(super::weather_flags((0.21, 0.9)), (true, false));
        assert_eq!(super::weather_flags((1.0, 0.95)), (true, true));
    }
}
