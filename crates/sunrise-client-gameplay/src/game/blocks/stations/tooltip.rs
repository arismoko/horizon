use super::*;
use mod_api::{ClientTooltipPositioner, DefaultTooltipPositioner};
use std::borrow::Cow;
use sunrise_client::tooltip_components::TooltipComponentPrepareError;
use {
    crate::game::client_item_extensions::DEFAULT_TOOLTIP_FONT_ID,
    crate::game::host::draw::{EngineTooltipFont, font_id_str},
};

/// Single integer anchor quantization shared by Gather wrapping, `render_tooltip_pre`
/// (`TooltipPlace`), and the active positioner. Vanilla GUI extraction truncates toward zero.
pub fn quantize_tooltip_anchor(at: (f32, f32)) -> [i32; 2] {
    [at.0 as i32, at.1 as i32]
}

/// Widget-relative tooltip placement. The rectangle is `[left, top, width, height]` in GUI pixels.
pub enum WidgetTooltipPositioner {
    Menu([i32; 4]),
    BelowOrAbove([i32; 4]),
}

impl ClientTooltipPositioner for WidgetTooltipPositioner {
    fn position_tooltip(
        &self,
        [screen_w, screen_h]: [i32; 2],
        [anchor_x, anchor_y]: [i32; 2],
        [w, h]: [i32; 2],
    ) -> [i32; 2] {
        match *self {
            Self::Menu([_, top, _, height]) => {
                // Vanilla MenuTooltipPositioner: beside the mouse, vertically clear of the widget.
                let mut x = anchor_x.wrapping_add(12);
                if x.wrapping_add(w) > screen_w.wrapping_sub(5) {
                    x = anchor_x.wrapping_sub(12).wrapping_sub(w).max(9);
                }
                let y = anchor_y.wrapping_add(3);
                let bottom = top.wrapping_add(height);
                let box_height = h.wrapping_add(6);
                let offset = |y: i32, edge: i32| {
                    let distance = y.wrapping_sub(edge).wrapping_abs().min(height);
                    let progress = distance as f32 / height as f32;
                    let start = height.wrapping_sub(3) as f32;
                    (f64::from(start + progress * (5.0 - start)) + 0.5).floor() as i32
                };
                let lowest = bottom.wrapping_add(3).wrapping_add(offset(0, 0));
                let y = if lowest.wrapping_add(box_height) <= screen_h.wrapping_sub(5) {
                    y.wrapping_add(offset(y, top))
                } else {
                    y.wrapping_sub(box_height.wrapping_add(offset(y, bottom)))
                };
                [x, y]
            }
            Self::BelowOrAbove([left, top, width, height]) => {
                // Vanilla BelowOrAboveWidgetTooltipPositioner: the cursor is not the anchor.
                let mut x = left.wrapping_add(3);
                let mut y = top.wrapping_add(height).wrapping_add(4);
                if y.wrapping_add(h).wrapping_add(3) > screen_h {
                    y = top.wrapping_sub(h).wrapping_sub(4);
                }
                if x.wrapping_add(w) > screen_w {
                    x = left
                        .wrapping_add(width)
                        .wrapping_sub(w)
                        .wrapping_sub(3)
                        .max(4);
                }
                [x, y]
            }
        }
    }
}

/// Internal prepared tooltip item retaining engine text and prepared bundle contents so the
/// engine never round-trips original text or bundles through the mod-API boundary on render.
pub enum PreparedTooltipItem {
    Text {
        text: sunrise_game_data::text::Text,
        color: u32,
    },
    Bundle {
        stack: ItemStack,
        contents: sunrise_items::bundle::Contents,
    },
    Custom(mod_api::PreparedTooltipComponent),
}

impl PreparedTooltipItem {
    #[cfg(any(test, feature = "test-support"))]
    pub fn as_text(&self) -> Option<(&sunrise_game_data::text::Text, u32)> {
        match self {
            Self::Text { text, color } => Some((text, *color)),
            _ => None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn as_bundle(&self) -> Option<&ItemStack> {
        match self {
            Self::Bundle { stack, .. } => Some(stack),
            _ => None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn as_custom(&self) -> Option<&mod_api::PreparedTooltipComponent> {
        match self {
            Self::Custom(component) => Some(component),
            _ => None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn downcast_ref<C: mod_api::ClientTooltipComponent + 'static>(&self) -> Option<&C> {
        self.as_custom().and_then(|c| c.downcast_ref::<C>())
    }
}

/// Outcome of gathering and preparing an item's tooltip components before `render_tooltip_pre`.
pub enum GatheredTooltip {
    /// Gathered and prepared components, with any Gather text wrapping already applied.
    Ready(Vec<PreparedTooltipItem>),
    /// A `render_tooltip_gather_components` listener explicitly returned `Flow::Cancel`.
    Cancelled,
    /// Preparing a gathered component failed (unregistered custom type, failed factory, or invalid
    /// bundle); vanilla throws `IllegalArgumentException` before Pre/Texture/paint, so the tooltip
    /// aborts cleanly with an error log instead of dropping the component or cancelling.
    PrepareError(TooltipComponentPrepareError),
}

#[cfg(any(test, feature = "test-support"))]
impl GatheredTooltip {
    pub fn unwrap_ready(self) -> Vec<PreparedTooltipItem> {
        match self {
            Self::Ready(components) => components,
            Self::Cancelled => panic!("expected GatheredTooltip::Ready, got Cancelled"),
            Self::PrepareError(err) => {
                panic!("expected GatheredTooltip::Ready, got PrepareError({err})")
            }
        }
    }
}

/// Unprepared image selected by `Item.getTooltipImage`; a vanilla bundle uses the source stack.
enum RawTooltipImage {
    Mod(mod_api::TooltipComponent),
    VanillaBundle,
}

/// NeoForge `ClientHooks.gatherTooltipComponentsFromElements` wrapping width calculation using
/// Java `int` wrapping arithmetic.
pub(in crate::game::client_side) fn wrap_tooltip_width(
    mut tooltip_text_width: i32,
    mouse_x: i32,
    screen_width: i32,
    max_width: i32,
) -> (i32, bool) {
    let mut needs_wrap = false;
    let mut tooltip_x = mouse_x.wrapping_add(12);
    if tooltip_x.wrapping_add(tooltip_text_width).wrapping_add(4) > screen_width {
        tooltip_x = mouse_x.wrapping_sub(16).wrapping_sub(tooltip_text_width);
        if tooltip_x < 4 {
            tooltip_text_width = if mouse_x > screen_width / 2 {
                mouse_x.wrapping_sub(12).wrapping_sub(8)
            } else {
                screen_width.wrapping_sub(16).wrapping_sub(mouse_x)
            };
            needs_wrap = true;
        }
    }
    if max_width > 0 && tooltip_text_width > max_width {
        tooltip_text_width = max_width;
        needs_wrap = true;
    }
    (tooltip_text_width, needs_wrap)
}

/// Emit one resolved engine text line into `out`, splitting via `Font::split_formatted_with_font`
/// when `wrap_width` is set and the text is non-empty (empty text stays as one empty line,
/// matching NeoForge `ClientHooks.splitLine`).
fn push_prepared_text(
    font: &sunrise_client::font::Font,
    default_font: &str,
    resolved: sunrise_game_data::text::Text,
    color: u32,
    wrap_width: Option<i32>,
    out: &mut Vec<PreparedTooltipItem>,
) {
    if let Some(wrap_width) = wrap_width
        && !resolved.is_empty()
    {
        for line in font.split_formatted_with_font(&resolved, wrap_width as f32, default_font) {
            out.push(PreparedTooltipItem::Text { text: line, color });
        }
        return;
    }
    out.push(PreparedTooltipItem::Text {
        text: resolved,
        color,
    });
}

/// Report a tooltip component preparation failure without latching or cancelling.
fn report_tooltip_prepare_error(err: &TooltipComponentPrepareError) {
    log::error!("tooltip component preparation failed: {err}");
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `getEmptyBundleDescriptionTextHeight`'s lines: the empty bundle's description wrapped at
    /// the grid's width in `default_font`.
    fn bundle_description_lines(&self, default_font: &str) -> usize {
        let text = sunrise_game_data::text::Text::translate(
            "item.minecraft.bundle.empty.description",
            &[],
        );
        self.client
            .gui
            .atlas
            .font
            .split_formatted_with_font(&text, 96.0, default_font)
            .len()
    }

    /// `Item.getTooltipImage`: asks the item's `ItemPart::tooltip_image` if one listens, falling
    /// back to vanilla's bundle component when `minecraft:tooltip_display` shows
    /// `minecraft:bundle_contents`.
    fn resolve_raw_tooltip_image(&self, stack: &ItemStack) -> Option<RawTooltipImage> {
        let mut component = None;
        if mod_api::__private::item_listens(mod_api::HookId::TooltipImage, &stack.item) {
            self.native_tooltip_image(stack, &mut component);
            if let Some(component) = component {
                return Some(RawTooltipImage::Mod(component));
            }
        }
        sunrise_client::item_tooltip::shows_bundle_tooltip(self.items(), stack)
            .then_some(RawTooltipImage::VanillaBundle)
    }

    fn prepare_bundle_tooltip(
        &self,
        stack: Cow<'_, ItemStack>,
    ) -> Result<PreparedTooltipItem, TooltipComponentPrepareError> {
        let contents = sunrise_items::bundle::Contents::of(self.items(), &stack)
            .ok_or(TooltipComponentPrepareError::InvalidBundle { owner: None })?;
        Ok(PreparedTooltipItem::Bundle {
            stack: stack.into_owned(),
            contents,
        })
    }

    /// Prepare a [`mod_api::TooltipComponent`] into a [`PreparedTooltipItem`]
    /// (`ClientTooltipComponent.create`): built-in bundles are prepared directly with `Items`,
    /// custom components go through the registered client factory table.
    pub fn prepare_tooltip_component(
        &self,
        component: &mod_api::TooltipComponent,
    ) -> Result<PreparedTooltipItem, TooltipComponentPrepareError> {
        if let Some(api_stack) = component.as_bundle() {
            let engine_stack = sunrise_gameplay_common::game::api_values::stack_value(api_stack)
                .map_err(|_| TooltipComponentPrepareError::InvalidBundle {
                    owner: component.__owner_id(),
                })?;
            return self.prepare_bundle_tooltip(Cow::Owned(engine_stack));
        }
        sunrise_client::tooltip_components::create(component).map(PreparedTooltipItem::Custom)
    }

    /// `AbstractContainerScreen.showTooltipWithItemInHand`: whether this stack's tooltip image
    /// shows while carrying another item, avoiding unnecessary `Contents::of` work for ordinary
    /// bundles and returning `false` on preparation errors.
    pub(in crate::game::client_side) fn tooltip_image_shows_with_carried(
        &self,
        stack: &ItemStack,
    ) -> bool {
        match self.resolve_raw_tooltip_image(stack) {
            Some(RawTooltipImage::VanillaBundle) => true,
            Some(RawTooltipImage::Mod(component)) => {
                match self.prepare_tooltip_component(&component) {
                    Ok(PreparedTooltipItem::Bundle { .. }) => true,
                    Ok(PreparedTooltipItem::Custom(prepared)) => {
                        prepared.show_tooltip_with_item_in_hand()
                    }
                    Ok(PreparedTooltipItem::Text { .. }) => false,
                    Err(err) => {
                        report_tooltip_prepare_error(&err);
                        false
                    }
                }
            }
            None => false,
        }
    }

    /// Initially measure width and height in encounter order, as `GuiGraphicsExtractor.tooltip`.
    pub fn measure_prepared_item(
        &self,
        item: &PreparedTooltipItem,
        font: &EngineTooltipFont<'_>,
    ) -> Option<(i32, i32)> {
        let width = match item {
            PreparedTooltipItem::Text { text, .. } => {
                self.client
                    .gui
                    .atlas
                    .font
                    .width_text_with_font(text, font.name()) as i32
            }
            PreparedTooltipItem::Bundle { .. } => sunrise_items::bundle::Contents::TOOLTIP_WIDTH,
            PreparedTooltipItem::Custom(component) => component.width(font)?,
        };
        Some((width, self.prepared_item_height(item, font)?))
    }

    fn prepared_item_height(
        &self,
        item: &PreparedTooltipItem,
        font: &EngineTooltipFont<'_>,
    ) -> Option<i32> {
        match item {
            PreparedTooltipItem::Text { .. } => Some(10),
            PreparedTooltipItem::Bundle { contents, .. } => {
                let desc_lines = if contents.items().is_empty() {
                    self.bundle_description_lines(font.name())
                } else {
                    0
                };
                Some(contents.tooltip_height(desc_lines, 9))
            }
            PreparedTooltipItem::Custom(component) => component.height(font),
        }
    }

    /// `RenderTooltipEvent.GatherComponents` followed by wrapping in the item's tooltip font
    /// (`ClientHooks.getTooltipFont` -> `ClientItemExtensions::tooltip_font`) and component
    /// preparation (`ClientHooks.gatherTooltipComponentsFromElements`).
    pub fn gather_tooltip_components(
        &self,
        stack: &ItemStack,
        lines: Vec<(sunrise_game_data::text::Text, u32)>,
        mouse_x: i32,
        screen: (f32, f32),
    ) -> GatheredTooltip {
        let raw_image = self.resolve_raw_tooltip_image(stack);

        // If the snapshot cannot cross the mod API, skip Gather and keep the engine content.
        let gather_input = || {
            let api_stack = stack
                .try_into_api()
                .map_err(|error| {
                    log::warn!("mod render_tooltip_gather_components stack refused: {error:?}");
                })
                .ok()?;
            let mut elements = Vec::with_capacity(lines.len() + usize::from(raw_image.is_some()));
            for (text, color) in &lines {
                let api_text = text
                    .try_into_api()
                    .map_err(|error| {
                        log::warn!("mod render_tooltip_gather_components text refused: {error:?}");
                    })
                    .ok()?;
                elements.push(TooltipElement::Text(api_text, *color));
            }
            let mut bundle_snapshot = None;
            let component = match &raw_image {
                Some(RawTooltipImage::Mod(component)) => Some(component.clone()),
                Some(RawTooltipImage::VanillaBundle) => {
                    let bundle = mod_api::TooltipComponent::bundle(api_stack.clone());
                    bundle_snapshot = Some(bundle.clone());
                    Some(bundle)
                }
                None => None,
            };
            if let Some(component) = component {
                elements.insert(
                    usize::from(!elements.is_empty()),
                    TooltipElement::Component(component),
                );
            }
            Some((api_stack, elements, bundle_snapshot))
        };
        if mod_api::__private::enabled_listened(mod_api::HookId::RenderTooltipGatherComponents)
            && let Some((api_stack, mut elements, vanilla_bundle_snapshot)) = gather_input()
        {
            let item_font = self.item_tooltip_font(stack);
            let item_font_str = font_id_str(&item_font);
            let font = &self.client.gui.atlas.font;
            let (mut max_width, mut cancel) = (-1, false);
            self.native_render_tooltip_gather_components(
                &api_stack,
                screen.0 as i32,
                screen.1 as i32,
                &mut elements,
                &mut max_width,
                &mut cancel,
            );
            if cancel {
                return GatheredTooltip::Cancelled;
            }

            enum DecodedElement {
                Text(sunrise_game_data::text::Text, u32),
                Component(mod_api::TooltipComponent),
            }

            let mut tooltip_text_width = None;
            let mut decoded = Vec::with_capacity(elements.len());
            for element in elements {
                let (element, width) = match element {
                    TooltipElement::Text(api_text, color) => {
                        let Ok(engine_text) = api_text.try_into_engine(&()) else {
                            continue;
                        };
                        let resolved = engine_text.resolve_legacy();
                        let width = font.width_formatted_with_font(&resolved, &item_font_str);
                        (DecodedElement::Text(resolved, color), width)
                    }
                    TooltipElement::Component(comp) => (DecodedElement::Component(comp), 0),
                };
                tooltip_text_width =
                    Some(tooltip_text_width.map_or(width, |max: i32| max.max(width)));
                decoded.push(element);
            }

            let (wrap_width, needs_wrap) = wrap_tooltip_width(
                tooltip_text_width.unwrap_or(0),
                mouse_x,
                screen.0 as i32,
                max_width,
            );
            let wrap = needs_wrap.then_some(wrap_width);
            let original_bundle = vanilla_bundle_snapshot
                .as_ref()
                .and_then(mod_api::TooltipComponent::as_bundle);
            let mut out = Vec::with_capacity(decoded.len());
            for element in decoded {
                match element {
                    DecodedElement::Text(resolved, color) => {
                        push_prepared_text(font, &item_font_str, resolved, color, wrap, &mut out);
                    }
                    DecodedElement::Component(comp) => {
                        // Unchanged bundle elements (including clones/reorders) retain the shared
                        // snapshot. Prepare the original directly instead of decoding its API view.
                        let prepared = if let Some(original) = original_bundle
                            && comp
                                .as_bundle()
                                .is_some_and(|current| std::ptr::eq(original, current))
                        {
                            self.prepare_bundle_tooltip(Cow::Borrowed(stack))
                        } else {
                            self.prepare_tooltip_component(&comp)
                        };
                        match prepared {
                            Ok(prepared) => out.push(prepared),
                            Err(err) => return GatheredTooltip::PrepareError(err),
                        }
                    }
                }
            }
            return GatheredTooltip::Ready(out);
        }

        // GuiGraphicsExtractor.setTooltipForNextFrame keeps each vanilla item line intact.
        // With no Gather dispatch, skip NeoForge wrapping as well as the API round-trip.
        let prepared_image = match raw_image
            .map(|image| match image {
                RawTooltipImage::VanillaBundle => self.prepare_bundle_tooltip(Cow::Borrowed(stack)),
                RawTooltipImage::Mod(component) => self.prepare_tooltip_component(&component),
            })
            .transpose()
        {
            Ok(prepared) => prepared,
            Err(err) => return GatheredTooltip::PrepareError(err),
        };
        let mut out: Vec<_> = lines
            .into_iter()
            .map(|(text, color)| PreparedTooltipItem::Text {
                text: text.resolve_legacy(),
                color,
            })
            .collect();
        if let Some(image) = prepared_image {
            out.insert(usize::from(!out.is_empty()), image);
        }
        GatheredTooltip::Ready(out)
    }

    /// `RenderTooltipEvent.Pre` for prepared `components`, hanging from quantized integer anchor
    /// `at`. Query the item font again here: Gather may have changed the provider's answer, but
    /// its line breaks are already fixed. Return the final anchor and font, or `None` when a
    /// listener cancels drawing (custom Pre draws stay in `batch`).
    pub fn tooltip_pre_components(
        &self,
        batch: &mut HudBatch,
        stack: Option<&ItemStack>,
        components: &mut [PreparedTooltipItem],
        at: [i32; 2],
        positioner: &dyn ClientTooltipPositioner,
    ) -> Option<([i32; 2], Cow<'static, mod_api::Identifier>)> {
        let font = stack.map_or(Cow::Borrowed(&*DEFAULT_TOOLTIP_FONT_ID), |stack| {
            self.item_tooltip_font(stack)
        });
        if !mod_api::__private::enabled_listened(mod_api::HookId::RenderTooltipPre) {
            return Some((at, font));
        }
        let api_stack = match stack {
            Some(s) => match s.try_into_api() {
                Ok(api) => Some(api),
                Err(error) => {
                    log::warn!("mod render_tooltip_pre stack refused: {error:?}");
                    return Some((at, font));
                }
            },
            None => None,
        };

        // Build a complete mod-API snapshot to lend to Pre without losing engine-owned text or
        // bundles. If any item cannot convert, skip Pre cleanly and leave the original engine
        // tooltip intact (consistent with unconvertible stacks).
        struct RestoreCustom<'a> {
            components: &'a mut [PreparedTooltipItem],
            lent: Vec<mod_api::PreparedTooltipComponent>,
        }
        impl Drop for RestoreCustom<'_> {
            fn drop(&mut self) {
                for (slot, item) in self.components.iter_mut().zip(self.lent.drain(..)) {
                    if let PreparedTooltipItem::Custom(dest) = slot {
                        *dest = item;
                    }
                }
            }
        }

        let mut lent = Vec::with_capacity(components.len());
        let mut ok = true;
        for item in components.iter_mut() {
            match item {
                PreparedTooltipItem::Text { text, color } => match text.try_into_api() {
                    Ok(api_text) => {
                        lent.push(mod_api::PreparedTooltipComponent::__text(api_text, *color));
                    }
                    Err(error) => {
                        log::warn!("mod render_tooltip_pre text refused: {error:?}");
                        ok = false;
                        break;
                    }
                },
                PreparedTooltipItem::Bundle { stack, contents } => match stack.try_into_api() {
                    Ok(api_stack) => {
                        let non_empty_height =
                            (!contents.items().is_empty()).then(|| contents.tooltip_height(0, 9));
                        lent.push(mod_api::PreparedTooltipComponent::__bundle(
                            api_stack,
                            non_empty_height,
                        ));
                    }
                    Err(error) => {
                        log::warn!("mod render_tooltip_pre bundle refused: {error:?}");
                        ok = false;
                        break;
                    }
                },
                PreparedTooltipItem::Custom(custom) => {
                    let placeholder =
                        mod_api::PreparedTooltipComponent::__text(mod_api::Text::literal(""), 0);
                    lent.push(std::mem::replace(custom, placeholder));
                }
            }
        }
        let restore = RestoreCustom { components, lent };
        if !ok {
            drop(restore);
            return Some((at, font));
        }

        let mut font = font.into_owned();
        let screen = [batch.width, batch.height];
        let mut place = mod_api::TooltipPlace { x: at[0], y: at[1] };
        let mut cancel = false;
        let mut draw = crate::game::host::draw::GameDrawContext::new(self, batch);
        self.native_render_tooltip_pre(mod_api::__private::RenderTooltipPreCall {
            stack: api_stack.as_ref(),
            components: &restore.lent,
            place: &mut place,
            screen,
            positioner,
            font: &mut font,
            draw: &mut draw,
            cancel: &mut cancel,
        });
        drop(restore);
        if cancel {
            return None;
        }
        Some(([place.x, place.y], Cow::Owned(font)))
    }

    /// `GuiGraphicsExtractor.tooltip`: run Pre, measure in Pre's chosen font without rewrapping,
    /// position with the caller's positioner, draw background (`render_tooltip_texture`), then
    /// run Pass 1 (`draw_text`) and Pass 2 (`draw_image`). Returns the drawn content rect
    /// `[x, y, w, h]` when drawn.
    pub fn draw_prepared_tooltip(
        &self,
        batch: &mut HudBatch,
        stack: Option<&ItemStack>,
        components: &mut [PreparedTooltipItem],
        at: [i32; 2],
        positioner: &dyn ClientTooltipPositioner,
        extra_space_after_first_line: bool,
    ) -> Option<[f32; 4]> {
        if components.is_empty() {
            return None;
        }
        let screen = (batch.width, batch.height);
        let (at, chosen_font) =
            self.tooltip_pre_components(batch, stack, components, at, positioner)?;

        let tooltip_font = EngineTooltipFont::new(&self.client.gui.atlas.font, &chosen_font);
        let chosen_font_str = tooltip_font.name();

        let mut w = 0_i32;
        let mut h = if extra_space_after_first_line && components.len() != 1 {
            0_i32
        } else {
            -2_i32
        };
        for component in components.iter() {
            let (cw, ch) = self.measure_prepared_item(component, &tooltip_font)?;
            if cw > w {
                w = cw;
            }
            h = h.wrapping_add(ch);
        }

        let [x, y] = positioner.position_tooltip([screen.0 as i32, screen.1 as i32], at, [w, h]);

        // A failed component pass discards this tooltip's entire paint, not Pre listener paint.
        let paint = batch.paint_checkpoint();
        self.tooltip_box(batch, stack, x as f32, y as f32, w as f32, h as f32);

        let mut local_y = y;
        for (i, component) in components.iter().enumerate() {
            match component {
                PreparedTooltipItem::Text { text, color } => {
                    batch.text_styled_with_font(
                        &self.client.gui.atlas,
                        text,
                        [x as f32, local_y as f32],
                        *color,
                        true,
                        chosen_font_str,
                    );
                }
                PreparedTooltipItem::Bundle { .. } => {}
                PreparedTooltipItem::Custom(custom) => {
                    let mut draw = crate::game::host::draw::GameDrawContext::new(self, batch)
                        .with_active_tooltip_font(&tooltip_font);
                    if !custom.__draw_text(&tooltip_font, x, local_y, &mut draw) {
                        drop(draw);
                        paint.discard(batch);
                        return None;
                    }
                }
            }
            let Some(ch) = self.prepared_item_height(component, &tooltip_font) else {
                paint.discard(batch);
                return None;
            };
            local_y =
                local_y
                    .wrapping_add(ch)
                    .wrapping_add(if extra_space_after_first_line && i == 0 {
                        2
                    } else {
                        0
                    });
        }

        let mut local_y = y;
        for (i, component) in components.iter().enumerate() {
            match component {
                PreparedTooltipItem::Text { .. } => {}
                PreparedTooltipItem::Bundle { contents, .. } => {
                    let description_lines = if contents.items().is_empty() {
                        self.bundle_description_lines(chosen_font_str)
                    } else {
                        0
                    };
                    let layout = contents.tooltip_layout(
                        x as f32,
                        local_y as f32,
                        w as f32,
                        description_lines,
                    );
                    self.draw_bundle_tooltip(batch, &layout, chosen_font_str);
                    self.draw_bundle_selected_name(
                        batch,
                        &layout,
                        x as f32,
                        local_y as f32,
                        w as f32,
                        chosen_font_str,
                    );
                }
                PreparedTooltipItem::Custom(custom) => {
                    let mut draw = crate::game::host::draw::GameDrawContext::new(self, batch)
                        .with_active_tooltip_font(&tooltip_font);
                    if !custom.__draw_image(&tooltip_font, x, local_y, w, h, &mut draw) {
                        drop(draw);
                        paint.discard(batch);
                        return None;
                    }
                }
            }
            let Some(ch) = self.prepared_item_height(component, &tooltip_font) else {
                paint.discard(batch);
                return None;
            };
            local_y =
                local_y
                    .wrapping_add(ch)
                    .wrapping_add(if extra_space_after_first_line && i == 0 {
                        2
                    } else {
                        0
                    });
        }

        Some([x as f32, y as f32, w as f32, h as f32])
    }

    /// A stack's tooltip: its ordered lines and a bundle's contents
    /// (`ClientBundleTooltip`: the 4 wide grid, the `+N` of what it leaves
    /// out, and the weight bar).
    pub fn draw_stack_tooltip(&mut self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        self.draw_stack_tooltip_with(batch, stack, false, x, y, Vec::new());
    }

    /// [`Self::draw_stack_tooltip`] with `TooltipFlag.isCreative` set when
    /// `creative` (a creative tab's own slot), and `after_name` inserted from
    /// line 1, as the creative screen adds its tab names.
    pub fn draw_stack_tooltip_with(
        &mut self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        creative: bool,
        x: f32,
        y: f32,
        after_name: Vec<(sunrise_game_data::text::Text, u32)>,
    ) {
        let mut lines = self.stack_tooltip_lines(stack, false, creative);
        if lines.is_empty()
            && !sunrise_client::item_tooltip::tooltip_display(stack).name_visible(creative)
        {
            return;
        }
        let after_name_index = lines.len().min(1);
        lines.splice(after_name_index..after_name_index, after_name);
        self.draw_stack_tooltip_lines(batch, stack, lines, x, y);
    }

    /// Draw already-built item lines, keeping the stack's image, style and gather hooks.
    pub fn draw_stack_tooltip_lines(
        &mut self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        lines: Vec<(sunrise_game_data::text::Text, u32)>,
        x: f32,
        y: f32,
    ) {
        let anchor = quantize_tooltip_anchor((x, y));
        let mut components = match self.gather_tooltip_components(
            stack,
            lines,
            anchor[0],
            (batch.width, batch.height),
        ) {
            GatheredTooltip::Ready(components) => components,
            GatheredTooltip::Cancelled => return,
            GatheredTooltip::PrepareError(err) => {
                report_tooltip_prepare_error(&err);
                return;
            }
        };
        self.draw_prepared_tooltip(
            batch,
            Some(stack),
            &mut components,
            anchor,
            &DefaultTooltipPositioner,
            true,
        );
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_tooltip_width_matches_neoforge_client_hooks() {
        // A: fits on the right side without wrapping (including exact screen_width boundary).
        assert_eq!(wrap_tooltip_width(60, 10, 320, -1), (60, false));
        assert_eq!(wrap_tooltip_width(100, 100, 216, -1), (100, false));
        // B: overflows right, flips left cleanly without wrapping (including exact tooltip_x == 4).
        assert_eq!(wrap_tooltip_width(80, 260, 320, -1), (80, false));
        assert_eq!(wrap_tooltip_width(80, 100, 190, -1), (80, false));
        // C: overflows both sides; cursor on right half (mouse_x > screen_width / 2).
        assert_eq!(wrap_tooltip_width(180, 180, 320, -1), (160, true));
        // D: overflows both sides; cursor on left half (mouse_x < screen_width / 2).
        assert_eq!(wrap_tooltip_width(210, 100, 320, -1), (204, true));
        // E: exact midpoint (mouse_x == screen_width / 2) uses the left-half formula.
        assert_eq!(wrap_tooltip_width(200, 160, 320, -1), (144, true));
        // F: explicit positive max_width narrows and wraps; >= width or <= 0 leaves wrapping off.
        assert_eq!(wrap_tooltip_width(100, 10, 320, 40), (40, true));
        assert_eq!(wrap_tooltip_width(100, 10, 320, 100), (100, false));
        assert_eq!(wrap_tooltip_width(100, 10, 320, 140), (100, false));
        assert_eq!(wrap_tooltip_width(100, 10, 320, 0), (100, false));
        // G: max_width tighter than screen wrap narrows further.
        assert_eq!(wrap_tooltip_width(210, 180, 320, 100), (100, true));
        // H: max_width wider than screen wrap keeps screen wrap width.
        assert_eq!(wrap_tooltip_width(210, 180, 320, 180), (160, true));
        // Extreme values never overflow.
        let _ = wrap_tooltip_width(i32::MAX, i32::MAX, i32::MAX, i32::MAX);
        let _ = wrap_tooltip_width(i32::MIN, i32::MIN, i32::MIN, i32::MIN);
    }
}
