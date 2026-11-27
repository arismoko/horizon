//! Placement transport state belongs to the actual connection and open menu.
use std::collections::{BTreeMap, HashMap};
use sunrise_sim::recipe_book::{ModBookIndex, placement_sync as sync};

// no wire: live connection authority and validated engine presentation caches.
#[derive(Default)]
pub struct State {
    pub client: sync::Client,
    pub servers: HashMap<u32, Server>,
    pub view: Option<View>,
}
// no wire: source ordinals actually published to this connection.
pub struct Server {
    pub peer: sync::Authority,
    pub displays: BTreeMap<String, u32>,
}
impl Server {
    pub fn new(epoch: u64) -> Self {
        Self {
            peer: sync::Authority::new(epoch),
            displays: BTreeMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.peer.clear();
        self.displays.clear();
    }
}
// no wire: an installed, validated catalogue, not an untrusted transport body.
pub struct View {
    pub identity: sync::CatalogIdentity,
    pub index: ModBookIndex,
}
