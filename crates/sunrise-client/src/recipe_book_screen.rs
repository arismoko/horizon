//! The recipe book's drawing (`RecipeBookComponent.extractRenderState`,
//! `extractTooltip` and `GhostSlots.extractRenderState`) as the list of GUI
//! calls vanilla's widgets make: the background, the search `EditBox`, the
//! `RecipeBookTabButton`s, the craftable filter `CycleButton`, the
//! `RecipeBookPage` with its `RecipeButton`s, page number and arrows, the
//! `OverlayRecipeComponent` and the ghost slots, each with the pose it is
//! drawn under. The game paints them; the render parity harness compares them.
use crate::{
    items::ItemStack,
    recipe_book::{Book, BookType, Display, Entry},
};

/// A styled run of text: its characters, colour (`-1` for the call's own) and italics.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub color: i32,
    pub italic: bool,
}
impl Run {
    pub fn plain(text: impl Into<String>) -> Self {
        Run {
            text: text.into(),
            color: -1,
            italic: false,
        }
    }
}

/// How an item is drawn: `GuiGraphicsExtractor.item`, `fakeItem` or `itemDecorations`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemCall {
    Item,
    Fake,
    Decorations,
}

/// One GUI call.
#[derive(Clone, Debug)]
pub enum Call {
    /// `blit` of a texture region (the book's background).
    Blit {
        texture: &'static str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        u: f32,
        v: f32,
        tw: i32,
        th: i32,
    },
    /// `blitSprite`: vanilla's id without its namespace, or a mod's `namespace:path`.
    Sprite {
        id: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Fill {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: i32,
    },
    /// `textHighlight` (the search box's selection).
    Highlight { x0: i32, y0: i32, x1: i32, y1: i32 },
    Text {
        runs: Vec<Run>,
        x: i32,
        y: i32,
        color: i32,
        shadow: bool,
    },
    Item {
        call: ItemCall,
        stack: ItemStack,
        x: i32,
        y: i32,
    },
    /// A widget's tooltip lines.
    Tooltip(Vec<Vec<Run>>),
    /// The hovered recipe button's tooltip: its display stack's own tooltip,
    /// then "Right Click for More" when it has several recipes.
    ItemTooltip { stack: ItemStack, more: bool },
    /// `nextStratum`.
    Stratum,
}

/// A call and the pose (`Matrix3x2f` m00, m01, m10, m11, m20, m21) it is made under.
#[derive(Clone, Debug)]
pub struct Drawn {
    pub call: Call,
    pub pose: [f32; 6],
}

pub const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// JOML `Matrix3x2f.translate`.
fn translate(m: [f32; 6], x: f32, y: f32) -> [f32; 6] {
    [
        m[0],
        m[1],
        m[2],
        m[3],
        m[0] * x + m[2] * y + m[4],
        m[1] * x + m[3] * y + m[5],
    ]
}
/// JOML `Matrix3x2f.scale`.
fn scale(m: [f32; 6], x: f32, y: f32) -> [f32; 6] {
    [m[0] * x, m[1] * x, m[2] * y, m[3] * y, m[4], m[5]]
}

/// The search box (`EditBox` fields).
#[derive(Clone, Debug, Default)]
pub struct Search {
    pub value: String,
    /// Cursor, selection end and first shown character, in characters.
    pub cursor: usize,
    pub highlight: usize,
    pub display: usize,
    pub focused: bool,
    /// Milliseconds since the box was focused (`TextCursorUtils.isCursorVisible`).
    pub blink_ms: u64,
}

pub use crate::recipe_book::mod_presentation::{ButtonView, OverlayRecipe, TabView};

/// The alternatives overlay of a button, at the corner [`overlay_origin`] gives.
#[derive(Clone, Debug)]
pub struct OverlayView {
    pub x: i32,
    pub y: i32,
    /// Craftable recipes first.
    pub recipes: Vec<OverlayRecipe>,
}

/// A ghost slot: its position in the menu, the stacks it cycles through,
/// and whether it is the result slot.
#[derive(Clone, Debug)]
pub struct GhostView {
    pub x: i32,
    pub y: i32,
    pub items: Vec<ItemStack>,
    pub result: bool,
}

/// What the recipe book shows.
pub struct Screen<'a> {
    pub book: BookType,
    /// Absolute book corner, shared with input. Overlay and ghost coordinates stay absolute.
    pub origin: (i32, i32),
    pub mouse: (i32, i32),
    /// `slotSelectTime.currentIndex()`.
    pub index: usize,
    pub filtering: bool,
    /// A mod book's own filter button look; none keeps the sprites and tooltips of `book`.
    pub filter_look: Option<&'a mod_api::RecipeBookFilterLook>,
    pub search: &'a Search,
    pub tabs: &'a [TabView],
    /// The current page's buttons.
    pub buttons: &'a [ButtonView],
    pub page: usize,
    pub pages: usize,
    pub overlay: Option<&'a OverlayView>,
    /// The container's corner, whether its result slot is big, and the ghost slots.
    pub ghost: Option<(i32, i32, bool, &'a [GhostView])>,
    /// Translations.
    pub translate: &'a dyn Fn(&str) -> String,
    /// `Font.width`.
    pub text_width: &'a dyn Fn(&str) -> i32,
}

impl Screen<'_> {
    /// `RecipeBookComponent.getXOrigin` and `getYOrigin`.
    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }
    fn hovered(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        let (mx, my) = self.mouse;
        mx >= x && my >= y && mx < x + w && my < y + h
    }
    /// `Font.plainSubstrByWidth`.
    fn substr_by_width<'s>(&self, s: &'s str, max: i32) -> &'s str {
        let mut end = 0;
        for (i, c) in s.char_indices() {
            if (self.text_width)(&s[..i + c.len_utf8()]) > max {
                break;
            }
            end = i + c.len_utf8();
        }
        &s[..end]
    }
}

/// Just the category tabs `draw` puts beside a book at `origin`, as it draws them.
#[cfg(test)]
fn draw_tabs_for_test(origin: (i32, i32), tabs: &[TabView]) -> Vec<Drawn> {
    let mut out = Vec::new();
    let (xo, yo) = origin;
    put_tabs(xo, yo, tabs, &mut |call, pose| {
        out.push(Drawn { call, pose })
    });
    out
}

/// `updateTabs`' tabs, and each one's icons: what `draw` puts for them.
fn put_tabs(xo: i32, yo: i32, tabs: &[TabView], put: &mut impl FnMut(Call, [f32; 6])) {
    let tab_x = xo - 30;
    let tab_y = yo + 3;
    for (n, tab) in tabs.iter().enumerate() {
        let (x, y) = (tab_x, tab_y + 27 * n as i32);
        let mut pose = IDENTITY;
        if tab.anim > 0.0 {
            let squeeze = squeeze(tab.anim);
            let (cx, cy) = ((x + 8) as f32, (y + 12) as f32);
            pose = translate(scale(translate(pose, cx, cy), 1.0, squeeze), -cx, -cy);
        }
        let sprite = if tab.selected {
            "recipe_book/tab_selected"
        } else {
            "recipe_book/tab"
        };
        let left = if tab.selected { -2 } else { 0 };
        put(sprite_call(sprite, x + left, y, 35, 27), pose);
        let fake = |id: &str, x: i32| Call::Item {
            call: ItemCall::Fake,
            stack: ItemStack::new(id, 1),
            x,
            y: y + 5,
        };
        match &tab.icons.1 {
            Some(second) => {
                put(fake(&tab.icons.0, x + 3 + left), pose);
                put(fake(second, x + 14 + left), pose);
            }
            None => put(fake(&tab.icons.0, x + 9 + left), pose),
        }
    }
}

/// The recipe book's calls, in vanilla's order.
pub fn draw(s: &Screen) -> Vec<Drawn> {
    let mut out = Vec::new();
    let mut put = |call: Call, pose: [f32; 6]| out.push(Drawn { call, pose });
    let (xo, yo) = s.origin();
    put(
        Call::Blit {
            texture: "textures/gui/recipe_book.png",
            x: xo,
            y: yo,
            w: 147,
            h: 166,
            u: 1.0,
            v: 1.0,
            tw: 256,
            th: 256,
        },
        IDENTITY,
    );
    search_box(s, xo + 25, yo + 13, &mut put);
    // updateTabs.
    put_tabs(xo, yo, s.tabs, &mut put);
    // The craftable filter.
    let (fx, fy) = (xo + 110, yo + 12);
    let hover = s.hovered(fx, fy, 26, 16);
    let highlight = if hover { "_highlighted" } else { "" };
    let sprite = match s.filter_look {
        Some(look) => match (s.filtering, hover) {
            (true, false) => look.enabled.clone(),
            (false, false) => look.disabled.clone(),
            (true, true) => look.enabled_highlighted.clone(),
            (false, true) => look.disabled_highlighted.clone(),
        },
        None => {
            let prefix = if s.book == BookType::Crafting {
                "recipe_book/"
            } else {
                "recipe_book/furnace_"
            };
            let state = if s.filtering {
                "filter_enabled"
            } else {
                "filter_disabled"
            };
            format!("{prefix}{state}{highlight}")
        }
    };
    put(sprite_call(&sprite, fx, fy, 26, 16), IDENTITY);
    if hover {
        let key = match (s.filtering, s.filter_look) {
            (false, _) => "gui.recipebook.toggleRecipes.all",
            (true, Some(look)) => look.tooltip.as_str(),
            (true, None) => match s.book {
                BookType::Crafting => "gui.recipebook.toggleRecipes.craftable",
                BookType::Furnace => "gui.recipebook.toggleRecipes.smeltable",
                BookType::BlastFurnace => "gui.recipebook.toggleRecipes.blastable",
                BookType::Smoker => "gui.recipebook.toggleRecipes.smokable",
            },
        };
        put(
            Call::Tooltip(vec![vec![Run::plain((s.translate)(key))]]),
            IDENTITY,
        );
    }
    // RecipeBookPage.extractRenderState.
    if s.pages > 1 {
        let text = (s.translate)("gui.recipebook.page")
            .replacen("%s", &(s.page + 1).to_string(), 1)
            .replacen("%s", &s.pages.to_string(), 1);
        let w = (s.text_width)(&text);
        put(
            Call::Text {
                runs: vec![Run::plain(text)],
                x: xo - w / 2 + 73,
                y: yo + 141,
                color: -1,
                shadow: true,
            },
            IDENTITY,
        );
    }
    let mut hovered = None;
    for (i, b) in s.buttons.iter().enumerate().take(20) {
        let (x, y) = (xo + 11 + 25 * (i as i32 % 5), yo + 31 + 25 * (i as i32 / 5));
        let many = b.entries.len() > 1;
        let sprite = match (b.has_craftable, many) {
            (true, true) => "recipe_book/slot_many_craftable",
            (true, false) => "recipe_book/slot_craftable",
            (false, true) => "recipe_book/slot_many_uncraftable",
            (false, false) => "recipe_book/slot_uncraftable",
        };
        let mut pose = IDENTITY;
        if b.anim > 0.0 {
            let squeeze = squeeze(b.anim);
            let (cx, cy) = ((x + 8) as f32, (y + 12) as f32);
            pose = translate(scale(translate(pose, cx, cy), squeeze, squeeze), -cx, -cy);
        }
        put(sprite_call(sprite, x, y, 25, 25), pose);
        let stack = display_stack(&b.entries, s.index);
        let mut offset = 4;
        if many && same_result_display(&b.entries) {
            put(
                Call::Item {
                    call: ItemCall::Item,
                    stack: stack.clone(),
                    x: x + offset + 1,
                    y: y + offset + 1,
                },
                pose,
            );
            offset -= 1;
        }
        put(
            Call::Item {
                call: ItemCall::Fake,
                stack: stack.clone(),
                x: x + offset,
                y: y + offset,
            },
            pose,
        );
        if s.hovered(x, y, 25, 25) {
            hovered = Some((stack, many));
        }
    }
    let arrows = [
        (
            s.pages > 1 && s.page + 1 < s.pages,
            xo + 93,
            "recipe_book/page_forward",
            "gui.recipebook.next_page",
        ),
        (
            s.pages > 1 && s.page > 0,
            xo + 38,
            "recipe_book/page_backward",
            "gui.recipebook.previous_page",
        ),
    ];
    for (visible, x, sprite, key) in arrows {
        if !visible {
            continue;
        }
        let hover = s.hovered(x, yo + 137, 12, 17);
        let id = if hover {
            format!("{sprite}_highlighted")
        } else {
            sprite.to_owned()
        };
        put(sprite_call(&id, x, yo + 137, 12, 17), IDENTITY);
        if hover {
            put(
                Call::Tooltip(vec![vec![Run::plain((s.translate)(key))]]),
                IDENTITY,
            );
        }
    }
    put(Call::Stratum, IDENTITY);
    if let Some(o) = s.overlay {
        overlay(s, o, &mut put);
    }
    // RecipeBookPage.extractTooltip.
    if let Some((stack, more)) = hovered
        && s.overlay.is_none()
    {
        put(Call::ItemTooltip { stack, more }, IDENTITY);
    }
    if let Some((left, top, big, ghosts)) = s.ghost {
        out.extend(ghost(s.index, left, top, big, ghosts));
    }
    out
}

/// `GhostSlots.extractRenderState` for a container whose corner is `(left, top)`.
pub fn ghost(index: usize, left: i32, top: i32, big: bool, ghosts: &[GhostView]) -> Vec<Drawn> {
    let mut out = Vec::new();
    let mut put = |call: Call, pose: [f32; 6]| out.push(Drawn { call, pose });
    let pose = translate(IDENTITY, left as f32, top as f32);
    for g in ghosts.iter().filter(|g| !g.items.is_empty()) {
        let (x, y) = (g.x, g.y);
        if g.result && big {
            put(fill(x - 4, y - 4, x + 20, y + 20, 822_018_048), pose);
        } else {
            put(fill(x, y, x + 16, y + 16, 822_018_048), pose);
        }
        let stack = g.items[index % g.items.len()].clone();
        put(
            Call::Item {
                call: ItemCall::Fake,
                stack: stack.clone(),
                x,
                y,
            },
            pose,
        );
        put(fill(x, y, x + 16, y + 16, 822_083_583), pose);
        if g.result {
            put(
                Call::Item {
                    call: ItemCall::Decorations,
                    stack,
                    x,
                    y,
                },
                pose,
            );
        }
    }
    out
}

/// The box the sprites of `calls` that `wanted` picks cover, each through its pose as the painter
/// draws it: the recipe book's tab column is its `recipe_book/tab` and `tab_selected` sprites,
/// squeezed as they animate. None when no such sprite was drawn.
pub fn sprite_bounds(calls: &[Drawn], wanted: impl Fn(usize, &str) -> bool) -> Option<[f32; 4]> {
    let mut bounds: Option<[f32; 4]> = None;
    for (index, drawn) in calls.iter().enumerate() {
        let Call::Sprite { id, x, y, w, h } = &drawn.call else {
            continue;
        };
        if !wanted(index, id) {
            continue;
        }
        let m = drawn.pose;
        let (x0, y0, x1, y1) = (*x as f32, *y as f32, (*x + *w) as f32, (*y + *h) as f32);
        for (px, py) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            // JOML `Matrix3x2f.transformPosition`.
            let (tx, ty) = (m[0] * px + m[2] * py + m[4], m[1] * px + m[3] * py + m[5]);
            bounds = Some(match bounds {
                None => [tx, ty, tx, ty],
                Some(b) => [b[0].min(tx), b[1].min(ty), b[2].max(tx), b[3].max(ty)],
            });
        }
    }
    bounds.map(|b| [b[0], b[1], b[2] - b[0], b[3] - b[1]])
}

/// Whether `id` is one of the recipe book's category tab sprites.
pub fn is_tab_sprite(id: &str) -> bool {
    id == "recipe_book/tab" || id == "recipe_book/tab_selected"
}

/// `1 + 0.1 sin(t / 15 * PI)`, the squeeze of new recipes' buttons and tabs.
fn squeeze(time: f32) -> f32 {
    1.0 + 0.1 * f64::from(time / 15.0 * std::f32::consts::PI).sin() as f32
}

fn sprite_call(id: &str, x: i32, y: i32, w: i32, h: i32) -> Call {
    Call::Sprite {
        id: id.to_owned(),
        x,
        y,
        w,
        h,
    }
}
fn fill(x0: i32, y0: i32, x1: i32, y1: i32, color: i32) -> Call {
    Call::Fill {
        x0,
        y0,
        x1,
        y1,
        color,
    }
}

/// `RecipeButton.getDisplayStack`.
pub fn display_stack(entries: &[Vec<ItemStack>], index: usize) -> ItemStack {
    let n = entries.len().max(1);
    let (offset, entry) = (index / n, index % n);
    match entries.get(entry) {
        Some(items) if !items.is_empty() => items[offset % items.len()].clone(),
        _ => ItemStack::new("minecraft:air", 0),
    }
}
/// `RecipeButton.allRecipesHaveSameResultDisplay`.
fn same_result_display(entries: &[Vec<ItemStack>]) -> bool {
    let mut items = entries.iter().flatten();
    let Some(first) = items.next() else {
        return true;
    };
    items.all(|s| first.same_item_same_components(s))
}

/// `EditBox.extractWidgetRenderState` for the recipe book's search box.
fn search_box(s: &Screen, x: i32, y: i32, put: &mut impl FnMut(Call, [f32; 6])) {
    let b = s.search;
    let (w, h) = (81, 9 + 5);
    let sprite = if b.focused {
        "widget/text_field_highlighted"
    } else {
        "widget/text_field"
    };
    put(sprite_call(sprite, x, y, w, h), IDENTITY);
    let (text_x, text_y) = (x + 4, y + (h - 8) / 2);
    let color = -1;
    let chars: Vec<char> = b.value.chars().collect();
    let tail: String = chars[b.display.min(chars.len())..].iter().collect();
    let displayed = s.substr_by_width(&tail, w - 8).to_owned();
    let shown = displayed.chars().count();
    let rel_cursor = b.cursor as i64 - b.display as i64;
    let on_screen = rel_cursor >= 0 && rel_cursor <= shown as i64;
    let visible = b.focused && (b.blink_ms / 300).is_multiple_of(2) && on_screen;
    let mut draw_x = text_x;
    let rel_highlight = (b.highlight as i64 - b.display as i64).clamp(0, shown as i64) as usize;
    let prefix = |n: usize| displayed.chars().take(n).collect::<String>();
    if !displayed.is_empty() {
        let half = if on_screen {
            prefix(rel_cursor as usize)
        } else {
            displayed.clone()
        };
        let width = (s.text_width)(&half);
        put(
            Call::Text {
                runs: runs_of(half),
                x: draw_x,
                y: text_y,
                color,
                shadow: true,
            },
            IDENTITY,
        );
        draw_x += width + 1;
    }
    let insert = b.cursor < chars.len() || chars.len() >= 50;
    let mut cursor_x = draw_x;
    if !on_screen {
        cursor_x = if rel_cursor > 0 { text_x + w } else { text_x };
    } else if insert {
        cursor_x -= 1;
        draw_x -= 1;
    }
    if !displayed.is_empty() && on_screen && (rel_cursor as usize) < shown {
        let rest: String = displayed.chars().skip(rel_cursor as usize).collect();
        put(
            Call::Text {
                runs: runs_of(rest),
                x: draw_x,
                y: text_y,
                color,
                shadow: true,
            },
            IDENTITY,
        );
    }
    if displayed.is_empty() && !b.focused {
        // SEARCH_HINT: gray, italic.
        put(
            Call::Text {
                runs: vec![Run {
                    text: (s.translate)("gui.recipebook.search_hint"),
                    color: 0xAAAAAA,
                    italic: true,
                }],
                x: draw_x,
                y: text_y,
                color,
                shadow: true,
            },
            IDENTITY,
        );
    }
    if rel_highlight as i64 != rel_cursor {
        let highlight_x = text_x + (s.text_width)(&prefix(rel_highlight));
        put(
            Call::Highlight {
                x0: cursor_x.min(x + w),
                y0: text_y - 1,
                x1: (highlight_x - 1).min(x + w),
                y1: text_y + 1 + 9,
            },
            IDENTITY,
        );
    }
    if visible {
        if insert {
            put(
                fill(cursor_x, text_y - 1, cursor_x + 1, text_y + 10, color),
                IDENTITY,
            );
        } else {
            put(
                Call::Text {
                    runs: vec![Run::plain("_")],
                    x: cursor_x,
                    y: text_y,
                    color,
                    shadow: true,
                },
                IDENTITY,
            );
        }
    }
}
fn runs_of(text: String) -> Vec<Run> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![Run::plain(text)]
    }
}

/// `OverlayRecipeComponent.init`'s corner for `total` recipes opened from the
/// button at `(button_x, button_y)` of the book at `(xo, yo)`.
pub fn overlay_origin(button_x: i32, button_y: i32, total: usize, xo: i32, yo: i32) -> (i32, i32) {
    let total = total as i32;
    let max_row = if total <= 16 { 4 } else { 5 };
    let rows = (total as f32 / max_row as f32).ceil() as i32;
    let (mut x, mut y) = (button_x, button_y);
    let width = 25.0f32;
    let (center_x, center_y) = (xo + 147 / 2, yo + 13 + 166 / 2);
    let right = (x + total.min(max_row) * 25) as f32;
    let max_left = (center_x + 50) as f32;
    if right > max_left {
        x = (x as f32 - width * ((right - max_left) / width) as i32 as f32) as i32;
    }
    let bottom = (y + rows * 25) as f32;
    let max_bottom = (center_y + 50) as f32;
    if bottom > max_bottom {
        y = (y as f32 - width * ((bottom - max_bottom) / width).ceil()) as i32;
    }
    let top = y as f32;
    let max_top = (center_y - 100) as f32;
    if top < max_top {
        y = (y as f32 - width * ((top - max_top) / width).ceil()) as i32;
    }
    (x, y)
}

/// `OverlayRecipeComponent.extractRenderState`.
fn overlay(s: &Screen, o: &OverlayView, put: &mut impl FnMut(Call, [f32; 6])) {
    let total = o.recipes.len() as i32;
    let max_row = if total <= 16 { 4 } else { 5 };
    let (x, y) = (o.x, o.y);
    let columns = total.min(max_row);
    let height = (total as f32 / max_row as f32).ceil() as i32;
    put(
        sprite_call(
            "recipe_book/overlay_recipe",
            x,
            y,
            columns * 25 + 8,
            height * 25 + 8,
        ),
        IDENTITY,
    );
    let family = if s.book == BookType::Crafting {
        "crafting"
    } else {
        "furnace"
    };
    for (i, r) in o.recipes.iter().enumerate() {
        let i = i as i32;
        let (bx, by) = (x + 4 + 25 * (i % max_row), y + 5 + 25 * (i / max_row));
        let hover = s.hovered(bx, by, 24, 24);
        let id = format!(
            "recipe_book/{family}_overlay{}{}",
            if r.craftable { "" } else { "_disabled" },
            if hover { "_highlighted" } else { "" }
        );
        put(sprite_call(&id, bx, by, 24, 24), IDENTITY);
        let (gx, gy) = ((bx + 2) as f32, (by + 2) as f32);
        // Vanilla's three cells occupy 20 pixels (two 7-pixel steps and
        // a 6-pixel item). Larger custom grids share those same inner bounds.
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (0.0_f32, 0.0_f32, 2.0_f32, 2.0_f32);
        for (cx, cy, _) in &r.cells {
            min_x = min_x.min(*cx as f32);
            min_y = min_y.min(*cy as f32);
            max_x = max_x.max(*cx as f32);
            max_y = max_y.max(*cy as f32);
        }
        let fit = (20.0 / ((max_x - min_x) * 7.0 + 6.0)).min(20.0 / ((max_y - min_y) * 7.0 + 6.0));
        for (cx, cy, items) in &r.cells {
            let pose = translate(
                scale(
                    translate(
                        IDENTITY,
                        gx + (3.0 + (*cx as f32 - min_x) * 7.0) * fit,
                        gy + (3.0 + (*cy as f32 - min_y) * 7.0) * fit,
                    ),
                    0.375 * fit,
                    0.375 * fit,
                ),
                -8.0,
                -8.0,
            );
            put(
                Call::Item {
                    call: ItemCall::Item,
                    stack: items[s.index % items.len()].clone(),
                    x: 0,
                    y: 0,
                },
                pose,
            );
        }
    }
}

/// The ingredient cells a recipe shows in the overlay
/// (`OverlayCraftingRecipeButton` / `OverlaySmeltingRecipeButton.calculateIngredientsPositions`).
pub fn overlay_cells(book: &Book, entry: &Entry) -> Vec<(i32, i32, Vec<ItemStack>)> {
    let stacks = |ingredient: &crate::crafting::Ingredient| -> Vec<ItemStack> {
        ingredient.shown.clone().unwrap_or_else(|| {
            book.items_of(ingredient)
                .iter()
                .map(|i| ItemStack::new(i, 1))
                .collect()
        })
    };
    let mut out = Vec::new();
    match &entry.display {
        Display::Furnace(i) => out.push((1, 1, stacks(i))),
        Display::Shaped { .. } => {
            let requirements = entry.requirements();
            for (ingredient, grid) in entry.grid_cells(3, 3) {
                if let Some(i) = ingredient {
                    out.push((grid as i32 % 3, grid as i32 / 3, stacks(requirements[i])));
                }
            }
        }
        Display::Shapeless(list) => {
            for (i, ingredient) in list.iter().enumerate() {
                out.push((i as i32 % 3, i as i32 / 3, stacks(ingredient)));
            }
        }
    }
    out.retain(|c| !c.2.is_empty());
    out
}

#[cfg(test)]
#[path = "recipe_book_screen/overlay_tests.rs"]
mod overlay_tests;

#[cfg(test)]
mod tab_column_tests {
    use super::{Drawn, IDENTITY, TabView, draw_tabs_for_test, is_tab_sprite, sprite_bounds};

    fn tab(selected: bool, anim: f32) -> TabView {
        TabView {
            icons: ("minecraft:compass".into(), None),
            selected,
            anim,
        }
    }

    fn column(tabs: &[TabView]) -> Option<[f32; 4]> {
        let calls: Vec<Drawn> = draw_tabs_for_test((54, 37), tabs);
        sprite_bounds(&calls, |_, id| is_tab_sprite(id))
    }

    /// `updateTabs` beside a book at (54, 37): x 24, from y 40, 35 x 27 each, 27 apart; the
    /// selected tab 2 further left; an animating one as the painter squeezes it.
    #[test]
    fn the_column_is_the_box_of_the_tab_sprites_drawn() {
        assert_eq!(column(&[]), None);
        assert_eq!(column(&[tab(false, 0.0)]), Some([24.0, 40.0, 35.0, 27.0]));
        assert_eq!(column(&[tab(true, 0.0)]), Some([22.0, 40.0, 35.0, 27.0]));
        assert_eq!(
            column(&[tab(true, 0.0), tab(false, 0.0), tab(false, 0.0)]),
            Some([22.0, 40.0, 37.0, 81.0])
        );
        // At y = 48, RecipeBookTabButton.extractContents' affine pose rounds the edges
        // differently from a center-relative calculation.
        let calls = draw_tabs_for_test((54, 45), &[tab(false, 7.5)]);
        assert_eq!(
            sprite_bounds(&calls, |_, id| is_tab_sprite(id)),
            Some([
                24.0,
                f32::from_bits(0x423b_3334),
                35.0,
                f32::from_bits(0x41ed_9998)
            ])
        );
        // A sprite drawn with no pose is its own rectangle.
        let lone = [Drawn {
            call: super::Call::Sprite {
                id: "recipe_book/page_forward".into(),
                x: 1,
                y: 2,
                w: 12,
                h: 17,
            },
            pose: IDENTITY,
        }];
        assert_eq!(
            sprite_bounds(&lone, |_, _| true),
            Some([1.0, 2.0, 12.0, 17.0])
        );
    }
}
