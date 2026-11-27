//! Weather and sky light for the player's level: whether it can have weather, the rain and thunder
//! levels, where it rains or snows (`Level.isRainingAt`, the motion-blocking height), the weather's
//! effects each tick and `Level.getSkyDarken` with `isDarkOutside`.
use super::*;
use crate::game::end_common::EndGameExt as _;
use crate::game::side_ops::world::SideOps as _;

/// `ClientLevel.getPrecipitationAt`: a loaded column's biome and the level's sea level
/// decide whether it snows, rains, or has no precipitation.
pub fn precipitation_at(
    world: &World,
    biomes: &Biomes,
    zoom: BiomeZoom,
    pos: BlockPos,
    sea_level: i32,
) -> Option<bool> {
    if !world.chunks.contains_key(&(pos.x >> 4, pos.z >> 4)) {
        return None;
    }
    // `LevelReader.getBiome` zooms first. If that quart's chunk is missing,
    // `ClientLevel.getUncachedNoiseBiome` supplies plains.
    let [x, y, z] = zoom.quart_at_block(pos).map(|q| q << 2);
    let id = world
        .noise_biome(x, y, z)
        .unwrap_or_else(|| biomes.id("minecraft:plains"));
    biomes.get(id).snows_at(pos, sea_level)
}

/// What `Level.precipitationAt` reads of the level in place: it rains (the rain level above 0.2),
/// the sky light is 15, the motion-blocking height, and the biome's precipitation there
/// (`ClientLevel.getPrecipitationAt`'s zoomed biome; snow is `Some(true)` there).
struct GamePrecipitation<'a, B, Z, L, R> {
    world: &'a World,
    properties: &'a sunrise_level::assets::Properties,
    biomes: B,
    biome_zoom: Z,
    sea_level: L,
    rain: R,
}

impl<'a, B, Z, L, R> sunrise_world::level::weather::PrecipitationReads
    for GamePrecipitation<'a, B, Z, L, R>
where
    B: Fn() -> &'a Biomes,
    Z: Fn() -> BiomeZoom,
    L: Fn() -> i32,
    R: Fn() -> bool,
{
    fn is_raining(&self) -> bool {
        (self.rain)()
    }
    fn can_see_sky(&self, p: BlockPos) -> bool {
        self.world.light(p.x, p.y, p.z) >> 4 == 15
    }
    fn motion_blocking_y(&self, p: BlockPos) -> i32 {
        crate::game::weather_height::motion_blocking_height(self.world, self.properties, p.x, p.z)
    }
    fn biome_precipitation(&self, p: BlockPos) -> sunrise_world::level::PrecipitationKind {
        use sunrise_world::level::PrecipitationKind;
        match precipitation_at(
            self.world,
            (self.biomes)(),
            (self.biome_zoom)(),
            p,
            (self.sea_level)(),
        ) {
            Some(true) => PrecipitationKind::Snow,
            Some(false) => PrecipitationKind::Rain,
            None => PrecipitationKind::None,
        }
    }
}

pub trait WeatherGameExt<'__sg>: crate::game::SideGame<'__sg> {
    /// The joined `ClientLevel`'s sea level, or the server level's generator setting.
    fn weather_sea_level(&self) -> i32 {
        self.side_state()
            .side_remote_sea_level()
            .unwrap_or_else(|| self.dim.sea_level())
    }

    /// `Level.isDarkOutside`.
    fn dark_outside(&self) -> bool {
        !self.dimension_type().has_fixed_time && self.sky_darken() >= 4
    }

    fn can_have_weather(&self) -> bool {
        self.world.shape.can_have_weather(self.dim == Dim::End)
    }

    /// The rain and thunder levels (`Level.getRainLevel`, `getThunderLevel`):
    /// the world's, where the level can have weather, else none.
    fn weather_levels(&self, partial: f32) -> (f32, f32) {
        if self.can_have_weather() {
            self.world.weather.levels(partial)
        } else {
            (0.0, 0.0)
        }
    }

    /// `Level.getSkyDarken`: the same dimension, time and weather calculation used by gameplay.
    fn sky_darken_in(&self, world: &World, dimension: Dim) -> i32 {
        // A client of a server has the environment its server's type made for the level it is
        // in; a parked level keeps the client's own for its dimension.
        let environment = match self
            .side_state()
            .side_remote_environment()
            .filter(|_| dimension == self.dim)
        {
            Some(own) => own,
            None => {
                let base = match dimension {
                    Dim::Overworld | Dim::Extra(_) => self.res.environment(),
                    Dim::Nether => self.res.nether_environment(),
                    Dim::End => self.res.end_environment(),
                };
                sunrise_level::environment::Environment::for_level(dimension.id(), base)
            }
        };
        let (rain, thunder) = crate::game::mod_api_host_common::weather_for_level(
            world.shape,
            dimension == Dim::End,
            self.world.weather.levels(1.0),
        );
        let clock = |name: &str| {
            if dimension == self.dim
                && let Some(time) = self.side_state().side_remote_named_clock_time(name)
            {
                time
            } else {
                self.command_clock_time(name)
            }
        };
        sunrise_level::environment::sky_darken_with_clock(environment, rain, thunder, &clock)
    }

    fn sky_darken(&self) -> i32 {
        if self.dim == Dim::Nether {
            return 11;
        }
        if self.dim == Dim::End {
            let clock = |name: &str| {
                self.side_state()
                    .side_remote_named_clock_time(name)
                    .unwrap_or_else(|| self.command_clock_time(name))
            };
            return sunrise_level::environment::sky_darken_with_clock(
                self.dim_environment(),
                0.0,
                0.0,
                &clock,
            );
        }
        let (rain, thunder) = self.weather_levels(1.0);
        // Sampled once per (dimension, time, weather): every mob asks each tick.
        let key = (
            self.dim,
            self.world.day_time,
            rain.to_bits(),
            thunder.to_bits(),
        );
        if let Some((k, darken)) = self.sky_darken_memo.get()
            && k == key
        {
            return darken;
        }
        let darken = self.sky_darken_in(&self.world, self.dim);
        self.sky_darken_memo.set(Some((key, darken)));
        darken
    }

    /// `Level.isRainingAt`: `Level.precipitationAt`
    /// (`sunrise_world::level::weather::precipitation_at`) is rain.
    fn raining_at(&self, p: BlockPos) -> bool {
        let reads = GamePrecipitation {
            world: &self.world,
            properties: &self.pack.assets.properties,
            biomes: || world_biomes!(self).as_ref(),
            biome_zoom: || self.side_state().streaming_state().biome_zoom,
            sea_level: || self.weather_sea_level(),
            rain: || self.weather_levels(1.0).0 > 0.2,
        };
        sunrise_world::level::weather::precipitation_at(&reads, p)
            == sunrise_world::level::PrecipitationKind::Rain
    }

    /// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)`: one above the
    /// highest block in `blocks_motion_in_heightmap` or holding a fluid.
    fn rain_height(&self, x: i32, z: i32) -> i32 {
        crate::game::weather_height::motion_blocking_height(
            &self.world,
            &self.pack.assets.properties,
            x,
            z,
        )
    }
}
impl<'__sg, T: crate::game::SideGame<'__sg>> WeatherGameExt<'__sg> for T {}
