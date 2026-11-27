use super::*;
use sunrise_gameplay_common::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;
use sunrise_items::creative_catalog::{Catalog, Holders};

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn server_creative_holders(&self) -> Option<Holders> {
        self.side_registry_sync()
            .map(|sync| sync.creative_holders())
    }

    pub(in crate::game) fn server_creative_catalog(&self) -> &Arc<Catalog> {
        &self.server.creative_catalog.0
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_set_creative_catalog(&mut self, catalog: Arc<Catalog>) {
        self.server.creative_catalog.0 = catalog;
    }
}
