//! The Advancements screen (`AdvancementsScreen`, `AdvancementTab`,
//! `AdvancementWidget`): one tab per displayed root, the tree laid out by
//! `TreeNodePosition`, dragged or scrolled inside the 252x140 window, with
//! the progress tooltip of the hovered advancement.

use super::{Screen, advancements_common::text_of};
use std::{collections::BTreeMap, sync::Arc, time::Instant};
use sunrise_client::{hud::HudBatch, ui};
use sunrise_game_data::text::{Style, Text};
use sunrise_sim::advancements::{Frame, Tree};

const INSIDE_W: i32 = 234;
const INSIDE_H: i32 = 113;
const FOOTER: f32 = 33.0;
/// The roots in the order the tabs appear.
const TAB_ORDER: [&str; 5] = [
    "minecraft:story/root",
    "minecraft:nether/root",
    "minecraft:end/root",
    "minecraft:adventure/root",
    "minecraft:husbandry/root",
];
/// `AdvancementTabType`: (kind, width, height, max).
const TAB_TYPES: [(Side, f32, f32, usize); 4] = [
    (Side::Above, 28.0, 32.0, 8),
    (Side::Below, 28.0, 32.0, 8),
    (Side::Left, 32.0, 28.0, 5),
    (Side::Right, 32.0, 28.0, 5),
];
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Above,
    Below,
    Left,
    Right,
}
fn tab_type(mut index: usize) -> Option<((Side, f32, f32, usize), usize)> {
    for t in TAB_TYPES {
        if index < t.3 {
            return Some((t, index));
        }
        index -= t.3;
    }
    None
}
/// Tab position relative to the window (`getX`/`getY`).
fn tab_pos((side, w, h, _): (Side, f32, f32, usize), i: usize) -> (f32, f32) {
    let i = i as f32;
    match side {
        Side::Above => ((w + 4.0) * i, -h + 4.0),
        Side::Below => ((w + 4.0) * i, 136.0),
        Side::Left => (-w + 4.0, h * i),
        Side::Right => (248.0, h * i),
    }
}
fn tab_sprite((side, ..): (Side, f32, f32, usize), i: usize, max: usize, selected: bool) -> String {
    let (first, middle, last) = match side {
        Side::Above => ("above_left", "above_middle", "above_right"),
        Side::Below => ("below_left", "below_middle", "below_right"),
        Side::Left => ("left_top", "left_middle", "left_bottom"),
        Side::Right => ("right_top", "right_middle", "right_bottom"),
    };
    let part = if i == 0 {
        first
    } else if i == max - 1 {
        last
    } else {
        middle
    };
    let sel = if selected { "_selected" } else { "" };
    format!("advancements/tab_{part}{sel}")
}
fn icon_offset(side: Side) -> (f32, f32) {
    match side {
        Side::Above => (6.0, 9.0),
        Side::Below => (6.0, 6.0),
        Side::Left => (10.0, 5.0),
        Side::Right => (6.0, 5.0),
    }
}
fn frame_sprite(frame: Frame, obtained: bool) -> &'static str {
    match (frame, obtained) {
        (Frame::Task, true) => "advancements/task_frame_obtained",
        (Frame::Task, false) => "advancements/task_frame_unobtained",
        (Frame::Goal, true) => "advancements/goal_frame_obtained",
        (Frame::Goal, false) => "advancements/goal_frame_unobtained",
        (Frame::Challenge, true) => "advancements/challenge_frame_obtained",
        (Frame::Challenge, false) => "advancements/challenge_frame_unobtained",
    }
}
fn box_sprite(obtained: bool) -> &'static str {
    if obtained {
        "advancements/box_obtained"
    } else {
        "advancements/box_unobtained"
    }
}
/// The layout of the loaded tree, cached per tree.
pub struct Cache {
    tree: Arc<Tree>,
    kids: Vec<Vec<usize>>,
    /// Widget pixel position (`x * 28`, `y * 27`).
    pub pos: Vec<Option<(i32, i32)>>,
    remote: Option<RemoteWidgets>,
}
struct RemoteWidgets {
    tabs: Vec<usize>,
    /// Widgets by root, in tab insertion order (including unattached widgets).
    by_root: Vec<Vec<usize>>,
}
fn displayed_roots(tree: &Tree) -> impl Iterator<Item = usize> + '_ {
    tree.list.iter().enumerate().filter_map(|(i, a)| {
        (a.parent.is_none() && a.display.as_ref().is_some_and(|d| d.background.is_some()))
            .then_some(i)
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabBounds(pub i32, pub i32, pub i32, pub i32);
impl Default for TabBounds {
    fn default() -> Self {
        Self(i32::MAX, i32::MAX, i32::MIN, i32::MIN)
    }
}
impl TabBounds {
    /// `AdvancementTab.addWidget`: widget endpoints are Java ints.
    fn add_widget(&mut self, x: i32, y: i32) {
        self.0 = self.0.min(x);
        self.1 = self.1.min(y);
        self.2 = self.2.max(x.wrapping_add(28));
        self.3 = self.3.max(y.wrapping_add(27));
    }
    fn center(self) -> (f64, f64) {
        let Self(min_x, min_y, max_x, max_y) = self;
        (
            f64::from(117 - min_x.wrapping_add(max_x) / 2),
            f64::from(56 - min_y.wrapping_add(max_y) / 2),
        )
    }
}
#[derive(Default)]
pub struct TabState {
    pub bounds: TabBounds,
    /// Scroll of this tab; `None` until centred.
    pub scroll: Option<(f64, f64)>,
    pub fade: f32,
    pub hovered: Option<usize>,
}
#[derive(Default)]
pub struct AdvUi {
    pub cache: Option<Cache>,
    /// The selected root in this screen, if present in its tabs.
    tab: Option<String>,
    tabs: BTreeMap<String, TabState>,
    /// The cursor at the last drag step while the left button is down.
    drag: Option<(f32, f32)>,
    scrolling: bool,
    from_pause: bool,
    pub last_frame: Option<Instant>,
}
impl AdvUi {
    pub fn tab_state(&self) -> Option<&TabState> {
        self.tabs.get(self.tab.as_ref()?)
    }
    pub fn tab_state_mut(&mut self) -> Option<&mut TabState> {
        self.tabs.get_mut(self.tab.as_ref()?)
    }
}
/// What the current frame shows: tabs and the connected widgets of the selected tab.
pub struct View {
    pub tabs: Vec<usize>,
    pub tab: Option<usize>,
    pub widgets: Vec<usize>,
    parent: Vec<Option<usize>>,
}
pub fn window(w: f32, h: f32) -> (f32, f32) {
    (((w - 252.0) / 2.0).floor(), ((h - 140.0) / 2.0).floor())
}
/// `ServerboundSeenAdvancementsPacket.openedTab`: `ClientAdvancements.setSelectedTab`
/// with `tellServer` true reports the selected tab when opening the screen,
/// picking a tab, or selecting a replacement after an update.
pub(in crate::game::client_side) fn opened_tab(
    id: &str,
) -> sunrise_protocol::packets::play::SeenAdvancements {
    sunrise_protocol::packets::play::SeenAdvancements {
        action: 0,
        tab: Some(id.to_owned()),
    }
}
/// `ServerboundSeenAdvancementsPacket.closedScreen()` (`AdvancementsScreen.removed`):
/// action 1, which carries no tab.
pub(in crate::game::client_side) fn closed_screen()
-> sunrise_protocol::packets::play::SeenAdvancements {
    sunrise_protocol::packets::play::SeenAdvancements {
        action: 1,
        tab: None,
    }
}

impl crate::game::ClientGame<'_> {
    pub fn open_advancements(&mut self, from_pause: bool) {
        self.client.adv_ui.0 = AdvUi {
            cache: self.client.adv_ui.0.cache.take(),
            tab: self.common.adv.selected_tab.clone(),
            from_pause,
            ..Default::default()
        };
        self.set_screen(Screen::Advancements);
        if self.client.screen != Screen::Advancements {
            return;
        }
        self.capture(false);
        if self.common.adv.remote_tree.is_some() {
            // AdvancementsScreen.init attaches a fresh listener (ClientAdvancements.setListener),
            // which replays roots to select the first tab before init reports the selection again.
            self.client.adv_ui.0.tab = None;
            self.remote_advancement_tabs_updated(false);
        }
        if let Some(tab) = self.adv_view().tab {
            self.select_advancement_tab(tab);
        } else {
            self.common.adv.selected_tab = None;
        }
    }
    pub fn close_advancements(&mut self) {
        // AdvancementsScreen.removed: the server hears the screen closed.
        self.send(sunrise_protocol::packets::play::Serverbound::SeenAdvancements(closed_screen()));
        if self.client.adv_ui.0.from_pause {
            self.set_screen(Screen::Paused);
        } else {
            self.set_screen(Screen::Playing);
            self.capture(true);
        }
    }
    /// `ClientAdvancements.setSelectedTab` with `tellServer` enabled.
    pub fn select_advancement_tab(&mut self, tab: usize) {
        let id = &self.common.adv.tree.list[tab].id;
        self.send(sunrise_protocol::packets::play::Serverbound::SeenAdvancements(opened_tab(id)));
        self.client.adv_ui.0.tab = Some(id.clone());
        self.common.adv.selected_tab = Some(id.clone());
    }
    /// Only the open screen listens to `onAdvancementsCleared` / `onAdvancementsUpdated`.
    pub(in crate::game) fn remote_advancement_tabs_updated(&mut self, reset: bool) {
        if self.client.screen != Screen::Advancements {
            return;
        }
        if reset {
            self.client.adv_ui.0.tab = None;
            self.client.adv_ui.0.tabs.clear();
        }
        let first = self.rebuild_remote_tabs();
        if self.client.adv_ui.0.tab_state().is_none() {
            self.client.adv_ui.0.tab = None;
            if let Some(tab) = first {
                self.select_advancement_tab(tab);
            }
        }
    }
    /// `AdvancementTab.copyPosition`: retain scroll, centring and bounds,
    /// not the hover or fade of the old widgets.
    fn rebuild_remote_tabs(&mut self) -> Option<usize> {
        self.adv_cache();
        let cache = self
            .client
            .adv_ui
            .0
            .cache
            .as_ref()
            .expect("advancement layout cached");
        let remote = cache.remote.as_ref().expect("remote advancement layout");
        let mut old_tabs = std::mem::take(&mut self.client.adv_ui.0.tabs);
        for &root in &remote.tabs {
            let id = &cache.tree.list[root].id;
            let widgets = &remote.by_root[root];
            // AdvancementTab.copyPosition replaces the new root widget's
            // initial bounds, then addAdvancement expands them for the tasks.
            let (mut state, widgets) = if let Some(old) = old_tabs.remove(id) {
                (
                    TabState {
                        bounds: old.bounds,
                        scroll: old.scroll,
                        ..Default::default()
                    },
                    &widgets[1..],
                )
            } else {
                (TabState::default(), widgets.as_slice())
            };
            for &widget in widgets {
                if let Some((x, y)) = cache.pos[widget] {
                    state.bounds.add_widget(x, y);
                }
            }
            self.client.adv_ui.0.tabs.insert(id.clone(), state);
        }
        remote.tabs.first().copied()
    }
    fn adv_cache(&mut self) {
        if self
            .client
            .adv_ui
            .0
            .cache
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(&c.tree, &self.common.adv.tree))
        {
            return;
        }
        let tree = self.common.adv.tree.clone();
        let (kids, pos, remote) = if let Some(server) = &self.common.adv.remote_tree {
            let tabs: Vec<_> = displayed_roots(&tree).take(26).collect();
            let links: Vec<_> = server.widget_links(&tree).collect();
            let mut kids = vec![Vec::new(); tree.list.len()];
            let mut by_root = vec![Vec::new(); tree.list.len()];
            for &root in &tabs {
                by_root[root].push(root);
            }
            for (i, &(root, parent)) in links.iter().enumerate() {
                if let Some(root) = root
                    && root != i
                    && !by_root[root].is_empty()
                    && tree.list[i].display.is_some()
                {
                    by_root[root].push(i);
                    if let Some(parent) = parent
                        && (parent == root || links[parent].0 == Some(root))
                        && tree.list[parent].display.is_some()
                    {
                        kids[parent].push(i);
                    }
                }
            }
            // AdvancementWidget uses Mth.floor on the float products, then
            // Java's narrowing conversion (saturating, NaN becomes zero).
            let pos = server
                .positions
                .iter()
                .map(|&(x, y)| Some(((x * 28.0).floor() as i32, (y * 27.0).floor() as i32)))
                .collect();
            (kids, pos, Some(RemoteWidgets { tabs, by_root }))
        } else {
            let pos = tree
                .layout()
                .into_iter()
                .map(|p| p.map(|(x, y)| (x * 28, (y * 27.0).floor() as i32)))
                .collect();
            (tree.children(), pos, None)
        };
        self.client.adv_ui.0.cache = Some(Cache {
            tree,
            kids,
            pos,
            remote,
        });
    }
    pub fn adv_counts(&self, i: usize) -> Option<(usize, usize)> {
        let a = &self.common.adv.tree.list[i];
        if self.common.adv.remote_tree.is_some() {
            self.common.adv.progress.remote_counts(&a.id)
        } else {
            (!a.criteria.is_empty()).then(|| self.common.adv.progress.counts(a))
        }
    }
    pub fn adv_done(&self, i: usize) -> bool {
        if self.common.adv.remote_tree.is_some() {
            self.adv_counts(i).is_some_and(|(got, total)| got == total)
        } else {
            self.common.adv.progress.done(&self.common.adv.tree.list[i])
        }
    }
    pub fn adv_view(&mut self) -> View {
        self.adv_cache();
        let c = self
            .client
            .adv_ui
            .0
            .cache
            .as_ref()
            .expect("advancement layout cached");
        let tree = &c.tree;
        let (tabs, visible) = if let Some(remote) = &c.remote {
            (remote.tabs.clone(), None)
        } else {
            let visible = tree.visible(&c.kids, |i| self.common.adv.progress.done(&tree.list[i]));
            let mut tabs: Vec<_> = displayed_roots(tree).filter(|&i| visible[i]).collect();
            tabs.sort_by_key(|&i| {
                let id = &tree.list[i].id;
                (
                    TAB_ORDER
                        .iter()
                        .position(|r| r == id)
                        .unwrap_or(TAB_ORDER.len()),
                    id.as_str(),
                )
            });
            tabs.truncate(26);
            (tabs, Some(visible))
        };
        let tab = self
            .client
            .adv_ui
            .0
            .tab
            .as_ref()
            .and_then(|id| tree.by_id.get(id).copied())
            .filter(|t| tabs.contains(t))
            .or_else(|| tabs.first().copied());
        let mut parent = vec![None; tree.list.len()];
        let mut widgets = Vec::new();
        if let Some(t) = tab {
            let mut stack = vec![(t, None)];
            let mut bounds = TabBounds::default();
            while let Some((i, up)) = stack.pop() {
                let next = if let Some((x, y)) = c.pos[i]
                    && visible.as_ref().is_none_or(|v| v[i])
                    && tree.list[i].display.is_some()
                {
                    parent[i] = up;
                    widgets.push(i);
                    if visible.is_some() {
                        bounds.add_widget(x, y);
                    }
                    Some(i)
                } else {
                    up
                };
                for &k in c.kids[i].iter().rev() {
                    stack.push((k, next));
                }
            }
            let id = &tree.list[t].id;
            if visible.is_some() {
                if let Some(state) = self.client.adv_ui.0.tabs.get_mut(id) {
                    state.bounds = bounds;
                } else {
                    self.client.adv_ui.0.tabs.insert(
                        id.clone(),
                        TabState {
                            bounds,
                            ..Default::default()
                        },
                    );
                }
            }
            if self.client.adv_ui.0.tab.as_ref() != Some(id) {
                self.client.adv_ui.0.tab = Some(id.clone());
            }
        } else {
            self.client.adv_ui.0.tab = None;
        }
        View {
            tabs,
            tab,
            widgets,
            parent,
        }
    }
    fn adv_scroll_by(&mut self, dx: f64, dy: f64) {
        let Some(state) = self.client.adv_ui.0.tab_state_mut() else {
            return;
        };
        let TabBounds(min_x, min_y, max_x, max_y) = state.bounds;
        let Some((sx, sy)) = state.scroll.as_mut() else {
            return;
        };
        for (scroll, delta, lo, hi, inside) in [
            (sx, dx, min_x, max_x, INSIDE_W),
            (sy, dy, min_y, max_y, INSIDE_H),
        ] {
            if hi.wrapping_sub(lo) > inside {
                let min = f64::from(hi.wrapping_sub(inside).wrapping_neg());
                let next = *scroll + delta;
                // Mth.clamp tests its lower bound first even when the
                // wrapped lower bound is greater than the upper bound.
                *scroll = if next < min {
                    min
                } else if next > 0.0 {
                    0.0
                } else {
                    next
                };
            }
        }
    }
    /// `AdvancementWidget`'s title lines, description lines and width.
    fn adv_widget_text(&self, i: usize) -> (Vec<Text>, Vec<Text>, f32) {
        let font = &self.client.gui.atlas.font;
        let a = &self.common.adv.tree.list[i];
        let Some(d) = &a.display else {
            return (Vec::new(), Vec::new(), 0.0);
        };
        let title = font.split(&text_of(&d.title), 163.0);
        let title_w = title
            .iter()
            .map(|l| font.width_text(l))
            .fold(0.0f32, f32::max)
            .max(80.0);
        let n = a.requirements.len();
        let progress_w = if n <= 1 {
            0.0
        } else {
            let t = Text::translate(
                "advancements.progress",
                &[Text::literal(n.to_string()), Text::literal(n.to_string())],
            );
            font.width_text(&t) + 8.0
        };
        let mut longest = 29.0 + title_w + progress_w;
        let color = match d.frame {
            Frame::Challenge => 0xAA00AA,
            _ => 0x55FF55,
        };
        let desc = text_of(&d.description).with_style(&Style::color(color));
        // findOptimalLines.
        let mut best: Option<(f32, Vec<Text>)> = None;
        for margin in [0.0, 10.0, -10.0, 25.0, -25.0] {
            let lines = font.split(&desc, longest - margin);
            let widest = lines.iter().map(|l| font.width_text(l)).fold(0.0, f32::max);
            let distance = (widest - longest).abs();
            if distance <= 10.0 {
                best = Some((distance, lines));
                break;
            }
            if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                best = Some((distance, lines));
            }
        }
        let desc = best.map(|b| b.1).unwrap_or_default();
        for l in &desc {
            longest = longest.max(font.width_text(l));
        }
        (title, desc, longest + 3.0 + 5.0)
    }
    pub fn draw_advancements(&mut self, batch: &mut HudBatch) {
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new(
                "advancements",
                super::screen_info::tr("gui.advancements", &[]),
            )
        });
        let (w, h) = (batch.width, batch.height);
        let view = self.adv_view();
        let (left, top) = window(w, h);
        let (ix, iy) = (left + 9.0, top + 18.0);
        // Centre each tab the first time it is drawn.
        let (sx, sy) = self
            .client
            .adv_ui
            .0
            .tab_state_mut()
            .map_or((0.0, 0.0), |tab| {
                let bounds = tab.bounds;
                *tab.scroll.get_or_insert_with(|| bounds.center())
            });
        let (sx, sy) = (sx.floor() as i32, sy.floor() as i32);
        self.adv_tick_hover(&view, sx, sy, left, top);
        let atlas = &self.client.gui.atlas;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let tree = self.common.adv.tree.clone();
        let tab = view.tab;
        match tab {
            None => batch.fill(atlas, ix, iy, 234.0, 113.0, 0xFF00_0000),
            Some(root) => {
                let start = batch.vertices.len();
                let bg = tree.list[root]
                    .display
                    .as_ref()
                    .and_then(|d| d.background.clone())
                    .unwrap_or_default();
                let bg = bg.strip_prefix("minecraft:").unwrap_or(&bg).to_owned();
                let (l, t) = ((sx % 16) as f32, (sy % 16) as f32);
                for x in -1..=15 {
                    for y in -1..=8 {
                        batch.sprite_scaled(
                            atlas,
                            &bg,
                            ix + l + 16.0 * x as f32,
                            iy + t + 16.0 * y as f32,
                            16.0,
                            16.0,
                            u32::MAX,
                        );
                    }
                }
                batch.clip_since(start, ix, iy, ix + 234.0, iy + 113.0);
            }
        }
        batch.sprite_region(
            atlas,
            "gui/advancements/window",
            [left, top, 252.0, 140.0],
            256.0,
            256.0,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let title = Text::translate("gui.advancements", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            12.0,
            u32::MAX,
            true,
        );
        // The inside.
        match tab {
            None => {
                let mid = ix + 117.0;
                for (key, y) in [
                    ("advancements.empty", iy + 56.0 - 4.0),
                    ("advancements.sad_label", iy + 113.0 - 9.0),
                ] {
                    let t = Text::translate(key, &[]);
                    let tw = atlas.font.width_text(&t);
                    batch.text_styled(atlas, &t, (mid - tw / 2.0).floor(), y, u32::MAX, true);
                }
            }
            Some(_) => {
                let start = batch.vertices.len();
                let all = self
                    .client
                    .adv_ui
                    .0
                    .cache
                    .as_ref()
                    .map(|c| c.pos.clone())
                    .unwrap_or_default();
                let pos = |i: usize| all.get(i).copied().flatten().unwrap_or_default();
                for background in [true, false] {
                    let col = if background { 0xFF00_0000 } else { u32::MAX };
                    let fill = |batch: &mut HudBatch, x0: i32, y0: i32, x1: i32, y1: i32| {
                        let (left, right) = (x0.min(x1), x0.max(x1));
                        let (top, bottom) = (y0.min(y1), y0.max(y1));
                        batch.fill(
                            atlas,
                            ix + left as f32,
                            iy + top as f32,
                            (i64::from(right) - i64::from(left)) as f32,
                            (i64::from(bottom) - i64::from(top)) as f32,
                            col,
                        );
                    };
                    let hline = |batch: &mut HudBatch, x0: i32, x1: i32, y: i32| {
                        fill(
                            batch,
                            x0.min(x1),
                            y,
                            x0.max(x1).wrapping_add(1),
                            y.wrapping_add(1),
                        );
                    };
                    let vline = |batch: &mut HudBatch, x: i32, y0: i32, y1: i32| {
                        let (a, b) = (y0.min(y1).wrapping_add(1), y0.max(y1));
                        if b > a {
                            fill(batch, x, a, x.wrapping_add(1), b);
                        }
                    };
                    for &i in &view.widgets {
                        let Some(p) = view.parent[i] else {
                            continue;
                        };
                        let (px, py) = pos(p);
                        let (mx, my) = pos(i);
                        let dep_x = sx.wrapping_add(px).wrapping_add(13);
                        let split_x = sx.wrapping_add(px).wrapping_add(30);
                        let dep_y = sy.wrapping_add(py).wrapping_add(13);
                        let my_x = sx.wrapping_add(mx).wrapping_add(13);
                        let my_y = sy.wrapping_add(my).wrapping_add(13);
                        if background {
                            let before = split_x.wrapping_sub(1);
                            let after = split_x.wrapping_add(1);
                            hline(batch, split_x, dep_x, dep_y.wrapping_sub(1));
                            hline(batch, after, dep_x, dep_y);
                            hline(batch, split_x, dep_x, dep_y.wrapping_add(1));
                            hline(batch, my_x, before, my_y.wrapping_sub(1));
                            hline(batch, my_x, before, my_y);
                            hline(batch, my_x, before, my_y.wrapping_add(1));
                            vline(batch, before, my_y, dep_y);
                            vline(batch, after, my_y, dep_y);
                        } else {
                            hline(batch, split_x, dep_x, dep_y);
                            hline(batch, my_x, split_x, my_y);
                            vline(batch, split_x, my_y, dep_y);
                        }
                    }
                }
                for &i in &view.widgets {
                    let a = &tree.list[i];
                    let Some(d) = &a.display else { continue };
                    let done = self.adv_done(i);
                    if d.hidden && !done {
                        continue;
                    }
                    let (x, y) = pos(i);
                    let (x, y) = (sx.wrapping_add(x), sy.wrapping_add(y));
                    let atlas = &self.client.gui.atlas;
                    batch.sprite_scaled(
                        atlas,
                        frame_sprite(d.frame, done),
                        ix + x.wrapping_add(3) as f32,
                        iy + y as f32,
                        26.0,
                        26.0,
                        u32::MAX,
                    );
                    self.draw_item(
                        batch,
                        &d.icon,
                        ix + x.wrapping_add(8) as f32,
                        iy + y.wrapping_add(5) as f32,
                    );
                }
                batch.clip_since(start, ix, iy, ix + 234.0, iy + 113.0);
            }
        }
        // The tabs and title.
        if view.tabs.len() > 1 {
            for (n, &t) in view.tabs.iter().enumerate() {
                let Some((ty, i)) = tab_type(n) else { continue };
                let (x, y) = tab_pos(ty, i);
                let sprite = tab_sprite(ty, i, ty.3, Some(t) == tab);
                sunrise_client::widgets::target_as(
                    [left + x, top + y, ty.1, ty.2],
                    sunrise_client::widgets::Kind::Tab(Some(t) == tab),
                );
                batch.sprite_scaled(
                    &self.client.gui.atlas,
                    &sprite,
                    left + x,
                    top + y,
                    ty.1,
                    ty.2,
                    u32::MAX,
                );
            }
            for (n, &t) in view.tabs.iter().enumerate() {
                let Some((ty, i)) = tab_type(n) else { continue };
                let (x, y) = tab_pos(ty, i);
                let (dx, dy) = icon_offset(ty.0);
                if let Some(d) = &tree.list[t].display {
                    self.draw_item(batch, &d.icon, left + x + dx, top + y + dy);
                }
            }
        }
        let atlas = &self.client.gui.atlas;
        let title = tab.and_then(|t| tree.list[t].display.as_ref()).map_or_else(
            || Text::translate("gui.advancements", &[]),
            |d| text_of(&d.title),
        );
        batch.text_styled(atlas, &title, left + 8.0, top + 6.0, 0xFF40_4040, false);
        // Done.
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
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        // Tooltips.
        if let Some(state) = self.client.adv_ui.0.tab_state() {
            let alpha = (state.fade * 255.0).floor() as u32;
            batch.fill(atlas, ix, iy, 234.0, 113.0, alpha << 24);
            if let Some(i) = state.hovered {
                self.adv_draw_hover(batch, i, ix, iy, sx, sy, left, w);
            }
        }
        // Tab tooltips: the tab title under the cursor.
        if view.tabs.len() > 1 {
            for (n, &t) in view.tabs.iter().enumerate() {
                let Some((ty, i)) = tab_type(n) else { continue };
                let (x, y) = tab_pos(ty, i);
                let (x, y) = (left + x, top + y);
                let (cx, cy) = self.client.cursor;
                if cx > x && cx < x + ty.1 && cy > y && cy < y + ty.2 {
                    if let Some(d) = &tree.list[t].display {
                        ui::tooltip(
                            batch,
                            &self.client.gui.atlas,
                            &[text_of(&d.title)],
                            self.client.cursor,
                        );
                    }
                    break;
                }
            }
        }
    }
    /// `AdvancementTab.tick`, run per frame at the tick rate's pace.
    pub fn adv_tick_hover(&mut self, view: &View, sx: i32, sy: i32, left: f32, top: f32) {
        let now = Instant::now();
        let ticks = self
            .client
            .adv_ui
            .0
            .last_frame
            .map_or(0.0, |t| (now - t).as_secs_f32() * 20.0)
            .min(5.0);
        self.client.adv_ui.0.last_frame = Some(now);
        let mx = (self.client.cursor.0 - left - 9.0).floor() as i32;
        let my = (self.client.cursor.1 - top - 18.0).floor() as i32;
        let mut hovered = None;
        if mx > 0 && mx < INSIDE_W && my > 0 && my < INSIDE_H {
            let widgets = self
                .client
                .adv_ui
                .0
                .cache
                .as_ref()
                .and_then(|cache| cache.remote.as_ref())
                .and_then(|remote| view.tab.map(|tab| &remote.by_root[tab]))
                .unwrap_or(&view.widgets);
            for &i in widgets {
                let hidden = self.common.adv.tree.list[i]
                    .display
                    .as_ref()
                    .is_some_and(|d| d.hidden);
                if hidden && !self.adv_done(i) {
                    continue;
                }
                let Some((x, y)) = self.client.adv_ui.0.cache.as_ref().and_then(|c| c.pos[i])
                else {
                    continue;
                };
                let (x0, y0) = (sx.wrapping_add(x), sy.wrapping_add(y));
                if mx >= x0 && mx <= x0.wrapping_add(26) && my >= y0 && my <= y0.wrapping_add(26) {
                    hovered = Some(i);
                    break;
                }
            }
        }
        if let Some(tab) = self.client.adv_ui.0.tab_state_mut() {
            tab.hovered = hovered;
            tab.fade = if hovered.is_some() {
                (tab.fade + 0.06 * ticks).clamp(0.0, 0.3)
            } else {
                (tab.fade - 0.12 * ticks).clamp(0.0, 1.0)
            };
        }
    }
    /// `AdvancementWidget.extractHover`, in GUI pixels.
    #[allow(clippy::too_many_arguments)]
    fn adv_draw_hover(
        &mut self,
        batch: &mut HudBatch,
        i: usize,
        ix: f32,
        iy: f32,
        sx: i32,
        sy: i32,
        left: f32,
        screen_w: f32,
    ) {
        let (title, desc, width) = self.adv_widget_text(i);
        let tree = self.common.adv.tree.clone();
        let a = &tree.list[i];
        let Some(d) = &a.display else { return };
        let Some((wx, wy)) = self.client.adv_ui.0.cache.as_ref().and_then(|c| c.pos[i]) else {
            return;
        };
        let width = width as i32;
        let (x, y) = (sx.wrapping_add(wx), sy.wrapping_add(wy));
        let title_bar_h = 9 * title.len() as i32 + 9 + 8;
        let title_top = y.wrapping_add((26 - title_bar_h) / 2);
        let title_bottom = title_top.wrapping_add(title_bar_h);
        let desc_text_h = desc.len() as i32 * 9;
        let desc_h = 6 + desc_text_h;
        let left_side = (left as i32)
            .wrapping_add(x)
            .wrapping_add(width)
            .wrapping_add(26)
            >= screen_w as i32;
        let (got, total) = self.adv_counts(i).unwrap_or_default();
        let progress = (total > 1).then(|| {
            Text::translate(
                "advancements.progress",
                &[
                    Text::literal(got.to_string()),
                    Text::literal(total.to_string()),
                ],
            )
        });
        let atlas = &self.client.gui.atlas;
        let progress_w = progress.as_ref().map_or(0.0, |t| atlas.font.width_text(t)) as i32;
        let top_side = title_bottom.wrapping_add(desc_h) >= INSIDE_H;
        let amount = if total == 0 {
            0.0
        } else {
            got as f32 / total as f32
        };
        let mut first_w = (amount * width as f32).floor() as i32;
        let (first, second, icon_frame) = if amount >= 1.0 {
            first_w = width / 2;
            (true, true, true)
        } else if first_w < 2 {
            first_w = width / 2;
            (false, false, false)
        } else if first_w > width - 2 {
            first_w = width / 2;
            (true, true, false)
        } else {
            (true, false, false)
        };
        let second_w = width - first_w;
        let title_left = if left_side {
            x.wrapping_sub(width).wrapping_add(32)
        } else {
            x
        };
        let bg_h = title_bar_h + desc_h;
        let f = |v: i32| v as f32;
        if !desc.is_empty() {
            let by = if top_side {
                title_bottom.wrapping_sub(bg_h)
            } else {
                title_top
            };
            ui::nine_slice(
                batch,
                atlas,
                "advancements/title_box",
                ix + f(title_left),
                iy + f(by),
                f(width),
                f(bg_h),
                u32::MAX,
            );
        }
        if first != second {
            for (sprite, src_x, at, w) in [
                (box_sprite(first), 0, title_left, first_w),
                (
                    box_sprite(second),
                    200 - second_w,
                    title_left.wrapping_add(first_w),
                    second_w,
                ),
            ] {
                let start = batch.vertices.len();
                ui::nine_slice(
                    batch,
                    atlas,
                    sprite,
                    ix + f(at.wrapping_sub(src_x)),
                    iy + f(title_top),
                    200.0,
                    f(title_bar_h),
                    u32::MAX,
                );
                batch.clip_since(
                    start,
                    ix + f(at),
                    iy + f(title_top),
                    ix + f(at.wrapping_add(w)),
                    iy + f(title_bottom),
                );
            }
        } else {
            ui::nine_slice(
                batch,
                atlas,
                box_sprite(first),
                ix + f(title_left),
                iy + f(title_top),
                f(width),
                f(title_bar_h),
                u32::MAX,
            );
        }
        batch.sprite_scaled(
            atlas,
            frame_sprite(d.frame, icon_frame),
            ix + f(x.wrapping_add(3)),
            iy + f(y),
            26.0,
            26.0,
            u32::MAX,
        );
        let desc_left = title_left.wrapping_add(5);
        let lines = |batch: &mut HudBatch, lines: &[Text], x: i32, y: i32, color: u32| {
            for (n, l) in lines.iter().enumerate() {
                batch.text_styled(
                    atlas,
                    l,
                    ix + f(x),
                    iy + f(y.wrapping_add(n as i32 * 9)),
                    color,
                    true,
                );
            }
        };
        let (title_x, progress_x) = if left_side {
            (desc_left, x.wrapping_sub(progress_w))
        } else {
            (
                x.wrapping_add(32),
                x.wrapping_add(width)
                    .wrapping_sub(progress_w)
                    .wrapping_sub(5),
            )
        };
        lines(batch, &title, title_x, title_top.wrapping_add(9), u32::MAX);
        if let Some(p) = &progress {
            batch.text_styled(
                atlas,
                p,
                ix + f(progress_x),
                iy + f(title_top.wrapping_add(9)),
                u32::MAX,
                true,
            );
        }
        let desc_y = if top_side {
            title_top.wrapping_sub(desc_text_h).wrapping_add(1)
        } else {
            title_bottom
        };
        lines(batch, &desc, desc_left, desc_y, 0xFF00_FF00);
        self.draw_item(
            batch,
            &d.icon,
            ix + f(x.wrapping_add(8)),
            iy + f(y.wrapping_add(5)),
        );
    }
    /// A left button press or release. Returns whether the screen itself
    /// handled the click (`AdvancementsScreen.mouseClicked` / `mouseReleased`:
    /// tabs and Done return `true`, blank space and releases return `false`).
    pub(in crate::game::client_side) fn advancements_mouse(
        &mut self,
        pressed: bool,
        size: [u32; 2],
    ) -> bool {
        if !pressed {
            self.client.adv_ui.0.drag = None;
            self.client.adv_ui.0.scrolling = false;
            return false;
        }
        let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let (w, h) = (b.width, b.height);
        if ui::hovered(
            self.client.cursor,
            (w / 2.0 - 100.0).floor(),
            h - FOOTER + 6.0,
            200.0,
            20.0,
        ) {
            self.play_audio("minecraft:ui.button.click", None, 0.25, 1.0);
            self.close_advancements();
            return true;
        }
        let view = self.adv_view();
        let (left, top) = window(w, h);
        for (n, &t) in view.tabs.iter().enumerate() {
            let Some((ty, i)) = tab_type(n) else { continue };
            let (x, y) = tab_pos(ty, i);
            let (x, y) = (left + x, top + y);
            let (cx, cy) = self.client.cursor;
            if cx > x && cx < x + ty.1 && cy > y && cy < y + ty.2 {
                // `ClientAdvancements.setSelectedTab(tab, true)`: the packet
                // goes out before the tab is compared, so a click on the tab
                // already open tells the server again.
                self.select_advancement_tab(t);
                return true;
            }
        }
        self.client.adv_ui.0.drag = Some(self.client.cursor);
        false
    }
    /// `mouseDragged` with the left button: the first step starts
    /// scrolling, later ones move the tree.
    pub(in crate::game::client_side) fn advancements_drag(&mut self) -> bool {
        let Some(last) = self.client.adv_ui.0.drag else {
            return false;
        };
        let (dx, dy) = (self.client.cursor.0 - last.0, self.client.cursor.1 - last.1);
        self.client.adv_ui.0.drag = Some(self.client.cursor);
        if !self.client.adv_ui.0.scrolling {
            self.client.adv_ui.0.scrolling = true;
            return true;
        }
        self.adv_view();
        self.adv_scroll_by(f64::from(dx), f64::from(dy));
        true
    }
    /// `AdvancementsScreen.mouseScrolled`: returns `true` only when a tab is
    /// selected; with no selected tab vanilla returns `false` so NeoForge
    /// fires `MouseScrolled.Post`.
    pub fn advancements_scroll(&mut self, x: f32, y: f32) -> bool {
        if self.adv_view().tab.is_none() {
            return false;
        }
        self.adv_scroll_by(f64::from(x) * 16.0, f64::from(y) * 16.0);
        true
    }
}

#[cfg(test)]
mod seen_advancement_tests {
    use super::*;
    use sunrise_core::vanilla_bytes::Writer;
    use sunrise_protocol::buf::{Codec, Reader};
    use sunrise_protocol::packets::play::SeenAdvancements;

    /// `ServerboundSeenAdvancementsPacket.openedTab` and `closedScreen`: an
    /// opened tab is action 0 with its advancement id, a closed screen action
    /// 1 with no id at all.
    #[test]
    fn an_opened_tab_carries_its_id_and_a_close_carries_none() {
        assert_eq!(
            opened_tab("minecraft:story/root"),
            SeenAdvancements {
                action: 0,
                tab: Some("minecraft:story/root".into()),
            }
        );
        assert_eq!(
            closed_screen(),
            SeenAdvancements {
                action: 1,
                tab: None,
            }
        );
    }

    /// The wire form `handleSeenAdvancements` reads back: the id follows only
    /// action 0, so a close is two bytes and a read of it yields no tab.
    #[test]
    fn a_closed_screen_writes_no_identifier() {
        let mut w = Writer::new();
        SeenAdvancements::write(&closed_screen(), &mut w);
        let bytes = w.data;
        assert_eq!(bytes, vec![1]);
        let mut r = Reader::new(&bytes);
        assert_eq!(
            SeenAdvancements::read(&mut r).unwrap(),
            SeenAdvancements {
                action: 1,
                tab: None,
            }
        );
        let mut w = Writer::new();
        SeenAdvancements::write(&opened_tab("minecraft:story/root"), &mut w);
        let bytes = w.data;
        let mut r = Reader::new(&bytes);
        assert_eq!(
            SeenAdvancements::read(&mut r).ok(),
            Some(opened_tab("minecraft:story/root"))
        );
    }
}
