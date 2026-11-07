/// The structure box a piece's `bounding_box` answers, in world blocks.
pub use horizon_core::bounding_box::BoundingBox;
pub use horizon_core::equipment_slot::EquipmentSlot;
#[horizon_mod_api_macros::wasm(via = String)]
pub use horizon_core::identifier::Identifier;
#[horizon_mod_api_macros::wasm(via = Tag)]
pub use horizon_core::nbt::Compound;
#[horizon_mod_api_macros::wasm(via = Vec<u8>)]
pub use horizon_core::nbt::Tag;
pub use horizon_core::use_animation::UseAnimation;
pub use {
    horizon_core::{
        connection_kind::ConnectionKind, dimension::Dimension as LevelDimension,
        neighbour_update::NeighbourUpdate,
    },
    horizon_mod_runtime::attachments::{
        AttachmentCodec, AttachmentError, AttachmentForm, AttachmentPolicy, AttachmentStore,
        AttachmentType, AttachmentValue, AttachmentWrite, Holder,
    },
    horizon_world::{
        level::{BlockId, LandingWrite},
        loot::{LootStack, NativeLootContext},
        registry::BlockStateId,
    },
};
// hidden: the game's maps behind the store, not part of the mod surface
#[doc(hidden)]
pub use horizon_mod_runtime::attachments::{AttachmentAccess, AttachmentHost, Attachments};
