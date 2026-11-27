#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_core::{geometry::Vec3, random::RandomSource};

    /// A mob of the prepared resources' type `kind` (None without them).
    pub fn mob(kind: &str) -> Option<sunrise_sim::mobs::Mob> {
        let root = std::env::var_os("SUNRISE_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        let types = sunrise_sim::mobs::MobTypes::load(&root).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        Some(sunrise_sim::mobs::Mob::new(
            7,
            t,
            sunrise_core::geometry::Vec3::new(1.5, 64.0, -2.5),
            0.0,
        ))
    }
    pub fn block_of(at: Vec3) -> [i32; 3] {
        [
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        ]
    }
    /// `AbstractHorse.handleEating`'s growth particle position draws:
    /// `getRandomX(1.0)`, `getRandomY()` and `getRandomZ(1.0)` take a double each.
    pub fn sparkle_draws(r: &mut sunrise_core::random::LegacyRandom) {
        for _ in 0..3 {
            r.next_double();
        }
    }
}
