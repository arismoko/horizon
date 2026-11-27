use sunrise_core::use_animation::UseAnimation;
#[cfg(test)]
use sunrise_gameplay_common::game::item_use_common::ItemUseState;
use sunrise_gameplay_common::game::item_use_common::hand_of;
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
use sunrise_items::items::{ItemInfo, ItemStack};

impl crate::game::ClientGame<'_> {
    /// What `Item.finishUsingItem` leaves in the hand on a server's client: a
    /// stack that is not a consumable stays as it is (a spyglass); a
    /// consumable loses one (`ItemStack.consume`), and once that empties the
    /// hand, its `use_remainder` takes the hand
    /// (`UseRemainder.convertIntoRemainder`). Neither in creative mode. A
    /// remainder of a stack that is not emptied goes to the inventory on the
    /// server and arrives with its inventory update.
    pub(in crate::game::client_side) fn finish_result(
        &self,
        stack: &ItemStack,
        info: &ItemInfo,
    ) -> Option<ItemStack> {
        if self.common.creative || self.consumable(stack).is_none() {
            return Some(stack.clone());
        }
        let mut left = stack.clone();
        left.count = left.count.saturating_sub(1);
        if left.count > 0 {
            return Some(left);
        }
        sunrise_sim::item_sync::use_remainder(stack, Some(info))
    }
    /// `LivingEntityUseItemEvent.Finish` on a server's client, whose
    /// `completeUsingItem` (`handleEntityEvent(9)`) ran for `stack` in `slot`.
    /// `ItemStack.finishUsingItem` runs first, as on the client: the use
    /// cooldown, then the hand holds `finish_result`. A mod's result is put in
    /// the slot; the server's inventory update replaces either. Finishing a use
    /// sends no creative packet in vanilla (its inventory-slot edits go out
    /// through the open creative screen's `CreativeInventoryListener`), so the
    /// slot counts as synced for this client's creative sync.
    pub fn remote_use_item_finished(&mut self, slot: usize, stack: &ItemStack, info: &ItemInfo) {
        self.client_apply_use_cooldown(stack);
        let mut result = self.finish_result(stack, info);
        let hand = hand_of(Some((slot, 0))).unwrap_or(self.common.hand);
        self.native_living_entity_use_item_finish(
            stack,
            Some(self.own_entity_id()),
            hand,
            0,
            &mut result,
        );
        let before = self.creative_stacks();
        'write: {
            if !self.common.creative && stack.count == 1 && self.consumable(stack).is_some() {
                // A listener replaced the used-up stack: its stack stays, and only the
                // hand write is skipped. `stopUsingItem` still ends the use below.
                if self.common.stack_in(slot) != Some(stack) {
                    break 'write;
                }
                *self.common.stack_in_mut(slot) = None;
                self.player_item_destroyed(stack, Some(hand));
            }
            *self.common.stack_in_mut(slot) = result.filter(|s| s.count > 0);
            self.remote_synced_since(&before);
        }
        self.stopped_using(stack, Some(self.own_entity_id()), hand, 0);
    }
    /// Client-side `UseCooldown.apply`: updates local cooldowns until server sync confirms or clears.
    pub(in crate::game) fn client_apply_use_cooldown(&mut self, stack: &ItemStack) {
        let default = self
            .common
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.use_cooldown.clone());
        let Some(component) = sunrise_items::cooldowns::component(stack, default.as_ref()) else {
            return;
        };
        let group = sunrise_items::cooldowns::group(stack, default.as_ref());
        self.common.cooldowns.add(group, component.ticks());
    }
    pub(in crate::game) fn client_consume_held_slot(&mut self, amount: u32) {
        let cell = self.common.held_mut();
        if let Some(stack) = cell.as_mut() {
            stack.count = stack.count.saturating_sub(amount);
            if stack.count == 0 {
                *cell = None;
            }
        }
    }
}

/// The first person pose of a hand using an item of `animation`
/// (`ItemInHandRenderer.renderArmWithItem`'s switch), for an item of no
/// pose of its own.
pub(in crate::game) fn use_pose(
    animation: UseAnimation,
    remaining: f32,
    duration: f32,
    partial: f32,
) -> sunrise_client::first_person::ItemPose {
    use sunrise_client::first_person::ItemPose;
    let ticks_used = duration - (remaining - partial + 1.0);
    match animation {
        UseAnimation::Eat | UseAnimation::Drink => ItemPose::Eat {
            remaining,
            duration,
            partial,
        },
        UseAnimation::Bow => ItemPose::Bow { ticks_used },
        UseAnimation::Trident => ItemPose::Trident { ticks_used },
        UseAnimation::Brush => ItemPose::Brush { remaining, partial },
        _ => ItemPose::Using,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[cfg(test)]
    use sunrise_core::use_animation::UseAnimation;
    #[cfg(test)]
    use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

    pub fn stand_in_first_person(game: &mut crate::game::ClientGame<'_>) {
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, sunrise_core::geometry::Vec3::ZERO)
                .unwrap(),
        );
        game.tick_player_model();
    }

    #[cfg(test)]
    /// `render_hand` draws in a hand that holds an item, where `render_arm`
    /// never fires. What the mod drew is in the hand's vertices, in a run of
    /// its own ahead of vanilla's, and it stays when the mod leaves vanilla's
    /// hand out. The pose the mod sets moves vanilla's hand, not what it
    /// drew. The empty off hand is asked too.
    #[test]
    #[ignore = "daily"]
    fn a_mod_draws_in_a_hand_that_holds_an_item() {
        use mod_api::{ClientHost, Flow, Hand, Hooks, ModEntry, Mods, RenderHandEvent};
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };

        // One quad a block wide, a block in front of the eyes.
        const MARK: [[f32; 3]; 4] = [
            [0.0, 1.0, -1.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 1.0, -1.0],
        ];
        struct Mod {
            cancel: Arc<AtomicBool>,
            shift: Arc<AtomicBool>,
            asked: Arc<Mutex<Vec<(Hand, bool)>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn render_hand(&self, _: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
                let holds_an_item = event.render.stack.is_some();
                self.asked
                    .lock()
                    .unwrap()
                    .push((event.render.hand, holds_an_item));
                if self.shift.load(Ordering::Relaxed) {
                    event.render.translation[0] += 1.0;
                }
                // Draw in the hand that holds the sword only.
                if holds_an_item {
                    let uv = [0.0, 0.0, 16.0, 16.0];
                    assert!(event.draw.quad("block/stone", MARK, uv, 0xFF_FF_FF));
                }
                if self.cancel.load(Ordering::Relaxed) {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        fn hold_a_sword(game: &mut crate::game::ClientGame<'_>) {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.common.inventory[game.common.selected] = Some(
                sunrise_items::items::ItemStack::new("minecraft:iron_sword", 1),
            );
            game.tick_player_model();
            game.client.visible_hands[0].1 = game.inventory[game.selected].clone();
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let shift = Arc::new(AtomicBool::new(false));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let hooks = Mod {
            cancel: cancel.clone(),
            shift: shift.clone(),
            asked: asked.clone(),
        };
        let mods: Mods = vec![ModEntry::new("hand", Box::new(hooks))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let mut drawn = None;
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                hold_a_sword(game);
                let kept = game.hand_geometry(0.0);
                shift.store(true, Ordering::Relaxed);
                let posed = game.hand_geometry(0.0).0;
                shift.store(false, Ordering::Relaxed);
                cancel.store(true, Ordering::Relaxed);
                drawn = Some((kept, posed, game.hand_geometry(0.0)));
            });
        });
        let ((kept, kept_runs), posed, (alone, alone_runs)) = drawn.unwrap();
        // Each frame both hands were asked, the empty off hand included.
        let frame = [(Hand::Main, true), (Hand::Off, false)];
        assert_eq!(*asked.lock().unwrap(), [frame, frame, frame].concat());
        // Cancelled: only the mod's quad, in a run with no rendered item id.
        assert!(!alone.is_empty());
        assert_eq!(alone_runs, [(0, 0)]);
        // The pose the mod set is not under its draw: the quad stays, and
        // vanilla's hand moves by the block asked along the view's x.
        assert_eq!(posed.len(), kept.len());
        let (quad, hand) = posed.split_at(alone.len());
        assert!(quad.iter().zip(&alone).all(|(a, b)| a.pos == b.pos));
        for (moved, still) in hand.iter().zip(&kept[alone.len()..]) {
            let by = (0..3)
                .map(|i| (moved.pos[i] - still.pos[i]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!((by - 1.0).abs() < 1e-4, "moved by {by}");
        }

        // The unmodified comparison owns a new resource graph and Parts snapshot.
        sunrise_gameplay_common::game::api_values::install_parts(&[]).unwrap();
        super::super::native_entity_refs::tests::with_client_game(|game| {
            hold_a_sword(game);
            let (vanilla, vanilla_runs) = game.hand_geometry(0.0);
            assert!(!vanilla.is_empty());
            // Kept: the same quad first, then vanilla's hand exactly as without a mod.
            assert_eq!(kept.len(), alone.len() + vanilla.len());
            assert!(kept.iter().zip(&alone).all(|(a, b)| a.pos == b.pos));
            let hand = &kept[alone.len()..];
            assert!(hand.iter().zip(&vanilla).all(|(a, b)| a.pos == b.pos));
            let shifted = vanilla_runs
                .iter()
                .map(|&(start, id)| (start + alone.len() as u32, id));
            let runs = std::iter::once((0, 0)).chain(shifted).collect::<Vec<_>>();
            assert_eq!(kept_runs, runs);
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn item_animation_dispatches_vanilla_parts_and_refuses_invalid_spears() {
        use mod_api::{Hooks, Host, ItemPart, ModEntry, Mods, Registrar, UseAnimationEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        #[derive(Clone)]
        struct Part(Arc<AtomicUsize>);
        #[mod_api::sunrise_part]
        impl ItemPart for Part {
            fn use_animation(&self, _: &dyn Host, event: &mut UseAnimationEvent<'_>) {
                self.0.fetch_add(1, Ordering::Relaxed);
                let expected = match mod_api::__private::item_key_id(event.stack.item()).unwrap() {
                    "minecraft:bow" => UseAnimation::Bow,
                    "minecraft:crossbow" => UseAnimation::Crossbow,
                    "minecraft:trident" => UseAnimation::Trident,
                    "minecraft:brush" => UseAnimation::Brush,
                    "minecraft:wooden_spear"
                        if event.stack.components().is_none_or(|d| {
                            d.as_nbt()
                                .get(&"!minecraft:kinetic_weapon".into())
                                .is_none()
                        }) =>
                    {
                        UseAnimation::Spear
                    }
                    "minecraft:stick"
                        if event.stack.components().is_some_and(|d| {
                            d.as_nbt().get(&"minecraft:kinetic_weapon".into()).is_some()
                        }) =>
                    {
                        UseAnimation::Spear
                    }
                    _ => UseAnimation::None,
                };
                assert_eq!(*event.animation, expected);
                *event.animation = if mod_api::__private::item_key_id(event.stack.item()).unwrap()
                    == "minecraft:stick"
                    || event.stack.components().is_some_and(|d| {
                        d.as_nbt()
                            .get(&"!minecraft:kinetic_weapon".into())
                            .is_some()
                    }) {
                    UseAnimation::Spear
                } else {
                    UseAnimation::Drink
                };
            }
        }
        struct Mod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                for id in [
                    "minecraft:bow",
                    "minecraft:crossbow",
                    "minecraft:trident",
                    "minecraft:brush",
                    "minecraft:wooden_spear",
                    "minecraft:stick",
                ] {
                    let item = r.item_key(id);
                    r.item_part(item, Part(self.0.clone()));
                }
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mut overridden_geometry = None;
        let mods: Mods = vec![ModEntry::new("animation", Box::new(Mod(calls.clone())))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                for id in [
                    "minecraft:bow",
                    "minecraft:crossbow",
                    "minecraft:trident",
                    "minecraft:brush",
                    "minecraft:wooden_spear",
                ] {
                    assert_eq!(
                        game.item_use_animation(&sunrise_items::items::ItemStack::new(id, 1)),
                        UseAnimation::Drink
                    );
                }
                assert_eq!(
                    game.item_use_animation(&sunrise_items::items::ItemStack::new(
                        "minecraft:stick",
                        1
                    )),
                    UseAnimation::None
                );
                let mut patched = sunrise_items::items::ItemStack::new("minecraft:bow", 1);
                patched.data_mut().set_other(
                    "minecraft:consumable",
                    Some(sunrise_core::nbt::Tag::compound([(
                        "animation",
                        sunrise_core::nbt::Tag::String("drink".into()),
                    )])),
                );
                assert_eq!(game.item_use_animation(&patched), UseAnimation::Drink);
                use sunrise_core::nbt::Tag;
                let mut kinetic_stick = sunrise_items::items::ItemStack::new("minecraft:stick", 1);
                kinetic_stick.data_mut().set_other(
                    "minecraft:kinetic_weapon",
                    Some(Tag::compound([("delay_ticks", Tag::Int(30))])),
                );
                assert!(matches!(
                    game.item_use_state(&kinetic_stick),
                    super::ItemUseState::Spear(_)
                ));
                let mut removed = sunrise_items::items::ItemStack::new("minecraft:wooden_spear", 1);
                removed.data_mut().set_other(
                    "!minecraft:kinetic_weapon",
                    Some(Tag::Compound(Default::default())),
                );
                assert_eq!(game.item_use_animation(&removed), UseAnimation::None);
                assert_eq!(calls.load(Ordering::Relaxed), 9);
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::ZERO,
                    )
                    .unwrap(),
                );
                game.common.inventory[game.common.selected] =
                    Some(sunrise_items::items::ItemStack::new("minecraft:bow", 1));
                game.using = Some((game.selected, 0));
                game.tick_player_model();
                assert_eq!(
                    game.client
                        .player_model
                        .as_ref()
                        .unwrap()
                        .extra
                        .use_animation,
                    UseAnimation::Drink
                );
                assert_eq!(calls.load(Ordering::Relaxed), 10);
                game.client.visible_hands[0].1 = game.inventory[game.selected].clone();
                game.use_duration = 72_000;
                let (overridden, _) = game.hand_geometry(0.0);
                assert_eq!(calls.load(Ordering::Relaxed), 11);
                // A scoped spear override on a component-patched ordinary item
                // must reach hand geometry without the old whitelist panic.
                game.common.inventory[game.common.selected] = Some(kinetic_stick);
                game.client.visible_hands[0].1 = game.inventory[game.selected].clone();
                let (kinetic_geometry, _) = game.hand_geometry(0.0);
                assert!(!kinetic_geometry.is_empty());
                overridden_geometry = Some(overridden);
            });
        });
        // The unmodified comparison owns a new resource graph and Parts snapshot.
        sunrise_gameplay_common::game::api_values::install_parts(&[]).unwrap();
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.common.inventory[game.common.selected] =
                Some(sunrise_items::items::ItemStack::new("minecraft:bow", 1));
            game.using = Some((game.selected, 0));
            game.tick_player_model();
            game.client.visible_hands[0].1 = game.inventory[game.selected].clone();
            game.use_duration = 72_000;
            let (vanilla, _) = game.hand_geometry(0.0);
            let overridden = overridden_geometry.unwrap();
            assert!(!overridden.is_empty());
            assert_eq!(overridden.len(), vanilla.len());
            assert!(overridden.iter().zip(&vanilla).any(|(a, b)| a.pos != b.pos));
        });
    }
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
