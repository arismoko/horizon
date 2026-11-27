//! Item registry numbers shared by API metadata reads and the game's scatter seeds.

impl crate::game::CommonGame<'_> {
    pub fn item_registry_id(&self, key: mod_api::ItemKey) -> Option<u32> {
        self.item_number(mod_api::__private::item_key_id(key)?)
    }

    pub fn item_number(&self, item: &str) -> Option<u32> {
        // This side's prepared report includes mod items after vanilla, in content-plan order.
        self.pack.item_registry_id(item)
    }
}
