//! Keys and mouse buttons over an open container screen: hotbar and pick keys, modifier keys,
//! drags, throws and the press/release of a slot click.
use super::*;

/// Q throws the whole stack with Control down: the same modifier word the
/// screens read (`has_control_down`), never a latched sprint toggle.
pub fn drop_whole_stack(modifiers: winit::keyboard::ModifiersState) -> bool {
    modifiers.control_key()
}

impl crate::game::ClientGame<'_> {
    /// The receiving container's `keyPressed` runs before the input Post hook.
    /// `AbstractContainerScreen` reports inventory close as handled but still
    /// performs swap, clone and throw before returning false for those actions.
    pub(in crate::game::client_side) fn container_screen_key(
        &mut self,
        raw: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> Result<bool> {
        let creative = self.client.screen == Screen::Creative;
        let search = creative && self.client.creative_screen.0.tab().search;
        self.client.creative_ignore_text = false;
        let binding = sunrise_client::keybinds::key_name(raw);
        let chat = binding == Some(self.client.settings.keys.get("key.chat"));
        if creative
            && !search
            && chat
            && let Some(tab) = self.client.creative_screen.0.search_tab()
        {
            select_creative_tab(
                &mut self.client.creative_screen.0,
                &mut self.client.inv_clicks,
                tab,
            );
            self.build_screen();
            self.client.creative_ignore_text = true;
            return Ok(true);
        }
        if raw == KeyCode::Escape {
            self.key_pressed(raw, raw)?;
            return Ok(true);
        }
        // A focused mounted child has already received the key. Do not type it
        // into an unrelated editor that happens to be focused behind it.
        if !self.client.screen_state.0.widgets.has_focus() {
            if search && !self.mod_browser_typing() {
                if self.search_tab_hotbar_key(raw) {
                    self.client.creative_ignore_text = true;
                    return Ok(true);
                }
                if let Some(shortcut) = shortcut {
                    self.creative_search_shortcut(shortcut);
                } else {
                    let before = self.client.creative_screen.0.search.value.clone();
                    let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
                    self.client.creative_screen.0.search.key(raw, ctrl, shift);
                    if self.client.creative_screen.0.search.value != before {
                        self.client.creative_screen.0.scroll = 0.0;
                    }
                }
                return Ok(true);
            }
            if self.recipe_search_focused() {
                self.recipe_key(raw, shortcut);
                return Ok(true);
            }
            if self.mod_browser_typing() {
                if self.mod_browser_key(raw, shortcut) {
                    return Ok(true);
                }
                // Unlike the vanilla search tabs, the browser's focused child
                // can decline a key and leave it to the container.
            } else if self.station_typing() {
                if let Some(shortcut) = shortcut {
                    self.station_shortcut(shortcut);
                } else {
                    self.station_key(raw);
                }
                return Ok(true);
            } else if !creative
                && (self.merchant_key(raw)
                    || (chat && self.recipe_focus_search())
                    || self.recipe_book_key(raw))
            {
                return Ok(true);
            }
        }
        if binding == Some(self.client.settings.keys.get("key.inventory")) {
            self.direct_craft_stop();
            self.close_inventory_key();
            return Ok(true);
        }
        if self.sort_key_press(raw) {
            return Ok(true);
        }
        if let Some(key) = self.client.settings.keys.container_hotbar_key(raw) {
            self.inventory_hotkey(key);
        }
        if binding == Some(self.client.settings.keys.get("key.pickItem")) {
            self.inventory_pick_key();
        } else if binding == Some(self.client.settings.keys.get("key.drop")) {
            self.inventory_hotkey(KeyCode::KeyQ);
        }
        Ok(false)
    }

    /// `AbstractContainerScreen.keyPressed`: the inventory binding closes the current menu.
    pub(in crate::game::client_side) fn close_inventory_key(&mut self) {
        if self.client.screen == Screen::Creative {
            self.pad_clear_focus();
            if let Some(c) = self.common.carried.take() {
                let _ = self.add_stack_to_inventory(c);
            }
            self.set_screen_with(Screen::Playing, |g| g.close_creative_menu());
        } else {
            self.close_container_client();
            self.set_screen(Screen::Playing);
        }
        self.capture(true);
    }

    /// `CreativeModeInventoryScreen.keyPressed` on the search tab: a hotbar
    /// key over a slot swaps before the search box sees it.
    pub(in crate::game::client_side) fn search_tab_hotbar_key(&mut self, code: KeyCode) -> bool {
        use sunrise_client::creative::{Hover, search_key_swaps};
        let numeric = HOTBAR_DIGITS.contains(&code) || NUMERIC_KEYS.contains(&code);
        if !numeric {
            return false;
        }
        let Some(key) = self.client.settings.keys.container_hotbar_key(code) else {
            return false;
        };
        let Some(size) = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let hover = match self.slot_at(&batch, self.client.cursor.0, self.client.cursor.1) {
            None => Hover::Nothing,
            Some(slot) if slot >= Creative::DISPLAY => Hover::Display {
                filled: self.creative_display_stack(slot).is_some(),
            },
            Some(_) => Hover::Slot,
        };
        search_key_swaps(numeric, true, hover, self.common.carried.is_some())
            && self.inventory_hotkey(key)
    }
    /// Number keys, F and Q over a slot of the open menu: SWAP with a hotbar
    /// slot or the off hand, THROW one or the whole stack.
    pub(in crate::game::client_side) fn inventory_hotkey(&mut self, key: KeyCode) -> bool {
        use sunrise_client::container_screen::{HotbarKey, hotbar_swap};
        use sunrise_sim::menu::Input;
        let Some(size) = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let Some(slot) = self.slot_at(&batch, self.client.cursor.0, self.client.cursor.1) else {
            return false;
        };
        let hotbar = HOTBAR_DIGITS
            .iter()
            .position(|k| *k == key)
            .map(HotbarKey::Slot)
            .or((key == KeyCode::KeyF).then_some(HotbarKey::Offhand));
        // checkHotbarKeyPressed refuses the swap while the cursor carries a
        // stack; the drop below is the next arm of keyPressed, which the menu
        // itself leaves to an empty hand.
        if let Some(button) = hotbar_swap(hotbar, self.common.carried.is_none()) {
            self.click_open_inventory(Some(slot), button, Input::Swap)
        } else if key == KeyCode::KeyQ {
            let all = drop_whole_stack(self.client.modifiers);
            self.click_open_inventory(Some(slot), i32::from(all), Input::Throw)
        } else {
            false
        }
    }
    /// The pick block key bound to a keyboard key, over a slot of the open
    /// menu (`AbstractContainerScreen.keyPressed`): `CLONE` with button 0. The
    /// search tab's box takes the key instead.
    pub(in crate::game::client_side) fn inventory_pick_key(&mut self) {
        let menu = (self.client.screen == Screen::Inventory
            || (self.client.screen == Screen::Creative
                && !self.client.creative_screen.0.tab().search))
            && !self.dialog_open();
        let Some(size) = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let Some(slot) = self
            .slot_at(&batch, self.client.cursor.0, self.client.cursor.1)
            .filter(|_| menu)
        else {
            return;
        };
        let _ = self.click_open_inventory(Some(slot), 0, sunrise_sim::menu::Input::Clone);
    }
    /// The plain inventory mouse press: starts a quick craft drag when a
    /// stack is held, gathers on a double click, else the normal click.
    pub(in crate::game::client_side) fn inventory_press(&mut self, right: bool) -> bool {
        use sunrise_client::container_screen::Target;
        use sunrise_sim::menu::Input;
        self.direct_craft_stop();
        if self.mod_browser_press(right) {
            return true;
        }
        if !right && self.sort_press() {
            return true;
        }
        // AbstractRecipeBookScreen lets the book handle its buttons before
        // AbstractContainerScreen starts a drag or chooses an outside slot.
        // Toggling it can move the inventory under the pointer before release.
        if self.recipe_book_click(right) {
            self.client.inv_clicks.skip_release();
            return true;
        }
        if self.native_screen_press(right) {
            self.client.inv_clicks.skip_release();
            return true;
        }
        let (slot, outside) = self.inventory_target();
        // mouseClicked: `lastClickSlot = slot` for every press that reaches
        // the container, and the double click is the same slot and button
        // within 250 ms. A click outside records no slot, ending the chain.
        let double = sunrise_client::container_screen::double_click(
            &mut self.client.last_inv_click,
            slot,
            right,
            std::time::Instant::now(),
        );
        let shift = self.has_shift_down();
        let target = Target::of(slot, outside);
        let input = self.client.inv_clicks.press(
            &target,
            Self::screen_button(right),
            self.common.carried.is_none(),
            shift,
            false,
        );
        if outside {
            // slotId -999: an empty hand throws nothing; a carried stack
            // starts a drag whose release drops it.
            match input {
                Some(input) => self.throw_click(right, input),
                None if self.client.inv_clicks.quick_crafting().is_some() => {
                    self.client.inv_drag = Some((i32::from(right), Vec::new()));
                }
                None => return false,
            }
            return true;
        }
        if self.tweak_press(slot, right, shift) {
            return true;
        }
        if let Some(slot) = slot
            && shift
            && slot != 200
        {
            // mouseClicked's QUICK_MOVE (and Sunrise's at-press QUICK_MOVE
            // of a carried stack, mouseReleased's in vanilla) remember the
            // stack it moves.
            if input == Some(Input::QuickMove) || (self.common.carried.is_some() && !double) {
                self.client.last_quick_moved = self.slot_view(slot);
            }
            if double && !right {
                // mouseReleased: every stack of lastQuickMoved's item in the
                // clicked slot's container is quick-moved.
                if let Some(input) = input
                    && !self.click_open_inventory(Some(slot), 0, input)
                {
                    // A screen the menu engine doesn't model: its own click.
                    self.click_inventory(right);
                    return true;
                }
                if let Some(moved) = self.client.last_quick_moved.clone() {
                    // The creative screen clicks its own menu, on a server too.
                    if self.client.remote.is_some() && self.client.screen != Screen::Creative {
                        self.remote_quick_move_all(slot, &moved);
                    } else {
                        self.menu_session(|menu, env, to_menu| {
                            Some(menu.quick_move_all(env, to_menu(slot)?, &moved).is_ok())
                        });
                        if self.client.remote.is_some() {
                            self.send_creative_slots();
                        }
                    }
                }
                self.client.last_inv_click = None;
                self.client.inv_clicks.skip_release();
                return true;
            }
        }
        if let Some(slot) = slot
            && slot != 200
        {
            // An item tab's display slots take no drags or PICKUP_ALL.
            let display = slot >= Creative::DISPLAY;
            let double = double
                && !shift
                && !right
                && !display
                && self.common.carried.is_some()
                && self.pick_all_allowed(slot);
            if double && self.click_open_inventory(Some(slot), 0, Input::PickupAll) {
                self.client.last_inv_click = None;
                // The double click's own release (`mouseReleased`'s PICKUP_ALL).
                self.client.inv_clicks.skip_release();
                return true;
            }
            if self.common.carried.is_some() {
                let ty = i32::from(right);
                // `mouseClicked` starts a quick craft drag whenever the hand holds a stack,
                // Shift or not: `mouseDragged` fills the slots the cursor crosses, so a
                // Shift held over the drag spreads the same way (#4166). The drag starts
                // empty on a Shift press: a release with nothing spread is `mouseReleased`'s
                // click on the hovered slot (Pickup, or QuickMove under Shift).
                if !shift && !display && self.quick_craft_accepts(slot, ty, 0) {
                    self.client.inv_drag = Some((ty, vec![slot]));
                } else {
                    self.client.inv_drag = Some((ty, Vec::new()));
                }
                return true;
            }
        }
        if self.creative_tab_open() {
            // CreativeModeInventoryScreen goes through the same mouseClicked:
            // a click now, or a drag that its release ends.
            match input {
                Some(input) => {
                    self.click_open_inventory(slot, i32::from(right), input);
                }
                None if self.client.inv_clicks.quick_crafting().is_some() => {
                    self.client.inv_drag = Some((i32::from(right), Vec::new()));
                }
                None => return false,
            }
            return true;
        }
        // Remote clients route inventory clicks through click_inventory -> remote_click.
        self.click_inventory(right)
    }
    /// Mouse movement while a quick craft drag is running.
    pub(in crate::game::client_side) fn inventory_drag_move(&mut self) -> bool {
        if self.tweaks_on() && self.tweak_drag_move(self.inventory_target().0) {
            return true;
        }
        let Some((ty, slots)) = self.client.inv_drag.clone() else {
            return self.common.carried.is_some() || self.inventory_target().0.is_some();
        };
        let Some(r) = self.client.renderer.0.as_ref() else {
            return true;
        };
        let batch = HudBatch::new(
            [r.config.width, r.config.height],
            self.client.settings.gui_scale_inputs(),
        );
        if let Some(slot) = self.slot_at(&batch, self.client.cursor.0, self.client.cursor.1)
            && slot != 200
            && slot < Creative::DISPLAY
            && !slots.contains(&slot)
            && self.quick_craft_accepts(slot, ty, slots.len())
            && let Some((_, slots)) = self.client.inv_drag.as_mut()
        {
            slots.push(slot);
        }
        true
    }
    /// `AbstractContainerMenu.canTakeItemForPickAll` for Sunrise slot `slot`
    /// of the open menu: false for a crafting result (a mod menu type's too),
    /// whose quick second click is another craft, not a PICKUP_ALL. True for a
    /// screen the menu engine doesn't model.
    fn pick_all_allowed(&mut self, slot: usize) -> bool {
        let mut allowed = true;
        self.menu_session(|menu, _, to_menu| {
            allowed = to_menu(slot).is_none_or(|i| menu.can_take_for_pick_all(i));
            Some(false)
        });
        allowed
    }
    /// The hovered slot and whether the cursor is outside the window.
    pub fn inventory_target(&self) -> (Option<usize>, bool) {
        let Some(r) = self.client.renderer.0.as_ref() else {
            return (None, false);
        };
        let batch = HudBatch::new(
            [r.config.width, r.config.height],
            self.client.settings.gui_scale_inputs(),
        );
        self.target_at(&batch)
    }
    /// `inventory_target` for the frame `batch` describes.
    pub fn target_at(&self, batch: &HudBatch) -> (Option<usize>, bool) {
        let slot = self.slot_at(batch, self.client.cursor.0, self.client.cursor.1);
        // A mod screen may place a slot beyond its image: a click on one is
        // not a click outside the window.
        let outside =
            self.clicked_outside(batch) && !(slot.is_some() && self.native_screen_shown());
        (slot, outside)
    }
    pub(in crate::game::client_side) fn screen_button(
        right: bool,
    ) -> sunrise_client::container_screen::Button {
        use sunrise_client::container_screen::Button;
        if right { Button::Right } else { Button::Left }
    }
    /// `Screen.hasShiftDown` / `InputWithModifiers.hasShiftDown`: 26.3 keeps
    /// no key cache — every event answers with its own modifiers bit
    /// (`InputWithModifiers.java:54-56`, `keys = 3`), and the pointer and
    /// keyboard events carry the OS's live word, refreshed at the focus. The
    /// window's `ModifiersChanged` is that word here; key releases can die
    /// on the way from the OS (a stuck Shift then kept every screen's Shift
    /// and a sneak the key no longer held: #4166's in-game report), so a
    /// cached key set is not vanilla's answer.
    pub fn has_shift_down(&self) -> bool {
        self.client.modifiers.shift_key()
    }
    /// The same live word for Control (`InputWithModifiers.hasControlDown`).
    pub(in crate::game::client_side) fn has_control_down(&self) -> bool {
        self.client.modifiers.control_key()
    }

    /// Real events word the modifiers by their own `ModifiersChanged`; a
    /// control socket's `key` is synthesized, so it words the bit directly
    /// and every reader can ask the one live state.
    pub(in crate::game::client_side) fn press_modifier_key(&mut self, key: KeyCode, down: bool) {
        sunrise_client_input::modifiers::set_key(
            &mut self.client.modifiers,
            &mut self.client.physical_keys,
            key,
            down,
        );
    }
    /// `hasClickedOutside` of the open screen at the cursor.
    pub(in crate::game::client_side) fn clicked_outside(&self, batch: &HudBatch) -> bool {
        use sunrise_client::container_screen::{Book, clicked_outside};
        let image = self.container_image(batch);
        let book = self
            .recipe_book_area(batch)
            .map(|(visible, tab_hovered)| Book {
                visible,
                selected_tab_hovered: tab_hovered,
            });
        clicked_outside(
            f64::from(self.client.cursor.0),
            f64::from(self.client.cursor.1),
            image,
            book,
        )
    }
    /// The open container screen's window: its place and size.
    pub fn container_image(&self, batch: &HudBatch) -> sunrise_client::container_screen::Image {
        match self.built_container(batch) {
            Some(layout) => Self::image_of(layout.image),
            None => Self::image_of(self.container_image_parts(batch)),
        }
    }
    /// `leftPos`, `topPos`, `imageWidth`, `imageHeight` from the game's state
    /// alone: the container build's own arithmetic, and the fallback when no
    /// build holds this size.
    pub(in crate::game::client_side) fn container_image_parts(&self, batch: &HudBatch) -> [f32; 4] {
        let (ox, oy) = self.inventory_origin_unbuilt(batch);
        let (shift, width, height) = if self.client.screen == Screen::Creative {
            (0.0, Creative::WIDTH, Creative::HEIGHT)
        } else {
            let (shift, width) = match self.common.station.as_ref().map(|s| s.kind) {
                Some(stations::Kind::Merchant) => (-50.0, 276.0),
                Some(stations::Kind::Beacon) => (beacon::SHIFT, 230.0),
                _ => (0.0, self.native_screen_width().unwrap_or(176.0)),
            };
            (shift, width, self.screen_height())
        };
        [ox + shift, oy, width, height]
    }
    /// The build's panel as `AbstractContainerScreen`'s `leftPos`/`topPos`/
    /// `imageWidth`/`imageHeight`, the shape `hasClickedOutside` reads.
    fn image_of([left, top, width, height]: [f32; 4]) -> sunrise_client::container_screen::Image {
        sunrise_client::container_screen::Image {
            left: left as i32,
            top: top as i32,
            width: width as i32,
            height: height as i32,
        }
    }
    /// A click outside the window (slot id -999): `THROW` does nothing; a
    /// `PICKUP` drops the carried stack (left) or one of it (right), thrown
    /// from the hand (`AbstractContainerMenu.doClick`, `LivingEntity.drop`).
    pub fn throw_click(&mut self, right: bool, input: sunrise_sim::menu::Input) {
        let button = i32::from(right);
        if self.client.remote.is_some() {
            if self.client.screen == Screen::Creative {
                self.remote_creative_drop(right);
            } else {
                self.remote_throw(right, input);
            }
            return;
        }
        let before = self.common.carried.clone();
        let _ = self.click_open_inventory(None, button, input);
        if self.common.carried != before {
            self.swing_arm();
        }
    }
    /// Settle the slot action before releasing a screen child. Return true
    /// only for an early swallowed release or a mouse tweak that took it.
    pub(in crate::game::client_side) fn inventory_mouse_release(&mut self, right: bool) -> bool {
        use sunrise_client::container_screen::{Release, Target};
        let tweaked = self.tweak_release();
        let (slot, outside) = self.inventory_target();
        let dragged = self
            .client
            .inv_drag
            .as_ref()
            .is_some_and(|(_, s)| !s.is_empty());
        let had_drag = self.client.inv_drag.is_some();
        let release = self.client.inv_clicks.release(
            &Target::of(slot, outside),
            Self::screen_button(right),
            self.common.carried.is_none(),
            dragged,
            self.has_shift_down(),
            false,
        );
        match (release, Target::of(slot, outside)) {
            (Release::QuickCraft, _) => self.inventory_release(),
            (Release::Click(input), Target::Outside) => {
                self.client.inv_drag = None;
                self.throw_click(right, input);
            }
            (Release::Click(input), Target::Slot(slot)) if had_drag => {
                // The drag never reached a slot: a plain click where it ends.
                self.client.inv_drag = None;
                if !self.click_open_inventory(Some(slot), i32::from(right), input) {
                    self.click_inventory(right);
                }
            }
            (Release::Swallowed, _) => {
                self.client.inv_drag = None;
                return true;
            }
            _ => self.client.inv_drag = None,
        }
        tweaked
    }
    /// A press of the pick block button (`keyPickItem`, the middle button by
    /// default) over an inventory screen: `mouseClicked` with `cloning`
    /// (infinite materials). An empty hand clones the hovered stack at its
    /// full size (`CLONE`), a carried stack starts a quick craft drag of type
    /// 2 that fills every slot it crosses; without infinite materials the
    /// press does nothing.
    pub(in crate::game::client_side) fn inventory_pick_press(&mut self) -> bool {
        use sunrise_client::container_screen::{Button, Target};
        let (slot, outside) = self.inventory_target();
        let was_crafting = self.client.inv_clicks.quick_crafting().is_some();
        let input = self.client.inv_clicks.press(
            &Target::of(slot, outside),
            Button::Middle,
            self.common.carried.is_none(),
            self.has_shift_down(),
            self.common.creative,
        );
        match (input, slot) {
            // Slot -999 (outside the window) clones nothing.
            (Some(input), Some(slot)) if !outside => {
                self.click_open_inventory(Some(slot), 2, input);
            }
            (None, _) if !was_crafting && self.client.inv_clicks.quick_crafting().is_some() => {
                // Item tab display slots take no drags.
                let first = slot.filter(|&s| {
                    s != 200 && s < Creative::DISPLAY && self.quick_craft_accepts(s, 2, 0)
                });
                self.client.inv_drag = Some((2, first.into_iter().collect()));
            }
            _ => return false,
        }
        true
    }
    /// The pick block button's release (`mouseReleased`): ends its quick
    /// craft drag, cancels another button's, or clones onto the slot under a
    /// carried stack (`CLONE`, which only an item tab's display slot takes).
    pub(in crate::game::client_side) fn inventory_pick_release(&mut self) -> bool {
        use sunrise_client::container_screen::{Button, Release, Target};
        let (slot, outside) = self.inventory_target();
        let dragged = self
            .client
            .inv_drag
            .as_ref()
            .is_some_and(|(_, s)| !s.is_empty());
        let release = self.client.inv_clicks.release(
            &Target::of(slot, outside),
            Button::Middle,
            self.common.carried.is_none(),
            dragged,
            self.has_shift_down(),
            true,
        );
        match (release, slot) {
            (Release::QuickCraft, _) => self.inventory_release(),
            (Release::Click(input), Some(slot)) if !outside => {
                self.client.inv_drag = None;
                self.click_open_inventory(Some(slot), 2, input);
            }
            (Release::Swallowed, _) => {
                self.client.inv_drag = None;
                return true;
            }
            _ => self.client.inv_drag = None,
        }
        false
    }
    /// Mouse release: finishes a quick craft drag (one slot is a plain click).
    pub(in crate::game::client_side) fn inventory_release(&mut self) {
        use sunrise_sim::menu::Input;
        let Some((ty, slots)) = self.client.inv_drag.take() else {
            return;
        };
        if slots.len() == 1 {
            if !self.click_open_inventory(Some(slots[0]), ty, Input::Pickup) {
                self.click_inventory(ty == 1);
            }
        } else {
            let mask = |header: i32| header | (ty << 2);
            let mut ops = vec![(None, mask(0), Input::QuickCraft)];
            ops.extend(
                slots
                    .into_iter()
                    .map(|slot| (Some(slot), mask(1), Input::QuickCraft)),
            );
            ops.push((None, mask(2), Input::QuickCraft));
            if !self.menu_ops(&ops) {
                // Not a menu the engine models: the drag is a plain click.
                self.click_inventory(ty == 1);
            }
        }
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
