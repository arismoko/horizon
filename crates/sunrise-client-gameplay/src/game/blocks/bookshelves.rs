//! Client-side enchanting particle power cache; shared queries live in `bookshelves_common.rs`.
use crate::game::ClientGame;
use sunrise_core::position::BlockPos;

impl ClientGame<'_> {
    /// Snapshot at world entry/parts reload, or after received tags and tag-update listeners
    /// are published. Particle workers receive state answers, never a host or mod callback.
    pub(in crate::game::client_side) fn resolve_enchant_particles(&self) {
        if self.common.is_logical_server() {
            return;
        }
        let enchant_power = &self.client.ambience.enchant_power;
        enchant_power.set_hooked(mod_api::__private::listened(
            mod_api::HookId::IsValidBookShelf,
        ));
        enchant_power.rebuild(
            self.common.pack.assets.registry.len(),
            |state| {
                let origin = BlockPos::new(0, 0, 0);
                // An already-active provider supplies no particle source.
                self.enchant_power_for_table(None, state, origin, origin)
                    .unwrap_or_default()
            },
            |state| self.enchant_transmits(state),
        );
    }
}

use sunrise_gameplay_common::game::bookshelves_common::BookshelvesCommonGameExt1 as _;
