//! `IItemExtension.canEquip` on the game's side (`native_can_equip`). The dispenser, a mob's
//! pickup and the two inventory menus ask it, each with the game as the read-only `Host` and the
//! target as a handle. A stack can be converted once for the parts and lent to every slot and
//! entity asked about it in a synchronous check.
use super::*;
use crate::game::api_values::TryIntoApi;
use sunrise_sim::mobs::equip::CanEquip;

/// A stack converted once for the parts of its item, ready for any number of `can_equip`
/// questions in a synchronous check. `DataOnly` when no part of the item listens (the component's
/// answer stands and nothing is converted); `Refused` when a listening stack cannot be converted.
pub enum EquipStack {
    DataOnly,
    Refused,
    Ready(Box<mod_api::ItemStack>),
}

impl crate::game::CommonGame<'_> {
    pub fn equip_stack(&self, stack: &ItemStack) -> EquipStack {
        if !mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item) {
            return EquipStack::DataOnly;
        }
        #[cfg(any(test, feature = "test-support"))]
        EQUIP_CONVERSIONS.with(|count| count.set(count.get() + 1));
        match stack.try_into_api() {
            Ok(stack) => EquipStack::Ready(Box::new(stack)),
            Err(_) => EquipStack::Refused,
        }
    }
}

pub trait EquipAnswersGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `LivingEntity.isEquippableInSlot` with `IItemExtension.canEquip`: `data` is the stack's
    /// `equippable` answer for the entity `entity` of type `kind` in `slot`, and the parts of
    /// `stack`'s item may change it. When a part would be asked and `entity` is no live entity of
    /// this side (a missing or stale handle), the slot is refused.
    fn can_equip(
        &self,
        stack: &EquipStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        let stack = match stack {
            EquipStack::DataOnly => return data,
            EquipStack::Refused => return false,
            EquipStack::Ready(stack) => stack,
        };
        let Some(entity) = self.entity(entity) else {
            return false;
        };
        let native = crate::game::native_hooks::NativeHookHost {
            host: <Self::State as crate::game::Side>::as_host(self),
        };
        let mut can_equip = data;
        native.native_can_equip(
            stack,
            slot,
            mod_api::LivingRef::new(entity),
            kind,
            &mut can_equip,
        );
        can_equip
    }

    /// `can_equip` lent to a menu click that runs while the game is at hand (the player's own
    /// inventory menu, predicted or authoritative).
    fn menu_can_equip(&self) -> MenuEquip<'_, '__sg, Self> {
        MenuEquip {
            game: self,
            resource_lifetime: std::marker::PhantomData,
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> EquipAnswersGameExt1<'__sg> for G {}

/// The game lent to a menu click as its `CanEquip`.
pub struct MenuEquip<'g, 'a, S: crate::game::SideGame<'a>> {
    game: &'g S,
    pub resource_lifetime: std::marker::PhantomData<fn(&'a ()) -> &'a ()>,
}

/// Call-scoped `CanEquip` that lazily converts one unchanged `ItemStack` at most once across a
/// synchronous candidate loop. Rust's `&'s ItemStack` borrow prevents mutation during the loop.
struct ScopedMenuEquip<'g, 'a, 's, S: crate::game::SideGame<'a>> {
    game: &'g S,
    stack: &'s ItemStack,
    prepared: std::cell::OnceCell<EquipStack>,
    pub resource_lifetime: std::marker::PhantomData<fn(&'a ()) -> &'a ()>,
}

impl<'__carrier, S: crate::game::SideGame<'__carrier>> CanEquip
    for ScopedMenuEquip<'_, '__carrier, '_, S>
{
    fn listens(&self, item: &str) -> bool {
        mod_api::__private::item_listens(mod_api::HookId::CanEquip, item)
    }
    fn can_equip(
        &self,
        stack: &ItemStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        if std::ptr::eq(stack, self.stack) {
            let prepared = self
                .prepared
                .get_or_init(|| self.game.equip_stack(self.stack));
            self.game.can_equip(prepared, slot, entity, kind, data)
        } else {
            let prepared = self.game.equip_stack(stack);
            self.game.can_equip(&prepared, slot, entity, kind, data)
        }
    }
    fn with_stack(&self, _stack: &ItemStack, body: &mut dyn FnMut(&dyn CanEquip)) {
        body(self);
    }
}

impl<'__carrier, S: crate::game::SideGame<'__carrier>> CanEquip for MenuEquip<'_, '__carrier, S> {
    fn listens(&self, item: &str) -> bool {
        mod_api::__private::item_listens(mod_api::HookId::CanEquip, item)
    }
    fn can_equip(
        &self,
        stack: &ItemStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        let prepared = self.game.equip_stack(stack);
        self.game.can_equip(&prepared, slot, entity, kind, data)
    }
    fn with_stack(&self, stack: &ItemStack, body: &mut dyn FnMut(&dyn CanEquip)) {
        let scoped = ScopedMenuEquip {
            game: self.game,
            stack,
            prepared: std::cell::OnceCell::new(),
            resource_lifetime: std::marker::PhantomData,
        };
        body(&scoped);
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! { static EQUIP_CONVERSIONS:std::cell::Cell<usize>=const{std::cell::Cell::new(0)}; }
#[cfg(any(test, feature = "test-support"))]
pub fn take_equip_conversions() -> usize {
    EQUIP_CONVERSIONS.with(|count| count.replace(0))
}

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
