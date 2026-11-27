use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShearDispatchRefused;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'g>: crate::game::SideGame<'g> {
    /// `IShearable.onSheared` on the single entity part at `part_slot` whose `is_shearable`
    /// answered `Some(true)`. Both sides.
    fn native_on_sheared(
        &mut self,
        entity: EntityRef,
        player: Option<PlayerRef>,
        shears: mod_api::ItemStackView<'_>,
        drops: &mut Vec<mod_api::ItemStack>,
        part_slot: (usize, usize),
        entity_kind: &str,
    ) -> Result<(), ShearDispatchRefused> {
        mod_api::dispatch_on_sheared(
            crate::game::execution_role::host_side(self),
            entity,
            player,
            shears,
            drops,
            part_slot,
            entity_kind,
        )
        .map_err(|()| ShearDispatchRefused)
    }
}

impl<'g, G: crate::game::SideGame<'g>> GameHooks<'g> for G {}
