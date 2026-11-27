use super::super::*;
use super::{CartKind, Minecart};

/// A lone TNT cart at (0.5, 64, 0.5); a silent one keeps `Silent:1b` where
/// `Entity.saveWithoutId` does.
pub fn tnt_cart(game: &mut crate::game::ServerGame<'_>, id: u32, silent: bool) {
    game.common.carts.clear();
    game.common
        .carts
        .push(Minecart::new(id, CartKind::Tnt, Vec3::new(0.5, 64.0, 0.5)));
    game.common.persist.raw.remove(&id);
    game.server.lan_entity_events.clear();
    if silent {
        let (tag, _) = sunrise_core::nbt::parse_snbt("{Silent:1b}").unwrap();
        game.common.persist.raw.insert(id, tag);
    }
}
