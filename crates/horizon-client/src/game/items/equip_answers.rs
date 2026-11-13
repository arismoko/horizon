//! `IItemExtension.canEquip` on the game's side (`native_can_equip`). The dispenser, a mob's
//! pickup and the two inventory menus ask it, each with the game as the read-only `Host` and the
//! target as a handle. A stack is converted for the parts once and lent to every slot and entity
//! asked about it.
use super::*;
use crate::game::api_values::TryIntoApi;
use horizon_client::mobs::equip::CanEquip;

/// A stack converted for the parts of its item, ready for any number of `can_equip` questions.
/// Empty when no part of the item listens (the component's answer stands and nothing is
/// converted), or when the stack cannot cross, which leaves the answer as the hooks' generated
/// crossings do.
pub(in crate::game) struct EquipStack(Option<mod_api::ItemStack>);

impl Game<'_> {
    /// Prepares `stack` for [`Self::can_equip`]: one table lookup when no part of its item listens.
    pub(in crate::game) fn equip_stack(&self, stack: &ItemStack) -> EquipStack {
        let listens = mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item);
        EquipStack(listens.then(|| stack.try_into_api().ok()).flatten())
    }

    /// `LivingEntity.isEquippableInSlot` with `IItemExtension.canEquip`: `data` is the stack's
    /// `equippable` answer for the entity `entity` of type `kind` in `slot`, and the parts of the
    /// prepared `stack`'s item may change it. When a part would be asked and `entity` is no live
    /// entity of this side (a missing or stale handle), the slot is refused.
    pub(in crate::game) fn can_equip(
        &self,
        stack: &EquipStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        let Some(stack) = &stack.0 else {
            return data;
        };
        let Some(entity) = self.entity(entity) else {
            return false;
        };
        let mut can_equip = data;
        self.native_can_equip(
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
    pub(in crate::game) fn menu_can_equip(&self) -> MenuEquip<'_, '_> {
        MenuEquip {
            game: self,
            prepared: Default::default(),
        }
    }
}

/// The game lent to a menu click as its `CanEquip`. The last stack asked about stays prepared,
/// so a click asking about one stack slot after slot converts it once.
pub(in crate::game) struct MenuEquip<'g, 'a> {
    game: &'g Game<'a>,
    prepared: std::cell::RefCell<Option<(ItemStack, EquipStack)>>,
}

impl CanEquip for MenuEquip<'_, '_> {
    fn can_equip(
        &self,
        stack: &ItemStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        if !mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item) {
            return data;
        }
        if self
            .prepared
            .borrow()
            .as_ref()
            .is_none_or(|(s, _)| s != stack)
        {
            *self.prepared.borrow_mut() = Some((stack.clone(), self.game.equip_stack(stack)));
        }
        let prepared = self.prepared.borrow();
        let Some((_, ready)) = prepared.as_ref() else {
            return data;
        };
        self.game.can_equip(ready, slot, entity, kind, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The part of an iron helmet: it lets the helmet on the feet only, and counts the asks.
    #[derive(Clone, Default)]
    struct FeetOnly(std::sync::Arc<AtomicUsize>);
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for FeetOnly {
        fn can_equip(&self, _: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
            event.can_equip = event.slot == "feet";
        }
    }
    struct Cobbler(FeetOnly);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Cobbler {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.0.clone());
        }
    }

    /// An iron helmet with enchantments, a name and its own `equippable`: a stack whose
    /// conversion for the parts allocates.
    fn rich_helmet() -> ItemStack {
        use horizon_core::nbt::Tag;
        let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
        helmet.set_crafting_enchantments(vec![("minecraft:protection".into(), 4)]);
        let data = helmet.data_mut();
        data.custom_name = Some(Tag::String("Crown".into()));
        let component = Tag::compound([("slot", Tag::String("head".into()))]);
        data.set_other("minecraft:equippable", Some(component));
        helmet
    }

    /// Runs `test` on a game with the cobbler's part installed, given what it counted.
    fn with_cobbler(test: impl FnOnce(&mut Game<'_>, &AtomicUsize)) {
        installed(|asked| {
            crate::game::native_entity_refs::tests::with_game(|game| {
                // The player in place, admitted as a joined one is, so its handle resolves.
                let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
                let mut body = horizon_client::motion::PlayerMotion::new(game.motion, at).unwrap();
                body.entity_id = 7;
                game.player = Some(body);
                game.bind_local_player_residency();
                test(game, asked)
            });
        });
    }

    /// Runs `test` with the cobbler's mod installed, given what its part counted.
    fn installed(test: impl FnOnce(&AtomicUsize)) {
        let part = FeetOnly::default();
        let cobbler = mod_api::ModEntry::new("cobbler", Box::new(Cobbler(part.clone())));
        let mods: mod_api::Mods = vec![cobbler].into();
        crate::entry::install_client_hooks();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || test(&part.0));
    }

    /// Asked once per candidate slot, a warmed `can_equip` allocates nothing, for a stack with
    /// components, on every path: a menu the game is lent to (the player's, a guest's), and the
    /// dispenser's and a pickup's prepared stack; nor does an item no part listens to. Each
    /// answer is the part's.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn warmed_can_equip_questions_allocate_nothing() {
        with_cobbler(|game, asked| {
            let (helmet, other) = (rich_helmet(), ItemStack::new("minecraft:diamond_helmet", 1));
            let (player, kind) = (game.own_entity_id() as u32, "minecraft:player");
            let slots = ["head", "chest", "legs", "feet"];
            let feet_only = [false, false, false, true];
            let lent = game.menu_can_equip();
            assert!(!lent.can_equip(&helmet, "head", player, kind, true));
            let (answers, allocations) = horizon_threads::allocation_count::measure(|| {
                slots.map(|slot| lent.can_equip(&helmet, slot, player, kind, slot == "head"))
            });
            assert_eq!((answers, allocations), (feet_only, 0));
            let (answers, allocations) = horizon_threads::allocation_count::measure(|| {
                slots.map(|slot| lent.can_equip(&other, slot, player, kind, slot == "head"))
            });
            assert_eq!((answers, allocations), ([true, false, false, false], 0));
            let prepared = game.equip_stack(&helmet);
            let (answers, allocations) = horizon_threads::allocation_count::measure(|| {
                slots.map(|slot| game.can_equip(&prepared, slot, player, kind, slot == "head"))
            });
            assert_eq!((answers, allocations), (feet_only, 0));
            assert_eq!(asked.load(Ordering::Relaxed), 1 + 4 + 4);
        });
    }

    /// A guest's right click with a bundle on the cursor on an empty armour slot takes the
    /// bundle's next stack out into it (`BundleItem.overrideStackedOnOther`), and the host's
    /// authoritative menu asks the parts about that inner stack: the cobbler's part keeps the iron
    /// helmet inside off the head, and the bundle, the armour and the inventory stay as they were;
    /// it lets it onto the feet, and the bundle is empty after.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_guests_bundle_puts_on_what_the_part_allows() {
        use horizon_client::bundle::Contents;
        use horizon_protocol::packets::play::{ContainerClick, Serverbound};
        installed(|asked| {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.reset_entity_residency();
                let mut bundle = ItemStack::new("minecraft:bundle", 1);
                let mut contents = Contents::of(game.items(), &bundle).unwrap();
                let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
                assert_eq!(contents.try_insert(game.items(), &mut helmet), 1);
                contents.store(&mut bundle);
                let held = |game: &Game<'_>| {
                    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                    let name = |s: &Option<ItemStack>| s.as_ref().map(|s| s.item.to_string());
                    let inside = (guest.carried.as_ref())
                        .and_then(|b| Contents::of(game.items(), b))
                        .map(|c| {
                            c.items()
                                .iter()
                                .map(|s| s.item.to_string())
                                .collect::<Vec<_>>()
                        });
                    let inventory = guest.inventory.iter().filter(|s| s.is_some()).count();
                    (guest.armor.each_ref().map(name), inside, inventory)
                };
                // Right clicks (button 1) of the pickup mode on menu slots 5 (head) and 8 (feet).
                let click = |game: &mut Game<'_>, slot: i16| {
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.carried = Some(bundle.clone());
                    });
                    let click = ContainerClick {
                        container: 0,
                        state: 0,
                        slot,
                        button: 1,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    };
                    game.lan_guest_packet(12, Serverbound::ContainerClick(click));
                };
                let helmet = "minecraft:iron_helmet".to_owned();
                click(game, 5);
                assert_eq!(
                    held(game),
                    ([None, None, None, None], Some(vec![helmet.clone()]), 0)
                );
                click(game, 8);
                assert_eq!(
                    held(game),
                    ([None, None, None, Some(helmet)], Some(Vec::new()), 0)
                );
                // The part was asked about the helmet taken out of the bundle.
                assert!(asked.load(Ordering::Relaxed) > 0);
            });
        });
    }
}
