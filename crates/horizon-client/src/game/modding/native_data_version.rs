//! The saves' data version for mods that write the game's formats themselves (schematics).
use super::Game;

impl Game<'_> {
    /// `SharedConstants.getCurrentVersion().dataVersion().version()`.
    pub(super) fn server_data_version(&self) -> i32 {
        horizon_world::DATA_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// The version a mod reads is the one the game's own saves carry.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_data_version_is_the_saves_own() {
        with_guests(|game| {
            let version = game.server_data_version();
            assert!(version > 4000, "{version}");
            assert_eq!(game.stats.to_json()["DataVersion"], version);
        });
    }
}
