//! Platform callbacks for engine fixtures shared by the gameplay and shell tests.

use crate::{__private::RuleSchema, DimensionSettings, ModEntry, Mods};
use std::sync::OnceLock;

pub type RegisterGameKeys =
    fn(Vec<String>, &[&str], &RuleSchema<'_>, fn(&DimensionSettings) -> bool);

#[non_exhaustive]
pub struct EngineTestPlatform {
    register_game_keys: RegisterGameKeys,
    install_parts_fixture: fn(&[ModEntry]) -> Result<(), Vec<String>>,
    with_mods: fn(Mods, Box<dyn FnOnce() + '_>),
}

impl EngineTestPlatform {
    pub fn new(
        register_game_keys: RegisterGameKeys,
        install_parts_fixture: fn(&[ModEntry]) -> Result<(), Vec<String>>,
        with_mods: fn(Mods, Box<dyn FnOnce() + '_>),
    ) -> Self {
        Self {
            register_game_keys,
            install_parts_fixture,
            with_mods,
        }
    }
}

#[cfg(any(test, feature = "testing"))]
static PLATFORM: OnceLock<EngineTestPlatform> = OnceLock::new();

pub fn install(platform: EngineTestPlatform) {
    assert!(
        PLATFORM.set(platform).is_ok(),
        "engine test platform installed twice"
    );
}

fn platform() -> &'static EngineTestPlatform {
    PLATFORM
        .get()
        .expect("engine test platform is not installed")
}

pub fn register_game_keys(
    ids: Vec<String>,
    mod_fluids: &[&str],
    schema: &RuleSchema<'_>,
    dimension_shape: fn(&DimensionSettings) -> bool,
) {
    (platform().register_game_keys)(ids, mod_fluids, schema, dimension_shape);
}

pub fn install_parts_fixture(mods: &[ModEntry]) -> Result<(), Vec<String>> {
    (platform().install_parts_fixture)(mods)
}

pub fn with_mods(mods: Mods, run: impl FnOnce()) {
    (platform().with_mods)(mods, Box::new(run));
}
