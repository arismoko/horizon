//! Native display handles read and write the resident `SynchedEntityData` fields.
use super::commands::Target;
use mod_api::{
    Billboard, DisplayKind, DisplayRef, EntityRef, ItemDisplayContext, ItemStackView,
    TextAlignment, Transformation,
};
use std::sync::Arc;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::api_values::{project_stack, stack_read_metadata};
use sunrise_items::items::ItemStack;
use sunrise_sim::text_display_data::resident::{
    DisplayComponent, DisplayContents, DisplayState, DisplayText,
};
use sunrise_world::registry::BlockStateId;

impl crate::game::ServerGame<'_> {
    fn resident_display_index(&self, display: DisplayRef) -> Option<usize> {
        let (Target::Other(_), Some(location)) =
            self.native_entity_target_with_location(display.entity())?
        else {
            return None;
        };
        self.server.others.0.locate(location.join)
    }

    fn resident_display_state(&self, display: DisplayRef) -> Option<&DisplayState> {
        self.server.others.0[self.resident_display_index(display)?]
            .display
            .as_deref()
    }

    fn display_text_state(&self, display: DisplayRef) -> Option<&DisplayText> {
        match &self.resident_display_state(display)?.contents {
            DisplayContents::Text(text) => Some(text),
            _ => None,
        }
    }

    fn display_edit(
        &mut self,
        display: DisplayRef,
        edit: impl FnOnce(&mut DisplayState) -> bool,
    ) -> bool {
        let Some(index) = self.resident_display_index(display) else {
            return false;
        };
        self.server.others.0.update(index, |entity| {
            entity
                .display
                .as_mut()
                .is_some_and(|state| edit(Arc::make_mut(state)))
        })
    }

    fn display_edit_text(
        &mut self,
        display: DisplayRef,
        edit: impl FnOnce(&mut DisplayText),
    ) -> bool {
        self.display_edit(display, |state| {
            let DisplayContents::Text(text) = &mut state.contents else {
                return false;
            };
            edit(text);
            true
        })
    }

    pub fn display(&self, entity: EntityRef) -> Option<DisplayRef> {
        let display = DisplayRef::new(entity);
        self.resident_display_state(display)?;
        Some(display)
    }

    pub(in crate::game) fn display_kind(&self, display: DisplayRef) -> Option<DisplayKind> {
        Some(match self.resident_display_state(display)?.contents {
            DisplayContents::Block(_) => DisplayKind::Block,
            DisplayContents::Item { .. } => DisplayKind::Item,
            DisplayContents::Text(_) => DisplayKind::Text,
        })
    }

    pub(in crate::game) fn display_transformation(
        &self,
        display: DisplayRef,
    ) -> Option<Transformation> {
        Some(self.resident_display_state(display)?.transformation)
    }

    pub fn display_set_transformation(
        &mut self,
        display: DisplayRef,
        value: Transformation,
    ) -> bool {
        self.display_edit(display, |state| {
            state.transformation = value;
            true
        })
    }

    pub(in crate::game) fn display_billboard(&self, display: DisplayRef) -> Option<Billboard> {
        Some(self.resident_display_state(display)?.billboard)
    }

    pub(in crate::game) fn display_set_billboard(
        &mut self,
        display: DisplayRef,
        value: Billboard,
    ) -> bool {
        self.display_edit(display, |state| {
            state.billboard = value;
            true
        })
    }

    pub(in crate::game) fn display_brightness(&self, display: DisplayRef) -> Option<(u8, u8)> {
        self.resident_display_state(display)?.brightness
    }

    pub(in crate::game) fn display_set_brightness(
        &mut self,
        display: DisplayRef,
        value: Option<(u8, u8)>,
    ) -> bool {
        // `Brightness.CODEC`: the two light levels range from 0 to 15.
        if value.is_some_and(|(block, sky)| block > 15 || sky > 15) {
            return false;
        }
        self.display_edit(display, |state| {
            state.brightness = value;
            true
        })
    }

    pub(in crate::game) fn display_view_range(&self, display: DisplayRef) -> Option<f32> {
        Some(self.resident_display_state(display)?.view_range)
    }

    pub(in crate::game) fn display_set_view_range(
        &mut self,
        display: DisplayRef,
        value: f32,
    ) -> bool {
        self.display_edit(display, |state| {
            state.view_range = value;
            true
        })
    }

    pub(in crate::game) fn display_shadow_radius(&self, display: DisplayRef) -> Option<f32> {
        Some(self.resident_display_state(display)?.shadow_radius)
    }

    pub fn display_set_shadow_radius(&mut self, display: DisplayRef, value: f32) -> bool {
        self.display_edit(display, |state| {
            state.shadow_radius = value;
            true
        })
    }

    pub(in crate::game) fn display_shadow_strength(&self, display: DisplayRef) -> Option<f32> {
        Some(self.resident_display_state(display)?.shadow_strength)
    }

    pub(in crate::game) fn display_set_shadow_strength(
        &mut self,
        display: DisplayRef,
        value: f32,
    ) -> bool {
        self.display_edit(display, |state| {
            state.shadow_strength = value;
            true
        })
    }

    pub(in crate::game) fn display_width(&self, display: DisplayRef) -> Option<f32> {
        Some(self.resident_display_state(display)?.width)
    }

    pub(in crate::game) fn display_set_width(&mut self, display: DisplayRef, value: f32) -> bool {
        self.display_edit(display, |state| {
            state.width = value;
            true
        })
    }

    pub(in crate::game) fn display_height(&self, display: DisplayRef) -> Option<f32> {
        Some(self.resident_display_state(display)?.height)
    }

    pub(in crate::game) fn display_set_height(&mut self, display: DisplayRef, value: f32) -> bool {
        self.display_edit(display, |state| {
            state.height = value;
            true
        })
    }

    pub(in crate::game) fn display_glow_color_override(&self, display: DisplayRef) -> Option<i32> {
        Some(self.resident_display_state(display)?.glow_color_override)
    }

    pub(in crate::game) fn display_set_glow_color_override(
        &mut self,
        display: DisplayRef,
        value: i32,
    ) -> bool {
        self.display_edit(display, |state| {
            state.glow_color_override = value;
            true
        })
    }

    pub(in crate::game) fn display_interpolation_duration(
        &self,
        display: DisplayRef,
    ) -> Option<i32> {
        Some(self.resident_display_state(display)?.interpolation_duration)
    }

    pub fn display_set_interpolation_duration(&mut self, display: DisplayRef, value: i32) -> bool {
        self.display_edit(display, |state| {
            state.interpolation_duration = value;
            true
        })
    }

    pub(in crate::game) fn display_start_interpolation(&self, display: DisplayRef) -> Option<i32> {
        Some(self.resident_display_state(display)?.start_interpolation)
    }

    pub fn display_set_start_interpolation(&mut self, display: DisplayRef, value: i32) -> bool {
        if !self.display_edit(display, |state| {
            state.start_interpolation = value;
            true
        }) {
            return false;
        }
        // `SynchedEntityData.set(DATA_..._START_DELTA_TICKS_ID, ticks, true)`.
        self.common.persist.force_meta(display.entity().id(), 8);
        true
    }

    pub(in crate::game) fn display_teleport_duration(&self, display: DisplayRef) -> Option<i32> {
        Some(self.resident_display_state(display)?.teleport_duration)
    }

    pub fn display_set_teleport_duration(&mut self, display: DisplayRef, value: i32) -> bool {
        // Same range as in `Display.readAdditionalSaveData`.
        self.display_edit(display, |state| {
            state.teleport_duration = value.clamp(0, 59);
            true
        })
    }

    pub(in crate::game) fn display_text(&self, display: DisplayRef) -> Option<Text> {
        Some(self.display_text_state(display)?.component.project_text())
    }

    pub(in crate::game) fn display_set_text(&mut self, display: DisplayRef, value: Text) -> bool {
        self.display_edit_text(display, |text| {
            text.component = Arc::new(DisplayComponent::from_text(&value));
        })
    }

    pub(in crate::game) fn display_line_width(&self, display: DisplayRef) -> Option<i32> {
        Some(self.display_text_state(display)?.line_width)
    }

    pub(in crate::game) fn display_set_line_width(
        &mut self,
        display: DisplayRef,
        value: i32,
    ) -> bool {
        self.display_edit_text(display, |text| text.line_width = value)
    }

    pub(in crate::game) fn display_background(&self, display: DisplayRef) -> Option<i32> {
        Some(self.display_text_state(display)?.background)
    }

    pub(in crate::game) fn display_set_background(
        &mut self,
        display: DisplayRef,
        value: i32,
    ) -> bool {
        self.display_edit_text(display, |text| text.background = value)
    }

    pub(in crate::game) fn display_text_opacity(&self, display: DisplayRef) -> Option<i8> {
        Some(self.display_text_state(display)?.opacity)
    }

    pub(in crate::game) fn display_set_text_opacity(
        &mut self,
        display: DisplayRef,
        value: i8,
    ) -> bool {
        self.display_edit_text(display, |text| text.opacity = value)
    }

    pub(in crate::game) fn display_text_shadow(&self, display: DisplayRef) -> Option<bool> {
        Some(self.display_text_state(display)?.shadow)
    }

    pub(in crate::game) fn display_set_text_shadow(
        &mut self,
        display: DisplayRef,
        value: bool,
    ) -> bool {
        self.display_edit_text(display, |text| text.shadow = value)
    }

    pub(in crate::game) fn display_see_through(&self, display: DisplayRef) -> Option<bool> {
        Some(self.display_text_state(display)?.see_through)
    }

    pub(in crate::game) fn display_set_see_through(
        &mut self,
        display: DisplayRef,
        value: bool,
    ) -> bool {
        self.display_edit_text(display, |text| text.see_through = value)
    }

    pub(in crate::game) fn display_default_background(&self, display: DisplayRef) -> Option<bool> {
        Some(self.display_text_state(display)?.default_background)
    }

    pub(in crate::game) fn display_set_default_background(
        &mut self,
        display: DisplayRef,
        value: bool,
    ) -> bool {
        self.display_edit_text(display, |text| text.default_background = value)
    }

    pub(in crate::game) fn display_alignment(&self, display: DisplayRef) -> Option<TextAlignment> {
        Some(self.display_text_state(display)?.alignment)
    }

    pub(in crate::game) fn display_set_alignment(
        &mut self,
        display: DisplayRef,
        value: TextAlignment,
    ) -> bool {
        self.display_edit_text(display, |text| text.alignment = value)
    }

    pub(in crate::game) fn display_block_state(&self, display: DisplayRef) -> Option<BlockStateId> {
        match self.resident_display_state(display)?.contents {
            DisplayContents::Block(state) => Some(state),
            _ => None,
        }
    }

    pub(in crate::game) fn display_set_block_state(
        &mut self,
        display: DisplayRef,
        state: BlockStateId,
    ) -> bool {
        self.display_edit(display, |data| {
            let DisplayContents::Block(block) = &mut data.contents else {
                return false;
            };
            *block = state;
            true
        })
    }

    pub(in crate::game) fn display_item_stack(
        &self,
        display: DisplayRef,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        let DisplayContents::Item {
            stack: Some(stack), ..
        } = &self.resident_display_state(display)?.contents
        else {
            return None;
        };
        let metadata = stack_read_metadata(stack).ok()?;
        let project = |metadata| project_stack(stack, metadata).ok();
        read(ItemStackView::resident(metadata, &project));
        Some(())
    }

    pub(in crate::game) fn display_set_item(
        &mut self,
        display: DisplayRef,
        value: Option<ItemStack>,
    ) -> bool {
        self.display_edit(display, |state| {
            let DisplayContents::Item { stack, .. } = &mut state.contents else {
                return false;
            };
            *stack = value
                .filter(|item| item.count != 0 && &*item.item != "minecraft:air")
                .map(Arc::new);
            true
        })
    }

    pub(in crate::game) fn display_item_context(
        &self,
        display: DisplayRef,
    ) -> Option<ItemDisplayContext> {
        match self.resident_display_state(display)?.contents {
            DisplayContents::Item { context, .. } => Some(context),
            _ => None,
        }
    }

    pub(in crate::game) fn display_set_item_context(
        &mut self,
        display: DisplayRef,
        value: ItemDisplayContext,
    ) -> bool {
        self.display_edit(display, |state| {
            let DisplayContents::Item { context, .. } = &mut state.contents else {
                return false;
            };
            *context = value;
            true
        })
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
