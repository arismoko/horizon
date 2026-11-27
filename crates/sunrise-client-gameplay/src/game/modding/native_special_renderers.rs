//! Game-side stack and draw context for special item renderers. The client item renderer owns
//! scoped submission, rollback and the session's callback failure policy.
use mod_api::{ItemDisplayContext, ItemStack, SpecialModelContext};
use std::num::NonZeroU32;
use sunrise_assets::item_model::{Context, DisplayContext, StackView};
use sunrise_client::item_render::SpecialModelInput;
use sunrise_gameplay_common::game::api_values::{engine_display_context, stack_value};
use sunrise_sim::item_models::stack_view;

pub(in crate::game::client_side) fn draw_input<'a>(
    game: &'a crate::game::ClientGame<'_>,
    stack: &StackView,
    context: &Context<'_>,
) -> Option<SpecialModelInput<'a>> {
    // Match vanilla's `ItemModelResolver.updateForTopItem`: empty stacks are skipped cleanly.
    let count = u32::try_from(stack.count).ok().and_then(NonZeroU32::new)?;
    let item = mod_api::__private::item_key(&stack.item)?;
    let Ok(display) =
        crate::game::client_side::item_model_context::display_context(&context.display);
    Some(SpecialModelInput {
        host: game,
        item,
        count,
        context: SpecialModelContext::new(
            display,
            context.level.map(|level| level.game_time),
            game.frame_world_partial_tick() as f32,
        ),
    })
}

/// Convert only an explicitly submitted nested stack; ordinary input stays borrowed.
pub(in crate::game::client_side) fn nested_stack(
    stack: &ItemStack,
    display: ItemDisplayContext,
) -> Option<(StackView, DisplayContext)> {
    let display = engine_display_context(display)?;
    // The item resolver's stack count is signed.
    i32::try_from(stack.count()).ok()?;
    let stack = stack_value(stack).ok()?;
    Some((stack_view(&stack), display))
}
