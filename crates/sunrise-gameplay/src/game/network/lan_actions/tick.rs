#[cfg(any(test, feature = "test-support"))]
impl crate::game::ServerGame<'_> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn tick_guests(&mut self) {
        use crate::game::server_side::lan_actions_server::building_cache;
        building_cache(self.server.lan.as_ref());
        self.tick_guests_inner();
        building_cache(None);
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "tests/dig_tick_order_tests.rs"]
pub mod dig_tick_order_tests;
