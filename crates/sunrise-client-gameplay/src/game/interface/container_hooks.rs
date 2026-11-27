//! The open menu screen as `sunrise_hooks::containers` offers it (docs/layers.md, "Container
//! screens"): the engine's slot clicks and queries, the `Container` view over them, and the
//! dispatch of input and drawing to the crates that registered hooks, at the screen events' own
//! moments. With none registered nothing here builds a view.
use super::stations::{self, Kind as StationKind};
use crate::game::client_side::key_codes::glfw_button;
use std::cell::RefCell;
use sunrise_client::hud::{HudAtlas, HudBatch, rgba};
use sunrise_gameplay_common::game::Screen;
use sunrise_hooks::{
    containers::{self as api, Button, Click, Container, Panel, Role, Slot, SlotId, Stack},
    hud,
};
use sunrise_items::items::ItemStack;
use sunrise_level::containers::{BlockEntity, FurnaceKind, ModMenuType};
use sunrise_sim::menu::Input;
use winit::{event::MouseButton, keyboard::KeyCode};

// Views built since the test began: none without a registrant.
#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The game a view reads, and writes through when it was built for input.
enum Backing<'a, 'g> {
    Shared(&'a crate::game::ClientGame<'g>),
    Owned(&'a mut crate::game::ClientGame<'g>),
}

/// The open screen for one call. Drawing's view is read-only: its clicks and rule queries do
/// nothing.
pub(in crate::game) struct Open<'a, 'g> {
    game: Backing<'a, 'g>,
    menu: String,
    origin: (f32, f32),
    panel: Panel,
    /// The open menu's game slots: an id outside them reads and does nothing.
    slots: Vec<usize>,
    /// Every stack this view handed out, by its `Stack::id`.
    seen: RefCell<Vec<ItemStack>>,
}

/// What a world's own open container is, for its menu id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Local {
    Inventory,
    Crafting,
    /// A chest-like block: `rows` when it is a generic 9-wide menu, the container's `size`.
    Chest {
        rows: Option<usize>,
        shulker: bool,
        size: usize,
    },
    Furnace(FurnaceKind),
    Brewing,
    Crafter,
    Station(StationKind),
    Mount,
}

/// Vanilla's menu type id for a world's own container: what a server would send for the same.
pub(in crate::game::client_side) fn local_menu_id(local: Local) -> Option<String> {
    let id = match local {
        Local::Inventory => "inventory".into(),
        Local::Crafting => "minecraft:crafting".into(),
        Local::Chest {
            rows: Some(rows), ..
        } => format!("minecraft:generic_9x{rows}"),
        Local::Chest { shulker: true, .. } => "minecraft:shulker_box".into(),
        Local::Chest { size: 5, .. } => "minecraft:hopper".into(),
        Local::Chest { size: 9, .. } => "minecraft:generic_3x3".into(),
        Local::Chest { .. } => return None,
        Local::Furnace(FurnaceKind::Furnace) => "minecraft:furnace".into(),
        Local::Furnace(FurnaceKind::BlastFurnace) => "minecraft:blast_furnace".into(),
        Local::Furnace(FurnaceKind::Smoker) => "minecraft:smoker".into(),
        Local::Brewing => "minecraft:brewing_stand".into(),
        Local::Crafter => "minecraft:crafter_3x3".into(),
        Local::Station(StationKind::Merchant) => "minecraft:merchant".into(),
        Local::Station(kind) => sunrise_gameplay_common::game::lan_station_data::menu_of(kind)?
            .0
            .into(),
        Local::Mount => sunrise_gameplay_common::game::native_menu_ids::MOUNT_MENU.into(),
    };
    Some(id)
}

/// The role of game slot `slot` in mod menu `menu`, whose slots use the disjoint mod-menu
/// range (`MOD_MENU_SLOT_BASE..`): `None` outside them.
fn mod_menu_role(menu: &ModMenuType, slot: usize) -> Option<Role> {
    let index = sunrise_gameplay_common::game::inventory_slots::container_slot_index(slot)
        .filter(|&i| i < menu.slots)?;
    Some(match menu.crafting {
        Some(c) if c.result == index => Role::Result,
        Some(c) if c.grid_slots().contains(&index) => Role::Input,
        _ if menu.results.contains(&index) => Role::Result,
        _ => Role::Storage,
    })
}

/// `0xAARRGGBB` as the batch's colour.
fn color(argb: u32) -> u32 {
    let [a, r, g, b] = argb.to_be_bytes();
    rgba(r, g, b, a)
}

/// A mouse button by the screen events' numbering.
fn button_of(button: MouseButton) -> Button {
    match glfw_button(button) {
        0 => Button::Left,
        1 => Button::Right,
        2 => Button::Middle,
        n => Button::Other(u16::try_from(n).unwrap_or(u16::MAX)),
    }
}

impl<'g> Open<'_, 'g> {
    fn game(&self) -> &crate::game::ClientGame<'g> {
        match &self.game {
            Backing::Shared(game) => game,
            Backing::Owned(game) => game,
        }
    }
    fn game_mut(&mut self) -> Option<&mut crate::game::ClientGame<'g>> {
        match &mut self.game {
            Backing::Shared(_) => None,
            Backing::Owned(game) => Some(game),
        }
    }
    /// `slot` as a game slot of the open menu.
    fn slot(&self, slot: SlotId) -> Option<usize> {
        let slot = slot.0 as usize;
        self.slots.contains(&slot).then_some(slot)
    }
    fn stack_of(&self, stack: &ItemStack) -> Stack {
        let mut seen = self.seen.borrow_mut();
        seen.push(stack.clone());
        Stack::new(
            stack.item.to_string(),
            stack.count,
            self.game().max_stack(stack),
            stack.merge_kind(),
            seen.len() as u64 - 1,
        )
    }
}

impl Container for Open<'_, '_> {
    fn menu(&self) -> &str {
        &self.menu
    }
    fn slots(&self) -> Vec<Slot> {
        let game = self.game();
        self.slots
            .iter()
            .map(|&i| {
                let (x, y) = game.slot_position(i);
                Slot::new(SlotId(i as u32), game.container_role(i), x, y)
            })
            .collect()
    }
    fn stack(&self, slot: SlotId) -> Option<Stack> {
        let stack = self.game().slot_view(self.slot(slot)?)?;
        Some(self.stack_of(&stack))
    }
    fn carried(&self) -> Option<Stack> {
        self.game().carried.as_ref().map(|s| self.stack_of(s))
    }
    fn name(&self, slot: SlotId) -> Option<String> {
        let game = self.game();
        let stack = game.slot_view(self.slot(slot)?)?;
        Some(game.stack_hover_name(&stack))
    }
    fn takes_one(&mut self, slot: SlotId, of: &Stack) -> bool {
        let Some(slot) = self.slot(slot) else {
            return false;
        };
        let Some(stack) = usize::try_from(of.id)
            .ok()
            .and_then(|i| self.seen.borrow().get(i).cloned())
        else {
            return false;
        };
        self.game_mut()
            .is_some_and(|g| g.slot_takes_one(slot, &stack))
    }
    fn landing(&mut self, slot: SlotId) -> Vec<SlotId> {
        let Some(slot) = self.slot(slot) else {
            return Vec::new();
        };
        self.game_mut()
            .map(|g| g.slot_landing(slot))
            .unwrap_or_default()
            .into_iter()
            .map(|i| SlotId(i as u32))
            .collect()
    }
    fn click(&mut self, slot: SlotId, click: Click) -> bool {
        let Some(slot) = self.slot(slot) else {
            return false;
        };
        let Some(game) = self.game_mut() else {
            return false;
        };
        match click {
            Click::Pick { right } => game.slot_click(slot, i32::from(right), Input::Pickup),
            Click::QuickMove => game.slot_click(slot, 0, Input::QuickMove),
            Click::Put { one } => game.slot_put(slot, one),
        }
    }
    fn spread(&self) -> usize {
        self.game()
            .client
            .inv_drag
            .as_ref()
            .map_or(0, |(_, slots)| slots.len())
    }
    fn cursor(&self) -> (f32, f32) {
        let (x, y) = self.game().client.cursor;
        (x - self.origin.0, y - self.origin.1)
    }
    fn shift(&self) -> bool {
        self.game().has_shift_down()
    }
    fn panel(&self) -> Panel {
        self.panel
    }
}

/// What a hook draws with, from the panel's corner.
struct Canvas<'a> {
    batch: &'a mut HudBatch,
    atlas: &'a HudAtlas,
    origin: (f32, f32),
}

impl hud::Canvas for Canvas<'_> {
    fn size(&self) -> (f32, f32) {
        (self.batch.width, self.batch.height)
    }
    fn tr(&self, key: &str) -> String {
        sunrise_game_data::lang::get(key)
    }
    fn text_width(&self, text: &str) -> f32 {
        HudBatch::text_width(self.atlas, text)
    }
    fn text(&mut self, x: f32, y: f32, text: &str, argb: u32, shadow: bool) {
        let (ox, oy) = self.origin;
        self.batch
            .text(self.atlas, text, ox + x, oy + y, color(argb), shadow);
    }
    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, argb: u32) {
        let (ox, oy) = self.origin;
        self.batch
            .fill(self.atlas, ox + x, oy + y, w, h, color(argb));
    }
    fn place(&mut self, _: hud::Corner, _: f32, _: f32) -> Option<(f32, f32)> {
        None
    }
}

impl<'g> crate::game::ClientGame<'g> {
    /// The menu type id of the open menu screen: the player's own, any container, workstation,
    /// trade, mount or mod menu; not creative's tabs, which have no menu type.
    fn container_menu_id(&self) -> Option<String> {
        if self.client.screen != Screen::Inventory {
            return None;
        }
        if self.mount_active() {
            return local_menu_id(Local::Mount);
        }
        if self.client.remote.is_some() {
            return Some(self.remote_menu_type().unwrap_or("inventory").to_owned());
        }
        if let Some(station) = &self.common.station {
            return local_menu_id(Local::Station(station.kind));
        }
        local_menu_id(match self.common.container_kind() {
            None if self.common.craft_size == 3 => Local::Crafting,
            None => Local::Inventory,
            Some(BlockEntity::Chest(_)) => Local::Chest {
                rows: self.chest_rows(),
                shulker: self.container_block_is_shulker(),
                size: self.common.open_container_size(),
            },
            Some(BlockEntity::Furnace(f)) => Local::Furnace(f.kind),
            Some(BlockEntity::Brewing(_)) => Local::Brewing,
            Some(BlockEntity::Crafter(_)) => Local::Crafter,
            Some(_) => return None,
        })
    }
    /// What a game slot is for in the open menu.
    pub(in crate::game::client_side) fn container_role(&self, slot: usize) -> Role {
        if let Some(role) = self
            .shown_mod_menu_type()
            .and_then(|menu| mod_menu_role(menu, slot))
        {
            return role;
        }
        match slot {
            0..9 => Role::Hotbar,
            9..36 => Role::Inventory,
            100..109 => Role::Input,
            200 | stations::RESULT => Role::Result,
            500..=504
            | sunrise_gameplay_common::mount_rules::SADDLE
            | sunrise_gameplay_common::mount_rules::ARMOR => Role::Equipment,
            s if (stations::INPUT..stations::RESULT).contains(&s) => Role::Input,
            s if self.is_mount_chest_slot(s) => Role::Storage,
            s if sunrise_gameplay_common::game::inventory_slots::container_slot_index(s)
                .is_some() =>
            {
                match self.common.container_kind() {
                    Some(BlockEntity::Furnace(_)) if slot == 302 => Role::Result,
                    Some(BlockEntity::Crafter(_)) if slot == 309 => Role::Result,
                    Some(
                        BlockEntity::Furnace(_) | BlockEntity::Brewing(_) | BlockEntity::Crafter(_),
                    ) => Role::Input,
                    _ => Role::Storage,
                }
            }
            _ => Role::Other,
        }
    }
    /// The open mod menu's type, when a mod's menu is open.
    fn shown_mod_menu_type(&self) -> Option<&'static ModMenuType> {
        if !matches!(self.container_kind(), Some(BlockEntity::Mod(_))) {
            return None;
        }
        self.remote_menu_type()
            .and_then(sunrise_level::containers::mod_menu_type)
    }
    /// The panel's corner, which slot positions are from, and its size, as `batch`'s frame lays
    /// the screen out.
    fn container_frame(&self, batch: &HudBatch) -> ((f32, f32), Panel) {
        let (origin, width, height) = match self.built_container(batch) {
            Some(layout) => (layout.origin, layout.image[2], layout.image[3]),
            None => (
                self.inventory_origin_unbuilt(batch),
                self.native_screen_width().unwrap_or(176.0),
                self.screen_height(),
            ),
        };
        // titleLabelY and inventoryLabelY (imageHeight - 94).
        (origin, Panel::new(width, height, 6.0, height - 94.0))
    }
    /// The view for input, when a hook is registered and the screen is a menu.
    fn open_container<'a>(&'a mut self) -> Option<Open<'a, 'g>> {
        if !api::engine::any() {
            return None;
        }
        if self.screen_over_playing() {
            return None;
        }
        let menu = self.container_menu_id()?;
        #[cfg(any(test, feature = "test-support"))]
        BUILT.with(|n| n.set(n.get() + 1));
        let slots = self.slots();
        let (origin, panel) = match self.client.renderer.0.as_ref() {
            Some(r) => self.container_frame(&HudBatch::new(
                [r.config.width, r.config.height],
                self.client.settings.gui_scale_inputs(),
            )),
            None => (
                (0.0, 0.0),
                Panel::new(176.0, self.screen_height(), 6.0, 72.0),
            ),
        };
        Some(Open {
            game: Backing::Owned(self),
            menu,
            origin,
            panel,
            slots,
            seen: RefCell::default(),
        })
    }
    fn container_target(&self) -> Option<SlotId> {
        self.inventory_target().0.map(|s| SlotId(s as u32))
    }
    /// A mouse button's release clears the latch of a press a hook took: true when the press was
    /// the hooks', so the release is too. Called for every real button-up, before any hook.
    pub fn container_button_up(&mut self, button: MouseButton) -> bool {
        self.client.container_buttons.remove(&button)
    }
    /// A mouse button down or up on a menu screen, once the mods' pre hooks let it through: true
    /// when a hook took it, or for the release of a press a hook took (`owned`). A release a hook
    /// takes from vanilla's own press ends vanilla's quick craft drag without a click.
    pub fn container_mouse(&mut self, button: MouseButton, pressed: bool, owned: bool) -> bool {
        if !api::engine::any() {
            return owned;
        }
        let slot = self.container_target();
        let Some(mut open) = self.open_container() else {
            return owned;
        };
        if !pressed {
            let taken = api::engine::release(&mut open, slot, button_of(button));
            if taken && !owned {
                self.container_cancel_drag();
            }
            return taken || owned;
        }
        let taken = api::engine::press(&mut open, slot, button_of(button));
        if taken {
            self.client.container_buttons.insert(button);
        }
        taken
    }
    /// Vanilla's quick craft drag ends with nothing clicked: its release, if it comes, does
    /// nothing.
    fn container_cancel_drag(&mut self) {
        self.client.inv_drag = None;
        self.client.inv_clicks.cancel_drag();
    }
    /// The cursor moved with a button held, once the mods' pre hook let it through: true when a
    /// hook took it. A drag taken from vanilla's own press ends its quick craft drag.
    pub fn container_drag(&mut self) -> bool {
        if !api::engine::any() {
            return false;
        }
        let Some(button) = self
            .client
            .drag_button
            .filter(|b| self.client.physical_buttons.contains(b))
        else {
            return false;
        };
        let slot = self.container_target();
        let Some(mut open) = self.open_container() else {
            return false;
        };
        let taken = api::engine::drag(&mut open, slot, button_of(button));
        if taken && !self.client.container_buttons.contains(&button) {
            self.container_cancel_drag();
        }
        taken
    }
    /// The wheel over a menu screen, once the mods' pre hook let it through: true when a hook
    /// took it.
    pub(in crate::game::client_side) fn container_wheel(&mut self, steps: f32) -> bool {
        if !api::engine::any() {
            return false;
        }
        let slot = self.container_target();
        let Some(mut open) = self.open_container() else {
            return false;
        };
        api::engine::wheel(&mut open, slot, steps)
    }
    /// A key press on a menu screen, once the mods' pre hook let it through, unless a text field
    /// has the focus: true when a hook took it.
    pub fn container_key(&mut self, key: KeyCode) -> bool {
        if !api::engine::any() || self.text_input_focused() {
            return false;
        }
        let Some(name) = sunrise_client::keybinds::key_name(key) else {
            return false;
        };
        let slot = self.container_target();
        let Some(mut open) = self.open_container() else {
            return false;
        };
        api::engine::key(&mut open, slot, name)
    }
    /// What the hooks draw over the open menu screen, at its foreground phase.
    pub fn draw_container_hooks(&self, batch: &mut HudBatch) {
        if !api::engine::any() {
            return;
        }
        let Some(menu) = self.container_menu_id() else {
            return;
        };
        #[cfg(any(test, feature = "test-support"))]
        BUILT.with(|n| n.set(n.get() + 1));
        let (origin, panel) = self.container_frame(batch);
        let open = Open {
            game: Backing::Shared(self),
            menu,
            origin,
            panel,
            slots: self.slots(),
            seen: RefCell::default(),
        };
        let mut canvas = Canvas {
            batch,
            atlas: &self.client.gui.atlas,
            origin,
        };
        api::engine::draw(&open, &mut canvas);
    }
}

impl crate::game::ClientGame<'_> {
    /// One ordinary click on a game slot. False when nothing took it.
    pub fn slot_click(&mut self, slot: usize, button: i32, input: Input) -> bool {
        if self.client.remote.is_none() {
            return false;
        }
        self.remote_menu_ops(&[(Some(slot), button, input)])
    }
    /// Puts the hand's stack (`right`: one of it) in `slot`, as a one-slot
    /// drag (`QUICK_CRAFT`) and not a click: a server that has the hand empty
    /// (the stack it was to come from just burned as fuel) ignores a drag, but
    /// a click would pick up what's in `slot`.
    pub(in crate::game::client_side) fn slot_put(&mut self, slot: usize, right: bool) -> bool {
        let kind = i32::from(right) << 2;
        let ops = [
            (None, kind, Input::QuickCraft),
            (Some(slot), kind | 1, Input::QuickCraft),
            (None, kind | 2, Input::QuickCraft),
        ];
        if self.client.remote.is_none() {
            return false;
        }
        self.remote_menu_ops(&ops)
    }
    /// `slot` takes one more of `stack`, by the open menu's rules.
    pub(in crate::game::client_side) fn slot_takes_one(
        &mut self,
        slot: usize,
        stack: &ItemStack,
    ) -> bool {
        let mut takes = false;
        self.menu_session(|menu, env, to_menu| {
            takes = to_menu(slot).is_some_and(|i| menu.takes_one(env, i, stack));
            Some(false)
        });
        takes
    }
    /// Where vanilla's quick move would put `slot`'s items, tried on a copy
    /// of the menu that's dropped (`Some(false)` keeps nothing).
    pub fn slot_landing(&mut self, slot: usize) -> Vec<usize> {
        let slots = self.slots();
        let mut landing = Vec::new();
        self.menu_session(|menu, env, to_menu| {
            let from = to_menu(slot)?;
            landing = menu
                .quick_move_landing(env, from)
                .into_iter()
                .filter_map(|i| slots.iter().copied().find(|&g| to_menu(g) == Some(i)))
                .collect();
            Some(false)
        });
        landing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_container_has_vanillas_menu_id() {
        let chest = |rows| Local::Chest {
            rows: Some(rows),
            shulker: false,
            size: rows * 9,
        };
        assert_eq!(
            local_menu_id(chest(3)).as_deref(),
            Some("minecraft:generic_9x3")
        );
        assert_eq!(
            local_menu_id(chest(6)).as_deref(),
            Some("minecraft:generic_9x6")
        );
        let shulker = Local::Chest {
            rows: None,
            shulker: true,
            size: 27,
        };
        assert_eq!(
            local_menu_id(shulker).as_deref(),
            Some("minecraft:shulker_box")
        );
        assert_eq!(
            local_menu_id(Local::Furnace(FurnaceKind::Smoker)).as_deref(),
            Some("minecraft:smoker")
        );
        assert_eq!(
            local_menu_id(Local::Station(StationKind::Anvil)).as_deref(),
            Some("minecraft:anvil")
        );
        assert_eq!(
            local_menu_id(Local::Station(StationKind::Merchant)).as_deref(),
            Some("minecraft:merchant")
        );
        assert_eq!(
            local_menu_id(Local::Mount).as_deref(),
            Some("sunrise:mount_inventory")
        );
    }

    #[test]
    fn a_mod_menus_result_past_slot_199_is_a_result() {
        let menu = ModMenuType {
            id: "test:big",
            owner: "test",
            slots: 201,
            player_inventory: true,
            data_slots: 0,
            owned_slots: None,
            results: vec![200],
            inputs: Vec::new(),
            links: None,
            crafting: None,
        };
        use sunrise_gameplay_common::game::inventory_slots::MOD_MENU_SLOT_BASE;
        assert_eq!(
            mod_menu_role(&menu, MOD_MENU_SLOT_BASE + 200),
            Some(Role::Result)
        );
        assert_eq!(
            mod_menu_role(&menu, MOD_MENU_SLOT_BASE),
            Some(Role::Storage)
        );
        assert_eq!(mod_menu_role(&menu, MOD_MENU_SLOT_BASE + 201), None);
        assert_eq!(mod_menu_role(&menu, 500), None);
        assert_eq!(mod_menu_role(&menu, 9), None);
    }

    #[test]
    fn stacks_that_merge_have_the_same_kind() {
        let book = |order: [&str; 2]| {
            let mut stack = ItemStack::new("minecraft:enchanted_book", 1);
            stack.data_mut().stored_enchantments =
                order.iter().map(|id| ((*id).into(), 1)).collect();
            stack
        };
        let (a, b) = (
            book(["minecraft:mending", "minecraft:unbreaking"]),
            book(["minecraft:unbreaking", "minecraft:mending"]),
        );
        assert_eq!(a, b);
        assert_eq!(a.merge_kind(), b.merge_kind());
        assert_ne!(
            a.merge_kind(),
            ItemStack::new("minecraft:enchanted_book", 1).merge_kind()
        );
    }
}
