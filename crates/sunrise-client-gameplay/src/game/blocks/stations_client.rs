//! Client-only workstation UI state (`StationUi`, `EnchantBook`, `CyclingSlot`, `rename_box`)
//! and the lockstep station open/clear helpers on `ClientGame`.
use crate::game::ClientGame;
use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_gameplay_common::game::stations_common::{NAME_CHARS, Station};
use sunrise_items::items::ItemStack;
use sunrise_level::random_source;

/// `AnvilScreen.init`: the unbordered box of 50 at the menu's name position,
/// focused; `slotChanged` makes it editable once the input slot has an item.
pub(in crate::game) fn rename_box() -> sunrise_client::ui::EditBox {
    let mut edit = sunrise_client::ui::EditBox::new(0.0, 0.0, 103.0, 12.0);
    edit.message = crate::game::screen_info::tr("container.repair", &[]);
    edit.bordered = false;
    edit.max_length = NAME_CHARS;
    edit.text_color = u32::MAX;
    // AnvilScreen also sets `setTextColorUneditable(-1)`: white either way.
    edit.text_color_uneditable = u32::MAX;
    // AnvilScreen sets `setInvertHighlightedTextColor(false)`.
    edit.invert_highlighted_text_color = false;
    edit.focused = true;
    edit.editable = false;
    edit
}

/// The book above the enchanting table's offers: `EnchantmentScreen`'s
/// `time`, `flip`, `oFlip`, `flipT`, `flipA`, `open`, `oOpen` and `last`.
pub(in crate::game) struct EnchantBook {
    pub(in crate::game) time: i32,
    pub(in crate::game) flip: f32,
    pub(in crate::game) o_flip: f32,
    pub(in crate::game) flip_t: f32,
    pub(in crate::game) flip_a: f32,
    pub(in crate::game) open: f32,
    pub(in crate::game) o_open: f32,
    pub(in crate::game) last: Option<ItemStack>,
    pub(in crate::game) random: LegacyRandom,
}

impl EnchantBook {
    pub(in crate::game) fn new(random: LegacyRandom) -> Self {
        Self {
            time: 0,
            flip: 0.0,
            o_flip: 0.0,
            flip_t: 0.0,
            flip_a: 0.0,
            open: 0.0,
            o_open: 0.0,
            last: None,
            random,
        }
    }

    /// `EnchantmentScreen.tickBook`: a new item in the enchanting slot picks a
    /// new page to turn to; the book opens while any offer has a cost.
    pub(in crate::game::client_side) fn tick(
        &mut self,
        input: Option<&ItemStack>,
        costs: [i32; 3],
    ) {
        if input != self.last.as_ref() {
            self.last = input.cloned();
            loop {
                let a = self.random.next_int_bound(4).unwrap_or(0);
                let b = self.random.next_int_bound(4).unwrap_or(0);
                self.flip_t += (a - b) as f32;
                if !(self.flip <= self.flip_t + 1.0 && self.flip >= self.flip_t - 1.0) {
                    break;
                }
            }
        }
        self.time += 1;
        self.o_flip = self.flip;
        self.o_open = self.open;
        let open = costs.iter().any(|&c| c != 0);
        self.open = (self.open + if open { 0.2 } else { -0.2 }).clamp(0.0, 1.0);
        let diff = ((self.flip_t - self.flip) * 0.4).clamp(-0.2, 0.2);
        self.flip_a += (diff - self.flip_a) * 0.9;
        self.flip += self.flip_a;
    }

    /// `EnchantmentScreen.extractBook`'s `open` and `flip` at the partial tick.
    pub(in crate::game::client_side) fn at(&self, partial: f32) -> (f32, f32) {
        (
            self.o_open + (self.open - self.o_open) * partial,
            self.o_flip + (self.flip - self.o_flip) * partial,
        )
    }
}

/// `CyclingSlotBackground`: the icons an empty slot cycles through, one every
/// 30 ticks, the next fading in over 4 (`CyclingSlotBackground.tick` in
/// `advance`, `CyclingSlotBackground.extractRenderState` in `shown`).
#[derive(Default)]
pub(in crate::game) struct CyclingSlot {
    pub(in crate::game) icons: Vec<&'static str>,
    pub(in crate::game) tick: u32,
    pub(in crate::game) index: usize,
    /// The client tick it was last advanced to.
    pub(in crate::game) at: Option<u64>,
}

impl CyclingSlot {
    pub(in crate::game) const CHANGE_TICKS: u32 = 30;
    pub(in crate::game) const FADE_TICKS: f32 = 4.0;

    /// `CyclingSlotBackground.tick(newIcons)` for each client tick since the last call.
    pub(in crate::game::client_side) fn advance(&mut self, now: u64, icons: &[&'static str]) {
        let ticks = self.at.map_or(0, |at| now.saturating_sub(at).min(600));
        self.at = Some(now);
        if self.icons != icons {
            self.icons = icons.to_vec();
            self.index = 0;
        }
        for _ in 0..ticks {
            if !self.icons.is_empty() {
                self.tick += 1;
                if self.tick.is_multiple_of(Self::CHANGE_TICKS) {
                    self.index = (self.index + 1) % self.icons.len();
                }
            }
        }
    }

    /// What `CyclingSlotBackground.extractRenderState` draws for an empty slot: the icon before
    /// this one fading out while this one fades in, with their alphas.
    pub(in crate::game::client_side) fn shown(&self, partial: f32) -> Vec<(&'static str, f32)> {
        if self.icons.is_empty() {
            return Vec::new();
        }
        let fading = self.icons.len() > 1 && self.tick >= Self::CHANGE_TICKS;
        let alpha = if fading {
            (((self.tick % Self::CHANGE_TICKS) as f32 + partial).min(Self::FADE_TICKS))
                / Self::FADE_TICKS
        } else {
            1.0
        };
        let mut shown = Vec::new();
        if alpha < 1.0 {
            let previous = (self.index + self.icons.len() - 1) % self.icons.len();
            shown.push((self.icons[previous], 1.0 - alpha));
        }
        shown.push((self.icons[self.index], alpha));
        shown
    }
}

/// `SmithingScreen`'s `EMPTY_SLOT_SMITHING_TEMPLATES`.
pub(in crate::game) const EMPTY_TEMPLATE_ICONS: [&str; 2] = [
    "smithing_template_armor_trim",
    "smithing_template_netherite_upgrade",
];

pub struct StationUi {
    /// The box itself (`AnvilScreen`'s `EditBox` of 50, unbordered).
    pub(in crate::game::client_side) rename: sunrise_client::ui::EditBox,
    /// Stonecutter: the screen's first shown recipe index (`startIndex`).
    pub start: usize,
    /// Stonecutter: the screen's `scrollOffs` (0 to 1) and `scrolling`, the
    /// scroller held by the mouse.
    pub scroll: f32,
    pub(in crate::game::client_side) scrolling: bool,
    /// Anvil: the input the rename box was last filled from.
    pub(in crate::game::client_side) anvil_last: Option<ItemStack>,
    /// Anvil on a server: the name last sent (the client menu's `itemName`).
    pub(in crate::game::client_side) anvil_sent: Option<String>,
    /// A server's beacon menu: the pyramid's levels (its data slot 0).
    pub(in crate::game::client_side) beacon_levels: Option<i32>,
    /// The smithing table's three `CyclingSlotBackground`s: the template, base and additions slots.
    pub(in crate::game::client_side) cycling: [CyclingSlot; 3],
    /// The enchanting table's book (`EnchantmentScreen`'s flip and open state).
    pub(in crate::game::client_side) book: EnchantBook,
}

impl StationUi {
    pub(in crate::game) fn new() -> Self {
        Self {
            rename: rename_box(),
            start: 0,
            scroll: 0.0,
            scrolling: false,
            anvil_last: None,
            anvil_sent: None,
            beacon_levels: None,
            cycling: Default::default(),
            book: EnchantBook::new(random_source::create()),
        }
    }
}

/// `StonecutterScreen.containerChanged`: a changed input puts the list back at the top.
pub(in crate::game::client_side) fn stonecutter_changed(ui: &mut StationUi) {
    ui.start = 0;
    ui.scroll = 0.0;
}

/// Clear state tied to the previous `UpdateRecipes` table.
pub(in crate::game::client_side) fn reset_stonecutter_generation(
    station: &mut Station,
    ui: &mut StationUi,
) {
    station.stone = Default::default();
    station.selected = None;
    station.result = None;
    ui.scrolling = false;
    stonecutter_changed(ui);
}

/// Moves the stonecutter's scroller to `offs`, and its first shown
/// recipe (`startIndex`) with it.
pub(in crate::game::client_side) fn stonecutter_scroll_to(
    station: &Station,
    ui: &mut StationUi,
    offs: f32,
) {
    ui.scroll = offs.clamp(0.0, 1.0);
    ui.start = super::stations::stonecutter_start(station.stone.recipes.len(), ui.scroll);
}

impl ClientGame<'_> {
    pub fn open_station_client(&mut self, station: Station) {
        self.common.station = Some(station);
        self.client.station_ui = Some(StationUi::new());
        self.client.station_loom = Default::default();
    }

    pub fn clear_station_client(&mut self) {
        self.common.station = None;
        self.client.station_ui = None;
        self.client.station_loom = Default::default();
    }

    pub(in crate::game) fn close_station_client(&mut self) {
        assert!(
            !self.is_logical_server(),
            "close_station_client called on Server role"
        );
        self.clear_station_client();
    }

    pub(in crate::game::client_side) fn station_and_ui(&self) -> Option<(&Station, &StationUi)> {
        let s = self.common.station.as_ref()?;
        let Some(ui) = self.client.station_ui.as_ref() else {
            debug_assert!(false, "open client station missing StationUi");
            return None;
        };
        Some((s, ui))
    }

    pub(in crate::game::client_side) fn station_and_ui_mut(
        &mut self,
    ) -> Option<(&mut Station, &mut StationUi)> {
        let s = self.common.station.as_mut()?;
        let Some(ui) = self.client.station_ui.as_mut() else {
            debug_assert!(false, "open client station missing StationUi");
            return None;
        };
        Some((s, ui))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn station_rename_box_for_test(&self) -> (String, bool) {
        let ui = self
            .client
            .station_ui
            .as_ref()
            .expect("open client station missing StationUi");
        (ui.rename.value.clone(), ui.rename.editable)
    }
}
