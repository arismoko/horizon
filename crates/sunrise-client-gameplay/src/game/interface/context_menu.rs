//! A mod's context menu over the open screen (`ClientHost::open_context_menu`): a list of
//! rows, separators and nested menus drawn above everything else on the screen, taking the
//! mouse and keyboard until it closes. Vanilla and NeoForge have no such widget.
//!
//! The menu belongs to the screen instance it opened on and closes with it. Its entries hold
//! the mod's closures, so they are dropped and run under the mod's [`CallbackOwner`]: a panic
//! in one turns that mod off instead of taking the game down.

use crate::{
    game::{ClientGame, host::draw::GameDrawContext},
    input_trace::InputOrigin,
};
use mod_api::{__private::game::CallbackOwner, DrawContext, client::widgets::ContextMenuEntry};
use std::collections::HashSet;
use sunrise_client::{gamepad::Button as PadButton, held::Source, hud::HudBatch};
use sunrise_gameplay_common::game::api_values::TryIntoEngine;
use winit::{event::MouseButton, keyboard::KeyCode};

/// Height of a row without an icon, in GUI pixels.
const ROW: f32 = 12.0;
/// Height of a row in a level where some row has an icon.
const ICON_ROW: f32 = 18.0;
/// Height of a separator.
const SEPARATOR: f32 = 5.0;
/// Space inside the border around the rows and before each label.
const PAD: f32 = 3.0;
/// Room for a nested menu's arrow at the right of its row.
const ARROW: f32 = 10.0;

const BACKGROUND: u32 = 0xF010_0010;
const BORDER: u32 = 0xFF50_00A0;
const HOVER: u32 = 0xFF40_40A0;
const TEXT: u32 = 0xFFFF_FFFF;
const DISABLED: u32 = 0xFF80_8080;
const LINE: u32 = 0xFF50_5050;

/// Mouse input source for press-to-release ownership isolation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum MouseSource {
    #[default]
    User,
    Agent,
    Game,
}

impl From<InputOrigin> for MouseSource {
    fn from(origin: InputOrigin) -> Self {
        match origin {
            InputOrigin::User => Self::User,
            InputOrigin::Agent => Self::Agent,
            InputOrigin::Game => Self::Game,
        }
    }
}

/// Key input source for press-to-release ownership isolation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum KeySource {
    Keyboard,
    Pad,
    Agent,
}

impl From<Source> for KeySource {
    fn from(source: Source) -> Self {
        match source {
            Source::Keyboard => Self::Keyboard,
            Source::Pad => Self::Pad,
            Source::Agent => Self::Agent,
        }
    }
}

const PAD_REPEAT_OWNED: u8 = 1 << 0;
const PAD_WHEEL_OWNED: u8 = 1 << 1;
const STICK_OWNED: u8 = 1 << 2;
const RIGHT_STICK_OWNED: u8 = 1 << 3;

/// Press-to-release ownership for input consumed by a context menu, kept across menu close or
/// screen replacement so releases and repeats never leak to the screen underneath.
#[derive(Default)]
pub(super) struct ContextMenuInput {
    owned_buttons: HashSet<(MouseSource, MouseButton)>,
    owned_keys: HashSet<(KeySource, KeyCode)>,
    unowned_keys: HashSet<(KeySource, KeyCode)>,
    owned_pad: HashSet<PadButton>,
    pad_flags: u8,
}

impl ContextMenuInput {
    pub(super) fn clear(&mut self) {
        if !self.owned_buttons.is_empty() {
            self.owned_buttons.clear();
        }
        if !self.owned_keys.is_empty() {
            self.owned_keys.clear();
        }
        if !self.unowned_keys.is_empty() {
            self.unowned_keys.clear();
        }
        if !self.owned_pad.is_empty() {
            self.owned_pad.clear();
        }
        if self.pad_flags != 0 {
            self.pad_flags = 0;
        }
    }

    pub(super) fn clear_agent(&mut self) {
        if !self.owned_buttons.is_empty() {
            self.owned_buttons
                .retain(|(source, _)| *source != MouseSource::Agent);
        }
        if !self.owned_keys.is_empty() {
            self.owned_keys
                .retain(|(source, _)| *source != KeySource::Agent);
        }
        if !self.unowned_keys.is_empty() {
            self.unowned_keys
                .retain(|(source, _)| *source != KeySource::Agent);
        }
    }

    /// True only while a context menu has outstanding ownership and another source holds `key`.
    #[inline(always)]
    pub(super) fn holds_other_source(&self, releasing: Source, key: KeyCode) -> bool {
        if self.owned_keys.is_empty() {
            return false;
        }
        let releasing: KeySource = releasing.into();
        self.owned_keys
            .iter()
            .any(|&(s, k)| k == key && s != releasing)
            || self
                .unowned_keys
                .iter()
                .any(|&(s, k)| k == key && s != releasing)
    }

    #[inline(always)]
    pub(super) fn clear_unowned_keys_if_idle(&mut self) {
        if self.owned_keys.is_empty() && !self.unowned_keys.is_empty() {
            self.unowned_keys.clear();
        }
    }

    #[inline(always)]
    pub(super) fn owned_keys_empty(&self) -> bool {
        self.owned_keys.is_empty()
    }

    pub(super) fn own_pad(&mut self, button: PadButton) {
        self.owned_pad.insert(button);
    }

    pub(super) fn owns_pad(&self, button: PadButton) -> bool {
        !self.owned_pad.is_empty() && self.owned_pad.contains(&button)
    }

    pub(super) fn take_pad(&mut self, button: PadButton) -> bool {
        !self.owned_pad.is_empty() && self.owned_pad.remove(&button)
    }

    pub(super) fn own_pad_repeat(&mut self) {
        self.pad_flags |= PAD_REPEAT_OWNED;
    }

    pub(super) fn take_pad_repeat_owned(&mut self) -> bool {
        if self.pad_flags & PAD_REPEAT_OWNED != 0 {
            self.pad_flags &= !PAD_REPEAT_OWNED;
            true
        } else {
            false
        }
    }

    pub(super) fn own_pad_wheel(&mut self) {
        self.pad_flags |= PAD_WHEEL_OWNED;
    }

    pub(super) fn take_pad_wheel_owned(&mut self) -> bool {
        if self.pad_flags & PAD_WHEEL_OWNED != 0 {
            self.pad_flags &= !PAD_WHEEL_OWNED;
            true
        } else {
            false
        }
    }

    pub(super) fn own_stick(&mut self) {
        self.pad_flags |= STICK_OWNED;
    }

    pub(super) fn stick_owned(&self) -> bool {
        self.pad_flags & STICK_OWNED != 0
    }

    pub(super) fn release_stick(&mut self) {
        self.pad_flags &= !STICK_OWNED;
    }

    pub(super) fn own_right_stick(&mut self) {
        self.pad_flags |= RIGHT_STICK_OWNED;
    }

    pub(super) fn right_stick_owned(&self) -> bool {
        self.pad_flags & RIGHT_STICK_OWNED != 0
    }

    pub(super) fn release_right_stick(&mut self) {
        self.pad_flags &= !RIGHT_STICK_OWNED;
    }

    #[inline(always)]
    pub(super) fn has_pad_stick_or_repeat(&self) -> bool {
        self.pad_flags != 0
    }

    #[inline(always)]
    pub(super) fn has_pad_ownership(&self) -> bool {
        self.pad_flags != 0 || !self.owned_pad.is_empty()
    }

    pub(super) fn clear_pad_stick_and_repeat(&mut self) {
        self.pad_flags = 0;
    }
}

/// Invalidates cached resolved labels and widths when fonts or language change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextKey {
    atlas_revision: u64,
    font_epoch: u64,
    lang_generation: u64,
}

struct CachedRowText {
    label: Option<String>,
    width: f32,
    fitted: Option<String>,
}

impl CachedRowText {
    fn display_label(&self) -> Option<&str> {
        self.fitted.as_deref().or(self.label.as_deref())
    }
}

struct CachedLevelText {
    /// Parent row index in level `k - 1` that opened this level (`None` for root).
    opener: Option<usize>,
    icons: bool,
    nested: bool,
    row_height: f32,
    max_label_width: f32,
    content_tops: Vec<f32>,
    content_height: f32,
    rows: Vec<CachedRowText>,
    /// Available text width `rows[*].fitted` was computed for (`f32::INFINITY` when untruncated).
    fitted_width: f32,
}

#[derive(Default)]
struct LayoutCache {
    text_key: Option<TextKey>,
    levels: Vec<CachedLevelText>,
    viewport: [f32; 2],
    at: [f32; 2],
    scrolls: Vec<f32>,
    openers: Vec<Option<usize>>,
    boxes: Vec<LevelBox>,
    dirty: bool,
    #[cfg(test)]
    rebuilds: usize,
}

/// An open context menu.
pub(in crate::game) struct ContextMenu {
    owner: CallbackOwner,
    /// The screen instance it opened on (`ScreenState::screen_instance`).
    instance: u64,
    at: [f32; 2],
    root: Vec<ContextMenuEntry>,
    /// One entry per open level: the row selected in it (by hover or the arrow keys). Level
    /// `k + 1` is the nested menu of level `k`'s selected row.
    selected: Vec<Option<usize>>,
    /// Per-level vertical scroll offset in GUI pixels.
    scroll: Vec<f32>,
    /// Level whose selected row should be scrolled into view on the next layout pass.
    scroll_to_selected: Option<usize>,
    /// Last hovered `(level, row)` for pointer entry detection.
    last_hovered: Option<(usize, usize)>,
    cache: LayoutCache,
}

impl Drop for ContextMenu {
    fn drop(&mut self) {
        let root = std::mem::take(&mut self.root);
        self.owner
            .dispose("context menu", "context_menu_drop", root);
    }
}

/// One visible row inside a level box: its entry index, full unclipped rect for text/icon
/// layout, and clipped hit rect inside the level's visible content area.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct VisibleRow {
    pub(super) index: usize,
    pub(super) rect: [f32; 4],
    pub(super) hit_rect: [f32; 4],
}

/// Where one open level and its visible rows are, in GUI pixels (`[x, y, w, h]`).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LevelBox {
    pub(super) rect: [f32; 4],
    pub(super) content_rect: [f32; 4],
    pub(super) rows: Vec<VisibleRow>,
    pub(super) scroll: f32,
    pub(super) max_scroll: f32,
}

impl LevelBox {
    fn empty() -> Self {
        Self {
            rect: [0.0; 4],
            content_rect: [0.0; 4],
            rows: Vec::new(),
            scroll: 0.0,
            max_scroll: 0.0,
        }
    }
}

fn label_of(entry: &ContextMenuEntry) -> Option<String> {
    match entry {
        ContextMenuEntry::Action { label, .. } | ContextMenuEntry::Submenu { label, .. } => {
            let text: sunrise_client::text::Text = label.try_into_engine(&()).unwrap_or_default();
            Some(text.plain())
        }
        // A separator, or a row kind a later API adds that this game can't draw.
        _ => None,
    }
}

fn icon_of(entry: &ContextMenuEntry) -> Option<&mod_api::ItemStack> {
    match entry {
        ContextMenuEntry::Action { icon, .. } | ContextMenuEntry::Submenu { icon, .. } => {
            icon.as_ref()
        }
        _ => None,
    }
}

fn enabled(entry: &ContextMenuEntry) -> bool {
    match entry {
        ContextMenuEntry::Action { enabled, .. } | ContextMenuEntry::Submenu { enabled, .. } => {
            *enabled
        }
        _ => false,
    }
}

fn opens_submenu(entry: &ContextMenuEntry) -> bool {
    enabled(entry)
        && matches!(entry, ContextMenuEntry::Submenu { entries, .. } if !entries.is_empty())
}

fn inside([mx, my]: [f32; 2], [x, y, w, h]: [f32; 4]) -> bool {
    mx >= x && my >= y && mx < x + w && my < y + h
}

/// Vertical offset and height of the scrollbar thumb inside a `ch`-tall content area.
fn scrollbar_thumb(
    ch: f32,
    content_height: f32,
    scroll: f32,
    max_scroll: f32,
) -> Option<(f32, f32)> {
    if ch <= 0.0 || content_height <= 0.0 || max_scroll <= 0.0 {
        return None;
    }
    let thumb_h = (ch * ch / content_height).clamp(6.0_f32.min(ch), ch);
    let frac = (scroll / max_scroll).clamp(0.0, 1.0);
    Some((frac * (ch - thumb_h), thumb_h))
}

fn truncate_label(label: &str, max_width: f32, width: &dyn Fn(&str) -> f32) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    if width(label) <= max_width {
        return label.to_owned();
    }
    let ellipsis = "...";
    if width(ellipsis) > max_width {
        let mut out = String::new();
        for ch in label.chars() {
            let prev = out.len();
            out.push(ch);
            if width(&out) > max_width {
                out.truncate(prev);
                break;
            }
        }
        return out;
    }
    let mut boundaries: Vec<usize> = label.char_indices().map(|(i, _)| i).collect();
    boundaries.push(label.len());
    let mut lo = 0usize;
    let mut hi = boundaries.len() - 1;
    let mut buf = String::with_capacity(label.len().min(64) + ellipsis.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        buf.clear();
        buf.push_str(&label[..boundaries[mid]]);
        buf.push_str(ellipsis);
        if width(&buf) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    buf.clear();
    buf.push_str(&label[..boundaries[lo]]);
    buf.push_str(ellipsis);
    buf
}

fn build_level_text(
    opener: Option<usize>,
    entries: &[ContextMenuEntry],
    width: &dyn Fn(&str) -> f32,
) -> CachedLevelText {
    let icons = entries.iter().any(|entry| icon_of(entry).is_some());
    let nested = entries
        .iter()
        .any(|entry| matches!(entry, ContextMenuEntry::Submenu { .. }));
    let row_height = if icons { ICON_ROW } else { ROW };
    let mut max_label_width = 0.0f32;
    let mut content_tops = Vec::with_capacity(entries.len());
    let mut rows = Vec::with_capacity(entries.len());
    let mut top = 0.0f32;
    for entry in entries {
        content_tops.push(top);
        let h = match entry {
            ContextMenuEntry::Separator => SEPARATOR,
            _ => row_height,
        };
        top += h;
        let label = label_of(entry);
        let w = label.as_deref().map_or(0.0, width);
        max_label_width = max_label_width.max(w);
        rows.push(CachedRowText {
            label,
            width: w,
            fitted: None,
        });
    }
    CachedLevelText {
        opener,
        icons,
        nested,
        row_height,
        max_label_width,
        content_tops,
        content_height: top,
        rows,
        fitted_width: f32::INFINITY,
    }
}

impl ContextMenu {
    fn new(owner: CallbackOwner, instance: u64, at: [f32; 2], root: Vec<ContextMenuEntry>) -> Self {
        Self {
            owner,
            instance,
            at,
            root,
            selected: vec![None],
            scroll: vec![0.0],
            scroll_to_selected: None,
            last_hovered: None,
            cache: LayoutCache {
                dirty: true,
                ..LayoutCache::default()
            },
        }
    }

    /// The rows of open level `k`.
    fn level(&self, k: usize) -> &[ContextMenuEntry] {
        let mut entries = self.root.as_slice();
        for row in self.selected.iter().take(k) {
            match row.and_then(|row| entries.get(row)) {
                Some(ContextMenuEntry::Submenu {
                    entries: nested, ..
                }) => entries = nested,
                _ => return &[],
            }
        }
        entries
    }

    fn level_mut(&mut self, k: usize) -> Option<&mut Vec<ContextMenuEntry>> {
        let mut entries = &mut self.root;
        for row in self.selected.iter().take(k) {
            match entries.get_mut((*row)?)? {
                ContextMenuEntry::Submenu {
                    entries: nested, ..
                } => entries = nested,
                _ => return None,
            }
        }
        Some(entries)
    }

    /// Ensure cached text metrics and viewport-fitted geometry are current, reusing cached data
    /// when font/language, viewport, scroll offsets and open levels are unchanged.
    fn ensure_layout(
        &mut self,
        size: [f32; 2],
        text_key: TextKey,
        width: &dyn Fn(&str) -> f32,
    ) -> &[LevelBox] {
        let size = [size[0].max(0.0), size[1].max(0.0)];
        let depth = self.selected.len();
        self.scroll.resize(depth, 0.0);

        if self.cache.text_key != Some(text_key) {
            self.cache.levels.clear();
            self.cache.text_key = Some(text_key);
            self.cache.dirty = true;
        }
        self.cache.levels.truncate(depth);
        for k in 0..depth {
            let opener = (k > 0).then(|| self.selected[k - 1]).flatten();
            let valid = self
                .cache
                .levels
                .get(k)
                .is_some_and(|cached| cached.opener == opener);
            if !valid {
                self.cache.levels.truncate(k);
                let entries = self.level(k);
                let built = build_level_text(opener, entries, width);
                self.cache.levels.push(built);
                self.cache.dirty = true;
            }
        }

        let viewport_changed = self.cache.viewport != size;
        let height_shrank = size[1] < self.cache.viewport[1];
        let scroll_target = self.scroll_to_selected.take();
        for k in 0..depth {
            let level = &self.cache.levels[k];
            let natural_h = PAD * 2.0 + level.content_height;
            let h = natural_h.min(size[1]);
            let visible_h = (h - PAD * 2.0).max(0.0);
            let max_scroll = (level.content_height - visible_h).max(0.0);
            let mut s = self.scroll[k].clamp(0.0, max_scroll);
            if (scroll_target == Some(k) || height_shrank)
                && let Some(row) = self.selected[k]
                && let Some(&row_top) = level.content_tops.get(row)
            {
                let row_bottom = level
                    .content_tops
                    .get(row + 1)
                    .copied()
                    .unwrap_or(level.content_height);
                if row_top < s {
                    s = row_top;
                } else if row_bottom > s + visible_h {
                    s = (row_bottom - visible_h).max(0.0);
                }
                s = s.clamp(0.0, max_scroll);
            }
            if s != self.scroll[k] {
                self.scroll[k] = s;
                self.cache.dirty = true;
            }
        }

        let openers = &self.selected[..depth.saturating_sub(1)];
        if !self.cache.dirty
            && !viewport_changed
            && self.cache.at == self.at
            && self.cache.scrolls == self.scroll
            && self.cache.openers.as_slice() == openers
            && self.cache.boxes.len() == depth
        {
            return &self.cache.boxes;
        }

        self.cache.boxes.truncate(depth);
        while self.cache.boxes.len() < depth {
            self.cache.boxes.push(LevelBox::empty());
        }
        for k in 0..depth {
            let (icons, nested, max_label_width, content_height) = {
                let level = &self.cache.levels[k];
                (
                    level.icons,
                    level.nested,
                    level.max_label_width,
                    level.content_height,
                )
            };
            let chrome_w =
                PAD * 2.0 + if icons { ICON_ROW } else { 0.0 } + if nested { ARROW } else { 0.0 };
            let w = (chrome_w + max_label_width).min(size[0]);
            let available_text = (w - chrome_w).max(0.0);
            {
                let level = &mut self.cache.levels[k];
                if level.fitted_width != available_text {
                    for row in &mut level.rows {
                        if let Some(label) = &row.label {
                            row.fitted = (row.width > available_text)
                                .then(|| truncate_label(label, available_text, width));
                        } else {
                            row.fitted = None;
                        }
                    }
                    level.fitted_width = available_text;
                }
            }
            let h = (PAD * 2.0 + content_height).min(size[1]);
            let visible_h = (h - PAD * 2.0).max(0.0);
            let max_scroll = (content_height - visible_h).max(0.0);
            let scroll = self.scroll[k].clamp(0.0, max_scroll);
            self.scroll[k] = scroll;

            let [mut x, mut y] = if k == 0 {
                self.at
            } else {
                let parent = &self.cache.boxes[k - 1];
                let opener = self.selected[k - 1]
                    .and_then(|row| parent.rows.iter().find(|r| r.index == row))
                    .map(|r| r.hit_rect)
                    .unwrap_or(parent.rect);
                let right = parent.rect[0] + parent.rect[2];
                let x = if right + w <= size[0] {
                    right
                } else {
                    parent.rect[0] - w
                };
                [x, opener[1] - PAD]
            };
            x = x.min((size[0] - w).max(0.0)).max(0.0);
            y = y.min((size[1] - h).max(0.0)).max(0.0);

            let content_top = y + PAD;
            let content_bottom = content_top + visible_h;
            let content_rect = [x + 1.0, content_top, (w - 2.0).max(0.0), visible_h];
            let level = &self.cache.levels[k];
            let level_box = &mut self.cache.boxes[k];
            level_box.rect = [x, y, w, h];
            level_box.content_rect = content_rect;
            level_box.scroll = scroll;
            level_box.max_scroll = max_scroll;
            level_box.rows.clear();
            let first = level
                .content_tops
                .partition_point(|&t| t <= scroll)
                .saturating_sub(1);
            for index in first..level.rows.len() {
                let row_top = level.content_tops[index];
                let screen_top = content_top + row_top - scroll;
                if screen_top >= content_bottom {
                    break;
                }
                let row_bottom = level
                    .content_tops
                    .get(index + 1)
                    .copied()
                    .unwrap_or(level.content_height);
                let screen_bottom = content_top + row_bottom - scroll;
                let vis_top = screen_top.max(content_top);
                let vis_bottom = screen_bottom.min(content_bottom);
                if vis_bottom <= vis_top {
                    continue;
                }
                level_box.rows.push(VisibleRow {
                    index,
                    rect: [
                        x + 1.0,
                        screen_top,
                        (w - 2.0).max(0.0),
                        row_bottom - row_top,
                    ],
                    hit_rect: [x + 1.0, vis_top, (w - 2.0).max(0.0), vis_bottom - vis_top],
                });
            }
        }
        #[cfg(test)]
        {
            self.cache.rebuilds += 1;
        }
        self.cache.viewport = size;
        self.cache.at = self.at;
        self.cache.scrolls.clone_from(&self.scroll);
        self.cache.openers.clear();
        self.cache.openers.extend_from_slice(openers);
        self.cache.dirty = false;
        &self.cache.boxes
    }

    /// The deepest level and entry index under `mouse`.
    fn hit(boxes: &[LevelBox], mouse: [f32; 2]) -> Option<(usize, Option<usize>)> {
        boxes.iter().enumerate().rev().find_map(|(k, level)| {
            inside(mouse, level.rect).then(|| {
                (
                    k,
                    level
                        .rows
                        .iter()
                        .find(|row| inside(mouse, row.hit_rect))
                        .map(|row| row.index),
                )
            })
        })
    }

    /// Update selection and submenu state for pointer motion over `hit`: opens an enabled
    /// nonempty submenu on pointer entry even if that row was already selected without an open
    /// child, while preserving a live child selection when hovering its parent.
    fn hover_at(&mut self, hit: Option<(usize, Option<usize>)>) {
        let now = hit.and_then(|(k, row)| row.map(|r| (k, r)));
        let entered = now != self.last_hovered;
        self.last_hovered = now;
        let Some((k, row)) = now else {
            return;
        };
        let reenters_closed_submenu = entered
            && self.selected.len() == k + 1
            && self.level(k).get(row).is_some_and(opens_submenu);
        if self.selected.get(k) != Some(&Some(row)) || reenters_closed_submenu {
            self.select(k, Some(row), true);
        }
    }

    /// Scroll level `k` by `delta_lines` (positive scrolls up toward earlier rows).
    fn scroll_by(&mut self, k: usize, delta_lines: f32) {
        let Some(level_text) = self.cache.levels.get(k) else {
            return;
        };
        let Some(level_box) = self.cache.boxes.get(k) else {
            return;
        };
        if level_box.max_scroll <= 0.0 || k >= self.scroll.len() {
            return;
        }
        let next =
            (self.scroll[k] - delta_lines * level_text.row_height).clamp(0.0, level_box.max_scroll);
        if next != self.scroll[k] {
            self.scroll[k] = next;
            self.cache.dirty = true;
        }
    }

    /// Selects `row` of level `k`, closing deeper levels, and opens its nested menu when it
    /// is an enabled, non-empty submenu.
    fn select(&mut self, k: usize, row: Option<usize>, open_nested: bool) {
        let closed_deeper = self.selected.len() > k + 1;
        self.selected.truncate(k + 1);
        self.scroll.truncate(k + 1);
        self.selected[k] = row;
        let opens = row
            .and_then(|row| self.level(k).get(row))
            .is_some_and(opens_submenu);
        if opens && open_nested {
            self.selected.push(None);
            self.scroll.push(0.0);
        }
        if closed_deeper || (opens && open_nested) {
            self.cache.dirty = true;
        }
    }

    fn close_deepest_level(&mut self) {
        if self.selected.len() > 1 {
            let keep = self.selected.len() - 1;
            self.selected.truncate(keep);
            self.scroll.truncate(keep);
            self.cache.dirty = true;
        }
    }

    /// The next selectable row of the deepest level after (or before, `step < 0`) the
    /// selected one, wrapping around, and marking it to scroll into view.
    fn step(&mut self, step: isize) {
        let k = self.selected.len() - 1;
        let entries = self.level(k);
        let len = entries.len() as isize;
        if len == 0 {
            return;
        }
        let mut at = self.selected[k].map_or(if step > 0 { -1 } else { len }, |r| r as isize);
        for _ in 0..len {
            at = (at + step).rem_euclid(len);
            if !matches!(entries[at as usize], ContextMenuEntry::Separator) {
                self.selected[k] = Some(at as usize);
                self.selected.truncate(k + 1);
                self.scroll.truncate(k + 1);
                self.scroll_to_selected = Some(k);
                return;
            }
        }
    }
}

/// What a press or key asked of the menu.
enum Outcome {
    Stay,
    Close,
    Pick(usize, usize),
}

impl ClientGame<'_> {
    /// `ClientHost::open_context_menu`: false (nothing opened) with no screen open, an empty
    /// list, while a pointer grab is active or pending, or outside a mod's call.
    pub(in crate::game) fn open_context_menu(
        &mut self,
        at: [f32; 2],
        entries: Vec<ContextMenuEntry>,
    ) -> bool {
        let Some(id) = mod_api::content::crash::current_native_mod() else {
            return false;
        };
        let owner = CallbackOwner::for_mod(id);
        if self.client.pointer_grab_request.get() == Some(false) {
            self.apply_pointer_grab_request();
        }
        let pointer_grabbed = self.client.pointer_grab_request.get() == Some(true)
            || self.mod_screen_grabs_pointer()
            || self.client.captured;
        if entries.is_empty()
            || !owner.is_enabled()
            || self.observed_screen_view().is_none()
            || !at.iter().all(|v| v.is_finite())
            || pointer_grabbed
        {
            owner.dispose("context menu", "context_menu_drop", entries);
            return false;
        }
        self.snapshot_unowned_keys_for_context_menu();
        self.client.screen_state.0.context_menu = Some(ContextMenu::new(
            owner,
            self.client.screen_state.0.screen_instance,
            at,
            entries,
        ));
        sunrise_client::preedit::reset();
        self.sync_ime();
        self.settle_pad_for_context_menu();
        true
    }

    /// Snapshot keys already held before a menu opens so only presses consumed by the menu are
    /// excluded from `restore_held_keys`.
    #[inline(always)]
    fn snapshot_unowned_keys_for_context_menu(&mut self) {
        let input = &mut self.client.screen_state.0.context_menu_input;
        for &key in &self.client.physical_keys {
            let source = if self.client.keys.held_by(&key, Source::Agent) {
                KeySource::Agent
            } else {
                KeySource::Keyboard
            };
            if !input.owned_keys.contains(&(source, key)) {
                input.unowned_keys.insert((source, key));
            }
        }
    }

    /// `ClientHost::close_context_menu`: false when none is open.
    pub(in crate::game) fn close_context_menu(&mut self) -> bool {
        self.live_context_menu();
        let closed = self.client.screen_state.0.context_menu.take().is_some();
        if closed {
            self.client
                .screen_state
                .0
                .context_menu_input
                .clear_unowned_keys_if_idle();
            self.clear_context_menu_pad_state();
            self.sync_ime();
        }
        closed
    }

    /// Single entry gate for keyboard dispatch: false when no menu is open and no key is owned.
    #[inline(always)]
    pub(super) fn context_menu_key_active(&self) -> bool {
        self.client.screen_state.0.context_menu.is_some()
            || !self
                .client
                .screen_state
                .0
                .context_menu_input
                .owned_keys
                .is_empty()
    }

    /// Single entry gate for controller menu polling: false when no menu or pad ownership is live.
    #[inline(always)]
    pub(super) fn context_menu_pad_active(&self) -> bool {
        self.client.screen_state.0.context_menu.is_some()
            || self
                .client
                .screen_state
                .0
                .context_menu_input
                .has_pad_ownership()
    }

    pub(in crate::game) fn has_context_menu(&self) -> bool {
        self.client.screen_state.0.context_menu.is_some()
    }

    /// Read-only live check for `&self` callers (such as `set_pointer_grab`).
    #[inline(always)]
    pub(super) fn has_live_context_menu(&self) -> bool {
        self.client
            .screen_state
            .0
            .context_menu
            .as_ref()
            .is_some_and(|menu| {
                menu.instance == self.client.screen_state.0.screen_instance
                    && menu.owner.is_enabled()
                    && self.observed_screen_view().is_some()
            })
    }

    /// Whether a menu is open over the current screen (dropping one whose screen closed or
    /// whose owner was disabled).
    #[inline(always)]
    pub(super) fn context_menu_open(&mut self) -> bool {
        self.live_context_menu()
    }

    /// Drops a menu whose screen closed, was replaced, or whose owner was disabled; whether
    /// one is still open. Closed path is a single `Option::is_none` check.
    #[inline(always)]
    pub(in crate::game) fn live_context_menu(&mut self) -> bool {
        let Some(menu) = self.client.screen_state.0.context_menu.as_ref() else {
            return false;
        };
        if menu.instance != self.client.screen_state.0.screen_instance
            || !menu.owner.is_enabled()
            || self.observed_screen_view().is_none()
        {
            self.client.screen_state.0.context_menu = None;
            self.client
                .screen_state
                .0
                .context_menu_input
                .clear_unowned_keys_if_idle();
            self.clear_context_menu_pad_state();
            return false;
        }
        true
    }

    #[inline(always)]
    fn context_menu_text_key(&self) -> TextKey {
        TextKey {
            atlas_revision: self.client.gui.atlas.revision,
            font_epoch: self.client.gui.atlas.font.epoch(),
            lang_generation: sunrise_game_data::lang::generation(),
        }
    }

    #[inline(always)]
    fn ensure_context_menu_layout(&mut self, size: [f32; 2]) {
        let key = self.context_menu_text_key();
        let atlas = &self.client.gui.atlas;
        if let Some(menu) = self.client.screen_state.0.context_menu.as_mut() {
            menu.ensure_layout(size, key, &|text| HudBatch::text_width(atlas, text));
        }
    }

    /// A mouse button while a menu is open: presses are owned by the menu through their
    /// matching release (even if the press closes the menu or replaces the screen), while
    /// releases for presses the screen already owned pass through for drag/widget cleanup.
    pub(super) fn context_menu_mouse(
        &mut self,
        source: MouseSource,
        button: MouseButton,
        pressed: bool,
    ) -> bool {
        let key = (source, button);
        if !pressed {
            return !self
                .client
                .screen_state
                .0
                .context_menu_input
                .owned_buttons
                .is_empty()
                && self
                    .client
                    .screen_state
                    .0
                    .context_menu_input
                    .owned_buttons
                    .remove(&key);
        }
        if !self.live_context_menu() {
            if !self
                .client
                .screen_state
                .0
                .context_menu_input
                .owned_buttons
                .is_empty()
            {
                self.client
                    .screen_state
                    .0
                    .context_menu_input
                    .owned_buttons
                    .remove(&key);
            }
            return false;
        }
        self.client
            .screen_state
            .0
            .context_menu_input
            .owned_buttons
            .insert(key);
        let (w, h) = self.gui_size();
        self.ensure_context_menu_layout([w, h]);
        let mouse = self.cursor();
        let left = button == MouseButton::Left;
        let Some(menu) = self.client.screen_state.0.context_menu.as_mut() else {
            return true;
        };
        let outcome = match ContextMenu::hit(&menu.cache.boxes, mouse) {
            None => Outcome::Close,
            Some((_, None)) => Outcome::Stay,
            Some((k, Some(row))) => {
                let entry = &menu.level(k)[row];
                if !left || !enabled(entry) {
                    Outcome::Stay
                } else if matches!(entry, ContextMenuEntry::Action { .. }) {
                    Outcome::Pick(k, row)
                } else {
                    menu.select(k, Some(row), true);
                    Outcome::Stay
                }
            }
        };
        self.finish_context_menu(outcome);
        true
    }

    /// Pointer motion while a menu is open: updates menu hover and stops delivery below the
    /// menu. Returns `false` when no menu is open.
    pub(super) fn context_menu_hover(&mut self, size: [f32; 2]) -> bool {
        if !self.live_context_menu() {
            return false;
        }
        self.ensure_context_menu_layout(size);
        let mouse = self.cursor();
        if let Some(menu) = self.client.screen_state.0.context_menu.as_mut() {
            let hit = ContextMenu::hit(&menu.cache.boxes, mouse);
            menu.hover_at(hit);
        }
        true
    }

    /// Mouse wheel while a menu is open: scrolls the hovered (or deepest) level and consumes
    /// the wheel so screens and widgets underneath never see it.
    #[inline(always)]
    pub(super) fn context_menu_wheel(&mut self, delta: f32) -> bool {
        if !self.live_context_menu() {
            return false;
        }
        if delta == 0.0 || !delta.is_finite() {
            return true;
        }
        let (w, h) = self.gui_size();
        self.ensure_context_menu_layout([w, h]);
        let mouse = self.cursor();
        if let Some(menu) = self.client.screen_state.0.context_menu.as_mut() {
            let target = ContextMenu::hit(&menu.cache.boxes, mouse)
                .map(|(k, _)| k)
                .unwrap_or_else(|| menu.selected.len().saturating_sub(1));
            menu.scroll_by(target, delta);
        }
        self.ensure_context_menu_layout([w, h]);
        true
    }

    /// Whether `key`'s current press from `source` is owned by the context menu (so its
    /// repeats/text/release stay out of the screen underneath even after the menu closes).
    #[inline(always)]
    pub(super) fn context_menu_owns_key(&self, source: Source, key: KeyCode) -> bool {
        !self
            .client
            .screen_state
            .0
            .context_menu_input
            .owned_keys
            .is_empty()
            && self
                .client
                .screen_state
                .0
                .context_menu_input
                .owned_keys
                .contains(&(source.into(), key))
    }

    /// Source bitmask (`1` = keyboard, `2` = agent) of non-menu holds for `key` when another
    /// source's press of `key` is owned by the context menu; `None` when no source is menu-owned.
    #[inline(always)]
    pub(super) fn context_menu_restore_sources(&self, key: KeyCode) -> Option<u8> {
        let input = &self.client.screen_state.0.context_menu_input;
        if input.owned_keys.is_empty() || !input.owned_keys.iter().any(|(_, k)| *k == key) {
            return None;
        }
        let mut mask = 0u8;
        if input.unowned_keys.contains(&(KeySource::Keyboard, key)) {
            mask |= 1;
        }
        if input.unowned_keys.contains(&(KeySource::Agent, key)) {
            mask |= 2;
        }
        Some(mask)
    }

    /// Shared window/socket key press, repeat and release routing for the context menu.
    #[inline(always)]
    pub(super) fn context_menu_key_event(
        &mut self,
        source: Source,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
    ) -> bool {
        if self.client.screen_state.0.context_menu.is_none()
            && self
                .client
                .screen_state
                .0
                .context_menu_input
                .owned_keys
                .is_empty()
            && self
                .client
                .screen_state
                .0
                .context_menu_input
                .unowned_keys
                .is_empty()
        {
            return false;
        }
        let owned = (source.into(), key);
        if !pressed {
            let input = &mut self.client.screen_state.0.context_menu_input;
            input.unowned_keys.remove(&owned);
            let swallowed = input.owned_keys.remove(&owned);
            if self.client.screen_state.0.context_menu.is_none()
                && input.owned_keys.is_empty()
                && !input.unowned_keys.is_empty()
            {
                input.unowned_keys.clear();
            }
            return swallowed;
        }
        if repeat {
            if self
                .client
                .screen_state
                .0
                .context_menu_input
                .owned_keys
                .contains(&owned)
            {
                if self.live_context_menu() && matches!(key, KeyCode::ArrowUp | KeyCode::ArrowDown)
                {
                    let _ = self.context_menu_key(key);
                }
                return true;
            }
            return self.live_context_menu();
        }
        if !self.live_context_menu() {
            let input = &mut self.client.screen_state.0.context_menu_input;
            input.owned_keys.remove(&owned);
            if input.owned_keys.is_empty() {
                if !input.unowned_keys.is_empty() {
                    input.unowned_keys.clear();
                }
            } else if input.owned_keys.iter().any(|(_, k)| *k == key) {
                input.unowned_keys.insert(owned);
            }
            return false;
        }
        self.client
            .screen_state
            .0
            .context_menu_input
            .unowned_keys
            .remove(&owned);
        self.client
            .screen_state
            .0
            .context_menu_input
            .owned_keys
            .insert(owned);
        self.context_menu_key(key)
    }

    /// A key while a menu is open: Escape closes it, Up and Down move between rows, Right or
    /// Enter opens a nested menu, Left closes one, Enter picks an action. Every key is the
    /// menu's. False when no menu is open.
    pub(super) fn context_menu_key(&mut self, key: KeyCode) -> bool {
        if !self.live_context_menu() {
            return false;
        }
        let Some(menu) = self.client.screen_state.0.context_menu.as_mut() else {
            return true;
        };
        let k = menu.selected.len() - 1;
        let outcome = match key {
            KeyCode::Escape => Outcome::Close,
            KeyCode::ArrowUp => {
                menu.step(-1);
                Outcome::Stay
            }
            KeyCode::ArrowDown => {
                menu.step(1);
                Outcome::Stay
            }
            KeyCode::ArrowLeft => {
                menu.close_deepest_level();
                Outcome::Stay
            }
            KeyCode::ArrowRight | KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                let chosen = menu.selected[k].map(|row| {
                    let entry = &menu.level(k)[row];
                    let action = matches!(entry, ContextMenuEntry::Action { .. });
                    (row, action, enabled(entry))
                });
                match chosen {
                    Some((row, true, true)) if key != KeyCode::ArrowRight => Outcome::Pick(k, row),
                    Some((row, false, _)) => {
                        menu.select(k, Some(row), true);
                        if menu.selected.len() > k + 1 {
                            menu.step(1);
                        }
                        Outcome::Stay
                    }
                    _ => Outcome::Stay,
                }
            }
            _ => Outcome::Stay,
        };
        self.finish_context_menu(outcome);
        true
    }

    #[inline(always)]
    fn finish_context_menu(&mut self, outcome: Outcome) {
        let (k, row) = match outcome {
            Outcome::Stay => return,
            Outcome::Close => {
                self.client.screen_state.0.context_menu = None;
                self.client
                    .screen_state
                    .0
                    .context_menu_input
                    .clear_unowned_keys_if_idle();
                self.clear_context_menu_pad_state();
                self.sync_ime();
                return;
            }
            Outcome::Pick(k, row) => (k, row),
        };
        // The menu closes first, then the action runs: it may open another menu or screen.
        let Some(mut menu) = self.client.screen_state.0.context_menu.take() else {
            return;
        };
        let picked = menu
            .level_mut(k)
            .and_then(|entries| entries.get_mut(row))
            .map(|entry| std::mem::replace(entry, ContextMenuEntry::Separator));
        let owner = menu.owner.clone();
        drop(menu);
        self.client
            .screen_state
            .0
            .context_menu_input
            .clear_unowned_keys_if_idle();
        self.clear_context_menu_pad_state();
        self.sync_ime();
        let Some(ContextMenuEntry::Action { on_select, .. }) = picked else {
            return;
        };
        if owner.is_enabled() {
            let _ = owner.call("context menu", "context_menu_select", || on_select(self));
        } else {
            owner.dispose("context menu", "context_menu_drop", on_select);
        }
        self.sync_ime();
    }

    /// Draws the open menu above the active screen and all overlays using the batch's GUI size.
    pub(super) fn draw_context_menu(&mut self, batch: &mut HudBatch) {
        if !self.live_context_menu() {
            return;
        }
        self.ensure_context_menu_layout([batch.width, batch.height]);
        let Some(menu) = self.client.screen_state.0.context_menu.as_ref() else {
            return;
        };
        let mouse = self.cursor();
        let hovered = ContextMenu::hit(&menu.cache.boxes, mouse);
        let mut draw = GameDrawContext::new(self, batch);
        for (k, level) in menu.cache.boxes.iter().enumerate() {
            let [x, y, w, h] = level.rect;
            if w <= 0.0 || h <= 0.0 {
                continue;
            }
            draw.fill([x, y, w, h], BORDER);
            if w > 2.0 && h > 2.0 {
                draw.fill([x + 1.0, y + 1.0, w - 2.0, h - 2.0], BACKGROUND);
            }
            let entries = menu.level(k);
            let cached_level = &menu.cache.levels[k];
            let icons = cached_level.icons;
            let draw_rows = |draw: &mut dyn DrawContext| {
                for row in &level.rows {
                    let Some(entry) = entries.get(row.index) else {
                        continue;
                    };
                    let [rx, ry, rw, rh] = row.rect;
                    if matches!(entry, ContextMenuEntry::Separator) {
                        draw.fill([rx + PAD, ry + rh / 2.0 - 0.5, rw - PAD * 2.0, 1.0], LINE);
                        continue;
                    }
                    let on = enabled(entry);
                    let lit = hovered == Some((k, Some(row.index)))
                        || menu.selected.get(k) == Some(&Some(row.index));
                    if on && lit {
                        draw.fill(row.hit_rect, HOVER);
                    }
                    let mut text_x = rx + PAD - 1.0;
                    if icons {
                        if let Some(icon) = icon_of(entry) {
                            draw.item_stack(icon, [text_x, ry + 1.0], 16.0);
                        }
                        text_x += ICON_ROW;
                    }
                    let color = if on { TEXT } else { DISABLED };
                    let text_y = ry + (rh - 8.0) / 2.0;
                    if let Some(label) = cached_level
                        .rows
                        .get(row.index)
                        .and_then(CachedRowText::display_label)
                    {
                        draw.text(label, [text_x, text_y], color, true);
                    }
                    if matches!(entry, ContextMenuEntry::Submenu { .. }) {
                        draw.text(">", [rx + rw - ARROW + 2.0, text_y], color, true);
                    }
                }
            };
            if level.max_scroll > 0.0 && level.content_rect[2] > 0.0 && level.content_rect[3] > 0.0
            {
                let _ = draw.with_scissor(level.content_rect, &mut |draw| draw_rows(draw));
                let [cx, cy, cw, ch] = level.content_rect;
                if let Some((offset, thumb_h)) = scrollbar_thumb(
                    ch,
                    cached_level.content_height,
                    level.scroll,
                    level.max_scroll,
                ) {
                    draw.fill([cx + cw - 2.0, cy + offset, 2.0, thumb_h], LINE);
                }
            } else {
                draw_rows(&mut draw);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn noop() -> impl FnOnce(&mut dyn mod_api::ClientHost) + Send + 'static {
        |_| {}
    }

    fn menu(root: Vec<ContextMenuEntry>) -> ContextMenu {
        ContextMenu::new(CallbackOwner::for_mod("test_mod"), 1, [10.0, 10.0], root)
    }

    fn width(text: &str) -> f32 {
        text.chars().count() as f32 * 6.0
    }

    fn key(atlas_revision: u64, font_epoch: u64, lang_generation: u64) -> TextKey {
        TextKey {
            atlas_revision,
            font_epoch,
            lang_generation,
        }
    }

    #[test]
    fn levels_stay_inside_the_window_and_nest_beside_their_row() {
        let mut m = menu(vec![
            ContextMenuEntry::action(mod_api::Text::literal("Copy"), noop()),
            ContextMenuEntry::separator(),
            ContextMenuEntry::submenu(
                mod_api::Text::literal("More"),
                vec![ContextMenuEntry::action(
                    mod_api::Text::literal("Paste"),
                    noop(),
                )],
            ),
        ]);
        let k = key(1, 1, 1);
        let boxes = m.ensure_layout([200.0, 100.0], k, &width);
        assert_eq!(boxes.len(), 1);
        // "Copy" and "More" are 24 wide, plus padding and the nested arrow.
        assert_eq!(boxes[0].rect, [10.0, 10.0, 40.0, 35.0]);
        assert_eq!(boxes[0].rows[1].rect[3], SEPARATOR);

        m.select(0, Some(2), true);
        let boxes = m.ensure_layout([200.0, 100.0], k, &width);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[1].rect[0], 50.0, "opens at its parent's right edge");
        assert_eq!(boxes[1].rect[1], boxes[0].rows[2].hit_rect[1] - PAD);

        // Near the bottom right corner both levels move back inside.
        m.at = [190.0, 95.0];
        let boxes = m.ensure_layout([200.0, 100.0], k, &width);
        for level in boxes {
            let [x, y, w, h] = level.rect;
            assert!(
                x >= 0.0 && y >= 0.0 && x + w <= 200.0 && y + h <= 100.0,
                "{level:?}"
            );
        }
        assert!(
            boxes[1].rect[0] < boxes[0].rect[0],
            "nests on the left with no room right"
        );
    }

    #[test]
    fn the_arrow_keys_skip_separators_and_wrap() {
        let mut m = menu(vec![
            ContextMenuEntry::action(mod_api::Text::literal("A"), noop()),
            ContextMenuEntry::separator(),
            ContextMenuEntry::action(mod_api::Text::literal("B"), noop()).enabled(false),
        ]);
        m.step(1);
        assert_eq!(m.selected, vec![Some(0)]);
        m.step(1);
        assert_eq!(m.selected, vec![Some(2)]);
        m.step(1);
        assert_eq!(m.selected, vec![Some(0)]);
        m.step(-1);
        assert_eq!(m.selected, vec![Some(2)]);
    }

    #[test]
    fn a_disabled_or_empty_nested_menu_does_not_open() {
        let mut m = menu(vec![
            ContextMenuEntry::submenu(
                mod_api::Text::literal("Off"),
                vec![ContextMenuEntry::action(
                    mod_api::Text::literal("X"),
                    noop(),
                )],
            )
            .enabled(false),
            ContextMenuEntry::submenu(mod_api::Text::literal("Empty"), Vec::new()),
            ContextMenuEntry::submenu(
                mod_api::Text::literal("On"),
                vec![ContextMenuEntry::action(
                    mod_api::Text::literal("Y"),
                    noop(),
                )],
            ),
        ]);
        m.select(0, Some(0), true);
        assert_eq!(m.selected.len(), 1);
        m.select(0, Some(1), true);
        assert_eq!(m.selected.len(), 1);
        m.select(0, Some(2), true);
        assert_eq!(m.selected.len(), 2);
        assert_eq!(m.level(1).len(), 1);
        // Selecting another row closes the nested level.
        m.select(0, Some(0), true);
        assert_eq!(m.selected, vec![Some(0)]);
    }

    #[test]
    fn hits_find_the_deepest_level_first() {
        let row = |index, rect| VisibleRow {
            index,
            rect,
            hit_rect: rect,
        };
        let boxes = vec![
            LevelBox {
                rect: [0.0, 0.0, 50.0, 50.0],
                content_rect: [1.0, 3.0, 48.0, 44.0],
                rows: vec![row(0, [1.0, 3.0, 48.0, 12.0])],
                scroll: 0.0,
                max_scroll: 0.0,
            },
            LevelBox {
                rect: [40.0, 0.0, 50.0, 50.0],
                content_rect: [41.0, 3.0, 48.0, 44.0],
                rows: vec![row(0, [41.0, 3.0, 48.0, 12.0])],
                scroll: 0.0,
                max_scroll: 0.0,
            },
        ];
        assert_eq!(ContextMenu::hit(&boxes, [45.0, 5.0]), Some((1, Some(0))));
        assert_eq!(ContextMenu::hit(&boxes, [5.0, 5.0]), Some((0, Some(0))));
        assert_eq!(ContextMenu::hit(&boxes, [5.0, 40.0]), Some((0, None)));
        assert_eq!(ContextMenu::hit(&boxes, [95.0, 5.0]), None);
    }

    #[test]
    fn oversized_menu_clamps_truncates_scrolls_and_tracks_resize() {
        // 12 rows without icons have natural height 150 > 100px viewport, and wide labels > 80px.
        let entries: Vec<_> = (0..12)
            .map(|i| {
                ContextMenuEntry::action(
                    mod_api::Text::literal(format!("VeryLongActionLabelNumber_{i:02}")),
                    noop(),
                )
            })
            .collect();
        let mut m = menu(entries);
        let k = key(1, 1, 1);

        let boxes = m.ensure_layout([80.0, 100.0], k, &width);
        let [x, y, w, h] = boxes[0].rect;
        assert_eq!([x, y, w, h], [0.0, 0.0, 80.0, 100.0]);
        assert!(boxes[0].max_scroll > 0.0);
        // Bottom row (index 11) is scrolled out of view initially.
        assert!(!boxes[0].rows.iter().any(|r| r.index == 11));
        // Wide labels are truncated to fit inside the available text width (80 - 2*PAD = 74).
        let fitted = m.cache.levels[0].rows[0].display_label().unwrap();
        assert!(fitted.ends_with("..."), "truncated label: {fitted}");
        assert!(width(fitted) <= 80.0 - PAD * 2.0);

        // Scrolling down brings row 11 into view and makes it hit-testable at its visible rect.
        m.scroll_by(0, -10.0);
        let boxes = m.ensure_layout([80.0, 100.0], k, &width);
        let row11 = boxes[0]
            .rows
            .iter()
            .find(|r| r.index == 11)
            .expect("row 11 visible after scrolling down");
        assert!(row11.hit_rect[1] >= boxes[0].content_rect[1]);
        assert!(row11.hit_rect[1] + row11.hit_rect[3] <= 100.0);
        let center = [
            row11.hit_rect[0] + row11.hit_rect[2] / 2.0,
            row11.hit_rect[1] + row11.hit_rect[3] / 2.0,
        ];
        assert_eq!(ContextMenu::hit(boxes, center), Some((0, Some(11))));

        // Keyboard stepping wraps to row 0 and scrolls it back into view.
        m.selected[0] = Some(11);
        m.step(1);
        assert_eq!(m.selected[0], Some(0));
        let boxes = m.ensure_layout([80.0, 100.0], k, &width);
        assert_eq!(boxes[0].scroll, 0.0);
        assert_eq!(boxes[0].rows[0].index, 0);

        // Stepping up wraps to row 11 and scrolls row 11 into view.
        m.step(-1);
        assert_eq!(m.selected[0], Some(11));
        let boxes = m.ensure_layout([80.0, 100.0], k, &width);
        assert!(boxes[0].rows.iter().any(|r| r.index == 11));

        // Resize larger: full label fits untruncated and scroll resets to 0.
        let boxes = m.ensure_layout([300.0, 200.0], k, &width);
        assert_eq!(boxes[0].max_scroll, 0.0);
        assert_eq!(boxes[0].scroll, 0.0);
        assert_eq!(boxes[0].rows.len(), 12);
        assert_eq!(
            m.cache.levels[0].rows[0].display_label(),
            Some("VeryLongActionLabelNumber_00")
        );

        // Resize back smaller with row 11 selected: box clamps and keeps row 11 scrolled in view.
        let boxes = m.ensure_layout([60.0, 80.0], k, &width);
        assert!(boxes[0].rect[2] <= 60.0 && boxes[0].rect[3] <= 80.0);
        assert!(boxes[0].rows.iter().any(|r| r.index == 11));
        let fitted = m.cache.levels[0].rows[11].display_label().unwrap();
        assert!(width(fitted) <= 60.0 - PAD * 2.0);
    }

    #[test]
    fn submenu_reopens_on_pointer_entry_and_preserves_live_child_selection() {
        let mut m = menu(vec![
            ContextMenuEntry::action(mod_api::Text::literal("Top"), noop()),
            ContextMenuEntry::submenu(
                mod_api::Text::literal("More"),
                vec![
                    ContextMenuEntry::action(mod_api::Text::literal("SubA"), noop()),
                    ContextMenuEntry::action(mod_api::Text::literal("SubB"), noop()),
                ],
            ),
        ]);
        // Keyboard Down selects submenu row 1 without opening its child.
        m.step(1);
        m.step(1);
        assert_eq!(m.selected, vec![Some(1)]);

        // Moving the pointer onto already-selected submenu row 1 opens its child.
        m.hover_at(Some((0, Some(1))));
        assert_eq!(m.selected, vec![Some(1), None]);

        // Selecting a child row and then hovering its parent preserves the live child selection.
        m.hover_at(Some((1, Some(1))));
        assert_eq!(m.selected, vec![Some(1), Some(1)]);
        m.hover_at(Some((0, Some(1))));
        assert_eq!(m.selected, vec![Some(1), Some(1)]);

        // Left closes the child; staying on the row does not immediately snap it back open,
        // while leaving and re-entering that row reopens it.
        m.close_deepest_level();
        assert_eq!(m.selected, vec![Some(1)]);
        m.hover_at(Some((0, Some(1))));
        assert_eq!(m.selected, vec![Some(1)]);
        m.hover_at(None);
        m.hover_at(Some((0, Some(1))));
        assert_eq!(m.selected, vec![Some(1), None]);
    }

    #[test]
    fn layout_cache_reuses_metrics_and_invalidates_on_font_lang_or_level_change() {
        let entries: Vec<_> = (0..120)
            .map(|i| {
                if i == 0 {
                    ContextMenuEntry::submenu(
                        mod_api::Text::literal("Sub"),
                        vec![ContextMenuEntry::action(
                            mod_api::Text::literal("Child"),
                            noop(),
                        )],
                    )
                } else {
                    ContextMenuEntry::action(mod_api::Text::literal(format!("Row {i}")), noop())
                }
            })
            .collect();
        let mut m = menu(entries);
        let calls = Cell::new(0usize);
        let measure = |text: &str| {
            calls.set(calls.get() + 1);
            width(text)
        };

        let k1 = key(1, 1, 1);
        let _ = m.ensure_layout([200.0, 120.0], k1, &measure);
        assert_eq!(calls.get(), 120, "initial build measures each row once");

        // Repeated steady-state frame/hover layout calls do zero measurement work.
        calls.set(0);
        let before_rebuilds = m.cache.rebuilds;
        let rows_ptr = m.cache.boxes[0].rows.as_ptr();
        for _ in 0..10 {
            let _ = m.ensure_layout([200.0, 120.0], k1, &measure);
        }
        assert_eq!(calls.get(), 0, "steady-state frames reuse cached metrics");
        assert_eq!(m.cache.rebuilds, before_rebuilds);

        // Crossing ordinary rows does not invalidate or reallocate row geometry.
        for row in 1..6 {
            m.hover_at(Some((0, Some(row))));
            let _ = m.ensure_layout([200.0, 120.0], k1, &measure);
        }
        assert_eq!(calls.get(), 0);
        assert_eq!(
            m.cache.rebuilds, before_rebuilds,
            "ordinary hover changes keep cached boxes"
        );
        assert_eq!(m.cache.boxes[0].rows.as_ptr(), rows_ptr);

        // Scrolling reuses cached text widths and the existing visible-row buffer.
        let rows_cap = m.cache.boxes[0].rows.capacity();
        m.scroll_by(0, -3.0);
        let _ = m.ensure_layout([200.0, 120.0], k1, &measure);
        assert_eq!(calls.get(), 0, "scrolling reuses cached label widths");
        assert!(m.cache.boxes[0].rows.capacity() >= rows_cap);

        // Opening a submenu measures only the newly opened child level.
        m.select(0, Some(0), true);
        let _ = m.ensure_layout([200.0, 120.0], k1, &measure);
        assert_eq!(calls.get(), 1, "only new submenu level is measured");

        // Font/language change invalidates and re-measures open levels.
        calls.set(0);
        let k2 = key(1, 1, 2);
        let _ = m.ensure_layout([200.0, 120.0], k2, &measure);
        assert_eq!(calls.get(), 121, "language bump invalidates cached levels");
    }

    #[test]
    fn scrollbar_thumb_handles_tiny_viewports_without_panicking() {
        let entries: Vec<_> = (0..12)
            .map(|i| ContextMenuEntry::action(mod_api::Text::literal(format!("Row {i}")), noop()))
            .collect();
        let mut m = menu(entries);
        let k = key(1, 1, 1);
        for h in 7..=11 {
            let (ch, scroll, max_scroll) = {
                let boxes = m.ensure_layout([100.0, h as f32], k, &width);
                (
                    boxes[0].content_rect[3],
                    boxes[0].scroll,
                    boxes[0].max_scroll,
                )
            };
            assert!(ch > 0.0 && ch < 6.0);
            let (offset, thumb_h) =
                scrollbar_thumb(ch, m.cache.levels[0].content_height, scroll, max_scroll)
                    .expect("thumb present for scrollable tiny viewport");
            assert!(offset >= 0.0 && thumb_h > 0.0 && offset + thumb_h <= ch + 1e-4);
        }
    }
}
