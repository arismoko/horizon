//! Creative inventory (CreativeModeInventoryScreen): vanilla's tabs and their
//! contents as built by the game's own display generators
//! (`creative-inventory.json`), a 9x5 scrolling grid and the search tab.
use crate::{
    Result, item_tooltip,
    items::{ItemStack, Items},
    lang,
    ui::EditBox,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
use sunrise_hooks::items::engine::CreativeOrder;
use sunrise_items::{
    creative_catalog::{Catalog, Kind, TabContents},
    item_data::component_snapshot,
};

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub items: Vec<ItemStack>,
    pub top: bool,
    pub column: u32,
    pub search: bool,
    pub inventory: bool,
    /// `CreativeModeTab.Type.HOTBAR`: the saved toolbars, filled when the
    /// tab is selected (`set_saved_hotbars`).
    pub hotbar: bool,
    /// The page of tabs it is on: vanilla's are 0, mods' after, as NeoForge
    /// pages them.
    pub page: u32,
    /// A Polytone creative tab modifier's `name`: its text (a translation key when
    /// the flag is set) and colour, over the tab's own.
    pub name: Option<(String, Option<u32>, bool)>,
}
impl Tab {
    /// The tab without its stacks.
    fn header(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            icon: self.icon.clone(),
            items: Vec::new(),
            top: self.top,
            column: self.column,
            search: self.search,
            inventory: self.inventory,
            hotbar: self.hotbar,
            page: self.page,
            name: self.name.clone(),
        }
    }
}
/// What the client's creative search trees (SessionSearchTrees) index for one
/// stack of the search tab: its creative tooltip lines and its item tags.
#[derive(Clone, Debug, Default)]
struct SearchEntry {
    /// The item id's namespace and path, lower-cased.
    namespace: String,
    path: String,
    /// Prepared English tooltip lines, never used on a holder-dependent stack.
    lines: Vec<String>,
    live_tooltip: bool,
    /// Item tags as (namespace, path), lower-cased.
    tags: Vec<(String, String)>,
}
pub struct Creative {
    /// The tabs shown (`CreativeModeTabs.tabs()`), with the stacks the current
    /// permissions put in them.
    pub tabs: Vec<Tab>,
    selected_id: String,
    /// The page of tabs shown.
    pub page: u32,
    /// Scroll position 0..1 (scrollOffs).
    pub scroll: f32,
    /// The scrollbar is held (scrolling): dragging moves it.
    pub scrolling: bool,
    /// CreativeModeInventoryScreen's search box: an unbordered `EditBox` of 50.
    pub search: EditBox,
    /// The search tab's stacks, in its display order.
    search_items: Vec<ItemStack>,
    /// The name a mod item's part gives each search stack (`Item.getName`), by index into
    /// `search_items`; empty while no part names any. Set by the game (`set_search_names`).
    search_names: Vec<Option<crate::text::Text>>,
    /// Counts `project`s: a new value means new search stacks to name.
    search_generation: u64,
    search_index: Vec<SearchEntry>,
    /// Definitions from the same catalog snapshot, rendered in the active language.
    holders: Arc<sunrise_items::creative_catalog::Holders>,
    items: Arc<Items>,
    catalog: Arc<Catalog>,
    headers: Vec<Tab>,
    /// Prepared search presentation only, keyed by item and exact components.
    metadata: HashMap<(Arc<str>, Option<sunrise_core::nbt::Tag>), SearchEntry>,
    /// Non-tabbed content items still live at the search UI's tail.
    ungrouped: Vec<ItemStack>,
    /// World item tags, inverted for search and Polytone presentation.
    item_tags: Option<HashMap<String, Vec<(String, String)>>>,
    contents: Option<Arc<[Arc<TabContents>]>>,
    /// ItemPickerMenu.items: what the grid shows, filled when the tab or the
    /// search changes (selectTab, refreshSearchResults), not on every frame.
    shown: Mutex<Option<Shown>>,
    /// SessionSearchTrees.creativeByNameSearch: the lower-cased tooltip lines
    /// of every search item, built on the first search and kept until the
    /// translations, item list or enchantment registry change.
    lines_cache: Mutex<Option<(u64, Arc<[Vec<String>]>)>>,
    /// The packs' Polytone creative tab modifiers, applied by `rebuild`.
    pack: Vec<crate::polytone_creative_tabs::TabModifier>,
}
#[cfg(any(test, feature = "test-resource-cache"))]
impl Creative {
    /// Clone loaded catalog state with fresh per-game UI cache mutexes for tests.
    pub fn clone_for_tests(&self) -> Self {
        Self {
            tabs: self.tabs.clone(),
            selected_id: self.selected_id.clone(),
            page: self.page,
            scroll: self.scroll,
            scrolling: self.scrolling,
            search: self.search.clone(),
            search_items: self.search_items.clone(),
            search_names: self.search_names.clone(),
            search_generation: self.search_generation,
            search_index: self.search_index.clone(),
            holders: Arc::clone(&self.holders),
            items: Arc::clone(&self.items),
            catalog: Arc::clone(&self.catalog),
            headers: self.headers.clone(),
            metadata: self.metadata.clone(),
            ungrouped: self.ungrouped.clone(),
            item_tags: self.item_tags.clone(),
            contents: self.contents.clone(),
            shown: Mutex::new(None),
            lines_cache: Mutex::new(None),
            pack: self.pack.clone(),
        }
    }
}
pub use crate::menu::{DisplayClick, click_display, slot_locked};
pub use sunrise_sim::item_names::{Names, live_holders};

fn append_unique(items: &mut Vec<ItemStack>, added: &[ItemStack]) {
    for stack in added {
        if stack.count > 0
            && stack.item.as_ref() != "minecraft:air"
            && !items
                .iter()
                .any(|item| item.same_item_same_components(stack))
        {
            items.push(stack.clone());
        }
    }
}

/// A text component's text, its colour (a name, or `#rrggbb`) and whether the
/// text is a `translate` key, to be resolved when drawn.
fn component_text(json: &Value) -> (String, Option<u32>, bool) {
    match json {
        Value::String(s) => (s.clone(), None, false),
        Value::Object(o) => {
            let translate = o.get("translate").and_then(Value::as_str);
            let text = o
                .get("text")
                .and_then(Value::as_str)
                .or(translate)
                .unwrap_or("")
                .to_owned();
            let translate = o.get("text").is_none() && translate.is_some();
            let color = o.get("color").and_then(Value::as_str).and_then(|c| {
                c.strip_prefix('#')
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| crate::text::named_color(c))
            });
            (text, color, translate)
        }
        _ => (String::new(), None, false),
    }
}
/// The grid's stacks for one tab, search text and language.
struct Shown {
    tab: usize,
    search: String,
    language_generation: u64,
    items: Arc<[ItemStack]>,
}
/// The stacks an item tab's display slots show, from `Creative::display_window`.
pub struct DisplayWindow {
    items: Arc<[ItemStack]>,
    first: usize,
}

impl DisplayWindow {
    /// The stack in a display slot, if that slot exists and holds a nonempty stack.
    pub fn stack(&self, slot: usize) -> Option<&ItemStack> {
        let cell = Creative::display_cell(slot)?;
        self.items.get(self.first + cell).filter(|s| s.count > 0)
    }

    /// Whether this snapshot has items beyond the visible grid.
    pub fn can_scroll(&self) -> bool {
        Creative::has_hidden_items(self.items.len())
    }
}

pub use sunrise_items::creative_catalog::{item_group_key, mod_item_group_key};

impl Creative {
    pub const WIDTH: f32 = 195.0;
    pub const HEIGHT: f32 = 136.0;
    /// How far a tab row reaches outside the window, above or below it.
    pub const TAB_ROW_DEPTH: f32 = 28.0;
    pub const TRASH_POSITION: (f32, f32) = (173.0, 112.0);

    /// Visible player slots in the creative inventory tab (game slot ids).
    pub fn inventory_slots() -> impl Iterator<Item = usize> {
        (0..36).chain(500..505)
    }

    /// CreativeModeInventoryScreen.selectTab positions its InventoryMenu wrappers.
    pub fn inventory_slot_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            0..9 => Some((9.0 + slot as f32 * 18.0, 112.0)),
            9..36 => Some((
                9.0 + ((slot - 9) % 9) as f32 * 18.0,
                54.0 + ((slot - 9) / 9) as f32 * 18.0,
            )),
            500..504 => {
                let position = slot - 500;
                Some((
                    54.0 + (position / 2) as f32 * 54.0,
                    6.0 + (position % 2) as f32 * 27.0,
                ))
            }
            504 => Some((35.0, 20.0)),
            _ => None,
        }
    }
    /// Game slot id of an item tab's first display slot (ItemPickerMenu's
    /// `CustomCreativeSlot`s, 9 by 5); its hotbar row keeps ids 0..9.
    pub const DISPLAY: usize = 900;
    const DISPLAY_ROWS: usize = 5;
    const DISPLAY_COLUMNS: usize = 9;
    const DISPLAY_CELLS: usize = Self::DISPLAY_ROWS * Self::DISPLAY_COLUMNS;

    /// Visible slots of an item tab (game slot ids).
    pub fn tab_slots() -> impl Iterator<Item = usize> {
        Self::display_slots().map(|(slot, _)| slot).chain(0..9)
    }

    /// An item tab's display slots, left to right then top to bottom: each one's game slot id
    /// and position (ItemPickerMenu's `CustomCreativeSlot(CONTAINER, i * 9 + j, 9 + j * 18,
    /// 18 + i * 18)`).
    pub fn display_slots() -> impl Iterator<Item = (usize, (f32, f32))> {
        (0..Self::DISPLAY_CELLS).map(|cell| (Self::DISPLAY + cell, Self::display_position(cell)))
    }

    fn display_position(cell: usize) -> (f32, f32) {
        (
            9.0 + (cell % Self::DISPLAY_COLUMNS) as f32 * 18.0,
            18.0 + (cell / Self::DISPLAY_COLUMNS) as f32 * 18.0,
        )
    }

    /// ItemPickerMenu's slot positions.
    pub fn tab_slot_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            0..9 => Some((9.0 + slot as f32 * 18.0, 112.0)),
            _ => Self::display_cell(slot).map(Self::display_position),
        }
    }

    /// The grid cell of display slot `slot`, None for any other slot.
    fn display_cell(slot: usize) -> Option<usize> {
        slot.checked_sub(Self::DISPLAY)
            .filter(|&cell| cell < Self::DISPLAY_CELLS)
    }
    /// The scrollbar track (insideScrollbar), relative to the screen origin.
    pub const SCROLL_X: f32 = 175.0;
    pub const SCROLL_Y: f32 = 18.0;
    pub const SCROLL_WIDTH: f32 = 14.0;
    pub const SCROLL_HEIGHT: f32 = 112.0;
    /// Load the prepared recipes and the UI-only search metadata.
    pub fn load(root: &Path) -> Result<Self> {
        Self::from_json(&serde_json::from_slice(&fs::read(
            root.join("creative-inventory.json"),
        )?)?)
    }

    pub fn from_json(raw: &Value) -> Result<Self> {
        let catalog = Arc::new(Catalog::parse(raw)?);
        let mut headers = Vec::new();
        let mut metadata = HashMap::new();
        let mut ungrouped = Vec::new();
        for (tab, recipe) in raw["tabs"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(catalog.tabs())
        {
            if recipe.kind == Kind::Search {
                for row in tab["search_metadata"].as_array().into_iter().flatten() {
                    let stack = sunrise_items::creative_catalog::parse_search_metadata_stack(row)?;
                    let strings = |key: &str| {
                        row[key]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(java_lower)
                    };
                    let (namespace, path) = split_id(&java_lower(&stack.item));
                    metadata.insert(
                        (stack.item.clone(), component_snapshot(&stack)),
                        SearchEntry {
                            namespace,
                            path,
                            lines: strings("lines").collect(),
                            live_tooltip: false,
                            tags: strings("tags").map(|tag| split_id(&tag)).collect(),
                        },
                    );
                    if row["ungrouped"] == true {
                        ungrouped.push(stack);
                    }
                }
            }
            let icon = tab["icon"]
                .as_str()
                .and_then(|s| crate::item_data::parse_stack(s).ok().flatten());
            headers.push(Tab {
                id: recipe.id.clone(),
                title: recipe.title.clone(),
                icon: icon.map_or_else(|| "minecraft:stone".to_owned(), |s| s.item.to_string()),
                items: Vec::new(),
                top: tab["row"] == "TOP",
                column: tab["column"].as_u64().unwrap_or(0) as u32,
                search: recipe.kind == Kind::Search,
                inventory: recipe.kind == Kind::Inventory,
                hotbar: recipe.kind == Kind::Hotbar,
                page: tab["page"].as_u64().unwrap_or(0) as u32,
                name: None,
            });
        }
        let mut creative = Self {
            tabs: Vec::new(),
            selected_id: headers
                .first()
                .map_or_else(String::new, |tab| tab.id.clone()),
            page: 0,
            scroll: 0.0,
            scrolling: false,
            search: {
                let mut search = EditBox::new(0.0, 0.0, 80.0, 9.0);
                search.bordered = false;
                search.max_length = 50;
                search.text_color = u32::MAX;
                search
            },
            search_items: Vec::new(),
            search_names: Vec::new(),
            search_generation: 0,
            search_index: Vec::new(),
            holders: Default::default(),
            items: Default::default(),
            catalog,
            headers,
            metadata,
            ungrouped,
            item_tags: None,
            contents: None,
            shown: Mutex::new(None),
            lines_cache: Mutex::new(None),
            pack: Vec::new(),
        };
        creative.project();
        Ok(creative)
    }

    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    /// Project the complete canonical lists onto the UI, retaining client-only presentation.
    fn project(&mut self) {
        let mut search = Vec::new();
        let mut tabs = Vec::new();
        for (i, recipe) in self.catalog.tabs().iter().enumerate() {
            let mut tab = self.headers[i].header();
            if let Some(contents) = &self.contents {
                tab.items.clone_from(&contents[i].display);
            }
            if tab.search {
                search = self.with_ungrouped(&tab.items);
            }
            self.apply_pack(&mut tab);
            if self.contents.is_none() || recipe.kind != Kind::Category || !tab.items.is_empty() {
                tabs.push(tab);
            }
        }
        self.tabs = tabs;
        self.search_items = search;
        self.search_names.clear();
        self.search_generation += 1;
        self.index_search();
    }

    /// Vanilla's creative order of `contents`, ungrouped items last: a stack's own place in the
    /// search tab, else its item's first. The built-in sorter ranks by it, and the platform reads it.
    pub fn stock_order(&self, contents: &[Arc<TabContents>]) -> CreativeOrder {
        let stacks = self.with_ungrouped(self.catalog.search_display(contents));
        CreativeOrder::new(stacks.iter().map(|s| (s.item.as_ref(), s.merge_kind())))
    }

    fn with_ungrouped(&self, search: &[ItemStack]) -> Vec<ItemStack> {
        let mut order = search.to_vec();
        append_unique(&mut order, &self.ungrouped);
        order
    }

    pub fn built_contents(&self) -> Option<&[Arc<TabContents>]> {
        self.contents.as_deref()
    }

    pub fn has_contents(&self) -> bool {
        self.contents.is_some()
    }

    /// Drop the previous world's projection when its holder provider changes.
    pub fn reset_contents(&mut self) {
        self.contents = None;
        self.holders = Arc::default();
        self.items = Arc::default();
        self.project();
    }

    fn index_search(&mut self) {
        *self
            .lines_cache
            .get_mut()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.search_index = self
            .search_items
            .iter()
            .map(|stack| {
                let key = (stack.item.clone(), component_snapshot(stack));
                let mut entry = self.metadata.get(&key).cloned().unwrap_or_else(|| {
                    let (namespace, path) = split_id(&java_lower(&stack.item));
                    SearchEntry {
                        namespace,
                        path,
                        ..Default::default()
                    }
                });
                entry.live_tooltip = matches!(
                    stack.item.as_ref(),
                    "minecraft:enchanted_book"
                        | "minecraft:painting"
                        | "minecraft:goat_horn"
                        | "minecraft:potion"
                        | "minecraft:splash_potion"
                        | "minecraft:lingering_potion"
                        | "minecraft:tipped_arrow"
                ) || !stack.stored_enchantments().is_empty()
                    || !stack.enchantments().is_empty()
                    || stack.data().is_some_and(|data| {
                        data.ominous_banner || data.other("minecraft:instrument").is_some()
                    });
                if let Some(tags) = &self.item_tags {
                    entry.tags = tags.get(stack.item.as_ref()).cloned().unwrap_or_default();
                }
                entry
            })
            .collect();
        self.forget_items();
    }

    /// Tag reload changes the search UI, not the cached catalog membership.
    pub fn set_item_tags(&mut self, tags: Option<&HashMap<String, Vec<String>>>) {
        self.item_tags = tags.map(|tags| {
            let mut by_item = HashMap::<String, Vec<_>>::new();
            let mut tags: Vec<_> = tags.iter().collect();
            tags.sort_unstable_by_key(|(tag, _)| *tag);
            for (tag, items) in tags {
                let tag = split_id(&java_lower(tag));
                for item in items {
                    by_item.entry(item.clone()).or_default().push(tag.clone());
                }
            }
            by_item
        });
        self.index_search();
    }

    /// Apply Polytone's stack, icon and name overlays without rebuilding the catalog.
    pub fn set_pack_modifiers(&mut self, pack: Vec<crate::polytone_creative_tabs::TabModifier>) {
        self.pack = pack;
        self.project();
        self.select_default_if_missing();
    }
    /// A tab after the modifiers aimed at it: its items, icon and name.
    fn apply_pack(&self, tab: &mut Tab) {
        let aimed = self
            .pack
            .iter()
            .filter(|m| m.targets.matches(&tab.id, |_| false));
        for m in aimed {
            let in_tag = |item: &str, tag: &str| {
                let (namespace, path) = split_id(tag);
                let in_tag = |tags: &[(String, String)]| {
                    tags.iter().any(|t| t.0 == namespace && t.1 == path)
                };
                match &self.item_tags {
                    Some(tags) => tags.get(item).is_some_and(|tags| in_tag(tags)),
                    None => self
                        .metadata
                        .iter()
                        .any(|((id, _), entry)| id.as_ref() == item && in_tag(&entry.tags)),
                }
            };
            let mut ids: Vec<String> = tab.items.iter().map(|s| s.item.to_string()).collect();
            m.apply(&mut ids, &in_tag);
            let mut old = std::mem::take(&mut tab.items);
            tab.items = ids
                .into_iter()
                .map(|id| match old.iter().position(|s| *s.item == *id) {
                    Some(i) => old.remove(i),
                    None => ItemStack::new(&id, 1),
                })
                .collect();
            if let Some(icon) = &m.icon {
                tab.icon = icon.clone();
            }
            if let Some(name) = &m.name {
                tab.name = Some(component_text(name));
            }
        }
    }
    /// Publish a complete build. Keep the selected tab's first row if the tab still exists;
    /// otherwise select vanilla's default tab (`refreshCurrentTabContents`).
    pub fn publish(
        &mut self,
        contents: Arc<[Arc<TabContents>]>,
        holders: Arc<sunrise_items::creative_catalog::Holders>,
        items: Arc<Items>,
        names: &Names,
    ) {
        if self
            .contents
            .as_ref()
            .is_some_and(|prior| Arc::ptr_eq(prior, &contents))
        {
            return;
        }
        let row = self.first_row(names);
        self.contents = Some(contents);
        self.holders = holders;
        self.items = items;
        self.project();
        if !self.select_default_if_missing() {
            // ItemPickerMenu.getScrollForRowIndex.
            let hidden = Self::hidden_rows(self.rows(names));
            self.scroll = if hidden > 0 {
                (row as f32 / hidden as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }

    fn select_default_if_missing(&mut self) -> bool {
        if self.tabs.iter().any(|t| t.id == self.selected_id) {
            return false;
        }
        // CreativeModeTabs.getDefaultTab.
        let default = self
            .tabs
            .iter()
            .position(|t| t.id == "minecraft:building_blocks");
        self.select(default.unwrap_or(0));
        true
    }
    /// The tabs without their stacks: what the screen draws of them each frame.
    pub fn tab_headers(&self) -> Vec<Tab> {
        self.tabs.iter().map(Tab::header).collect()
    }
    pub fn tab(&self) -> &Tab {
        if self.tabs.is_empty() {
            // An empty export (`{"tabs":[]}`) or a rebuild that filtered
            // every category tab away leaves no tabs. `tabs.len() - 1`
            // would underflow; return a shared empty tab instead, so
            // `set_permissions`/`select`/`items` keep working.
            static EMPTY_TAB: OnceLock<Tab> = OnceLock::new();
            return EMPTY_TAB.get_or_init(|| Tab {
                id: String::new(),
                title: String::new(),
                icon: "minecraft:stone".to_owned(),
                items: Vec::new(),
                top: true,
                column: 0,
                search: false,
                inventory: false,
                hotbar: false,
                page: 0,
                name: None,
            });
        }
        &self.tabs[self.selected_index()]
    }
    /// The selected tab's position in the current projection, for input and rendering.
    pub fn selected_index(&self) -> usize {
        self.tabs
            .iter()
            .position(|tab| tab.id == self.selected_id)
            .unwrap_or(0)
    }
    /// The search tab's index (`CreativeModeTabs.searchTab`), the tab the chat
    /// key opens while another one is shown.
    pub fn search_tab(&self) -> Option<usize> {
        self.tabs.iter().position(|t| t.search)
    }
    /// selectTab: a different tab empties the search box; the scrollbar goes to the top.
    /// A tab on another page shows that page.
    pub fn select(&mut self, tab: usize) {
        let Some(t) = self.tabs.get(tab) else {
            return;
        };
        if t.id != self.selected_id {
            self.search.set_value("");
            self.selected_id.clone_from(&t.id);
        }
        if !Self::on_every_page(t) {
            self.page = t.page;
        }
        self.scroll = 0.0;
        // The search box is shown, focused and impossible to unfocus only on
        // its own tab; another tab hides it and takes the focus away.
        let search = self.tab().search;
        if search {
            self.search.reset_blink();
        }
        self.search.focused = search;
    }
    /// How many pages of tabs there are (1 without mods' tabs).
    pub fn pages(&self) -> u32 {
        self.tabs.iter().map(|t| t.page + 1).max().unwrap_or(1)
    }
    /// The tabs every page shows (NeoForge's): the right-hand ones, saved
    /// hotbars, search, operator blocks and the inventory.
    fn on_every_page(t: &Tab) -> bool {
        t.column >= 5
    }
    /// Whether tab `t` shows on the page shown.
    pub fn on_page(&self, t: &Tab) -> bool {
        t.page == self.page || Self::on_every_page(t)
    }
    /// Shows page `page` (NeoForge's page buttons) and selects its first tab,
    /// unless the selected tab is one every page shows.
    pub fn set_page(&mut self, page: u32) {
        let page = page.min(self.pages() - 1);
        let kept = self
            .tabs
            .get(self.selected_index())
            .is_some_and(Self::on_every_page);
        if !kept
            && let Some(first) = self
                .tabs
                .iter()
                .position(|t| t.page == page && !Self::on_every_page(t))
        {
            self.select(first);
        }
        self.page = page;
    }
    /// The search tab's stacks.
    pub fn search_items(&self) -> &[ItemStack] {
        &self.search_items
    }
    /// Which projection of the search stacks this is: it changes whenever they are made again.
    pub fn search_generation(&self) -> u64 {
        self.search_generation
    }
    /// The search stacks' resolved name overrides, by index into
    /// [`Self::search_items`]: the search matches them as the stacks' name lines. Nothing but
    /// `None`s keeps the prepared index.
    pub fn set_search_names(&mut self, names: Vec<Option<crate::text::Text>>) {
        self.search_names = if names.iter().any(Option::is_some) {
            names
        } else {
            Vec::new()
        };
        *self.lines_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.shown.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    /// Opening the screen: vanilla rebuilds CreativeModeInventoryScreen, whose
    /// init() makes a fresh search box and a zeroed scrollbar; only the tab
    /// persists (its static selectedTab). selectTab on the reopened screen
    /// takes the oldTab != tab branch, so it empties and refocuses the box.
    pub fn reopen(&mut self) {
        self.search.set_value("");
        self.release();
        self.scroll = 0.0;
        self.search.focused = self.tab().search;
        if self.search.focused {
            self.search.reset_blink();
        }
    }
    /// `CreativeModeInventoryScreen.refreshSearchResults`: indices into `search_items`,
    /// matching tag text after `#` or tooltip text and ids in the name search.
    pub fn search_results(&self, term: &str, names: &Names) -> Vec<usize> {
        let english = lang::code() == lang::DEFAULT
            && names.display_get("itemGroup.search").as_deref() == names.get("itemGroup.search");
        let local = if term.is_empty() || term.starts_with('#') {
            Arc::from([])
        } else if english
            && self.search_names.is_empty()
            && self
                .search_index
                .iter()
                .all(|e| !e.live_tooltip && !e.lines.is_empty())
        {
            // The exported English lines are the whole index.
            Arc::from([])
        } else {
            let generation = lang::generation();
            let mut cache = self.lines_cache.lock().unwrap_or_else(|e| e.into_inner());
            match cache.as_ref().filter(|(built, _)| *built == generation) {
                Some((_, lines)) => lines.clone(),
                None => {
                    let lines = self.localized_lines(names);
                    *cache = Some((generation, lines.clone()));
                    lines
                }
            }
        };
        self.search_results_in(term, names, &local, english)
    }
    /// `SessionSearchTrees.getTooltipLines`: holder-dependent entries use only
    /// the active definitions and the selected language, never their prepared names.
    fn localized_lines(&self, names: &Names) -> Arc<[Vec<String>]> {
        self.search_items
            .iter()
            .enumerate()
            .map(|(i, stack)| {
                let mut lines = item_tooltip::lines(
                    stack,
                    self.items.get(&stack.item),
                    &item_tooltip::NoAttributeListener,
                    item_tooltip::TooltipHolders::Live(&self.holders),
                    true,
                    &|key| names.display_get(key),
                    // The search cache has no attribute catalog.
                    &|_| None,
                );
                if let Some(first) = lines.first_mut() {
                    let name = match self.search_names.get(i).and_then(Option::as_ref) {
                        Some(name) => crate::scoreboard::comp_text(name.component_view().as_ref()),
                        None => crate::stations::hover_name_text(names, stack),
                    };
                    *first = crate::clipboard::strip_formatting(&name.plain());
                }
                lines
                    .iter()
                    .map(|line| java_trim(line))
                    .filter(|line| !line.is_empty())
                    .map(java_lower)
                    .collect()
            })
            .collect()
    }
    fn search_results_in(
        &self,
        term: &str,
        names: &Names,
        local: &[Vec<String>],
        english: bool,
    ) -> Vec<usize> {
        let all = 0..self.search_index.len();
        if term.is_empty() {
            return all.collect();
        }
        if let Some(tag) = term.strip_prefix('#') {
            // IdSearchTree over the stacks' tags.
            let text = java_lower(tag);
            return all
                .filter(|&i| {
                    let tags = &self.search_index[i].tags;
                    match text.split_once(':') {
                        None => any(tags.iter().map(|t| t.1.as_str()), &text),
                        Some((ns, path)) => {
                            let (ns, path) = (java_trim(ns), java_trim(path));
                            any(tags.iter().map(|t| t.0.as_str()), ns)
                                && any(tags.iter().map(|t| t.1.as_str()), path)
                        }
                    }
                })
                .collect();
        }
        // FullTextSearchTree: tooltip lines in the selected language, and
        // the item id after a colon. Only non-holder stacks may use the
        // prepared English tooltip or the plain item-name fallback.
        let text = java_lower(term);
        all.filter(|&i| {
            let e = &self.search_index[i];
            let local = local.get(i).map_or(&[][..], Vec::as_slice);
            let named = self.search_names.get(i).is_some_and(Option::is_some);
            let hit = |term: &str| {
                if !named && !e.live_tooltip && english && !e.lines.is_empty() {
                    any(e.lines.iter().map(String::as_str), term)
                } else if !named && local.is_empty() && !e.live_tooltip {
                    java_lower(&names.item(&self.search_items[i].item)).contains(term)
                } else {
                    any(local.iter().map(String::as_str), term)
                }
            };
            match text.split_once(':') {
                None => hit(&text),
                Some((ns, path)) => {
                    let (ns, path) = (java_trim(ns), java_trim(path));
                    e.namespace.contains(ns) && (e.path.contains(path) || hit(path))
                }
            }
        })
        .collect()
    }
    /// Items shown by the selected tab (the search tab filters by `search`),
    /// worked out again only when the tab, the search or the language changed.
    pub fn items(&self, names: &Names) -> Arc<[ItemStack]> {
        if self.tabs.is_empty() {
            return Arc::from([]);
        }
        let tab = self.selected_index();
        let search = if self.tab().search {
            self.search.value.as_str()
        } else {
            ""
        };
        let language_generation = lang::generation();
        let mut shown = self.shown.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = shown.as_ref()
            && s.tab == tab
            && s.search == search
            && s.language_generation == language_generation
        {
            return s.items.clone();
        }
        let items: Arc<[ItemStack]> = if self.tab().search {
            self.search_results(search, names)
                .into_iter()
                .map(|i| self.search_items[i].clone())
                .collect()
        } else {
            self.tab().items.as_slice().into()
        };
        *shown = Some(Shown {
            tab,
            search: search.to_owned(),
            language_generation,
            items: items.clone(),
        });
        items
    }
    /// What the display slots show (`ItemPickerMenu.scrollTo`): the tab's list from its first
    /// visible row, resolved once for any number of slots.
    pub fn display_window(&self, names: &Names) -> DisplayWindow {
        let items = self.items(names);
        let hidden = Self::hidden_rows(Self::rows_for(items.len()));
        let first = self.first_row_of(hidden) * Self::DISPLAY_COLUMNS;
        DisplayWindow { items, first }
    }
    /// `CreativeModeInventoryScreen.selectTab` for the HOTBAR tab: the nine
    /// saved rows of `HotbarManager` as the tab's items, an empty stack (count
    /// 0) for an empty slot.
    pub fn set_saved_hotbars(&mut self, items: Vec<ItemStack>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.hotbar) {
            tab.items = items;
        }
        self.forget_items();
    }
    /// Drop the shown items, so the next `items` works them out again: a
    /// reload can rename items without changing the Search label.
    pub fn forget_items(&self) {
        *self.shown.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    pub fn rows(&self, names: &Names) -> usize {
        Self::rows_for(self.items(names).len())
    }
    fn rows_for(item_count: usize) -> usize {
        item_count.div_ceil(Self::DISPLAY_COLUMNS)
    }
    /// ItemPickerMenu.calculateRowCount: rows beyond the display grid.
    fn hidden_rows(row_count: usize) -> i32 {
        row_count as i32 - Self::DISPLAY_ROWS as i32
    }
    /// First visible grid row for the scroll position (getRowIndexForScroll).
    pub fn first_row(&self, names: &Names) -> usize {
        self.first_row_of(Self::hidden_rows(self.rows(names)))
    }
    /// `first_row` with `hidden` rows beyond the display grid.
    fn first_row_of(&self, hidden: i32) -> usize {
        ((self.scroll * hidden as f32 + 0.5) as i32).max(0) as usize
    }
    /// CreativeModeInventoryScreen.canScroll: more items than fit, on a tab with a scrollbar.
    pub fn can_scroll(&self, names: &Names) -> bool {
        !self.tab().inventory && Self::has_hidden_items(self.items(names).len())
    }
    fn has_hidden_items(item_count: usize) -> bool {
        item_count > Self::DISPLAY_CELLS
    }
    /// insideScrollbar, for a point relative to the screen origin.
    pub fn inside_scrollbar(&self, x: f32, y: f32) -> bool {
        x >= Self::SCROLL_X
            && y >= Self::SCROLL_Y
            && x < Self::SCROLL_X + Self::SCROLL_WIDTH
            && y < Self::SCROLL_Y + Self::SCROLL_HEIGHT
            && !self.tab().inventory
    }
    /// mouseClicked on the scrollbar: it is held only if the tab can scroll
    /// (the thumb doesn't move until the mouse does).
    pub fn press_scrollbar(&mut self, x: f32, y: f32, names: &Names) -> bool {
        if !self.inside_scrollbar(x, y) {
            return false;
        }
        self.scrolling = self.can_scroll(names);
        true
    }
    /// mouseDragged while the scrollbar is held, at a cursor height relative to
    /// the screen origin, wherever the cursor is (even off the track or the window).
    pub fn drag_scrollbar(&mut self, y: f32) -> bool {
        if !self.scrolling {
            return false;
        }
        let (top, bottom) = (Self::SCROLL_Y, Self::SCROLL_Y + Self::SCROLL_HEIGHT);
        self.scroll = ((y - top - 7.5) / (bottom - top - 15.0)).clamp(0.0, 1.0);
        true
    }
    /// mouseReleased.
    pub fn release(&mut self) -> bool {
        std::mem::take(&mut self.scrolling)
    }
    /// mouseScrolled: ItemPickerMenu.subtractInputFromScroll.
    pub fn wheel(&mut self, amount: f64, names: &Names) {
        if !self.can_scroll(names) {
            return;
        }
        let rows = Self::hidden_rows(self.rows(names)) as f64;
        self.scroll = (self.scroll - (amount / rows) as f32).clamp(0.0, 1.0);
    }
    /// The scroller sprite's offset from the top of the track.
    pub fn scroller_offset(&self) -> f32 {
        ((Self::SCROLL_HEIGHT - 17.0) * self.scroll) as i32 as f32
    }
    /// Visible categories whose canonical search list contains `stack` by item
    /// and exact components, in registry order. UI-only additions never decide membership.
    pub fn tabs_containing(&self, stack: &ItemStack) -> Vec<usize> {
        let Some(contents) = &self.contents else {
            return Vec::new();
        };
        self.catalog
            .tabs()
            .iter()
            .zip(contents.iter())
            .filter(|(recipe, _)| recipe.kind == Kind::Category)
            .filter(|(_, contents)| {
                contents
                    .search
                    .iter()
                    .any(|shown| same_components(shown, stack))
            })
            .filter_map(|(recipe, _)| self.tabs.iter().position(|tab| tab.id == recipe.id))
            .collect()
    }
    /// The `#namespace:path` lines `getTooltipFromContainerItem` adds for a
    /// search tab stack while the search is a `#` query: the item tags
    /// `updateVisibleTags` keeps for the text after `#` (the path contains it;
    /// with a colon, the namespace and the path each contain their part) that
    /// the stack has.
    pub fn search_tag_lines(&self, stack: &ItemStack) -> Vec<String> {
        let Some(text) = self.search.value.strip_prefix('#') else {
            return Vec::new();
        };
        let Some(entry) = self
            .search_items
            .iter()
            .position(|s| same_components(s, stack))
            .and_then(|i| self.search_index.get(i))
        else {
            return Vec::new();
        };
        let visible = |(ns, path): &(String, String)| match text.split_once(':') {
            None => path.contains(text),
            Some((n, p)) => ns.contains(java_trim(n)) && path.contains(java_trim(p)),
        };
        entry
            .tags
            .iter()
            .filter(|t| visible(t))
            .map(|(ns, path)| format!("#{ns}:{path}"))
            .collect()
    }
    /// Tab button position relative to the screen origin (CreativeModeInventoryScreen.getTabX/Y).
    pub fn tab_position(tab: &Tab) -> (f32, f32) {
        let mut x = tab.column as f32 * 27.0;
        if tab.column >= 5 {
            // Right-aligned tabs (search, operator, inventory).
            x = Self::WIDTH - 27.0 * (7 - tab.column) as f32 + 1.0;
        }
        let y = if tab.top {
            -Self::TAB_ROW_DEPTH
        } else {
            Self::HEIGHT - 4.0
        };
        (x, y)
    }
    /// `CreativeModeInventoryScreen.checkTabHovering`: the tab whose icon
    /// rectangle, 3 pixels in from its 26x32 slot (`isHovering(x + 3, y + 3,
    /// 21, 27)`), holds the cursor, among the tabs of the page shown. The
    /// screen draws its display name.
    pub fn hovered_tab(&self, origin: (f32, f32), cursor: (f32, f32)) -> Option<usize> {
        self.tabs.iter().position(|tab| {
            if !self.on_page(tab) {
                return false;
            }
            let (x, y) = Self::tab_position(tab);
            let (x, y) = (origin.0 + x + 3.0, origin.1 + y + 3.0);
            cursor.0 >= x && cursor.0 < x + 21.0 && cursor.1 >= y && cursor.1 < y + 27.0
        })
    }
    /// `CreativeModeInventoryScreen.extractTabButton`'s cursor: an unselected
    /// tab asks for the pointing hand when the whole-pixel mouse, relative to
    /// the screen origin, is strictly inside its 26x32 button.
    pub fn tab_wants_hand(tab: &Tab, selected: bool, mouse: (f32, f32)) -> bool {
        let (x, y) = Self::tab_position(tab);
        let (mx, my) = (mouse.0.floor(), mouse.1.floor());
        !selected && mx > x && my > y && mx < x + 26.0 && my < y + 32.0
    }
}
/// Whether a suffix array of `strings` has a suffix starting with `text`:
/// some string contains it (none for no strings, even when `text` is empty).
fn any<'a>(strings: impl IntoIterator<Item = &'a str>, text: &str) -> bool {
    strings.into_iter().any(|s| s.contains(text))
}
/// `ItemStack.isSameItemSameComponents`, the count left out, as the
/// tabs' `ItemStackLinkedSet` compares: no components and empty ones match.
fn same_components(a: &ItemStack, b: &ItemStack) -> bool {
    let none = crate::item_data::StackData::default();
    a.item == b.item
        && a.damage == b.damage
        && a.data().unwrap_or(&none) == b.data().unwrap_or(&none)
}
/// `CreativeModeInventoryScreen.getTooltipFromContainerItem`: the plain
/// tooltip only for a category tab's own grid slots (`CustomCreativeSlot`);
/// everywhere else in the screen the tab names (and, on the search tab's
/// grid, the tag lines) follow the item's name.
pub fn tooltip_lists_tabs(category_tab: bool, grid_slot: bool) -> bool {
    !(category_tab && grid_slot)
}
/// String.toLowerCase(Locale.ROOT).
fn java_lower(s: &str) -> String {
    s.to_lowercase()
}
/// String.trim: strips code points up to U+0020.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}
/// Identifier.parse's namespace and path (default namespace `minecraft`).
fn split_id(id: &str) -> (String, String) {
    match id.split_once(':') {
        Some((ns, path)) => (ns.to_owned(), path.to_owned()),
        None => ("minecraft".to_owned(), id.to_owned()),
    }
}
/// The slot under the cursor, for the search tab's keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hover {
    Nothing,
    /// A display slot (`isCreativeSlot`), showing an item or past the end.
    Display {
        filled: bool,
    },
    /// Any other slot: the hotbar row, or the inventory tab's slots.
    Slot,
}

/// `CreativeModeInventoryScreen.keyPressed` on the search tab: whether a
/// key is a hotbar swap on the hovered slot instead of text for the search
/// box. `numeric` is `getNumericKeyValue().isPresent()` (the digit and keypad
/// keys); `hotbar_key` is a `keyHotbarSlots` key. Over an empty display slot
/// the key types (`doQuickSwap` is false), and `checkHotbarKeyPressed` needs
/// a hovered slot and an empty cursor.
pub fn search_key_swaps(numeric: bool, hotbar_key: bool, hover: Hover, carrying: bool) -> bool {
    let quick_swap = hover != Hover::Display { filled: false };
    quick_swap && numeric && hotbar_key && !carrying && hover != Hover::Nothing
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_items::creative_catalog::Parameters;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    /// One click on a display slot showing one stone (max 64; pearls 16).
    fn display(
        carried: Option<ItemStack>,
        button: i32,
        input: crate::menu::Input,
    ) -> (Option<ItemStack>, DisplayClick) {
        let mut carried = carried;
        let max = |s: &ItemStack| if s.item.ends_with("pearl") { 16 } else { 64 };
        let effect = click_display(&mut carried, Some(&stack("stone", 1)), max, button, input);
        (carried, effect)
    }

    fn row(stack: impl AsRef<str>) -> Value {
        serde_json::json!({"kind":"stack", "snbt":stack.as_ref(), "visibility":"PARENT_AND_SEARCH_TABS"})
    }

    fn search_only(stack: impl AsRef<str>) -> Value {
        let mut row = row(stack);
        row["visibility"] = serde_json::json!("SEARCH_TAB_ONLY");
        row
    }

    fn fixture(tabs: Value) -> Creative {
        let mut c = Creative::from_json(&serde_json::json!({
            "schema":4, "item_features":{}, "potion_features":{}, "tabs":tabs,
        }))
        .unwrap();
        c.contents = Some(c.catalog.build(
            Parameters {
                operator: false,
                features: &[],
                holders: &Default::default(),
                items: &crate::items::Items::empty(),
            },
            None,
            |_, _, _| {},
        ));
        c.project();
        c
    }

    /// Tabs as the export lists them: two block tabs sharing the oak log,
    /// ingredients with the top-level book only, the search tab with every
    /// stack (a level 1 book only there) and its tags, the inventory.
    fn tooltip_tabs() -> Creative {
        let book = |level: u8| {
            format!(
                r#"{{components:{{"minecraft:stored_enchantments":{{"minecraft:sharpness":{level}}}}},count:1,id:"minecraft:enchanted_book"}}"#
            )
        };
        let log = r#"{count:1,id:"minecraft:oak_log"}"#;
        let dirt = r#"{count:1,id:"minecraft:dirt"}"#;
        let tab = |id: &str, kind: &str, build: Vec<Value>| serde_json::json!({"id":id,"type":kind,"title":id,"icon":log,"build":build});
        let mut search = tab("minecraft:search", "SEARCH", vec![]);
        search["search_metadata"] = serde_json::json!([
            {"stack":log,"tags":["minecraft:logs","minecraft:oak_logs","minecraft:logs_that_burn"]},
            {"stack":dirt,"tags":["minecraft:dirt"]},
        ]);
        fixture(serde_json::json!([
            tab("minecraft:building_blocks", "CATEGORY", vec![row(log)]),
            tab(
                "minecraft:natural_blocks",
                "CATEGORY",
                vec![row(log), row(dirt)]
            ),
            tab("minecraft:hotbar", "HOTBAR", vec![]),
            search,
            tab(
                "minecraft:ingredients",
                "CATEGORY",
                vec![search_only(book(1)), row(book(5)), {
                    let mut row = row(r#"{id:"minecraft:book",count:1}"#);
                    row["visibility"] = "PARENT_TAB_ONLY".into();
                    row
                }]
            ),
            tab("minecraft:inventory", "INVENTORY", vec![]),
        ]))
    }

    #[test]
    fn holder_projection_uses_registry_and_tag_order() {
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::{
            packets::configuration::{RegistryTags, TagEntry, UpdateTags},
            registry::{Builtin, Dynamic, Ids},
        };
        let builtin = Builtin {
            registries: HashMap::from([(
                "minecraft:potion".into(),
                Ids::from_names(vec!["minecraft:water".into(), "minecraft:awkward".into()]),
            )]),
        };
        let mut dynamic = Dynamic {
            registries: HashMap::from([
                (
                    "minecraft:enchantment".into(),
                    vec![
                        (
                            "test:z".into(),
                            Some(Tag::compound([("max_level", Tag::Int(3))])),
                        ),
                        (
                            "test:a".into(),
                            Some(Tag::compound([("max_level", Tag::Int(7))])),
                        ),
                    ],
                ),
                (
                    "minecraft:painting_variant".into(),
                    vec![(
                        "test:mural".into(),
                        Some(Tag::compound([
                            ("width", Tag::Int(3)),
                            ("height", Tag::Int(4)),
                        ])),
                    )],
                ),
                (
                    "minecraft:instrument".into(),
                    vec![("test:first".into(), None), ("test:second".into(), None)],
                ),
                (
                    "minecraft:banner_pattern".into(),
                    vec![("test:pattern".into(), None)],
                ),
            ]),
            ..Default::default()
        };
        dynamic.add_tags(&UpdateTags {
            registries: vec![
                RegistryTags {
                    registry: "minecraft:painting_variant".into(),
                    tags: vec![TagEntry {
                        name: "minecraft:placeable".into(),
                        ids: vec![0],
                    }],
                },
                RegistryTags {
                    registry: "minecraft:instrument".into(),
                    tags: vec![TagEntry {
                        name: "minecraft:goat_horns".into(),
                        ids: vec![1, 0],
                    }],
                },
            ],
        });
        let holders = live_holders(&builtin, &dynamic);
        assert_eq!(holders.potions, ["minecraft:water", "minecraft:awkward"]);
        let enchantments: Vec<_> = holders
            .enchantments
            .iter()
            .map(|e| (e.id.as_str(), e.max_level.get()))
            .collect();
        assert_eq!(enchantments, [("test:z", 3), ("test:a", 7)]);
        let mural = &holders.paintings[0];
        assert_eq!(
            (
                mural.id.as_str(),
                mural.width.get(),
                mural.height.get(),
                mural.placeable
            ),
            ("test:mural", 3, 4, true)
        );
        assert_eq!(holders.horns, ["test:second", "test:first"]);
        assert!(holders.banner_patterns.contains("test:pattern"));
    }

    #[test]
    fn search_indexes_the_replaced_enchantment_description_even_at_the_same_level() {
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::registry::{Builtin, Dynamic};
        let raw = serde_json::json!({
            "schema":4, "item_features":{}, "potion_features":{}, "tabs":[
                {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[
                    {"kind":"enchantments","all_levels":false,"visibility":"PARENT_AND_SEARCH_TABS"}
                ]},
                {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[], "search_metadata":[
                    {"stack":r#"{components:{"minecraft:stored_enchantments":{"minecraft:sharpness":5}},count:1,id:"minecraft:enchanted_book"}"#, "lines":["Enchanted Book","Sharpness V"]}
                ]}
            ]
        });
        let mut c = Creative::from_json(&raw).unwrap();
        let items = Arc::new(Items::empty());
        let names = Names::default();
        let mut rebuild = |description: &str| {
            let dynamic = Dynamic {
                registries: HashMap::from([(
                    "minecraft:enchantment".into(),
                    vec![(
                        "minecraft:sharpness".into(),
                        Some(Tag::compound([
                            ("max_level", Tag::Int(5)),
                            ("description", Tag::String(description.into())),
                        ])),
                    )],
                )]),
                ..Default::default()
            };
            let holders = live_holders(&Builtin::default(), &dynamic);
            c.reset_contents();
            let lists = c.catalog().build(
                Parameters {
                    features: &[],
                    operator: false,
                    holders: &holders,
                    items: &items,
                },
                None,
                |_, _, _| {},
            );
            c.publish(lists, Arc::new(holders), items.clone(), &names);
            assert_eq!(
                c.search_items()[0].stored_enchantments(),
                [(Arc::from("minecraft:sharpness"), 5)]
            );
            (
                c.search_results("sharpness", &names),
                c.search_results("honed edge", &names),
            )
        };
        assert_eq!(rebuild("Sharpness"), (vec![0], vec![]));
        assert_eq!(rebuild("Honed Edge"), (vec![], vec![0]));
    }

    #[test]
    fn search_matches_visible_lore_on_a_holder_backed_stack() {
        let visible = r#"{id:"minecraft:enchanted_book",count:1,components:{"minecraft:lore":["Ink under moonlight"]}}"#;
        let hidden = r#"{id:"minecraft:enchanted_book",count:1,components:{"minecraft:lore":["Ink under moonlight"],"minecraft:tooltip_display":{hidden_components:["minecraft:lore"]}}}"#;
        let creative = fixture(serde_json::json!([
            {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[row(visible), row(hidden)]},
            {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[]}
        ]));
        assert_eq!(creative.search_items().len(), 2);
        assert_eq!(
            creative.search_results("under moonlight", &Names::default()),
            [0]
        );
    }

    #[test]
    fn search_matches_a_written_book_s_author_only_while_its_content_is_visible() {
        let book = |hidden: &str| {
            format!(
                r#"{{id:"minecraft:written_book",count:1,components:{{"minecraft:enchantments":{{"minecraft:sharpness":1}},"minecraft:written_book_content":{{title:{{raw:"Notes"}},author:"Ada Lovelace",pages:[{{raw:"one"}}]}}{hidden}}}}}"#
            )
        };
        let visible = book("");
        let hidden = book(
            r#","minecraft:tooltip_display":{hidden_components:["minecraft:written_book_content"]}"#,
        );
        let creative = fixture(serde_json::json!([
            {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[row(&visible), row(&hidden)]},
            {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[]}
        ]));
        assert_eq!(creative.search_items().len(), 2);
        let language = lang::Language::of("en_us", [("book.byAuthor".into(), "by %1$s".into())]);
        lang::scoped(language, || {
            assert_eq!(
                creative.search_results("by ada lovelace", &Names::default()),
                [0]
            );
        });
    }

    #[test]
    fn search_matches_visible_attributes_alongside_live_painting_lines() {
        use sunrise_items::creative_catalog::{Holders, Painting};
        let stack = |hidden: &str| {
            format!(
                r#"{{id:"minecraft:painting",count:1,components:{{
                    "minecraft:painting/variant":"test:mural",
                    "minecraft:attribute_modifiers":[{{type:"minecraft:armor",id:"test:bonus",
                        amount:2.0d,operation:"add_value",slot:"head"}}],
                    "minecraft:tooltip_display":{{hidden_components:[{hidden}]}}
                }}}}"#
            )
        };
        let mut creative = fixture(serde_json::json!([
            {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[
                row(stack("")),
                row(stack(r#""minecraft:attribute_modifiers""#)),
                row(stack(r#""minecraft:painting/variant""#))
            ]},
            {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[]}
        ]));
        creative.holders = Arc::new(Holders {
            paintings: vec![Painting {
                id: "test:mural".into(),
                width: 3.try_into().unwrap(),
                height: 2.try_into().unwrap(),
                placeable: true,
                title: Some(serde_json::json!("Winter Orchard")),
                author: Some(serde_json::json!("Yuki Tanaka")),
            }],
            ..Default::default()
        });
        let map: HashMap<String, String> = HashMap::from([
            ("attribute.name.armor".into(), "Armor".into()),
            ("attribute.modifier.plus.0".into(), "+%s %s".into()),
            ("painting.dimensions".into(), "%s by %s".into()),
        ]);
        let names = Names::load(&sunrise_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/minecraft/lang/en_us.json".into(),
                sunrise_assets::resource_source::Resource::bytes(serde_json::to_vec(&map).unwrap()),
            )]
            .into(),
        ));
        let lang = lang::Language::of("en_us", map);
        let (local, _) = lang::scoped(lang, || creative.localized_lines(&names));
        let search = |term| creative.search_results_in(term, &names, &local, false);
        assert_eq!(search("+2 armor"), [0, 2]);
        assert_eq!(search("winter orchard"), [0, 1]);
        assert_eq!(search("yuki tanaka"), [0, 1]);
        assert_eq!(search("3 by 2"), [0, 1]);
    }

    #[test]
    fn search_uses_the_active_language_for_an_applied_enchantment() {
        use sunrise_items::creative_catalog::{Enchantment, Holders};
        let tool = r#"{id:"minecraft:wooden_pickaxe",count:1,components:{"minecraft:enchantments":{"test:veins":2},"minecraft:unbreakable":{}}}"#;
        let mut creative = Creative::from_json(&serde_json::json!({
            "schema":4, "item_features":{}, "potion_features":{}, "tabs":[
                {"id":"minecraft:ingredients", "title":"Ingredients", "type":"CATEGORY", "build":[row(tool)]},
                {"id":"minecraft:search", "title":"Search Items", "type":"SEARCH", "build":[], "search_metadata":[
                    {"stack":tool, "lines":["Wooden Pickaxe", "Stale Vein Finder II"]}
                ]}
            ]
        }))
        .unwrap();
        let holders = Holders {
            enchantments: vec![Enchantment {
                id: "test:veins".into(),
                max_level: 3.try_into().unwrap(),
                description: serde_json::json!({"translate":"test.enchantment.veins"}),
            }],
            ..Default::default()
        };
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        fs::write(
            root.join("items.json"),
            r#"{"minecraft:wooden_pickaxe":{"minecraft:attribute_modifiers":[{
                "type":"minecraft:attack_damage","id":"minecraft:base_attack_damage",
                "amount":3.0,"operation":"add_value","slot":"mainhand"
            }]}}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(root).unwrap();
        let items = Arc::new(
            Items::catalog(root, &sunrise_world::loot::PackOverlay::NONE, &defaults).unwrap(),
        );
        let lists = creative.catalog().build(
            Parameters {
                features: &[],
                operator: false,
                holders: &holders,
                items: &items,
            },
            None,
            |_, _, _| {},
        );
        let names = Names::default();
        creative.publish(lists, Arc::new(holders), items, &names);
        let language = lang::Language::of(
            "en_us",
            [
                ("test.enchantment.veins".into(), "Ore Sense".into()),
                ("enchantment.level.2".into(), "II".into()),
                (
                    "attribute.name.attack_damage".into(),
                    "Attack Damage".into(),
                ),
                ("attribute.modifier.plus.0".into(), "+%s %s".into()),
                ("item.unbreakable".into(), "Unbreakable".into()),
            ],
        );
        lang::scoped(language, || {
            assert_eq!(creative.search_results("ore sense ii", &names), [0]);
            assert_eq!(creative.search_results("+3 attack damage", &names), [0]);
            assert_eq!(creative.search_results("unbreakable", &names), [0]);
            assert!(
                creative
                    .search_results("stale vein finder", &names)
                    .is_empty()
            );
            assert!(creative.search_results("veins", &names).is_empty());
        });
    }

    #[test]
    fn tag_reload_updates_search_without_asking_for_new_tab_contents() {
        let mut c = tooltip_tabs();
        let names = Names::default();
        let previous = c.contents.clone().unwrap();
        assert_eq!(c.search_results("#oak_logs", &names), [0]);
        c.set_item_tags(Some(&HashMap::from([(
            "test:replacement".into(),
            vec!["minecraft:oak_log".into()],
        )])));
        assert_eq!(c.search_results("#replacement", &names), [0]);
        assert!(c.search_results("#oak_logs", &names).is_empty());
        assert!(Arc::ptr_eq(&previous, c.contents.as_ref().unwrap()));
    }

    #[test]
    fn tooltip_lists_the_tabs_that_contain_the_stack() {
        let c = tooltip_tabs();
        let ids = |s: &ItemStack| -> Vec<&str> {
            c.tabs_containing(s)
                .into_iter()
                .map(|i| c.tabs[i].id.as_str())
                .collect()
        };
        // In tab order, the search and inventory tabs left out; the count
        // doesn't matter.
        assert_eq!(
            ids(&stack("oak_log", 37)),
            ["minecraft:building_blocks", "minecraft:natural_blocks"]
        );
        assert_eq!(ids(&stack("dirt", 1)), ["minecraft:natural_blocks"]);
        // A stack no tab holds: other components, or no creative item.
        let mut damaged = stack("oak_log", 1);
        damaged.damage = 3;
        assert!(ids(&damaged).is_empty());
        assert!(ids(&stack("barrier", 1)).is_empty());
        // A level 1 book is a search-tab-only stack of the ingredients tab.
        let books: Vec<&ItemStack> = c
            .search_items()
            .iter()
            .filter(|s| &*s.item == "minecraft:enchanted_book")
            .collect();
        assert_eq!(ids(books[0]), ["minecraft:ingredients"]);
        assert_eq!(ids(books[1]), ["minecraft:ingredients"]);
        let ingredients = c
            .tabs
            .iter()
            .find(|tab| tab.id == "minecraft:ingredients")
            .unwrap();
        assert!(
            ingredients
                .items
                .iter()
                .any(|s| s.item.as_ref() == "minecraft:book")
        );
        assert!(
            ids(&stack("book", 1)).is_empty(),
            "the display-only stack is not a member"
        );
    }

    #[test]
    fn tooltip_tab_names_skip_only_a_category_tab_grid() {
        assert!(!tooltip_lists_tabs(true, true));
        // The hotbar row of a category tab, the search tab's grid and hotbar,
        // the inventory tab.
        assert!(tooltip_lists_tabs(true, false));
        assert!(tooltip_lists_tabs(false, true));
        assert!(tooltip_lists_tabs(false, false));
    }

    #[test]
    fn tag_search_lists_the_matching_tags_of_the_stack() {
        let mut c = tooltip_tabs();
        let log = stack("oak_log", 1);
        assert!(c.search_tag_lines(&log).is_empty());
        c.search.set_value("logs");
        assert!(c.search_tag_lines(&log).is_empty());
        c.search.set_value("#logs");
        assert_eq!(
            c.search_tag_lines(&log),
            [
                "#minecraft:logs",
                "#minecraft:oak_logs",
                "#minecraft:logs_that_burn"
            ]
        );
        c.search.set_value("#oak");
        assert_eq!(c.search_tag_lines(&log), ["#minecraft:oak_logs"]);
        c.search.set_value("#mine : burn");
        assert_eq!(c.search_tag_lines(&log), ["#minecraft:logs_that_burn"]);
        assert!(c.search_tag_lines(&stack("dirt", 1)).is_empty());
    }

    #[test]
    fn search_tab_number_keys_swap_over_a_slot_before_typing() {
        let filled = Hover::Display { filled: true };
        let empty = Hover::Display { filled: false };
        // Over an item, the hotbar row or another slot: the hotbar swap.
        assert!(search_key_swaps(true, true, filled, false));
        assert!(search_key_swaps(true, true, Hover::Slot, false));
        // Over an empty display slot, nothing or with a carried stack: text.
        assert!(!search_key_swaps(true, true, empty, false));
        assert!(!search_key_swaps(true, true, Hover::Nothing, false));
        assert!(!search_key_swaps(true, true, filled, true));
        // Letters always type; so do the digits no hotbar key is bound to.
        assert!(!search_key_swaps(false, false, filled, false));
        assert!(!search_key_swaps(true, false, filled, false));
    }

    #[test]
    fn display_slot_clicks_follow_slot_clicked() {
        use crate::menu::Input::*;
        // An empty hand takes the shown stack, or a full one with Shift.
        assert_eq!(display(None, 0, Pickup).0, Some(stack("stone", 1)));
        assert_eq!(display(None, 1, Pickup).0, Some(stack("stone", 1)));
        assert_eq!(display(None, 0, QuickMove).0, Some(stack("stone", 64)));
        // The same item grows by one (left), fills (Shift) or shrinks (right).
        assert_eq!(
            display(Some(stack("stone", 5)), 0, Pickup).0,
            Some(stack("stone", 6))
        );
        assert_eq!(
            display(Some(stack("stone", 64)), 0, Pickup).0,
            Some(stack("stone", 64))
        );
        assert_eq!(
            display(Some(stack("stone", 5)), 0, QuickMove).0,
            Some(stack("stone", 64))
        );
        assert_eq!(
            display(Some(stack("stone", 5)), 1, Pickup).0,
            Some(stack("stone", 4))
        );
        assert_eq!(display(Some(stack("stone", 1)), 1, QuickMove).0, None);
        // Another item is deleted (left) or loses one (right).
        assert_eq!(display(Some(stack("ender_pearl", 9)), 0, Pickup).0, None);
        assert_eq!(display(Some(stack("ender_pearl", 9)), 0, QuickMove).0, None);
        assert_eq!(
            display(Some(stack("ender_pearl", 9)), 1, Pickup).0,
            Some(stack("ender_pearl", 8))
        );
        // Number keys fill a hotbar slot, F fills off-hand, Q drops copies, pick block clones.
        assert_eq!(
            display(None, 3, Swap).1,
            DisplayClick::Hotbar(3, stack("stone", 64))
        );
        assert_eq!(
            display(None, 40, Swap).1,
            DisplayClick::Offhand(stack("stone", 64))
        );
        assert_eq!(
            display(None, 0, Throw).1,
            DisplayClick::Drop(stack("stone", 1))
        );
        assert_eq!(
            display(None, 1, Throw).1,
            DisplayClick::Drop(stack("stone", 64))
        );
        assert_eq!(display(None, 2, Clone).0, Some(stack("stone", 64)));
        assert_eq!(
            display(Some(stack("dirt", 2)), 2, Clone).0,
            Some(stack("dirt", 2))
        );
    }

    #[test]
    fn an_empty_display_slot_deletes_or_shrinks_the_carried_stack() {
        use crate::menu::Input::Pickup;
        let mut carried = Some(stack("dirt", 3));
        click_display(&mut carried, None, |_| 64, 1, Pickup);
        assert_eq!(carried, Some(stack("dirt", 2)));
        click_display(&mut carried, None, |_| 64, 0, Pickup);
        assert_eq!(carried, None);
    }

    fn creative(items: usize) -> Creative {
        let mut stacks: Vec<String> = ["anvil", "crafting_table", "wooden_hoe"]
            .iter()
            .map(|i| format!("{{count:1,id:\"minecraft:{i}\"}}"))
            .collect();
        stacks.extend((0..items).map(|i| format!("{{count:1,id:\"minecraft:item_{i}\"}}")));
        let mut lines = vec![
            vec!["Anvil"],
            vec!["Crafting Table"],
            vec!["Wooden Hoe", "When in Main Hand:", "-3 Attack Speed"],
        ];
        lines.extend(vec![vec!["Thing"]; items]);
        let mut tags = vec![
            vec!["minecraft:anvil"],
            vec![],
            vec!["minecraft:hoes", "minecraft:enchantable/mining"],
        ];
        tags.extend(vec![vec![]; items]);
        let build: Vec<_> = stacks
            .iter()
            .enumerate()
            .map(|(i, stack)| {
                if i < 2 {
                    row(stack)
                } else {
                    search_only(stack)
                }
            })
            .collect();
        let metadata: Vec<_> = stacks.iter().zip(lines).zip(tags).map(|((stack,lines),tags)| {
            serde_json::json!({"stack":stack,"lines":lines,"tags":tags})
        }).collect();
        fixture(serde_json::json!([
            {"id": "minecraft:functional_blocks", "type": "CATEGORY", "row": "TOP", "column": 3, "title": "Functional Blocks",
             "icon": "{count:1,id:\"minecraft:oak_sign\"}", "build":build},
            {"id": "minecraft:op_blocks", "type": "CATEGORY", "row": "BOTTOM", "column": 5, "title": "Operator Utilities",
             "icon": "{count:1,id:\"minecraft:command_block\"}", "build":[]},
            {"id": "minecraft:search", "type": "SEARCH", "row": "TOP", "column": 6, "title": "Search Items",
             "icon": "{count:1,id:\"minecraft:compass\"}", "build":[], "search_metadata":metadata},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": "{count:1,id:\"minecraft:chest\"}", "build":[]}
        ]))
    }

    #[test]
    fn inventory_tab_slot_layout_matches_vanilla() {
        assert_eq!(Creative::inventory_slots().count(), 41);
        for (slot, expected) in [
            (0, (9.0, 112.0)),
            (8, (153.0, 112.0)),
            (9, (9.0, 54.0)),
            (35, (153.0, 90.0)),
            (500, (54.0, 6.0)),
            (501, (54.0, 33.0)),
            (502, (108.0, 6.0)),
            (503, (108.0, 33.0)),
            (504, (35.0, 20.0)),
        ] {
            assert_eq!(Creative::inventory_slot_position(slot), Some(expected));
        }
        assert_eq!(Creative::inventory_slot_position(100), None);
        assert_eq!(Creative::TRASH_POSITION, (173.0, 112.0));
        let mut c = creative(90);
        c.select(2);
        assert!(c.tab().inventory);
        assert!(!c.can_scroll(&Names::default()));
    }

    #[test]
    fn inventory_tab_can_return_to_category_and_search() {
        let mut c = creative(90);
        for (selected, inventory, search) in [
            (2, true, false),
            (0, false, false),
            (1, false, true),
            (2, true, false),
        ] {
            c.search.set_value("anvil");
            c.scroll = 0.5;
            c.select(selected);
            assert_eq!(c.selected_index(), selected);
            assert_eq!(c.tab().inventory, inventory);
            assert_eq!(c.tab().search, search);
            assert!(c.search.value.is_empty());
            assert_eq!(c.scroll, 0.0);
        }
    }

    #[test]
    fn hidden_tabs_are_skipped() {
        let c = creative(0);
        let ids: Vec<&str> = c.tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "minecraft:functional_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        assert_eq!(c.tabs[0].items[0].item.as_ref(), "minecraft:anvil");
        let mut c = c;
        c.select(1);
        c.search.set_value("anvil");
        c.scroll = 0.5;
        c.select(1);
        assert_eq!((c.search.value.as_str(), c.scroll), ("anvil", 0.0));
        c.select(0);
        assert_eq!(c.search.value, "");
    }

    #[test]
    fn stepping_through_tabs_turns_their_pages() {
        let stack = |id: &str| format!("{{count:1,id:\"minecraft:{id}\"}}");
        let tab = |id: &str, column: u32, page: u32| {
            serde_json::json!({"id": format!("minecraft:{id}"), "type": "CATEGORY", "row": "TOP",
                "column": column, "page": page, "title": id, "icon": stack("stone"),
                "build": [row(stack("stone"))]})
        };
        let mut c = fixture(serde_json::json!([
            tab("first", 0, 0),
            tab("second", 1, 0),
            tab("modded", 0, 1),
            tab("more", 1, 2),
            tab("everywhere", 6, 0)
        ]));
        assert_eq!(c.pages(), 3);
        let count = c.tabs.len();
        let mut pages = Vec::new();
        for step in 1..=count {
            let tab = (c.selected_index() + 1) % count;
            c.select(tab);
            assert_eq!(c.selected_index(), tab, "step {step}");
            assert!(
                c.on_page(&c.tabs[tab]),
                "tab {tab} selected on another page"
            );
            pages.push(c.page);
        }
        // A tab every page shows keeps the page it was reached from; then it wraps.
        assert_eq!(pages, [0, 1, 2, 2, 0]);
        // Backwards off the first tab, across pages, the same way.
        c.select((c.selected_index() + count - 1) % count);
        assert_eq!(c.page, 0);
        c.select((c.selected_index() + count - 1) % count);
        assert_eq!(c.page, 2);
    }

    /// An export built with permissions: the operator tab's stacks, and their
    /// place at the end of the search tab, only with them.
    fn with_operator_tab() -> Creative {
        let stack = |id: &str| format!("{{count:1,id:\"minecraft:{id}\"}}");
        fixture(serde_json::json!([
            {"id": "minecraft:building_blocks", "type": "CATEGORY", "row": "TOP", "column": 0, "title": "Building Blocks",
             "icon": stack("bricks"), "build": [row(stack("stone"))]},
            {"id": "minecraft:op_blocks", "type": "CATEGORY", "row": "BOTTOM", "column": 5, "title": "Operator Utilities",
             "icon": stack("command_block"), "build": [], "operator_build": [row(stack("command_block")),row(stack("barrier"))]},
            {"id": "minecraft:search", "type": "SEARCH", "row": "TOP", "column": 6, "title": "Search Items",
             "icon": stack("compass"), "build": [], "search_metadata": [
                 {"stack":stack("stone"), "lines":["Stone"]},
                 {"stack":stack("command_block"), "lines":["Command Block"]},
                 {"stack":stack("barrier"), "lines":["Barrier"]}
             ]},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": stack("chest"), "build": []}
        ]))
    }

    fn tab_ids(c: &Creative) -> Vec<&str> {
        c.tabs.iter().map(|t| t.id.as_str()).collect()
    }

    fn publish_with(
        c: &mut Creative,
        permissions: bool,
        names: &Names,
        mods: impl Fn(&str, bool) -> Vec<ItemStack>,
    ) {
        let lists = c.catalog.build(
            Parameters {
                operator: permissions,
                features: &[],
                holders: &Default::default(),
                items: &crate::items::Items::empty(),
            },
            c.built_contents(),
            |tab, _, current| {
                let stacks = mods(tab.id.as_str(), permissions);
                append_unique(&mut current.display, &stacks);
                append_unique(&mut current.search, &stacks);
            },
        );
        c.publish(lists, Arc::default(), Arc::default(), names);
    }

    fn publish(c: &mut Creative, permissions: bool, names: &Names) {
        publish_with(c, permissions, names, |_, _| Vec::new());
    }

    fn ids(items: &[ItemStack]) -> Vec<&str> {
        items.iter().map(|s| s.item.as_ref()).collect()
    }

    #[test]
    fn the_stock_order_is_the_search_tabs_places_without_publishing_anything() {
        let c = with_operator_tab();
        let build = |operator| {
            c.catalog.build(
                Parameters {
                    operator,
                    features: &[],
                    holders: &Default::default(),
                    items: &crate::items::Items::empty(),
                },
                None,
                |_, _, _| {},
            )
        };
        let place =
            |order: &CreativeOrder, id: &str| order.of(id, ItemStack::new(id, 1).merge_kind());
        let normal = c.stock_order(&build(false));
        assert_eq!(place(&normal, "minecraft:stone"), Some(0));
        assert_eq!(place(&normal, "minecraft:barrier"), None);
        let operator = c.stock_order(&build(true));
        assert_eq!(place(&operator, "minecraft:command_block"), Some(1));
        assert_eq!(place(&operator, "minecraft:barrier"), Some(2));
    }

    #[test]
    fn operator_stacks_stay_out_without_permissions() {
        let c = with_operator_tab();
        let names = Names::default();
        assert_eq!(
            tab_ids(&c),
            [
                "minecraft:building_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        assert_eq!(c.search_items().len(), 1);
        assert!(c.search_results("command", &names).is_empty());
        assert_eq!(c.search_results("", &names), [0]);
    }

    #[test]
    fn operator_tab_comes_and_goes_with_permissions() {
        let mut c = with_operator_tab();
        let names = Names::default();
        publish(&mut c, true, &names);
        assert_eq!(
            tab_ids(&c),
            [
                "minecraft:building_blocks",
                "minecraft:op_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        let op: Vec<&str> = c.tabs[1].items.iter().map(|s| s.item.as_ref()).collect();
        assert_eq!(op, ["minecraft:command_block", "minecraft:barrier"]);
        assert_eq!(c.search_results("command", &names), [1]);
        assert_eq!(c.search_results("", &names), [0, 1, 2]);
        // The search tab keeps its place across the rebuild.
        c.select(2);
        c.search.set_value("block");
        publish(&mut c, false, &names);
        assert!(c.tab().search);
        assert_eq!(c.search.value, "block");
        assert!(c.items(&names).is_empty());
        c.reset_contents(); // the operator header now precedes Search again
        publish(&mut c, false, &names);
        assert!(c.tab().search);
        assert_eq!(c.search.value, "block");
        // The selected operator tab is left empty: the default tab instead.
        publish(&mut c, true, &names);
        c.select(1);
        assert_eq!(c.items(&names).len(), 2);
        publish(&mut c, false, &names);
        assert_eq!(c.tab().id, "minecraft:building_blocks");
        assert_eq!(c.items(&names)[0].item.as_ref(), "minecraft:stone");
    }

    #[test]
    fn mods_stacks_follow_the_tabs_own_and_join_the_search_tab() {
        let mut c = with_operator_tab();
        let names = Names::default();
        let mods = |tab: &str, _: bool| match tab {
            "minecraft:building_blocks" => vec![
                ItemStack::new("ae2:sky_stone_block", 1),
                // Already in the tab and the search tab: not listed twice.
                ItemStack::new("minecraft:stone", 1),
                // Empty stacks are left out.
                ItemStack::new("ae2:quartz_block", 0),
                ItemStack::new("minecraft:air", 1),
                // Listed once, though accepted twice.
                ItemStack::new("ae2:sky_stone_block", 1),
            ],
            _ => Vec::new(),
        };
        publish_with(&mut c, false, &names, mods);
        assert_eq!(
            ids(&c.tabs[0].items),
            ["minecraft:stone", "ae2:sky_stone_block"]
        );
        assert_eq!(
            ids(c.search_items()),
            ["minecraft:stone", "ae2:sky_stone_block"]
        );
        // Searched by its name, which it has no exported line for.
        assert_eq!(c.search_results("sky", &names), [1]);
        assert_eq!(c.search_results("ae2:", &names), [1]);
        c.select(0);
        assert_eq!(
            ids(&c.items(&names)),
            ["minecraft:stone", "ae2:sky_stone_block"]
        );
    }

    #[test]
    fn a_rebuild_asks_the_mods_again() {
        let mut c = with_operator_tab();
        let names = Names::default();
        // An operator-only stack a mod lists with permissions alone.
        let mods = |tab: &str, permissions: bool| {
            if tab == "minecraft:op_blocks" && permissions {
                vec![ItemStack::new("ae2:debug_card", 1)]
            } else {
                Vec::new()
            }
        };
        publish_with(&mut c, false, &names, mods);
        assert_eq!(
            tab_ids(&c),
            [
                "minecraft:building_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        publish_with(&mut c, true, &names, mods);
        assert_eq!(
            ids(&c.tabs[1].items),
            [
                "minecraft:command_block",
                "minecraft:barrier",
                "ae2:debug_card"
            ]
        );
        assert_eq!(
            ids(c.search_items()),
            [
                "minecraft:stone",
                "minecraft:command_block",
                "minecraft:barrier",
                "ae2:debug_card"
            ]
        );
        // Without permissions the mod's stack goes with the tab.
        publish_with(&mut c, false, &names, mods);
        assert_eq!(ids(c.search_items()), ["minecraft:stone"]);
    }

    #[test]
    fn a_mods_stack_brings_back_an_empty_category_tab() {
        let mut c = with_operator_tab();
        let names = Names::default();
        // `shouldDisplay`: a category tab with a stack is shown, whoever added it.
        publish_with(&mut c, false, &names, |tab, _| {
            if tab == "minecraft:op_blocks" {
                vec![ItemStack::new("ae2:creative_energy_cell", 1)]
            } else {
                Vec::new()
            }
        });
        assert_eq!(tab_ids(&c)[1], "minecraft:op_blocks");
        assert_eq!(ids(&c.tabs[1].items), ["ae2:creative_energy_cell"]);
    }

    #[test]
    fn saved_hotbars_tab_shows_its_rows_and_locks_the_hint() {
        let mut c = fixture(serde_json::json!([
            {"id": "minecraft:hotbar", "type": "HOTBAR", "row": "TOP", "column": 5, "title": "Saved Hotbars",
             "icon": "{count:1,id:\"minecraft:bookshelf\"}", "build": []},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": "{count:1,id:\"minecraft:chest\"}", "build": []}
        ]));
        assert!(c.tab().hotbar);
        assert!(!c.tab_headers()[1].hotbar);
        let mut hint = stack("paper", 1);
        hint.data_mut().set_other(
            "minecraft:creative_slot_lock",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        // A saved sword in slot 0, an empty slot, then the hint.
        c.set_saved_hotbars(vec![stack("diamond_sword", 1), stack("air", 0), hint]);
        let names = Names::default();
        let shown = c.display_window(&names);
        let cell = |i| shown.stack(Creative::DISPLAY + i).cloned();
        assert_eq!(cell(0), Some(stack("diamond_sword", 1)));
        assert_eq!(cell(1), None);
        assert_eq!(cell(3), None);
        let hint = cell(2).unwrap();
        assert!(slot_locked(&hint));
        assert!(!slot_locked(&stack("diamond_sword", 1)));
        // CustomCreativeSlot.mayPickup: the hint is never taken, and a carried
        // stack stays as it is.
        use crate::menu::Input::*;
        for (carried, button, input) in [
            (None, 0, Pickup),
            (None, 0, QuickMove),
            (None, 2, Clone),
            (None, 3, Swap),
            (None, 0, Throw),
            (Some(stack("dirt", 2)), 0, Pickup),
            (Some(stack("dirt", 2)), 1, Pickup),
        ] {
            let mut carried = carried;
            let before = carried.clone();
            let effect = click_display(&mut carried, Some(&hint), |_| 64, button, input);
            assert_eq!(effect, DisplayClick::Nothing);
            assert_eq!(carried, before);
        }
    }

    /// `CreativeModeInventoryScreen.keyPressed`: from a tab that is not the
    /// search, the chat key selects `CreativeModeTabs.searchTab`, which focuses
    /// the search box and empties the query the other tab left.
    #[test]
    fn the_chat_key_opens_the_search_tab() {
        let mut c = creative(0);
        c.search.set_value("anvil");
        let tab = c.search_tab().expect("the fixture has a search tab");
        assert!(c.tabs[tab].search);
        assert!(!c.tab().search);
        c.select(tab);
        assert!(c.tab().search);
        assert!(c.search.focused);
        assert_eq!(c.search.value, "", "another tab empties the query");
    }

    #[test]
    fn search_matches_vanilla_trees() {
        let c = creative(0);
        let names = Names::default();
        let q = |s: &str| c.search_results(s, &names);
        assert_eq!(q(""), [0, 1, 2]);
        assert_eq!(q("ANVIL"), [0]);
        assert_eq!(q("table"), [1]);
        assert_eq!(q("a"), [0, 1, 2]);
        // Tooltip lines, not just names; no trimming without a colon.
        assert_eq!(q("attack speed"), [2]);
        assert_eq!(q(" anvil"), Vec::<usize>::new());
        // A colon searches the id: namespace, then the path or a tooltip line.
        assert_eq!(q("minecraft:crafting_t"), [1]);
        assert_eq!(q(" minecraft : hoe "), [2]);
        assert_eq!(q("mine:anvil"), [0]);
        assert_eq!(q("other:anvil"), Vec::<usize>::new());
        assert_eq!(q(":"), [0, 1, 2]);
        // Tags after "#": only stacks with tags.
        assert_eq!(q("#"), [0, 2]);
        assert_eq!(q("#hoe"), [2]);
        assert_eq!(q("#minecraft:enchantable/"), [2]);
        assert_eq!(q("#craft"), Vec::<usize>::new());
    }

    #[test]
    fn search_matches_the_display_name_in_the_selected_language() {
        let c = creative(0);
        let map: HashMap<String, String> = HashMap::from([
            ("item.minecraft.anvil".to_owned(), "铁砧".to_owned()),
            ("item.minecraft.wooden_hoe".to_owned(), "木锄".to_owned()),
        ]);
        let names = Names::load(&sunrise_assets::resource_source::ResourceSource::indexed(
            [(
                "assets/minecraft/lang/en_us.json".into(),
                sunrise_assets::resource_source::Resource::bytes(serde_json::to_vec(&map).unwrap()),
            )]
            .into(),
        ));
        let lang = lang::Language::of("zh_cn", map);
        let (local, _) = lang::scoped(lang, || c.localized_lines(&names));
        let q = |s: &str| c.search_results_in(s, &names, &local, false);
        assert_eq!(q("铁砧"), [0]);
        assert_eq!(q("木锄"), [2]);
        // Only the selected language's lines count outside English.
        assert!(q("anvil").is_empty());
        assert_eq!(c.search_results_in("anvil", &names, &local, true), [0]);
        // A colon's path searches the display name too.
        assert_eq!(q("minecraft:铁砧"), [0]);
    }

    #[test]
    fn search_matches_every_localized_tooltip_line() {
        // Vanilla's FullTextSearchTree indexes the selected language's
        // tooltip lines; the export's English lines do not apply.
        let c = creative(0);
        let names = Names::default();
        let lang = lang::Language::of(
            "ru_ru",
            HashMap::from([("item.minecraft.anvil".into(), "Наковальня".into())]),
        );
        let (local, _) = lang::scoped(lang, || c.localized_lines(&names));
        let q = |s: &str, english| c.search_results_in(s, &names, &local, english);
        assert_eq!(q("наков", false), [0]);
        assert!(q("anvil", false).is_empty());
        assert_eq!(q("anvil", true), [0]);
    }

    #[test]
    fn creative_search_rerenders_retained_names_when_language_changes() {
        let mut creative = creative(0);
        let names = Names::default();
        let key = "item.test.anvil_name";
        let english = lang::Language::of("en_us", [(key.into(), "Metalworking".into())]);
        lang::scoped(english, || {
            creative.set_search_names(vec![Some(crate::text::Text::translate(key, &[]))]);
            assert_eq!(creative.search_results("metalworking", &names), [0]);
        });

        let german = lang::Language::of("de_de", [(key.into(), "Schmieden".into())]);
        lang::scoped(german, || {
            assert_eq!(creative.search_results("schmieden", &names), [0]);
            assert!(creative.search_results("metalworking", &names).is_empty());
        });
    }

    #[test]
    fn search_strips_legacy_formatting_across_name_components() {
        let mut creative = creative(0);
        let name = crate::text::Text::literal("Ruby§").append(crate::text::Text::literal("aGem"));
        creative.set_search_names(vec![Some(name)]);
        assert_eq!(creative.search_results("rubygem", &Names::default()), [0]);
    }

    #[test]
    fn scrollbar_drag_follows_the_mouse_while_held() {
        let mut c = creative(90);
        let names = Names::default();
        c.select(1);
        // Pressing the track holds the scrollbar without moving it.
        assert!(c.press_scrollbar(180.0, 60.0, &names));
        assert!(c.scrolling);
        assert_eq!(c.scroll, 0.0);
        // Dragging maps the cursor height onto the track (minus half the thumb).
        assert!(c.drag_scrollbar(18.0 + 7.5 + 48.5));
        assert!((c.scroll - 0.5).abs() < 1e-6);
        // The cursor may leave the track, the panel or the window: it keeps scrolling.
        c.drag_scrollbar(400.0);
        assert_eq!(c.scroll, 1.0);
        c.drag_scrollbar(-50.0);
        assert_eq!(c.scroll, 0.0);
        c.drag_scrollbar(18.0 + 7.5 + 97.0 * 0.25);
        assert!((c.scroll - 0.25).abs() < 1e-6);
        let rows = (93usize.div_ceil(9) - 5) as f32;
        assert_eq!(c.first_row(&names), (0.25 * rows + 0.5) as usize);
        // Releasing stops it.
        c.release();
        assert!(!c.drag_scrollbar(400.0));
        assert!((c.scroll - 0.25).abs() < 1e-6);
        // Off the track, or with too few items, it isn't held.
        assert!(!c.press_scrollbar(170.0, 60.0, &names));
        assert!(!c.press_scrollbar(180.0, 130.0, &names));
        c.select(0);
        assert!(c.press_scrollbar(180.0, 60.0, &names));
        assert!(!c.scrolling);
    }

    /// `CreativeModeInventoryScreen.selectTab`: the search box is focused on
    /// its own tab and loses the focus when another tab is shown (and, hidden,
    /// is no longer the field a key types into).
    #[test]
    fn the_search_box_loses_the_focus_with_its_tab() {
        let mut c = creative(0);
        let search = c
            .tabs
            .iter()
            .position(|t| t.search)
            .expect("the fixture has a search tab");
        c.select(search);
        assert!(c.search.focused);
        c.select(0);
        assert!(!c.search.focused, "another tab takes the focus away");
        assert!(!c.tab().search);
        c.select(search);
        assert!(c.search.focused, "and the search tab takes it back");
    }

    #[test]
    fn creative_tab_titles_have_vanilla_keys() {
        assert_eq!(
            item_group_key("minecraft:building_blocks"),
            Some("itemGroup.buildingBlocks")
        );
        assert_eq!(
            item_group_key("minecraft:food_and_drinks"),
            Some("itemGroup.foodAndDrink")
        );
        assert_eq!(item_group_key("minecraft:hotbar"), Some("itemGroup.hotbar"));
        assert_eq!(item_group_key("minecraft:nonsense"), None);
    }

    #[test]
    fn wheel_moves_one_row_per_notch() {
        let mut c = creative(90);
        let names = Names::default();
        c.select(1);
        let rows = (93usize.div_ceil(9) - 5) as f32;
        c.wheel(-1.0, &names);
        assert!((c.scroll - 1.0 / rows).abs() < 1e-6);
        assert_eq!(c.first_row(&names), 1);
        c.wheel(5.0, &names);
        assert_eq!(c.scroll, 0.0);
        assert_eq!(c.scroller_offset(), 0.0);
        c.scroll = 1.0;
        assert_eq!(c.scroller_offset(), 95.0);
    }

    #[test]
    fn search_edits_at_its_cursor() {
        use winit::keyboard::KeyCode;
        let mut c = creative(0);
        c.select(1);
        c.search.insert_text("hello world");
        for _ in 0..5 {
            assert!(c.search.key(KeyCode::ArrowLeft, false, false));
        }
        c.search.insert_text("X");
        assert_eq!(c.search.value, "hello Xworld");
        c.search.key(KeyCode::Home, false, true);
        c.search.insert_text("Y");
        assert_eq!(c.search.value, "Yworld");
        // Ctrl+Left jumps a word; one step right and Backspace deletes before the cursor.
        c.search.key(KeyCode::ArrowLeft, true, false);
        c.search.key(KeyCode::ArrowRight, false, false);
        c.search.key(KeyCode::Backspace, false, false);
        assert_eq!(c.search.value, "world");
    }

    #[test]
    fn unselected_tabs_want_the_hand_strictly_inside() {
        let tab = |top, column| Tab {
            id: String::new(),
            title: String::new(),
            icon: String::new(),
            items: Vec::new(),
            top,
            column,
            search: false,
            inventory: false,
            hotbar: false,
            page: 0,
            name: None,
        };
        // Top row, second column: the button spans x 27..53, y -28..4.
        let t = tab(true, 1);
        assert!(Creative::tab_wants_hand(&t, false, (40.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, true, (40.0, -10.0)));
        // Its edges ask for nothing; the mouse counts in whole pixels.
        assert!(!Creative::tab_wants_hand(&t, false, (27.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (27.9, -10.0)));
        assert!(Creative::tab_wants_hand(&t, false, (28.0, -10.0)));
        assert!(Creative::tab_wants_hand(&t, false, (52.9, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (53.0, -10.0)));
        assert!(!Creative::tab_wants_hand(&t, false, (40.0, -28.0)));
        assert!(Creative::tab_wants_hand(&t, false, (40.0, -27.0)));
        assert!(Creative::tab_wants_hand(&t, false, (40.0, 3.9)));
        assert!(!Creative::tab_wants_hand(&t, false, (40.0, 4.0)));
        // Bottom row, right-aligned inventory column: x 169..195, y 132..164.
        let b = tab(false, 6);
        assert!(Creative::tab_wants_hand(&b, false, (180.0, 140.0)));
        assert!(!Creative::tab_wants_hand(&b, false, (169.0, 140.0)));
        assert!(Creative::tab_wants_hand(&b, false, (170.0, 140.0)));
        assert!(!Creative::tab_wants_hand(&b, false, (180.0, 132.0)));
    }

    /// An empty export (`{"tabs":[]}`) leaves no tabs. `tab()` and
    /// `items()` used `tabs.len() - 1`, which underflows and panics;
    /// both now handle the empty case.
    #[test]
    fn empty_creative_tabs_do_not_panic_on_tab_or_items() {
        let c = fixture(serde_json::json!([]));
        assert!(c.tabs.is_empty());
        // Fallback tab: no panic, empty id, not search.
        assert_eq!(c.tab().id, "");
        assert!(!c.tab().search);
        let names = Names::default();
        assert!(c.items(&names).is_empty());
        assert!(c.tab_headers().is_empty());
        assert_eq!(c.search_tab(), None);
    }

    /// `CreativeModeInventoryScreen.checkTabHovering`: the 3-pixel inset of a
    /// tab's 26x32 slot (21x27) is the hover area whose name the screen shows.
    #[test]
    fn only_a_tabs_inset_icon_is_hovered() {
        let screen = tooltip_tabs();
        let tab = &screen.tabs[0];
        let (x, y) = Creative::tab_position(tab);
        let at = |dx: f32, dy: f32| screen.hovered_tab((0.0, 0.0), (x + dx, y + dy));
        assert_eq!(at(4.0, 4.0), Some(0));
        assert_eq!(at(23.5, 4.0), Some(0));
        assert_eq!(at(2.0, 4.0), None);
        assert_eq!(at(24.0, 4.0), None);
        assert_eq!(at(4.0, 2.0), None);
        assert_eq!(at(4.0, 30.0), None);
    }

    /// NeoForge's pages: a tab on another page is not hovered, even where it
    /// shares a slot with one on the page shown (#2646).
    #[test]
    fn only_the_shown_pages_tabs_are_hovered() {
        let tab = |id: &str, column: u32, page: u32| {
            serde_json::json!({"id": format!("minecraft:{id}"), "type": "CATEGORY", "row": "TOP",
                "column": column, "page": page, "title": id, "build": []})
        };
        let raw = serde_json::json!({"schema": 4, "item_features": {}, "potion_features": {}, "tabs": [
            tab("first", 0, 0), tab("second", 1, 0), tab("modded", 0, 1), tab("everywhere", 6, 0)
        ]});
        let mut c = Creative::from_json(&raw).unwrap();
        c.set_page(1);
        let at = |c: &Creative, i: usize| {
            let (x, y) = Creative::tab_position(&c.tabs[i]);
            c.hovered_tab((0.0, 0.0), (x + 4.0, y + 4.0))
        };
        assert_eq!(at(&c, 0), Some(2));
        assert_eq!(at(&c, 1), None);
        assert_eq!(at(&c, 3), Some(3));
    }
}

#[cfg(test)]
mod source_name_tests {
    #[test]
    fn item_names_read_selected_namespace_resources_without_files() {
        use sunrise_assets::resource_source::{Resource, ResourceSource};
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
            "assets/reader/lang/en_us.json".into(),
            Resource::bytes(br#"{"item.reader.widget":"Selected widget"}"#.to_vec()),
        )]));
        let names = super::Names::load(&source);
        assert_eq!(names.item("reader:widget"), "Selected widget");
    }
}
