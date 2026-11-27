use std::collections::HashMap;

#[derive(Default)]
pub struct NativeNet {
    pub placement: super::native_book_placement::State,
    pub book_connections: HashMap<u32, sunrise_sim::recipe_book::settings_connection::Connection>,
    pub book_view: sunrise_sim::recipe_book::settings_sync::ClientView,
    pub book_request: u64,
    pub book_ready: bool,
}
