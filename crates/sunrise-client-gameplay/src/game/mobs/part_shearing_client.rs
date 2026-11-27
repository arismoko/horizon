//! Client-side shear prediction for entity parts (`ShearsItem.interactLivingEntity`).
use crate::game::ClientGame;
use mod_api::PlayerRef;
use sunrise_gameplay_common::game::{
    api_values::{project_stack, stack_read_metadata},
    held_items::HeldItemsGameExt1 as _,
    native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
    native_hooks::{is_shearable::GameHooks as _, on_sheared::GameHooks as _},
    player_identity::PlayerIdentityGameExt1 as _,
    side_ops::presentation::PresentationSideGameExt as _,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::Hand;

impl ClientGame<'_> {
    fn part_shearable(
        &mut self,
        index: usize,
        player: Option<PlayerRef>,
        shears: &ItemStack,
    ) -> Option<((usize, usize), bool)> {
        if !self.common.mobs[index].alive() {
            return None;
        }
        let kind = std::sync::Arc::clone(&self.common.mobs[index].kind);
        let kind = &kind.id;
        if !mod_api::__private::has_scoped_hook(mod_api::HookId::IsShearable)
            || !mod_api::__private::entity_part_hooked(
                mod_api::HookId::IsShearable,
                "is_shearable",
                kind,
            )
        {
            return None;
        }
        let entity = self.entity(self.common.mobs[index].id)?;
        let metadata = stack_read_metadata(shears).ok()?;
        let project = |metadata| project_stack(shears, metadata).ok();
        let shears_view = mod_api::ItemStackView::resident(metadata, &project);
        let mut shearable = None;
        let slot = self.native_is_shearable(entity, player, shears_view, &mut shearable, kind)?;
        shearable.map(|answer| (slot, answer))
    }

    /// `ShearsItem.interactLivingEntity` on a client that predicts the click: both sides ask
    /// `isShearable` and call `onSheared` (its drops are the server's alone to spawn, and the
    /// shears are damaged there). True when a part sheared the mob, ending the click as the
    /// server's SUCCESS does.
    pub(in crate::game) fn predict_part_shear(&mut self, index: usize, hand: Hand) -> bool {
        let Some(shears) = self.with_hand(hand, |g| g.common.held().cloned()) else {
            return false;
        };
        if shears.item.as_ref() != "minecraft:shears" {
            return false;
        }
        let player = self.local_player();
        let Some((part, true)) = self.part_shearable(index, player, &shears) else {
            return false;
        };
        let (id, kind) = (
            self.common.mobs[index].id,
            std::sync::Arc::clone(&self.common.mobs[index].kind),
        );
        let (Some(entity), Ok(metadata)) = (self.entity(id), stack_read_metadata(&shears)) else {
            return false;
        };
        let project = |metadata| project_stack(&shears, metadata).ok();
        let view = mod_api::ItemStackView::resident(metadata, &project);
        // The drops are the server's: this copy only changes itself.
        let mut drops = Vec::new();
        if self
            .native_on_sheared(entity, player, view, &mut drops, part, &kind.id)
            .is_err()
        {
            return false;
        }
        self.with_hand(hand, |g| g.swing_arm());
        true
    }
}
