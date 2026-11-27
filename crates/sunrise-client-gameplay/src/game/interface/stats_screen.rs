//! The Statistics screen (`StatsScreen`): General, Items and Mobs tabs over
//! the player's statistics, with the Done button.
use super::Screen;
use sunrise_client::{hud::HudBatch, ui};
use sunrise_core::java_number::{double_to_string, format_decimal2};
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use winit::keyboard::KeyCode;

/// `Stats.CUSTOM` in registration order with its formatter.
const CUSTOM: &[(&str, Format)] = &[
    ("leave_game", Format::Default),
    ("play_time", Format::Time),
    ("total_world_time", Format::Time),
    ("time_since_death", Format::Time),
    ("time_since_rest", Format::Time),
    ("sneak_time", Format::Time),
    ("walk_one_cm", Format::Distance),
    ("crouch_one_cm", Format::Distance),
    ("sprint_one_cm", Format::Distance),
    ("walk_on_water_one_cm", Format::Distance),
    ("fall_one_cm", Format::Distance),
    ("climb_one_cm", Format::Distance),
    ("fly_one_cm", Format::Distance),
    ("walk_under_water_one_cm", Format::Distance),
    ("minecart_one_cm", Format::Distance),
    ("boat_one_cm", Format::Distance),
    ("pig_one_cm", Format::Distance),
    ("happy_ghast_one_cm", Format::Distance),
    ("horse_one_cm", Format::Distance),
    ("aviate_one_cm", Format::Distance),
    ("swim_one_cm", Format::Distance),
    ("strider_one_cm", Format::Distance),
    ("nautilus_one_cm", Format::Distance),
    ("jump", Format::Default),
    ("drop", Format::Default),
    ("damage_dealt", Format::Tenths),
    ("damage_dealt_absorbed", Format::Tenths),
    ("damage_dealt_resisted", Format::Tenths),
    ("damage_taken", Format::Tenths),
    ("damage_blocked_by_shield", Format::Tenths),
    ("damage_absorbed", Format::Tenths),
    ("damage_resisted", Format::Tenths),
    ("deaths", Format::Default),
    ("mob_kills", Format::Default),
    ("animals_bred", Format::Default),
    ("player_kills", Format::Default),
    ("fish_caught", Format::Default),
    ("talked_to_villager", Format::Default),
    ("traded_with_villager", Format::Default),
    ("eat_cake_slice", Format::Default),
    ("fill_cauldron", Format::Default),
    ("use_cauldron", Format::Default),
    ("clean_armor", Format::Default),
    ("clean_banner", Format::Default),
    ("clean_shulker_box", Format::Default),
    ("interact_with_brewingstand", Format::Default),
    ("interact_with_beacon", Format::Default),
    ("inspect_dropper", Format::Default),
    ("inspect_hopper", Format::Default),
    ("inspect_dispenser", Format::Default),
    ("play_noteblock", Format::Default),
    ("tune_noteblock", Format::Default),
    ("pot_flower", Format::Default),
    ("trigger_trapped_chest", Format::Default),
    ("open_enderchest", Format::Default),
    ("enchant_item", Format::Default),
    ("play_record", Format::Default),
    ("interact_with_furnace", Format::Default),
    ("interact_with_crafting_table", Format::Default),
    ("open_chest", Format::Default),
    ("sleep_in_bed", Format::Default),
    ("sleep_in_straw_bed", Format::Default),
    ("open_shulker_box", Format::Default),
    ("open_barrel", Format::Default),
    ("interact_with_blast_furnace", Format::Default),
    ("interact_with_smoker", Format::Default),
    ("interact_with_lectern", Format::Default),
    ("interact_with_campfire", Format::Default),
    ("interact_with_cartography_table", Format::Default),
    ("interact_with_loom", Format::Default),
    ("interact_with_stonecutter", Format::Default),
    ("bell_ring", Format::Default),
    ("raid_trigger", Format::Default),
    ("raid_win", Format::Default),
    ("interact_with_anvil", Format::Default),
    ("interact_with_grindstone", Format::Default),
    ("target_hit", Format::Default),
    ("interact_with_smithing_table", Format::Default),
];
#[derive(Clone, Copy)]
enum Format {
    Default,
    Tenths,
    Distance,
    Time,
}
fn two(v: f64) -> String {
    format_decimal2(v, false)
}
/// `StatFormatter`.
fn format(f: Format, v: i32) -> String {
    match f {
        Format::Default => {
            // NumberFormat.getIntegerInstance(Locale.US): thousands separators.
            let mut digits = [0; 14]; // sign, 10 digits and 3 commas
            let mut i = digits.len();
            let mut value = v.unsigned_abs();
            let mut group = 0;
            loop {
                i -= 1;
                digits[i] = b'0' + (value % 10) as u8;
                value /= 10;
                if value == 0 {
                    break;
                }
                group += 1;
                if group == 3 {
                    i -= 1;
                    digits[i] = b',';
                    group = 0;
                }
            }
            if v < 0 {
                i -= 1;
                digits[i] = b'-';
            }
            std::str::from_utf8(&digits[i..])
                .expect("ASCII digits")
                .into()
        }
        Format::Tenths => two(f64::from(v) * 0.1),
        Format::Distance => {
            let m = f64::from(v) / 100.0;
            let km = m / 1000.0;
            if km > 0.5 {
                format!("{} km", two(km))
            } else if m > 0.5 {
                format!("{} m", two(m))
            } else {
                format!("{v} cm")
            }
        }
        Format::Time => {
            let s = f64::from(v) / 20.0;
            let min = s / 60.0;
            let h = min / 60.0;
            let d = h / 24.0;
            let y = d / 365.0;
            if y > 0.5 {
                format!("{} y", two(y))
            } else if d > 0.5 {
                format!("{} d", two(d))
            } else if h > 0.5 {
                format!("{} h", two(h))
            } else if min > 0.5 {
                format!("{} min", two(min))
            } else {
                // Java's double + " s".
                let mut seconds = double_to_string(s);
                seconds.push_str(" s");
                seconds
            }
        }
    }
}
/// The item columns: (stat type, header sprite).
const COLUMNS: [(&str, &str); 6] = [
    ("mined", "statistics/block_mined"),
    ("broken", "statistics/item_broken"),
    ("crafted", "statistics/item_crafted"),
    ("used", "statistics/item_used"),
    ("picked_up", "statistics/item_picked_up"),
    ("dropped", "statistics/item_dropped"),
];
/// `ItemStatisticsList`'s rows: items with a positive item column, plus
/// `Block.asItem` (`block_item`) of blocks with a positive `mined` count,
/// whose stat is keyed by block; air is dropped.
fn item_rows(
    stats: &sunrise_gameplay_common::stats::Stats,
    block_item: impl Fn(&str) -> String,
) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for (ty, _) in COLUMNS {
        for (k, v) in stats
            .values
            .get(&format!("minecraft:{ty}"))
            .into_iter()
            .flatten()
        {
            let item = if ty == "mined" {
                block_item(k)
            } else {
                k.clone()
            };
            if *v > 0 && item != "minecraft:air" && !items.contains(&item) {
                items.push(item);
            }
        }
    }
    items
}
/// `ItemRow`'s value in column `ty`: `mined` is the count of a `BlockItem`'s
/// block (`item_block`, `BlockItem.getBlock`), none ("-") for other items.
fn item_stat(
    stats: &sunrise_gameplay_common::stats::Stats,
    ty: &str,
    item: &str,
    item_block: impl Fn(&str) -> Option<String>,
) -> Option<i32> {
    if ty == "mined" {
        item_block(item).map(|block| stats.get(ty, &block))
    } else {
        Some(stats.get(ty, item))
    }
}
/// `ItemRowComparator.compare`: by the column's value (-1 for none), equal
/// values by item registry id, the whole order reversed when `order` is -1.
fn sort_rows(rows: &mut [String], order: i32, key: impl Fn(&str) -> (i32, usize)) {
    rows.sort_by_cached_key(|i| key(i));
    if order < 0 {
        rows.reverse();
    }
}
/// The rows for `present` mob ids in `registry` order; ids the registry lacks
/// (a modded type) follow in their first-seen order.
fn mobs_in_registry_order(registry: &[String], present: Vec<String>) -> Vec<String> {
    let set: std::collections::HashSet<&str> = present.iter().map(String::as_str).collect();
    let mut rows: Vec<String> = registry
        .iter()
        .filter(|id| set.contains(id.as_str()))
        .cloned()
        .collect();
    let known: std::collections::HashSet<&str> = registry.iter().map(String::as_str).collect();
    for id in present {
        if !known.contains(id.as_str()) && !rows.contains(&id) {
            rows.push(id);
        }
    }
    rows
}
#[derive(Default)]
pub(in crate::game) struct StatsUi {
    tab: usize,
    /// The list's scrolling: the wheel and the held scrollbar.
    scroll: sunrise_client::ui::Scroll,
    sort: Option<(usize, i32)>,
}
impl StatsUi {
    /// `TabNavigationBar.keyPressed` (`MenuTabBar.keyPressed`), which
    /// `StatsScreen.keyPressed` runs before the screen's own keys: with
    /// Control held, Ctrl+Tab and Ctrl+Shift+Tab cycle the tabs and
    /// Ctrl+1..9 (Ctrl+0 is the tenth) pick one directly; a tab that cannot be
    /// chosen is not switched to, the key is still taken. Whether the tab bar
    /// took the key.
    fn key(&mut self, key: KeyCode, ctrl: bool, shift: bool, active: [bool; 3]) -> bool {
        if !ctrl {
            return false;
        }
        let Some(tab) = tab_key(self.tab, &active, key, shift) else {
            return false;
        };
        if active[tab] && tab != self.tab {
            self.tab = tab;
            self.scroll.amount = 0.0;
        }
        true
    }
}
/// `InputConstants.Key.getNumericKeyValue`: the main row's digits and the
/// keypad's, 1..9 and 0.
fn numeric_key_value(key: KeyCode) -> Option<usize> {
    Some(match key {
        KeyCode::Digit1 | KeyCode::Numpad1 => 1,
        KeyCode::Digit2 | KeyCode::Numpad2 => 2,
        KeyCode::Digit3 | KeyCode::Numpad3 => 3,
        KeyCode::Digit4 | KeyCode::Numpad4 => 4,
        KeyCode::Digit5 | KeyCode::Numpad5 => 5,
        KeyCode::Digit6 | KeyCode::Numpad6 => 6,
        KeyCode::Digit7 | KeyCode::Numpad7 => 7,
        KeyCode::Digit8 | KeyCode::Numpad8 => 8,
        KeyCode::Digit9 | KeyCode::Numpad9 => 9,
        KeyCode::Digit0 | KeyCode::Numpad0 => 0,
        _ => return None,
    })
}
/// `TabNavigationBar.getNextTabIndex`: the tab a key picks, or none. A digit
/// names a tab (`Math.floorMod(digit - 1, 10)`, then clamped to the last tab);
/// Tab moves to the next (Shift: the previous) tab that can be chosen, one
/// full turn at most.
fn tab_key(current: usize, active: &[bool], key: KeyCode, shift: bool) -> Option<usize> {
    if active.is_empty() {
        return None;
    }
    if let Some(digit) = numeric_key_value(key) {
        // `Math.floorMod(digit - 1, 10)`, then `Mth.clamp` to the last tab.
        return Some((digit + 9) % 10).map(|i| i.min(active.len() - 1));
    }
    if key != KeyCode::Tab {
        return None;
    }
    let step: isize = if shift { -1 } else { 1 };
    let count = active.len() as isize;
    let mut i = (current as isize + step).rem_euclid(count);
    for _ in 0..count {
        if active[i as usize] {
            return Some(i as usize);
        }
        i = (i + step).rem_euclid(count);
    }
    None
}
/// `MenuTabBar` geometry: three tabs over at most 400 pixels.
fn tabs(w: f32) -> (f32, f32) {
    let total = w.min(400.0) - 28.0;
    let tab = ((total / 3.0) / 2.0).round() * 2.0;
    let x0 = ((w - tab * 3.0) / 2.0).floor();
    (x0, tab)
}
const TAB_H: f32 = 24.0;
const FOOTER: f32 = 33.0;
impl crate::game::ClientGame<'_> {
    pub fn open_stats(&mut self) {
        // StatsScreen.init: a server's client asks for its statistics.
        if self.client.remote.is_some() {
            self.send(sunrise_protocol::packets::play::Serverbound::ClientCommand(
                sunrise_protocol::packets::play::ClientCommand { action: 1 },
            ));
        }
        self.client.stats_ui = StatsUi::default();
        self.set_screen(Screen::Stats);
    }
    /// The list's scrollbar column (right of the 280 pixel list) and the
    /// pixels it scrolls.
    fn stats_bar(w: f32) -> f32 {
        ((w - 280.0) / 2.0).floor() + 280.0 + 6.0
    }
    fn stats_content(&self) -> f32 {
        let (row_h, rows) = self.stats_rows();
        row_h * rows as f32 + 4.0
    }
    fn stats_rows(&self) -> (f32, usize) {
        match self.client.stats_ui.tab {
            0 => (14.0, CUSTOM.len()),
            1 => (22.0, self.stat_items().len() + 1),
            _ => (36.0, self.stat_mobs().len()),
        }
    }
    /// Whether tab `i` can be chosen: the Items and Mobs tabs are active only
    /// while their list has rows.
    fn stats_tab_active(&self, i: usize) -> bool {
        match i {
            1 => !self.stat_items().is_empty(),
            2 => !self.stat_mobs().is_empty(),
            _ => true,
        }
    }
    /// Items (and block items) with any non-zero column.
    fn stat_items(&self) -> Vec<String> {
        let mut items = item_rows(&self.common.stats, |block| {
            self.common
                .pack
                .assets
                .registry
                .default_state(block)
                .map_or_else(
                    || "minecraft:air".to_owned(),
                    |s| self.common.res.facts().get(s).item.to_string(),
                )
        });
        if let Some((col, order)) = self.client.stats_ui.sort {
            let ty = COLUMNS[col].0;
            // `Item.getId`: one pass over the registry, not a scan per row
            // (the screen asks for its rows several times a frame).
            let ids: std::collections::HashMap<&str, usize> = self
                .common
                .res
                .items()
                .registry_order
                .iter()
                .enumerate()
                .map(|(n, id)| (&**id, n))
                .collect();
            sort_rows(&mut items, order, |i| {
                let id = ids.get(i).copied().unwrap_or(usize::MAX);
                (self.stat_item(ty, i).unwrap_or(-1), id)
            });
        }
        items
    }
    fn stat_item(&self, ty: &str, item: &str) -> Option<i32> {
        item_stat(&self.common.stats, ty, item, |i| {
            let state = self.common.res.items().get(i)?.block?;
            Some(self.common.res.facts().get(state).name.to_string())
        })
    }
    /// `MobsStatisticsList.<init>`: `BuiltInRegistries.ENTITY_TYPE` in registry
    /// (network id) order, the types killed or killing the player.
    fn stat_mobs(&self) -> Vec<String> {
        let counted = |ty: &str| {
            let map = self.common.stats.values.get(&format!("minecraft:{ty}"));
            map.into_iter()
                .flatten()
                .filter(|(_, v)| **v > 0)
                .map(|(k, _)| k.clone())
                .collect::<Vec<_>>()
        };
        let present: Vec<String> = ["killed", "killed_by"]
            .into_iter()
            .flat_map(counted)
            .collect();
        let registry = sunrise_gameplay_common::game::villager_client::builtin_registries(
            &self.common.options.resources,
        );
        let order = registry
            .as_deref()
            .and_then(|b| b.get("minecraft:entity_type"));
        mobs_in_registry_order(order.map_or(&[], |ids| &ids.names), present)
    }
    /// `MobRow`: the mob's name, its kills and killed-by lines, and their
    /// colours (grey when none).
    fn mob_row(&self, mob: &str) -> (Text, Text, Text, (u32, u32)) {
        let name = Text::translate(&format!("entity.{}", mob.replace(':', ".")), &[]);
        let kills = self.common.stats.get("killed", mob);
        let by = self.common.stats.get("killed_by", mob);
        let k = if kills == 0 {
            Text::translate(
                "stat_type.minecraft.killed.none",
                std::slice::from_ref(&name),
            )
        } else {
            Text::translate(
                "stat_type.minecraft.killed",
                &[Text::literal(kills.to_string()), name.clone()],
            )
        };
        let b = if by == 0 {
            Text::translate(
                "stat_type.minecraft.killed_by.none",
                std::slice::from_ref(&name),
            )
        } else {
            Text::translate(
                "stat_type.minecraft.killed_by",
                &[name.clone(), Text::literal(by.to_string())],
            )
        };
        let colors = (
            if kills > 0 { 0xFFBA_BABA } else { 0xFF80_8080 },
            if by > 0 { 0xFFBA_BABA } else { 0xFF80_8080 },
        );
        (name, k, b, colors)
    }
    pub(in crate::game::client_side) fn draw_stats(&mut self, batch: &mut HudBatch) {
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new(
                "stats",
                super::screen_info::tr("gui.stats", &[]),
            )
        });
        let (w, h) = (batch.width, batch.height);
        let atlas = &self.client.gui.atlas;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let (top, bottom) = (TAB_H, h - FOOTER);
        ui::separators(batch, atlas, top, bottom, true);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        // The tab bar.
        let mut tab_tip = false;
        let (x0, tw) = tabs(w);
        const TABS: [&str; 3] = ["stat.generalButton", "stat.itemsButton", "stat.mobsButton"];
        // Its TabNavigationBar: the hovered tab, else the current one (a
        // GridLayoutTab's extra narration is empty).
        sunrise_client::widgets::narratable(|| {
            use sunrise_client::widgets::{ListKind, Narratable, Row, What};
            let row = |i: usize| {
                let mut row = Row::new(i, Text::translate(TABS[i], &[]).plain());
                row.hint = Some(String::new());
                row
            };
            let hovered = (0..TABS.len())
                .find(|&i| ui::hovered(self.client.cursor, x0 + tw * i as f32, 0.0, tw, TAB_H));
            Narratable::new(
                What::List {
                    kind: ListKind::Tabs,
                    total: TABS.len(),
                    hovered: hovered.map(row),
                    selected: Some(row(self.client.stats_ui.tab.min(TABS.len() - 1))),
                },
                [x0, 0.0, tw * TABS.len() as f32, TAB_H],
            )
            .key(sunrise_client::widgets::key_of(&"stats/tabs"))
        });
        for (i, key) in TABS.into_iter().enumerate() {
            let x = x0 + tw * i as f32;
            let selected = self.client.stats_ui.tab == i;
            let active = self.stats_tab_active(i);
            // Only one that can be chosen: the bumpers step from the selected tab by clicking.
            if active {
                sunrise_client::widgets::target_as(
                    [x, 0.0, tw, TAB_H],
                    sunrise_client::widgets::Kind::Tab(selected),
                );
            }
            let hover = ui::hovered(self.client.cursor, x, 0.0, tw, TAB_H);
            if hover && !active {
                tab_tip = true;
            }
            let sprite = match (selected, hover) {
                (true, true) => "widget/tab_selected_highlighted",
                (true, false) => "widget/tab_selected",
                (false, true) => "widget/tab_highlighted",
                (false, false) => "widget/tab",
            };
            ui::nine_slice(batch, atlas, sprite, x, 0.0, tw, TAB_H, u32::MAX);
            ui::menu_tab_label(
                batch,
                atlas,
                &Text::translate(key, &[]),
                (x, 0.0, tw, TAB_H),
                selected,
                active,
                sunrise_client::hud_overlays::now_ms(),
            );
            if selected {
                batch.fill(atlas, x + 2.0, TAB_H - 3.0, tw - 4.0, 1.0, u32::MAX);
            }
        }
        // The list.
        let (row_h, rows) = self.stats_rows();
        let content = row_h * rows as f32 + 4.0;
        self.client.stats_ui.scroll.clamp(top, bottom, content);
        let left = ((w - 280.0) / 2.0).floor();
        let scroll = self.client.stats_ui.scroll.amount;
        let row_y = |i: usize| top + 4.0 + i as f32 * row_h - scroll;
        let visible = |y: f32| y >= top && y + row_h <= bottom;
        // The list's row under the mouse (`getHovered`).
        let hovered_row = {
            let (cx, cy) = self.client.cursor;
            let i = ((cy - (top + 4.0) + scroll) / row_h).floor();
            let inside = cy >= top && cy < bottom && cx >= left && cx < left + 280.0;
            (inside && i >= 0.0 && (i as usize) < rows).then_some(i as usize)
        };
        let list_rect = [0.0, top, w, bottom - top];
        let list_key = sunrise_client::widgets::key_of(&("stats/list", self.client.stats_ui.tab));
        let tr = super::screen_info::tr;
        // GeneralStatisticsList and MobsStatisticsList: ObjectSelectionLists
        // narrating the hovered row (none is selected).
        let object_list = |narration: &dyn Fn(usize) -> String| {
            sunrise_client::widgets::narratable(|| {
                use sunrise_client::widgets::{ListKind, Narratable, Row};
                let hovered = hovered_row.map(|i| Row::new(i, narration(i)));
                Narratable::list(ListKind::Object, rows, hovered, None, list_rect).key(list_key)
            });
        };
        // The hovered item's tooltip, drawn over the list.
        let mut tip = None;
        match self.client.stats_ui.tab {
            0 => {
                let mut sorted: Vec<(Text, String)> = CUSTOM
                    .iter()
                    .map(|(k, f)| {
                        let label = Text::translate(&format!("stat.minecraft.{k}"), &[]);
                        let v = self.common.stats.get("custom", &format!("minecraft:{k}"));
                        (label, format(*f, v))
                    })
                    .collect();
                sorted.sort_by_key(|(l, _)| l.plain());
                object_list(&|i| {
                    let (label, value) = &sorted[i];
                    tr("narrator.select", &[&format!("{} {value}", label.plain())])
                });
                for (i, (label, value)) in sorted.iter().enumerate() {
                    let y = row_y(i);
                    if !visible(y) {
                        continue;
                    }
                    let color = if i % 2 == 0 { u32::MAX } else { 0xFFBA_BABA };
                    let ty = y + (row_h - 8.0) / 2.0;
                    batch.text_styled(atlas, label, left + 2.0, ty.floor(), color, true);
                    let vw = HudBatch::text_width(atlas, value);
                    batch.text(
                        atlas,
                        value,
                        left + 280.0 - vw - 4.0,
                        ty.floor(),
                        color,
                        true,
                    );
                }
            }
            1 => {
                let items = self.stat_items();
                if items.is_empty() {
                    self.draw_stats_footer(batch, w, h);
                    self.draw_screen_phase(
                        batch,
                        sunrise_client::screen_draw::RenderPhase::Foreground,
                    );
                    return;
                }
                // ItemStatisticsList, a ContainerObjectSelectionList: the
                // header's sort buttons and each row's item.
                sunrise_client::widgets::narratable(|| {
                    sunrise_client::widgets::Narratable::container_list(
                        rows,
                        hovered_row,
                        list_rect,
                    )
                    .key(list_key)
                });
                // The header row of sort buttons.
                let y = row_y(0);
                if visible(y) {
                    for (c, (ty, sprite)) in COLUMNS.iter().enumerate() {
                        let x = left + 75.0 + 40.0 * c as f32 - 18.0;
                        let hover = ui::hovered(self.client.cursor, x, y + 1.0, 18.0, 18.0);
                        // StatSortButton: an ImageButton named by its stat
                        // type, which is its tooltip too.
                        sunrise_client::widgets::narratable(|| {
                            let name = tr(&format!("stat_type.minecraft.{ty}"), &[]);
                            sunrise_client::widgets::Narratable::button(
                                name.clone(),
                                [x, y + 1.0, 18.0, 18.0],
                                true,
                            )
                            .hint(name)
                            .key(sunrise_client::widgets::key_of(&("stats/sort", c)))
                            .child_of(list_key, 0)
                        });
                        let bg = if hover {
                            "statistics/header"
                        } else {
                            "container/slot"
                        };
                        batch.sprite(atlas, bg, x, y + 1.0);
                        batch.sprite(atlas, sprite, x, y + 1.0);
                        if let Some((sc, order)) = self.client.stats_ui.sort
                            && sc == c
                        {
                            let s = if order < 0 {
                                "statistics/sort_down"
                            } else {
                                "statistics/sort_up"
                            };
                            batch.sprite(atlas, s, x, y + 1.0);
                        }
                    }
                }
                for (i, item) in items.iter().enumerate() {
                    let y = row_y(i + 1);
                    if !visible(y) {
                        continue;
                    }
                    let stack = ItemStack::new(item, 1);
                    // ItemRowWidget, an ItemDisplayWidget: "Item: <name>".
                    sunrise_client::widgets::narratable(|| {
                        let name = self.common.stack_hover_name_text(&stack).plain();
                        sunrise_client::widgets::Narratable::text(
                            tr("narration.item", &[&name]),
                            [left + 1.0, y + 1.0, 18.0, 18.0],
                        )
                        .key(sunrise_client::widgets::key_of(&("stats/item", item)))
                        .child_of(list_key, i + 1)
                        .without_tooltip()
                    });
                    batch.sprite(
                        &self.client.gui.atlas,
                        "container/slot",
                        left + 1.0,
                        y + 1.0,
                    );
                    self.draw_item(batch, &stack, left + 2.0, y + 2.0);
                    // `ItemRowWidget.extractTooltip`: the item's tooltip at the slot's corner.
                    if ui::hovered(self.client.cursor, left + 1.0, y + 1.0, 18.0, 18.0) {
                        tip = Some((stack.clone(), left + 19.0, y + 19.0));
                    }
                    let atlas = &self.client.gui.atlas;
                    let color = if (i + 1) % 2 == 0 {
                        u32::MAX
                    } else {
                        0xFFBA_BABA
                    };
                    for (c, (ty, _)) in COLUMNS.iter().enumerate() {
                        let x = left + 75.0 + 40.0 * c as f32;
                        let text = match self.stat_item(ty, item) {
                            Some(v) => format(Format::Default, v),
                            None => Text::translate("stats.none", &[]).plain(),
                        };
                        let tw = HudBatch::text_width(atlas, &text);
                        batch.text(atlas, &text, x - tw, (y + 7.0).floor(), color, true);
                    }
                }
            }
            _ => {
                let mobs = self.stat_mobs();
                object_list(&|i| {
                    let (_, kills, killed_by, _) = self.mob_row(&mobs[i]);
                    let both = sunrise_client::widgets::join_for_narration(&[
                        &kills.plain(),
                        &killed_by.plain(),
                    ]);
                    tr("narrator.select", &[&both])
                });
                for (i, mob) in mobs.iter().enumerate() {
                    let y = row_y(i);
                    if !visible(y) {
                        continue;
                    }
                    let (name, k, b, (ck, cb)) = self.mob_row(mob);
                    batch.text_styled(atlas, &name, left + 2.0, y + 1.0, u32::MAX, true);
                    batch.text_styled(atlas, &k, left + 12.0, y + 10.0, ck, true);
                    batch.text_styled(atlas, &b, left + 12.0, y + 19.0, cb, true);
                }
            }
        }
        let atlas = &self.client.gui.atlas;
        ui::scrollbar(
            batch,
            atlas,
            Self::stats_bar(w),
            top,
            bottom,
            content,
            self.client.stats_ui.scroll.amount,
        );
        self.draw_stats_footer(batch, w, h);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        if let Some((stack, x, y)) = tip {
            self.draw_stack_tooltip(batch, &stack, x, y);
        }
        // The empty tab's Tooltip.create(gui.stats.none_found).
        if tab_tip {
            let atlas = &self.client.gui.atlas;
            let tip = Text::translate("gui.stats.none_found", &[]);
            // Tooltip.splitTooltip: font.split(message, 170).
            ui::tooltip(
                batch,
                atlas,
                &atlas.font.split(&tip, 170.0),
                self.client.cursor,
            );
        }
    }
    fn draw_stats_footer(&self, batch: &mut HudBatch, w: f32, h: f32) {
        let atlas = &self.client.gui.atlas;
        let (bx, by) = ((w / 2.0 - 100.0).floor(), h - FOOTER + 6.0);
        let hover = ui::hovered(self.client.cursor, bx, by, 200.0, 20.0);
        ui::action_button(
            batch,
            atlas,
            sunrise_client::widgets::Action::Done,
            bx,
            by,
            200.0,
            20.0,
            &Text::translate("gui.done", &[]),
            true,
            hover,
        );
    }
    /// `StatsScreen.keyPressed`: the tab bar sees the key first
    /// (`MenuTabBar.keyPressed`), and the rest of `Screen.keyPressed` is
    /// Escape, which the screens' keys already handle.
    pub(in crate::game::client_side) fn stats_key(&mut self, key: KeyCode) {
        let active = [true, self.stats_tab_active(1), self.stats_tab_active(2)];
        let ctrl = self.has_control_down_with_quirk();
        let shift = self.has_shift_down();
        let before = self.client.stats_ui.tab;
        if self.client.stats_ui.key(key, ctrl, shift, active) && self.client.stats_ui.tab != before
        {
            // TabManager.setCurrentTab(.., true): the click of a tab press.
            self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
        }
    }
    pub(in crate::game::client_side) fn stats_click(&mut self, size: [u32; 2]) -> bool {
        // A press on the scrollbar holds it and clicks nothing else.
        if self.stats_press_scrollbar(size) {
            return true;
        }
        let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let (w, h) = (b.width, b.height);
        let (x0, tw) = tabs(w);
        for i in 0..3 {
            if ui::hovered(self.client.cursor, x0 + tw * i as f32, 0.0, tw, TAB_H)
                && self.client.stats_ui.tab != i
                && self.stats_tab_active(i)
            {
                self.client.stats_ui.tab = i;
                self.client.stats_ui.scroll.amount = 0.0;
                self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
        }
        if ui::hovered(
            self.client.cursor,
            (w / 2.0 - 100.0).floor(),
            h - FOOTER + 6.0,
            200.0,
            20.0,
        ) {
            self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
            self.set_screen(Screen::Paused);
            return true;
        }
        let mut handled = false;
        if self.client.stats_ui.tab == 1 {
            let left = ((w - 280.0) / 2.0).floor();
            let y = TAB_H + 4.0 - self.client.stats_ui.scroll.amount;
            for c in 0..COLUMNS.len() {
                let x = left + 75.0 + 40.0 * c as f32 - 18.0;
                if ui::hovered(self.client.cursor, x, y + 1.0, 18.0, 18.0) {
                    handled = true;
                    // sortByColumn: descending, ascending, off.
                    self.client.stats_ui.sort = match self.client.stats_ui.sort {
                        Some((sc, -1)) if sc == c => Some((c, 1)),
                        Some((sc, _)) if sc == c => None,
                        _ => Some((c, -1)),
                    };
                    self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
                }
            }
        }
        handled
    }
    pub(in crate::game::client_side) fn stats_scroll(&mut self, amount: f32) -> bool {
        let (row_h, _) = self.stats_rows();
        let h = self.gui_size().1;
        if !(TAB_H..h - FOOTER).contains(&self.client.cursor.1) {
            return false;
        }
        let content = self.stats_content();
        self.client
            .stats_ui
            .scroll
            .wheel(amount, row_h / 2.0, TAB_H, h - FOOTER, content);
        true
    }
    /// A press on the statistics list's scrollbar holds it
    /// (`AbstractSelectionList.updateScrolling`).
    pub(in crate::game::client_side) fn stats_press_scrollbar(&mut self, size: [u32; 2]) -> bool {
        let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let (w, h) = (b.width, b.height);
        let content = self.stats_content();
        self.client.stats_ui.scroll.press(
            self.client.cursor,
            Self::stats_bar(w),
            TAB_H,
            h - FOOTER,
            content,
        )
    }
    /// The pointer moved while the scrollbar is held.
    pub(in crate::game::client_side) fn stats_drag(&mut self) -> bool {
        if !self.client.stats_ui.scroll.held() {
            return false;
        }
        let h = self.gui_size().1;
        let content = self.stats_content();
        self.client
            .stats_ui
            .scroll
            .drag(self.client.cursor.1, TAB_H, h - FOOTER, content);
        true
    }
    /// The button came up: the scrollbar lets go.
    pub(in crate::game::client_side) fn stats_release(&mut self) -> bool {
        let handled = self.client.stats_ui.scroll.held();
        self.client.stats_ui.scroll.release();
        handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode::{Digit0, Digit1, Digit2, Digit3, Numpad8, Tab};

    #[test]
    fn stat_format_uses_java_grouping_and_time() {
        assert_eq!(format(Format::Time, 39_960), "0.55 h");
        // Temurin 25 NumberFormat and Double.toString on the converted values.
        assert_eq!(format(Format::Default, i32::MIN), "-2,147,483,648");
        assert_eq!(format(Format::Time, -373928161), "-1.869640805E7 s");
        assert_eq!(format(Format::Time, 1), "0.05 s");
    }

    /// Registry (network id) order, not alphabetical: a killed-by-only mob
    /// earlier in the registry precedes a killed-only one later; unknown ids
    /// follow.
    #[test]
    fn mob_rows_follow_registry_order() {
        let reg: Vec<String> = [
            "minecraft:enderman",
            "minecraft:ender_dragon",
            "minecraft:zombie",
        ]
        .map(String::from)
        .to_vec();
        let present = [
            "minecraft:zombie",
            "minecraft:mod_b",
            "minecraft:ender_dragon",
            "minecraft:enderman",
            "minecraft:mod_a",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            mobs_in_registry_order(&reg, present),
            [
                "minecraft:enderman",
                "minecraft:ender_dragon",
                "minecraft:zombie",
                "minecraft:mod_b",
                "minecraft:mod_a"
            ]
        );
    }

    /// `ItemStatisticsList`: a mined block's row is its item (`Block.asItem`),
    /// air dropped, and a `BlockItem`'s Mined column reads its block's count.
    /// The block <-> item pairs are vanilla 26.3's (`Items`: wheat seeds place
    /// wheat, redstone places redstone wire, a torch places torch and wall
    /// torch; water has no item).
    #[test]
    fn mined_blocks_count_under_the_item_that_places_them() {
        let mut stats = sunrise_gameplay_common::stats::Stats::default();
        stats.add("mined", "minecraft:wheat", 3);
        stats.add("mined", "minecraft:redstone_wire", 5);
        stats.add("mined", "minecraft:wall_torch", 7);
        stats.add("mined", "minecraft:torch", 2);
        stats.add("mined", "minecraft:water", 4);
        stats.add("used", "minecraft:wheat_seeds", 1);
        stats.add("crafted", "minecraft:stick", 6);
        let block_item = |b: &str| {
            match b {
                "minecraft:wheat" => "minecraft:wheat_seeds",
                "minecraft:redstone_wire" => "minecraft:redstone",
                "minecraft:wall_torch" | "minecraft:torch" => "minecraft:torch",
                _ => "minecraft:air",
            }
            .to_owned()
        };
        let item_block = |i: &str| {
            match i {
                "minecraft:wheat_seeds" => Some("minecraft:wheat"),
                "minecraft:redstone" => Some("minecraft:redstone_wire"),
                "minecraft:torch" => Some("minecraft:torch"),
                _ => None,
            }
            .map(str::to_owned)
        };
        let mut rows = item_rows(&stats, block_item);
        rows.sort();
        assert_eq!(
            rows,
            [
                "minecraft:redstone",
                "minecraft:stick",
                "minecraft:torch",
                "minecraft:wheat_seeds"
            ]
        );
        let mined = |i: &str| item_stat(&stats, "mined", i, item_block);
        assert_eq!(mined("minecraft:wheat_seeds"), Some(3));
        assert_eq!(mined("minecraft:redstone"), Some(5));
        assert_eq!(
            mined("minecraft:torch"),
            Some(2),
            "the torch block's own count"
        );
        assert_eq!(mined("minecraft:stick"), None, "not a block item: '-'");
        assert_eq!(
            item_stat(&stats, "used", "minecraft:wheat_seeds", item_block),
            Some(1)
        );
    }

    /// `ItemRowComparator` with `sortByColumn`'s first click (order -1): the
    /// biggest value first, "-" (-1) last, equal values by item registry id,
    /// that order reversed too; the second click (order 1) is the mirror.
    #[test]
    fn item_rows_sort_by_value_then_registry_id() {
        let rows = || ["c", "a", "none", "b", "d"].map(str::to_owned);
        // (value, registry id)
        let key = |i: &str| match i {
            "a" => (5, 0),
            "b" => (2, 1),
            "c" => (5, 2),
            "d" => (0, 3),
            _ => (-1, 4),
        };
        let mut down = rows();
        sort_rows(&mut down, -1, key);
        assert_eq!(down, ["c", "a", "b", "d", "none"]);
        let mut up = rows();
        sort_rows(&mut up, 1, key);
        assert_eq!(up, ["none", "d", "b", "a", "c"]);
    }

    /// `StatsScreen.keyPressed`: with Control held the tab bar takes the key -
    /// Ctrl+Tab and Ctrl+Shift+Tab cycle the tabs, Ctrl+1..9 pick one (Ctrl+0
    /// is the tenth, past the last tab) - and without it the key goes on.
    #[test]
    fn control_and_tab_switch_the_statistics_tabs() {
        let all = [true, true, true];
        let mut ui = StatsUi::default();
        assert_eq!(ui.tab, 0);
        for expected in [1, 2, 0] {
            assert!(ui.key(Tab, true, false, all));
            assert_eq!(ui.tab, expected, "Ctrl+Tab");
        }
        assert!(ui.key(Tab, true, true, all));
        assert_eq!(ui.tab, 2, "Ctrl+Shift+Tab wraps back to the last tab");
        assert!(ui.key(Digit1, true, false, all));
        assert_eq!(ui.tab, 0);
        assert!(ui.key(Digit2, true, false, all));
        assert_eq!(ui.tab, 1);
        assert!(ui.key(Numpad8, true, false, all));
        assert_eq!(ui.tab, 2, "a digit past the last tab picks the last one");
        assert!(ui.key(Digit0, true, false, all));
        assert_eq!(ui.tab, 2, "0 is the tenth tab, clamped to the last one");
        // Without Control the tab bar leaves the key for the rest of the screen.
        assert!(!ui.key(Tab, false, false, all));
        assert!(!ui.key(Digit3, false, false, all));
        assert_eq!(ui.tab, 2, "the last tab stays");
    }

    /// `StatsScreen.setTabActiveStateAndTooltip`: the Items and Mobs tabs that
    /// have no rows cannot be chosen, so the cycle skips them; a digit naming
    /// one is still taken, and switches nothing.
    #[test]
    fn an_empty_tab_is_skipped_by_the_keyboard() {
        let only_general_and_mobs = [true, false, true];
        let mut ui = StatsUi::default();
        assert!(ui.key(Tab, true, false, only_general_and_mobs));
        assert_eq!(ui.tab, 2, "the empty Items tab is skipped");
        assert!(ui.key(Tab, true, false, only_general_and_mobs));
        assert_eq!(ui.tab, 0);
        assert!(ui.key(Tab, true, true, only_general_and_mobs));
        assert_eq!(ui.tab, 2);
        assert!(ui.key(Digit2, true, false, only_general_and_mobs));
        assert_eq!(ui.tab, 2, "an empty tab is not switched to");
    }

    /// Switching tabs starts the new list at the top, as a tab press does.
    #[test]
    fn a_tab_switch_scrolls_back_to_the_top() {
        let mut ui = StatsUi::default();
        ui.scroll.amount = 120.0;
        assert!(ui.key(Tab, true, false, [true, true, true]));
        assert_eq!(ui.tab, 1);
        assert_eq!(ui.scroll.amount, 0.0);
    }
}
