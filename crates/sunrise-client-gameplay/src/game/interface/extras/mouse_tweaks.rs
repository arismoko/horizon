//! Sunrise's inventory extras (#332, `docs/extras.md`), Mouse Tweaks-style
//! controls and sorting, both off by default: while an extra is off its
//! hooks return false and the screen clicks exactly as vanilla's. Each action is made of ordinary slot clicks
//! (the menu engine's, or `ContainerClick`s on a server), so it works
//! wherever vanilla's clicks do. They run in the player inventory, the
//! chest-like containers (chests, barrels, ender chests, shulker boxes,
//! hoppers, dispensers), furnaces and crafting tables.
//!
//! - Right-drag holding a stack: one item into each slot entered, at once.
//! - Left-drag holding a stack, before vanilla's drag has spread over a
//!   second slot: each slot entered holding the same item is gathered into
//!   the hand (when it all fits); with Shift, those slots are quick-moved.
//! - Shift + left-drag with an empty hand: every slot entered is quick-moved.
//! - The wheel over a slot, with an empty hand: down moves one of its items
//!   where a Shift click would send it (`quickMoveStack`: fuel to the fuel
//!   slot, a smeltable to the input), up brings one of the same item from the
//!   other side. Down over a result slot takes one craft or the whole furnace
//!   output to the inventory; nothing ever goes into a result slot.
//!
//! Sorting, which clicks through `slot_click` too, is in `sorting.rs`.
use sunrise_client::creative::Creative;
use sunrise_gameplay_common::game::Screen;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::menu::Input;

#[derive(Default)]
pub struct Tweaks {
    pub drag: Option<Drag>,
    /// Wheel steps not yet acted on (a trackpad scrolls in fractions).
    wheel: f32,
}
/// A server's menu the tweaks run in: their clicks are vanilla's in these,
/// and the menu engine knows which slot takes what. Not the menus whose slots
/// cost or trigger something (anvils, enchanting, merchants, beacons, crafters).
pub(in crate::game::client_side) fn tweak_menu(name: &str) -> bool {
    name == "inventory"
        || name.starts_with("minecraft:generic_9x")
        || matches!(
            name,
            "minecraft:shulker_box"
                | "minecraft:hopper"
                | "minecraft:generic_3x3"
                | "minecraft:furnace"
                | "minecraft:blast_furnace"
                | "minecraft:smoker"
                | "minecraft:crafting"
        )
}
/// A drag the tweaks follow.
#[derive(Clone, Debug)]
pub struct Drag {
    pub right: bool,
    /// The stack in hand at the press and its count since, as the drag's
    /// own clicks change it (a server answers them later).
    pub held: Option<ItemStack>,
    /// The drag replaced vanilla's: its release clicks nothing.
    pub active: bool,
    /// `physical_presses` at the press that started it: a later press is another drag.
    pub press: u64,
    pub visited: Vec<usize>,
}

impl crate::game::ClientGame<'_> {
    /// The open screen is one the tweaks run in, with the extra on.
    pub(in crate::game::client_side) fn tweaks_on(&self) -> bool {
        if !self.client.settings.extras.mouse_tweaks
            || self.client.screen != Screen::Inventory
            || self.common.station.is_some()
            || self.mount_active()
        {
            return false;
        }
        if self.client.remote.is_some() {
            self.remote_menu_type().is_none_or(tweak_menu)
        } else {
            matches!(self.container_kind(), None | Some(BlockEntity::Chest(_)))
        }
    }
    /// A press on `slot`: true when the tweaks took it (a right-drag
    /// placing), else vanilla's press runs and the tweaks watch the drag.
    pub(in crate::game::client_side) fn tweak_press(
        &mut self,
        slot: Option<usize>,
        right: bool,
        shift: bool,
    ) -> bool {
        self.client.tweaks.drag = None;
        let Some(slot) = slot.filter(|&s| s != 200 && s < Creative::DISPLAY) else {
            return false;
        };
        if !self.tweaks_on() {
            return false;
        }
        let held = self.common.carried.clone();
        if right {
            // A result slot takes nothing: its press stays vanilla's.
            if shift || held.is_none() || self.tweak_result(slot) {
                return false;
            }
            // RMB: the press places at once, then the drag goes on.
            let mut drag = Drag {
                right,
                held,
                active: true,
                visited: vec![slot],
                press: self.client.physical_presses,
            };
            self.tweak_place(&mut drag, slot);
            self.client.inv_drag = None;
            self.client.inv_clicks.skip_release();
            self.client.tweaks.drag = Some(drag);
            return true;
        }
        if held.is_some() || shift {
            self.client.tweaks.drag = Some(Drag {
                right,
                held,
                active: false,
                visited: vec![slot],
                press: self.client.physical_presses,
            });
        }
        false
    }
    /// Places one held item in `slot` when it takes it.
    fn tweak_place(&mut self, drag: &mut Drag, slot: usize) {
        let Some(held) = drag.held.clone() else {
            return;
        };
        if self.slot_takes_one(slot, &held) && self.slot_put(slot, true) {
            drag.held = (held.count > 1).then(|| ItemStack {
                count: held.count - 1,
                ..held
            });
        }
    }
    /// The cursor moved while a button is held: true when the tweaks own
    /// the drag (vanilla's quick craft drag doesn't grow).
    pub fn tweak_drag_move(&mut self, slot: Option<usize>) -> bool {
        let Some(mut drag) = self.client.tweaks.drag.take() else {
            return false;
        };
        // A drag whose button went up unseen (another handler took its release, or the next press) is over.
        let button = if drag.right {
            winit::event::MouseButton::Right
        } else {
            winit::event::MouseButton::Left
        };
        if !self.client.physical_buttons.contains(&button)
            || drag.press != self.client.physical_presses
        {
            return false;
        }
        let active = drag.active;
        let entered =
            slot.filter(|s| *s != 200 && *s < Creative::DISPLAY && !drag.visited.contains(s));
        if let Some(slot) = entered {
            drag.visited.push(slot);
            let stack = self.slot_view(slot);
            let shift = self.has_shift_down();
            if drag.right {
                self.tweak_place(&mut drag, slot);
            } else if let Some(held) = drag.held.clone() {
                // Holding a stack: only slots of the same item, and only
                // while vanilla's drag hasn't spread.
                let spread = self
                    .client
                    .inv_drag
                    .as_ref()
                    .is_some_and(|(_, s)| s.len() > 1);
                if let Some(s) = stack.filter(|s| s.same_item_same_components(&held))
                    && (drag.active || !spread)
                {
                    if !drag.active {
                        drag.active = true;
                        self.client.inv_drag = None;
                        self.client.inv_clicks.skip_release();
                    }
                    if shift {
                        self.slot_click(slot, 0, Input::QuickMove);
                    } else if held.count + s.count <= self.common.max_stack(&s)
                        && self.slot_click(slot, 0, Input::Pickup)
                        && self.slot_click(slot, 0, Input::Pickup)
                    {
                        // Put the hand's stack in the slot, then take it all.
                        drag.held = Some(ItemStack {
                            count: held.count + s.count,
                            ..held
                        });
                    }
                }
            } else if shift && stack.is_some() {
                drag.active = true;
                self.slot_click(slot, 0, Input::QuickMove);
            }
        }
        let owned = drag.active || active;
        self.client.tweaks.drag = Some(drag);
        owned
    }
    /// The button went up: the drag ends.
    pub(in crate::game::client_side) fn tweak_release(&mut self) -> bool {
        self.client.tweaks.drag.take().is_some()
    }
    /// The wheel over an inventory screen: true when the tweaks used it.
    pub(in crate::game::client_side) fn tweak_scroll(
        &mut self,
        delta: f32,
        slot: Option<usize>,
    ) -> bool {
        if !self.client.settings.extras.mouse_tweaks_wheel
            || !self.tweaks_on()
            || self.common.carried.is_some()
        {
            return false;
        }
        let Some(slot) = slot else {
            self.client.tweaks.wheel = 0.0;
            return false;
        };
        self.client.tweaks.wheel += delta;
        let steps = self.client.tweaks.wheel.trunc();
        self.client.tweaks.wheel -= steps;
        for _ in 0..steps.abs() as u32 {
            if steps < 0.0 {
                self.tweak_push(slot);
            } else {
                self.tweak_pull(slot);
            }
        }
        true
    }
    /// `slot` is a result slot of the open menu.
    pub fn tweak_result(&mut self, slot: usize) -> bool {
        let mut result = false;
        self.menu_session(|menu, _, to_menu| {
            result = to_menu(slot).is_some_and(|i| menu.is_result(i));
            Some(false)
        });
        result
    }
    /// The slots across from `slot`: a container's or crafting grid's face
    /// the inventory (hotbar first, from the right), the inventory's face
    /// them; without one, hotbar and main inventory trade.
    fn other_side(&self, slot: usize) -> Vec<usize> {
        let table = self.common.craft_size == 3;
        let own: Vec<usize> = self
            .slots()
            .into_iter()
            .filter(|&s| {
                sunrise_gameplay_common::game::inventory_slots::container_slot_index(s).is_some()
                    || (table && (100..109).contains(&s))
            })
            .collect();
        let player = || (0..9).rev().chain((9..36).rev());
        match slot {
            s if (100..109).contains(&s)
                || sunrise_gameplay_common::game::inventory_slots::container_slot_index(s)
                    .is_some() =>
            {
                player().collect()
            }
            0..36 if !own.is_empty() => own,
            0..9 => (9..36).collect(),
            9..36 => (0..9).collect(),
            _ => Vec::new(),
        }
    }
    /// Wheel down: one item of `slot` to where a quick move sends it. A
    /// result can't be split or put back: one craft (or the furnace's whole
    /// output) goes, when it all fits.
    fn tweak_push(&mut self, slot: usize) {
        let Some(stack) = self.slot_view(slot) else {
            return;
        };
        let landing = self.slot_landing(slot);
        let Some(&target) = landing.first() else {
            return;
        };
        if self.tweak_result(slot) {
            if self.slot_click(slot, 0, Input::Pickup) {
                for target in landing {
                    self.slot_put(target, false);
                }
            }
            return;
        }
        if self.slot_click(slot, 0, Input::Pickup) {
            self.slot_put(target, true);
            if stack.count > 1 {
                self.slot_put(slot, false);
            }
        }
    }
    /// Wheel up: one item of the same kind from the other side into `slot`.
    /// A result slot takes none and gives none.
    fn tweak_pull(&mut self, slot: usize) {
        let Some(stack) = self.slot_view(slot) else {
            return;
        };
        if !self.slot_takes_one(slot, &stack) {
            return;
        }
        let mut source = None;
        for s in self.other_side(slot) {
            if let Some(v) = self.slot_view(s)
                && v.same_item_same_components(&stack)
                && !self.tweak_result(s)
            {
                source = Some((s, v.count));
                break;
            }
        }
        let Some((source, count)) = source else {
            return;
        };
        if self.slot_click(source, 0, Input::Pickup) {
            self.slot_put(slot, true);
            if count > 1 {
                self.slot_put(source, false);
            }
        }
    }
}
