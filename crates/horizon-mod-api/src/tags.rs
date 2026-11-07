//! Resolved tag names and the read-only merged-tag service.
use crate::{FluidKey, Identifier, ItemKey};

/// A process-lifetime tag name resolved once through [`Tags::key`].
/// Membership belongs to the current authoritative data reload, not to the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: the runtime alone creates name slots
#[horizon_mod_api_macros::wasm(host_constructor = from_slot)]
pub struct TagKey(pub(crate) u32);

impl TagKey {
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        horizon_mod_runtime::registry_keys::with_tag_name(slot, |id| {
            id.map(|_| Self(slot)).ok_or("unknown TagKey slot")
        })
    }
}

/// Read merged item and fluid tags without borrowing the game's registry structs.
/// Unknown tags answer false or an empty member list. Membership reads answer `None`
/// on clients, before authoritative tags are loaded, or while their key table is unavailable.
pub trait Tags {
    /// Resolve a validated name once. The name need not currently have members;
    /// retain its key across reloads. None if the runtime cannot allocate a name slot.
    fn key(&self, _id: &Identifier) -> Option<TagKey> {
        crate::phase::unprovided("Tags::key", "tags_key", crate::phase::LoadAccess::Metadata)
    }
    /// Whether `item` is in the current item tag. An unknown tag is false;
    /// unavailable authoritative tags or item mappings answer None.
    fn is_in_tag(&self, _item: ItemKey, _tag: TagKey) -> Option<bool> {
        crate::phase::unprovided(
            "Tags::is_in_tag",
            "tags_is_in_tag",
            crate::phase::LoadAccess::Metadata,
        )
    }
    /// Item members in vanilla's order, including unrequested vanilla items.
    /// Unknown tags are empty; unavailable authoritative tags or item mappings answer None.
    fn items(&self, _tag: TagKey) -> Option<Vec<ItemKey>> {
        crate::phase::unprovided(
            "Tags::items",
            "tags_items",
            crate::phase::LoadAccess::Metadata,
        )
    }
    /// Fluid members in vanilla's order. Unknown tags are empty;
    /// unavailable authoritative tags or fluid mappings answer None.
    fn fluids(&self, _tag: TagKey) -> Option<Vec<FluidKey>> {
        crate::phase::unprovided(
            "Tags::fluids",
            "tags_fluids",
            crate::phase::LoadAccess::Metadata,
        )
    }
}

/// Resolve a tag name at the game boundary.
pub fn tag_key(id: &Identifier) -> Option<TagKey> {
    horizon_mod_runtime::registry_keys::tag_slot(id).map(TagKey)
}

/// Borrow a tag's canonical name within the runtime's read lock, without formatting or cloning.
pub fn with_tag_name<R>(key: TagKey, read: impl FnOnce(Option<&str>) -> R) -> R {
    horizon_mod_runtime::registry_keys::with_tag_name(key.0, read)
}
