//! Metadata reads use the bootstrap snapshot, never the world's logical side.

impl crate::game::CommonGame<'_> {
    pub fn dist(&self) -> mod_api::Dist {
        sunrise_level::runtime_env::runtime_environment::get().dist()
    }
    pub fn game_version(&self) -> &'static str {
        sunrise_level::runtime_env::runtime_environment::get().game_version()
    }
    pub fn server_data_version(&self) -> i32 {
        sunrise_core::version::DATA_VERSION
    }
    pub fn sunrise_version(&self) -> &'static str {
        sunrise_level::runtime_env::runtime_environment::get().sunrise_version()
    }
    pub fn game_tests_enabled(&self) -> bool {
        sunrise_level::runtime_env::runtime_environment::get().game_tests_enabled()
    }
}
