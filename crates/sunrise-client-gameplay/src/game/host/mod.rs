#[cfg(feature = "client")]
use super::ClientGame;
use mod_api::__private::game::*;
use mod_api::*;

#[cfg(feature = "client")]
impl Host for ClientGame<'_> {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}

#[cfg(feature = "client")]
impl ClientHost for ClientGame<'_> {
    fn client_backend_mut(&mut self) -> Option<ClientBackendMutRef<'_>> {
        Some(ClientBackendMutRef::of(self))
    }
}
#[cfg(feature = "client")]
impl Backend for ClientGame<'_> {}
#[cfg(feature = "client")]
impl ClientBackendMut for ClientGame<'_> {}
#[cfg(feature = "client")]
sunrise_gameplay_common::game::native_tags::impl_tags!(
    ClientGame<'_>,
    game,
    sunrise_gameplay_common::game::tables::WorldTablesExt::native_tag_items(game),
    sunrise_gameplay_common::game::tables::WorldTablesExt::loaded_biome_keys(game)
);

include!(concat!(env!("OUT_DIR"), "/game_host_generated.rs"));

pub(crate) mod services;

pub(in crate::game) mod phase_contexts;

pub mod draw;
