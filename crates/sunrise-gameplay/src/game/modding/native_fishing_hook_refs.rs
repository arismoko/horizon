//! Fishing-hook handles for native server mods. A cast hook lives with the
//! player who cast it, not in a level list, so a handle finds it through them.
use super::commands::Target;
use mod_api::{EntityRef, FishingHookRef};

impl crate::game::ServerGame<'_> {
    /// Narrows an entity to a cast fishing hook. None for another kind, a
    /// stale handle, a client, or a hook taken out for its tick.
    pub fn fishing_hook(&self, entity: EntityRef) -> Option<FishingHookRef> {
        matches!(self.native_entity_target(entity)?, Target::Hook(_))
            .then(|| FishingHookRef::new(entity))
    }

    /// `FishingHook.getPlayerOwner`: the player who cast it. None for a stale
    /// handle.
    pub(in crate::game) fn fishing_hook_player_owner(
        &self,
        hook: FishingHookRef,
    ) -> Option<EntityRef> {
        self.fishing_hook(hook.entity())?;
        let (_, owner) = self.fishing_hook_by_id(hook.entity().id())?;
        let id = match owner {
            // The one whose turn it is: their handle is the acting entity.
            Target::Player => self.server.lan_acting_entity?,
            guest => u32::try_from(self.network_entity_id(guest)?).ok()?,
        };
        self.entity(id)
    }

    /// `FishingHook.getHookedIn`: what it holds. None when it holds nothing,
    /// what it holds is gone, or for a stale handle.
    pub(in crate::game) fn fishing_hook_hooked_in(
        &self,
        hook: FishingHookRef,
    ) -> Option<EntityRef> {
        self.fishing_hook(hook.entity())?;
        let (found, _) = self.fishing_hook_by_id(hook.entity().id())?;
        self.entity(found.hooked_in()?)
    }

    /// `FishingHook.isOpenWaterFishing`, which the `fishing` loot's
    /// `in_open_water` predicate reads. None for a stale handle.
    pub(in crate::game) fn fishing_hook_open_water(&self, hook: FishingHookRef) -> Option<bool> {
        self.fishing_hook(hook.entity())?;
        let (found, _) = self.fishing_hook_by_id(hook.entity().id())?;
        Some(found.open_water())
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
