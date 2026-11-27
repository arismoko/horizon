//! Network ids of registry entries for mods (`registry_protocol_id`).
use sunrise_core::identifier::Identifier;

pub trait NativeRegistryIdsGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The network id of `name` in `registry`: a client's own connection's registries (never the
    /// integrated server's), a server's synchronized ones, as its connections are sent them.
    /// None with no connection.
    fn registry_protocol_id(&self, registry: &Identifier, name: &Identifier) -> Option<i32> {
        if let Some(names) = self.side_remote_registry_names() {
            return names.id(registry, name);
        }
        self.side_server_registry_protocol_id(registry, name)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeRegistryIdsGameExt1<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;
