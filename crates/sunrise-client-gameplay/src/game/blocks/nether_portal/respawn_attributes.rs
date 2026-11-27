//! What a client's new player takes from the old one when a `ClientboundRespawnPacket`
//! replaces it (`ClientPacketListener.handleRespawn`): `AttributeMap.assignAllValues` when the
//! packet keeps the attribute modifiers (a level change, through a portal or not: the worn
//! armour's among them), `assignBaseValues` after a death. The new player is made once its
//! position arrives, so what the server sends for it in between waits here, as vanilla's new
//! `LocalPlayer` already holds it.
use sunrise_protocol::packets::play::AttributeSnapshot;
use sunrise_world::attributes::Instance;

/// The respawn in progress (`Game::portals`).
#[derive(Default)]
pub(in crate::game) struct RespawnAttributes {
    /// A respawn packet came: whether it keeps the modifiers.
    pub(in crate::game) keep_modifiers: Option<bool>,
    pub(in crate::game) carried: Option<Carried>,
}

pub(in crate::game) struct Carried {
    pub(in crate::game) keep_modifiers: bool,
    pub(in crate::game) instances: Vec<(String, Instance)>,
    /// `UpdateAttributes` of the player's own entity that came before its new player.
    pub(in crate::game) early: Vec<Vec<AttributeSnapshot>>,
}
