use sunrise_client::item_tooltip::{
    AttributeFilter, AttributeTooltipContext, AttributeTooltipListener,
};
use sunrise_game_data::text::Text;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
use sunrise_items::items::ItemStack;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `AddAttributeTooltipsEvent`: the lines that follow a stack's attribute lines. Push lines
    /// to `lines`, each with a colour as `rgba` packs it (`u32::MAX` keeps the text's own).
    /// `shown` is whether the stack's display shows the attribute modifiers; the hook is asked
    /// either way. `player` and `player_entity_id` are the local player's, or `None` for recipe
    /// book search and a stack nested in another's tooltip (a crossbow's projectile). Asked when
    /// lines are built for display, recipe book search or `ItemResource::tooltip`. Display and
    /// resource-query builds skip it when the stack hides its entire tooltip. Reentrant line
    /// queries do not dispatch it. Client-side, only while a mod listens.
    pub(in crate::game) fn native_add_attribute_tooltips(
        &self,
        call: mod_api::__private::AddAttributeTooltipsCall<'_>,
    ) {
        self.fire_add_attribute_tooltips(call);
    }
}

/// The attribute tooltip listener of the game: it fires `gather_skipped_attribute_tooltips` and
/// `add_attribute_tooltips` for item tooltip line builds and the recipe book's search text.
/// `player` is the local player's uuid and entity id, none for the search
/// text and for a nested stack (a crossbow's projectile).
#[cfg(feature = "client")]
pub(in crate::game) struct HookedAttributeTooltips<'a, 'g> {
    game: &'a crate::game::ClientGame<'g>,
    player: Option<(u128, i32)>,
}

#[cfg(feature = "client")]
impl<'a, 'g> HookedAttributeTooltips<'a, 'g> {
    #[cfg(feature = "client")]
    pub(in crate::game) fn new(
        game: &'a crate::game::ClientGame<'g>,
        player: Option<(u128, i32)>,
    ) -> Self {
        Self { game, player }
    }
}

#[cfg(feature = "client")]
impl AttributeTooltipListener for HookedAttributeTooltips<'_, '_> {
    fn skipped(&self, stack: &ItemStack, ctx: &AttributeTooltipContext) -> AttributeFilter {
        let Ok(stack) = stack.try_into_api() else {
            return AttributeFilter::default();
        };
        let (mut skip_all, mut ids, mut groups) = (false, Vec::new(), Vec::new());
        self.game.native_gather_skipped_attribute_tooltips(
            mod_api::__private::GatherSkippedAttributeTooltipsCall {
                player: self.player.map(|p| p.0),
                player_entity_id: self.player.map(|p| p.1),
                stack: &stack,
                advanced: ctx.advanced,
                creative: ctx.creative,
                skip_all: &mut skip_all,
                skipped_ids: &mut ids,
                skipped_groups: &mut groups,
            },
        );
        AttributeFilter {
            skip_all,
            ids,
            groups,
        }
    }

    fn extra(
        &self,
        stack: &ItemStack,
        ctx: &AttributeTooltipContext,
        shown: bool,
    ) -> Vec<(Text, u32)> {
        let Ok(stack) = stack.try_into_api() else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        self.game
            .native_add_attribute_tooltips(mod_api::__private::AddAttributeTooltipsCall {
                player: self.player.map(|p| p.0),
                player_entity_id: self.player.map(|p| p.1),
                stack: &stack,
                shown,
                advanced: ctx.advanced,
                creative: ctx.creative,
                lines: &mut lines,
            });
        // Refuse the appender's whole writeback if any line cannot cross.
        lines.try_into_engine(&()).unwrap_or_default()
    }
}
