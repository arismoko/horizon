use crate::game::ClientGame;
use sunrise_client::{
    hud::HudBatch,
    widgets::{Cursor, request_cursor},
};
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::stations_common::{INPUT, Kind, RESULT, Station};
use sunrise_gameplay_common::game::trading_common::{MerchantState, merchant_display_name};
use sunrise_items::items::ItemStack;
use sunrise_sim::merchant::{self, MerchantOffer, TradeContext};

impl ClientGame<'_> {
    /// `MerchantScreen` rendering.
    pub fn draw_merchant(&mut self, batch: &mut HudBatch) -> bool {
        let (ox, oy) = self.inventory_origin(batch);
        let local = (self.client.cursor.0 - ox, self.client.cursor.1 - oy);
        let Some(m) = self
            .common
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
        else {
            return false;
        };
        let (progress, custom) = (m.progress(), m.name.clone());
        let (profession, level, xp, scroll, shop, future, focused, dragging) = (
            m.session
                .profession
                .trim_start_matches("minecraft:")
                .to_owned(),
            m.session.level,
            m.session.xp,
            m.scroll,
            m.session.selection_hint,
            m.session.future_xp,
            m.focused,
            m.dragging,
        );
        let offers = m.session.offers.clone();
        let costs: Vec<ItemStack> = offers.iter().map(|o| o.cost_a(self.items())).collect();
        self.render_transparent_background("merchant", batch);
        let atlas = &self.client.gui.atlas;
        if let Some(mut r) = atlas.sprites.get("gui/container/villager").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.u1 = r.u0 + 276.0 * du;
            r.v1 = r.v0 + 166.0 * dv;
            batch.rect_uv(ox + SHIFT, oy, 276.0, 166.0, r, u32::MAX);
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let tr = |k: &str, f: &str| sunrise_game_data::lang::get_or(k, f);
        let name = merchant_display_name(progress, &profession, custom);
        // The screen's title is the merchant's name; its label adds the level.
        super::screen_info::name_container(|| name.plain());
        // `StringDecomposer.iterateFormatted`: `§` codes a name holds apply at draw time.
        let title = merchant_title(progress, level, name).resolve_legacy();
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ox + SHIFT + (49.0 + 138.0 - (tw / 2.0).floor()),
            oy + 6.0,
            GREY,
            false,
        );
        let trades = tr("merchant.trades", "Trades");
        let tw = HudBatch::text_width(atlas, &trades);
        batch.text(
            atlas,
            &trades,
            ox + SHIFT + 5.0 - (tw / 2.0).floor() + 48.0,
            oy + 6.0,
            GREY,
            false,
        );
        batch.text(
            atlas,
            &tr("container.inventory", "Inventory"),
            ox + SHIFT + 107.0,
            oy + 72.0,
            GREY,
            false,
        );
        if offers.get(shop).is_some_and(MerchantOffer::out_of_stock) {
            batch.sprite(
                atlas,
                "container/villager/out_of_stock",
                ox + SHIFT + 182.0,
                oy + 35.0,
            );
        }
        // Experience bar (levels 1-4).
        if progress && level < 5 {
            batch.sprite(
                atlas,
                "container/villager/experience_bar_background",
                ox + SHIFT + 136.0,
                oy + 16.0,
            );
            let min = merchant::min_xp(level);
            if xp >= min && merchant::can_level_up(level) {
                let mult = 102.0 / (merchant::max_xp(level) - min) as f32;
                let w = ((mult * (xp - min) as f32).floor()).min(102.0);
                batch.sprite_part(
                    atlas,
                    "container/villager/experience_bar_current",
                    ox + SHIFT + 136.0,
                    oy + 16.0,
                    w / 102.0,
                );
                if future > 0 {
                    let fw = (future as f32 * mult).floor().min(102.0 - w);
                    if let Some(mut r) = atlas
                        .sprites
                        .get("container/villager/experience_bar_result")
                        .copied()
                    {
                        let du = (r.u1 - r.u0) / r.w as f32;
                        r.u0 += w * du;
                        r.u1 = r.u0 + fw * du;
                        batch.rect_uv(ox + SHIFT + 136.0 + w, oy + 16.0, fw, 5.0, r, u32::MAX);
                    }
                }
            }
        }
        // Scroller (`extractScroller`): its thumb, or the disabled sprite.
        if let Some([x, y, w, h]) = scroller_rect(offers.len(), scroll) {
            batch.sprite(atlas, "container/villager/scroller", ox + x, oy + y);
            if let Some(cursor) = scroller_cursor(local, [x, y, w, h], dragging) {
                request_cursor(cursor);
            }
        } else {
            batch.sprite(
                atlas,
                "container/villager/scroller_disabled",
                ox + SHIFT + 94.0,
                oy + 18.0,
            );
        }
        // Offer buttons.
        let mut offer_tooltip: Option<ItemStack> = None;
        for (row, (index, offer)) in offers.iter().enumerate().skip(scroll).take(7).enumerate() {
            let (bx, by) = (ox + SHIFT + 5.0, oy + 18.0 + 20.0 * row as f32);
            let hover = local.0 >= 5.0 + SHIFT
                && local.0 < 93.0 + SHIFT
                && local.1 >= 18.0 + 20.0 * row as f32
                && local.1 < 38.0 + 20.0 * row as f32;
            let sprite = if hover || focused == Some(row) {
                "widget/button_highlighted"
            } else {
                "widget/button"
            };
            batch.sprite_scaled(&self.client.gui.atlas, sprite, bx, by, 88.0, 20.0, u32::MAX);
            let y = oy + 16.0 + 1.0 + 20.0 * row as f32 + 2.0;
            let base = offer.base_cost_a.stack.clone();
            let cost_a = costs[index].clone();
            let x = ox + SHIFT + 10.0;
            if base.count == cost_a.count {
                self.draw_item(batch, &cost_a, x, y);
            } else {
                // Discounted: the base price struck through, then the new price.
                self.draw_item(
                    batch,
                    &ItemStack {
                        count: 1,
                        ..cost_a.clone()
                    },
                    x,
                    y,
                );
                let atlas = &self.client.gui.atlas;
                let t = base.count.to_string();
                batch.text(
                    atlas,
                    &t,
                    x + 17.0 - HudBatch::text_width(atlas, &t),
                    y + 9.0,
                    u32::MAX,
                    true,
                );
                let t = cost_a.count.to_string();
                batch.text(
                    atlas,
                    &t,
                    x + 14.0 + 17.0 - HudBatch::text_width(atlas, &t),
                    y + 9.0,
                    u32::MAX,
                    true,
                );
                batch.sprite(
                    atlas,
                    "container/villager/discount_strikethrough",
                    x + 7.0,
                    y + 12.0,
                );
            }
            if let Some(b) = offer.cost_b() {
                self.draw_item(batch, &b, ox + SHIFT + 40.0, y);
            }
            let arrow = if offer.out_of_stock() {
                "trade_arrow_out_of_stock"
            } else {
                "trade_arrow"
            };
            batch.sprite(
                &self.client.gui.atlas,
                &format!("container/villager/{arrow}"),
                ox + SHIFT + 60.0,
                y + 3.0,
            );
            self.draw_item(batch, &offer.result, ox + SHIFT + 73.0, y);
            if hover {
                let rel = local.0 - (5.0 + SHIFT);
                offer_tooltip = if rel < 20.0 {
                    Some(cost_a)
                } else if rel > 30.0 && rel < 50.0 {
                    offer.cost_b()
                } else if rel > 65.0 {
                    Some(offer.result.clone())
                } else {
                    None
                };
            }
        }
        // Slots, hover highlight, carried stack, tooltip.
        let slots: Vec<usize> = (0..36).chain([INPUT, INPUT + 1, RESULT]).collect();
        let drag = self.drag_view();
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        for i in slots {
            let (sx, sy) = self.slot_position(i);
            let stack = self.slot_mut(i).clone();
            self.draw_slot_item(batch, drag.as_ref(), i, &stack, ox + sx, oy + sy);
        }
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        // An out-of-stock trade: "the villager needs to restock" over the
        // result (isHovering(186, 35, 22, 21), canRestock).
        let deprecated = progress
            && offers.get(shop).is_some_and(MerchantOffer::out_of_stock)
            && local.0 >= 186.0 + SHIFT - 1.0
            && local.0 < 186.0 + SHIFT + 22.0 + 1.0
            && local.1 >= 35.0 - 1.0
            && local.1 < 35.0 + 21.0 + 1.0;
        let slot_tooltip = if !deprecated && offer_tooltip.is_none() {
            self.slot_at(batch, self.client.cursor.0, self.client.cursor.1)
                .and_then(|slot| self.slot_mut(slot).clone())
        } else {
            None
        };
        let browser_tip = self.draw_mod_browser(batch);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        if self.common.carried.is_some() {
            self.draw_carried(batch);
        }
        // MerchantScreen.extractContents offers the restock message and trade preview before
        // AbstractContainerScreen.extractTooltip. Only the actual slot checks the carried item.
        if deprecated {
            let text = sunrise_game_data::lang::get_or(
                "merchant.deprecated",
                "Villagers restock up to two times per day.",
            );
            let lines: Vec<(String, u32)> =
                text.split('\n').map(|l| (l.to_owned(), u32::MAX)).collect();
            self.draw_tooltip_lines(batch, &lines, self.client.cursor.0, self.client.cursor.1);
        } else if let Some(stack) =
            offer_tooltip.or_else(|| slot_tooltip.filter(|s| self.tooltip_with_carried(s)))
        {
            self.draw_stack_tooltip(batch, &stack, self.client.cursor.0, self.client.cursor.1);
        }
        if let Some(tooltip) = browser_tip {
            self.draw_mod_browser_tooltip(batch, tooltip);
        }
        true
    }
    /// Clicks on the offer list; true when one was hit. A press on the
    /// scroller holds it (`MerchantScreen.mouseClicked`).
    pub(in crate::game::client_side) fn merchant_click(&mut self, local: (f32, f32)) -> bool {
        let Some(m) = self
            .common
            .station
            .as_mut()
            .and_then(|s| s.merchant.as_mut())
        else {
            return false;
        };
        m.dragging = on_scroller(local, m.session.offers.len());
        if m.dragging {
            return true;
        }
        for i in 0..7 {
            let (x, y) = (5.0 + SHIFT, 18.0 + 20.0 * i as f32);
            if local.0 >= x
                && local.0 < x + 88.0
                && local.1 >= y
                && local.1 < y + 20.0
                && i + m.scroll < m.session.offers.len()
            {
                let index = i + m.scroll;
                if let Some(m) = self
                    .common
                    .station
                    .as_mut()
                    .and_then(|s| s.merchant.as_mut())
                {
                    m.focused = Some(i);
                }
                self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
                self.try_move_items(index);
                return true;
            }
        }
        false
    }
    /// `MerchantScreen.mouseDragged`: the held scroller follows the cursor.
    pub(in crate::game::client_side) fn merchant_drag(&mut self) -> bool {
        let Some(size) = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let (_, oy) = self.inventory_origin(&HudBatch::new(
            size,
            self.client.settings.gui_scale_inputs(),
        ));
        let y = self.client.cursor.1 - oy;
        if let Some(m) = self
            .common
            .station
            .as_mut()
            .and_then(|s| s.merchant.as_mut())
            && m.dragging
        {
            m.scroll = drag_scroll(y, m.session.offers.len());
            return true;
        }
        false
    }
    /// `AbstractButton.keyPressed` on the focused offer button: Enter or
    /// Space presses it again, key repeats included, so holding Space
    /// refills the payment slots after every trade. Focus stays on the
    /// button through slot clicks (`ContainerEventHandler.mouseClicked`).
    pub(in crate::game::client_side) fn merchant_key(
        &mut self,
        key: winit::keyboard::KeyCode,
    ) -> bool {
        use winit::keyboard::KeyCode;
        if !matches!(key, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
            return false;
        }
        let Some(m) = self
            .common
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
        else {
            return false;
        };
        // Buttons past the last offer are hidden, and a hidden button is inactive.
        let Some(index) = m
            .focused
            .map(|row| row + m.scroll)
            .filter(|&index| index < m.session.offers.len())
        else {
            return false;
        };
        self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
        self.try_move_items(index);
        true
    }
    /// The offer buttons a controller can focus (GUI origin relative).
    pub(in crate::game::client_side) fn merchant_targets(&self) -> Vec<[f32; 4]> {
        let Some(m) = self
            .common
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
        else {
            return Vec::new();
        };
        (0..7)
            .filter(|i| i + m.scroll < m.session.offers.len())
            .map(|i| [5.0 + SHIFT, 18.0 + 20.0 * i as f32, 88.0, 20.0])
            .collect()
    }
    pub(in crate::game::client_side) fn open_merchant_state_client(
        &mut self,
        state: MerchantState,
    ) {
        assert!(
            !self.is_logical_server(),
            "open_merchant_state_client called on Server role"
        );
        let mut s = Station::new_kind(Kind::Merchant, [0, 0, 0]);
        s.merchant = Some(Box::new(state));
        self.open_station_client(s);
        self.set_screen(super::Screen::Inventory);
        self.capture(false);
        self.set_mapping_held("key.use", false);
    }
    /// Opens a client-only merchant preview without a mob (`sunrise-ctl trade`).
    pub(in crate::game::client_side) fn open_preview_merchant(
        &mut self,
        profession: &str,
        variant: &str,
        level: i32,
    ) {
        assert!(
            !self.is_logical_server(),
            "open_preview_merchant called on Server role"
        );
        let profession = if profession.contains(':') {
            profession.to_owned()
        } else {
            format!("minecraft:{profession}")
        };
        let variant = format!("minecraft:{}", variant.trim_start_matches("minecraft:"));
        let level = level.clamp(1, 5);
        let origin = self
            .common
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let mut offers = Vec::new();
        for l in 1..=level {
            let set = self.stations().trades.set_for(&profession, l);
            if let Some(sequence) = self.stations().trades.sequence(&set) {
                let mut random = merchant::sequence_random(self.common.level.seed, &sequence);
                let ctx = TradeContext {
                    items: self.items(),
                    enchantments: &self.stations().enchantments,
                    variant: &variant,
                    origin,
                    maps: None,
                };
                offers.extend(
                    self.stations()
                        .trades
                        .offers_from_set(&ctx, &set, &mut random),
                );
            }
        }
        let mut session = merchant::Session::new(offers, true);
        session.profession = profession;
        session.variant = variant;
        session.level = level;
        session.xp = merchant::min_xp(level);
        session.origin = origin;
        self.open_merchant_state_client(MerchantState {
            session: Box::new(session),
            scroll: 0,
            dragging: false,
            focused: None,
            mob: None,
            name: None,
        });
    }
    /// A server's trading window (no mob here: the server keeps it).
    pub fn open_remote_merchant(&mut self, session: merchant::Session, name: Option<Text>) {
        self.open_merchant_state_client(MerchantState {
            session: Box::new(session),
            scroll: 0,
            dragging: false,
            focused: None,
            mob: None,
            name,
        });
    }
    /// `MerchantScreen.postButtonClick`: predict payment move locally and send SelectTrade.
    pub fn try_move_items(&mut self, index: usize) {
        self.menu_session(|menu, env, _| {
            menu.merchant_select(env, index);
            Some(true)
        });
        if self.client.remote.is_some() {
            if let Some(m) = self
                .common
                .station
                .as_mut()
                .and_then(|s| s.merchant.as_mut())
            {
                m.session.selection_hint = index;
            }
            self.send(sunrise_protocol::packets::play::Serverbound::SelectTrade(
                sunrise_protocol::packets::play::SelectTrade { item: index as i32 },
            ));
        }
    }
}

/// The merchant screen is 100 px wider than the 176-px menus it shares an origin with.
pub(in crate::game) const SHIFT: f32 = -50.0;

pub(in crate::game) const GREY: u32 = 0xFF40_4040;

/// `MerchantScreen.extractLabels`: a villager's title is `merchant.title` with
/// the name and its level, so the name keeps the style it was given.
pub(in crate::game) fn merchant_title(progress: bool, level: i32, name: Text) -> Text {
    // `extractLabels` joins the level for levels 1 to 5 only (`VillagerData`
    // clamps a villager to 1; a level past 5 draws the name alone).
    if !progress || !(1..=5).contains(&level) {
        return name;
    }
    let level = Text::translate_with_fallback(&format!("merchant.level.{level}"), "", &[]);
    Text::translate_with_fallback("merchant.title", "%s - %s", &[name, level])
}

/// Player inventory slot positions in `MerchantMenu` (x from 108).
pub(in crate::game) fn inventory_slot(i: usize) -> (f32, f32) {
    match i {
        0..9 => (108.0 + SHIFT + i as f32 * 18.0, 142.0),
        _ => (
            108.0 + SHIFT + ((i - 9) % 9) as f32 * 18.0,
            84.0 + ((i - 9) / 9) as f32 * 18.0,
        ),
    }
}

/// Whether a press at `local` (GUI origin relative) lands on the offer list's
/// scroller, which only takes it when there are more than 7 offers.
pub(in crate::game) fn on_scroller(local: (f32, f32), offers: usize) -> bool {
    let x = SHIFT + 94.0;
    offers > 7
        && local.0 > x
        && local.0 < x + 6.0
        && local.1 > 18.0
        && local.1 <= 18.0 + 139.0 + 1.0
}

/// `MerchantScreen.extractScroller`: the thumb's rectangle (GUI origin
/// relative) while the offers overflow; the track's 139 px, the thumb's 27.
pub(in crate::game) fn scroller_rect(offers: usize, scroll: usize) -> Option<[f32; 4]> {
    let steps = offers as i32 + 1 - 7;
    if steps <= 1 {
        return None;
    }
    let left = 139 - (27 + (steps - 1) * 139 / steps);
    let step = 1 + left / steps + 139 / steps;
    let mut y = (scroll as i32 * step).min(113);
    if scroll as i32 == steps - 1 {
        y = 113;
    }
    Some([SHIFT + 94.0, 18.0 + y as f32, 6.0, 27.0])
}

/// `MerchantScreen.extractScroller`'s cursor: only the thumb asks, a hand
/// until it is dragged and the vertical resize while it is held.
pub(in crate::game) fn scroller_cursor(
    local: (f32, f32),
    rect: [f32; 4],
    dragging: bool,
) -> Option<Cursor> {
    let [x, y, w, h] = rect;
    (local.0 >= x && local.0 < x + w && local.1 >= y && local.1 <= y + h).then_some(if dragging {
        Cursor::ResizeNs
    } else {
        Cursor::PointingHand
    })
}

/// The first shown offer for the scroller dragged to `y` (GUI origin
/// relative): the 139-px track starts at 18 and the scroller is 27 px tall.
pub(in crate::game) fn drag_scroll(y: f32, offers: usize) -> usize {
    let max = offers.saturating_sub(7) as i32;
    let offs = (y - 18.0 - 13.5) / (139.0 - 27.0);
    ((offs * max as f32 + 0.5) as i32).clamp(0, max) as usize
}

use sunrise_gameplay_common::game::tables::GameItems as _;
