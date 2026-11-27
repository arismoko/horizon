//! Writes to a living entity for native server mods, through a `LivingRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles"): `LivingEntity.swing`,
//! the arrows and bee stingers stuck in a player or a mob (`setArrowCount`,
//! `setStingerCount`, `removeArrowTime`, `removeStingerTime`),
//! `Mob.setLeftHanded`, `Mob.setCanPickUpLoot`, `useItemRemaining`,
//! `getUsedItemHand`, `startUsingItem` and `releaseUsingItem`, a player's
//! Riptide spin (`startAutoSpinAttack`) and the style and colour of a
//! waypoint (`Waypoint.Icon`). Reads return None and writes false for a stale
//! handle, on a client, and for a kind Sunrise keeps no such state for (each
//! doc says which).
use super::commands::Target;
use mod_api::{Identifier, LivingRef};
use sunrise_sim::lan::waypoints::Icon;
use sunrise_sim::mobs::stuck::Stuck;
use {sunrise_sim::first_person_sim::Hand, sunrise_sim::first_person_sim::SwingAnimation};

impl crate::game::ServerGame<'_> {
    /// The UUID of the player a living handle names.
    pub(in crate::game::server_side) fn native_living_player(
        &mut self,
        living: LivingRef,
    ) -> Option<u128> {
        match self.native_living_target(living)? {
            Target::Player => Some(self.native_acting_id()),
            Target::Guest(conn) => self.command_guest(conn).map(|g| g.uuid),
            _ => None,
        }
    }
    /// Runs `f` with the player a handle names acting, None when it didn't run.
    pub(in crate::game::server_side) fn native_living_as_player<T>(
        &mut self,
        living: LivingRef,
        f: impl FnOnce(&mut Self) -> T,
    ) -> Option<T> {
        let uuid = self.native_living_player(living)?;
        let mut out = None;
        self.native_as_player(uuid, |game| out = Some(f(game)));
        out
    }
    /// What is stuck in the player or the mob a handle names.
    pub(in crate::game::server_side) fn native_living_stuck(
        &mut self,
        living: LivingRef,
    ) -> Option<&mut Stuck> {
        match self.native_living_target(living)? {
            Target::Player => Some(&mut self.server.combat.0.stuck),
            Target::Guest(conn) => {
                Some(&mut self.server.guest_combat.0.entry(conn).or_default().stuck)
            }
            Target::Mob(id) => self
                .common
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .map(|m| &mut m.extra.stuck),
            _ => None,
        }
    }

    /// `LivingEntity.swing(hand, DEFAULT, sendToSwingingEntity)`: starts the
    /// arm swing (shortened by haste, lengthened by mining fatigue) unless
    /// one is under way and past its first half, and tells the players
    /// around at the end of the tick, and the swinger too when
    /// `send_to_self`. True when a swing started. Mobs and players; false
    /// for armor stands and the dragon, and for a player other than the
    /// acting one while another player acts.
    pub fn living_swing(&mut self, living: LivingRef, hand: Hand, send_to_self: bool) -> bool {
        match self.native_living_target(living) {
            Some(Target::Mob(id)) => self
                .common
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .is_some_and(|m| m.swing_hand_modified(hand == Hand::Off, SwingAnimation::DEFAULT)),
            Some(Target::Player | Target::Guest(_)) => {
                let Some(uuid) = self.native_living_player(living) else {
                    return false;
                };
                let mut started = false;
                self.native_as_player(uuid, |game| {
                    let before = game.swings_started;
                    game.swing_hand(hand, SwingAnimation::DEFAULT);
                    started = game.swings_started != before;
                    let current = game.swing.current;
                    if let (true, true, Some(c), Some(id)) = (
                        started,
                        send_to_self,
                        current,
                        game.server.lan_acting_entity,
                    ) {
                        game.server.guest_packets.push(
                            sunrise_gameplay_common::game::lan_actions::swing_packet(
                                id as i32,
                                c.hand,
                                c.animation,
                            ),
                        );
                    }
                });
                started
            }
            _ => false,
        }
    }

    /// `LivingEntity.getArrowCount`: the arrows stuck in a player or a mob.
    /// None for armor stands and the dragon, which Sunrise keeps no count
    /// for.
    pub fn living_arrow_count(&mut self, living: LivingRef) -> Option<i32> {
        Some(self.native_living_stuck(living)?.arrows)
    }
    /// `LivingEntity.setArrowCount`: players around see the arrows (vanilla
    /// does not limit the count). False for armor stands and the dragon.
    pub fn living_set_arrow_count(&mut self, living: LivingRef, count: i32) -> bool {
        self.native_living_stuck(living)
            .map(|s| s.arrows = count)
            .is_some()
    }
    /// `LivingEntity.getStingerCount`: the bee stingers stuck in a player or
    /// a mob. None for armor stands and the dragon.
    pub fn living_stinger_count(&mut self, living: LivingRef) -> Option<i32> {
        Some(self.native_living_stuck(living)?.stingers)
    }
    /// `LivingEntity.setStingerCount`: players around see the stingers. False
    /// for armor stands and the dragon.
    pub fn living_set_stinger_count(&mut self, living: LivingRef, count: i32) -> bool {
        self.native_living_stuck(living)
            .map(|s| s.stingers = count)
            .is_some()
    }
    /// `LivingEntity.removeArrowTime`: the ticks until the next arrow falls
    /// out of a player or a mob; at 0 or less it restarts at
    /// `20 * (30 - arrows)`. False for armor stands and the dragon.
    pub fn living_set_arrow_removal_ticks(&mut self, living: LivingRef, ticks: i32) -> bool {
        self.native_living_stuck(living)
            .map(|s| s.remove_arrow_time = ticks)
            .is_some()
    }
    /// `LivingEntity.removeStingerTime`, as `set_arrow_removal_ticks` for the
    /// stingers.
    pub fn living_set_stinger_removal_ticks(&mut self, living: LivingRef, ticks: i32) -> bool {
        self.native_living_stuck(living)
            .map(|s| s.remove_stinger_time = ticks)
            .is_some()
    }

    /// `LivingEntity.getUseItemRemainingTicks`: the ticks left of the item a
    /// player or a mob uses, 0 when it uses none. A mob's is the item's use
    /// duration less the ticks its use counter has run. None for armor stands
    /// and the dragon.
    pub fn living_item_use_remaining(&self, living: LivingRef) -> Option<i32> {
        match self.native_living_target(living)? {
            Target::Player => Some(
                sunrise_gameplay_common::game::item_use_common::remaining_of(
                    self.common.using,
                    self.common.use_duration,
                ),
            ),
            Target::Guest(conn) => Some(self.server.lan.as_ref()?.play.get(&conn)?.use_remaining()),
            Target::Mob(id) => self.mob_item_use_remaining(id),
            _ => None,
        }
    }
    /// Sets `LivingEntity.useItemRemaining` of the item a player uses (the
    /// way a mod's tick moves the end of the use): at 1 the use completes on
    /// the next tick, at 0 or less it runs until the player lets go, as in
    /// vanilla. It moves the deadline the use-item Tick hook moves, so only
    /// the uses that count it (food, drink, mod items) follow it: shield, bow,
    /// trident, spear, instrument and bundle uses ignore it, and a call from
    /// inside the Tick hook itself is overwritten by that hook's result.
    ///
    /// A mob's use never runs out, so the write moves its use counter to where
    /// `ticks` are left; what reads the counter (a spear's stabs, a bow's draw)
    /// follows it. False when the entity uses nothing, and for armor stands and
    /// the dragon.
    pub fn living_set_item_use_remaining(&mut self, living: LivingRef, ticks: i32) -> bool {
        match self.native_living_target(living) {
            Some(Target::Mob(id)) => self.mob_set_item_use_remaining(id, ticks),
            Some(Target::Player) => sunrise_gameplay_common::game::item_use_common::set_remaining(
                self.common.using,
                &mut self.common.use_duration,
                ticks,
            ),
            Some(Target::Guest(conn)) => self
                .server
                .lan
                .as_mut()
                .and_then(|h| h.play.get_mut(&conn))
                .is_some_and(|p| p.set_use_remaining(ticks)),
            _ => false,
        }
    }
    /// `LivingEntity.startUsingItem`: starts using the item in `hand`, for its
    /// `getUseDuration` as a mod's use-item Start hook left it, without the
    /// item's own `use` (no ammo check, no sound). True when the use started:
    /// false for an empty hand, a use under way, a refused Start, armor stands
    /// and the dragon, and a player other than the acting one while another
    /// player acts.
    ///
    /// A player's client lets go of a use whose button isn't down, as in
    /// vanilla (`Minecraft.handleKeybinds`), so that use lasts until its next
    /// tick, which releases it; an item with no use of its own ends then too.
    /// A mob's use shows and its goals see it, but it never runs out: only
    /// `release_using_item` or a goal ends it. False for a piglin with
    /// something in its off hand and a drinking witch, whose counter is their
    /// own timer.
    pub fn living_start_using_item(&mut self, living: LivingRef, hand: Hand) -> bool {
        match self.native_living_target(living) {
            Some(Target::Mob(id)) => self.start_mob_using_item(id, hand),
            Some(Target::Player | Target::Guest(_)) => self
                .native_living_as_player(living, |game| {
                    game.with_hand(hand, Self::start_using_item)
                })
                .unwrap_or(false),
            _ => false,
        }
    }
    /// `LivingEntity.releaseUsingItem`: lets go of the item in use as the use
    /// button does: the use-item Stop hook, the item's own release (a player's
    /// bow or trident shoots by how long it was held) and a mod's
    /// `release_using`, then the use is over. True when a use was released:
    /// false when none runs, for armor stands and the dragon, and for a player
    /// other than the acting one while another player acts. A mob's bow or
    /// trident has no release of its own, as in vanilla.
    pub fn living_release_using_item(&mut self, living: LivingRef) -> bool {
        match self.native_living_target(living) {
            Some(Target::Mob(id)) => self.release_mob_using_item(id),
            Some(Target::Player | Target::Guest(_)) => self
                .native_living_as_player(living, Self::release_using_item)
                .unwrap_or(false),
            _ => false,
        }
    }

    /// `LivingEntity.startAutoSpinAttack(ticks, damage)` of a player: the
    /// spin of a Riptide launch for `ticks` ticks, ended by the first mob its
    /// box touches, which takes `damage`. Its motion counts it down and the
    /// players around see the spin (living flag 4). A `ticks` of 0 or less ends
    /// the spin and clears its damage. False for the other kinds, a dead
    /// player and a non-finite `damage`.
    pub fn living_set_riptiding(&mut self, living: LivingRef, ticks: i32, damage: f32) -> bool {
        if !damage.is_finite() {
            return false;
        }
        self.native_living_as_player(living, |game| {
            let Some(p) = game.player.as_mut() else {
                return false;
            };
            p.auto_spin_attack_ticks = ticks.max(0);
            p.auto_spin_attack_dmg = if ticks > 0 { damage } else { 0.0 };
            true
        })
        .unwrap_or(false)
    }

    /// `/waypoint modify <entity> style`: `edit` changes the icon of the
    /// waypoint a handle names, which is then tracked again. False for the
    /// dragon.
    pub(in crate::game::server_side) fn native_edit_waypoint_icon(
        &mut self,
        living: LivingRef,
        edit: impl FnOnce(&mut Icon),
    ) -> bool {
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_) | Target::Mob(_) | Target::Stand(_))) => {
                let mut icon = self.waypoint_icon(t);
                edit(&mut icon);
                self.set_waypoint_icon(t, icon);
                true
            }
            _ => false,
        }
    }
    /// `Waypoint.Icon.style` of the waypoint a handle names, as
    /// `/waypoint modify <entity> style` sets it: a player's reaches the
    /// players it transmits to. False for the dragon.
    pub fn living_set_waypoint_style(&mut self, living: LivingRef, style: &Identifier) -> bool {
        self.native_edit_waypoint_icon(living, |icon| icon.style = style.to_string())
    }
    /// `Waypoint.Icon.color` of the waypoint a handle names: the RGB colour,
    /// or none to show a player in their team's colour. False for the dragon.
    pub fn living_set_waypoint_color(&mut self, living: LivingRef, rgb: Option<u32>) -> bool {
        self.native_edit_waypoint_icon(living, |icon| {
            icon.color = rgb.map(|rgb| (rgb & 0xFF_FFFF) as i32);
        })
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

#[cfg(test)]
#[path = "native_living_writes_observer_tests.rs"]
mod observer_tests;

#[cfg(test)]
#[path = "native_living_writes_client.rs"]
mod use_fixture_tests;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::Stuck;
    use super::observer_tests::tests::Uses;
    use mod_api::client::first_person::Hand;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::{
        native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
        native_living_body::NativeLivingBodyGameExt3 as _,
        player_identity::PlayerIdentityGameExt1 as _,
    };
    use sunrise_items::items::ItemStack;
    use sunrise_sim::mobs::Mob;

    #[allow(unused_imports)]
    pub(in crate::game) use super::use_fixture_tests::tests::with_uses;

    #[test]
    #[ignore = "daily"]
    fn tick_cancel_and_nonpositive_durations_finish_without_releasing() {
        with_uses(|game, uses, player| {
            game.creative = false;
            for (cancel, duration) in [
                (true, None),
                (false, Some(0)),
                (false, Some(-7)),
                (false, Some(1)),
            ] {
                for item in ["minecraft:apple", "minecraft:potion", "minecraft:bow"] {
                    *uses.lock().unwrap() = Uses {
                        cancel_tick: cancel,
                        tick_duration: duration,
                        ..Uses::default()
                    };
                    game.potions.effects.clear();
                    let stack = ItemStack::new(item, 2);
                    game.common.inventory[game.common.selected] =
                        Some(if item == "minecraft:potion" {
                            stack.with_potion("minecraft:swiftness")
                        } else {
                            stack
                        });
                    game.inventory[5] = Some(ItemStack::new("minecraft:arrow", 8));
                    assert!(game.living_start_using_item(player, Hand::Main));
                    let projectiles = game.creatures.projectiles.len();
                    game.tick_using_server();
                    assert_eq!(game.using, None, "{item} {cancel} {duration:?}");
                    assert_eq!(game.creatures.projectiles.len(), projectiles);
                    let expected_count = if item == "minecraft:bow" { 2 } else { 1 };
                    assert_eq!(
                        game.inventory[game.selected].as_ref().unwrap().count,
                        expected_count
                    );
                    let uses = uses.lock().unwrap();
                    assert_eq!(uses.finishes, 1);
                    assert!(uses.stops.is_empty());
                    assert!(uses.releases.is_empty());
                    if item == "minecraft:apple" {
                        assert_eq!(uses.effects, usize::from(duration == Some(1) && !cancel));
                    } else if item == "minecraft:potion" {
                        assert!(
                            game.potions.effects.get("speed").is_some(),
                            "potion effect was not applied"
                        );
                    }
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_crossbows_run_out_counter_keeps_decrementing_until_release() {
        with_uses(|game, uses, player| {
            for (cancel, duration, first, second) in [
                (true, None, -2, -3),
                (false, Some(0), -1, -2),
                (false, Some(1), 0, -1),
                (false, Some(-7), -8, -9),
            ] {
                *uses.lock().unwrap() = Uses {
                    cancel_tick: cancel,
                    tick_duration: duration,
                    ..Uses::default()
                };
                game.common.inventory[game.common.selected] =
                    Some(ItemStack::new("minecraft:crossbow", 1));
                assert!(game.living_start_using_item(player, Hand::Main));
                game.tick_using_server();
                assert_eq!(game.living_item_use_remaining(player), Some(first));
                uses.lock().unwrap().cancel_tick = false;
                uses.lock().unwrap().tick_duration = None;
                game.tick_using_server();
                assert_eq!(game.living_item_use_remaining(player), Some(second));
                assert_eq!(game.living_used_item_hand(player), Some(Hand::Main));
                assert_eq!(uses.lock().unwrap().finishes, 0);
                assert!(game.living_release_using_item(player));
                assert_eq!(game.using, None);
                let uses = uses.lock().unwrap();
                assert_eq!(uses.stops, [(Some(20), Hand::Main, second)]);
                assert_eq!(uses.tick_remaining.last(), Some(&first));
                assert_eq!(uses.releases, [(Some(20), second)]);
                assert_eq!(uses.finishes, 0);
            }
        });
    }

    /// Every item-use hook names the player who uses, as NeoForge's
    /// `LivingEntityUseItemEvent` carries its entity: an apple eaten (Start,
    /// the may-still-use check, Tick, `onUseTick`, Finish), then a bow drawn
    /// and let go (Stop, `releaseUsing`), each use ending in `onStopUsing`.
    #[test]
    #[ignore = "daily"]
    fn the_item_use_hooks_name_the_player_who_uses() {
        with_uses(|game, uses, player| {
            assert_eq!(game.own_entity_id(), 20);
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:apple", 2));
            uses.lock().unwrap().start_duration = Some(2);
            assert!(game.living_start_using_item(player, Hand::Main));
            for _ in 0..3 {
                game.tick_using_server();
            }
            assert_eq!(game.using, None);
            assert_eq!(uses.lock().unwrap().finishes, 1);
            uses.lock().unwrap().start_duration = None;
            game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:bow", 1));
            assert!(game.living_start_using_item(player, Hand::Main));
            game.tick_using_server();
            assert!(game.living_release_using_item(player));
            let heard = std::mem::take(&mut uses.lock().unwrap().heard);
            for hook in [
                "start",
                "can_continue_using",
                "tick",
                "on_use_tick",
                "finish",
                "stop",
                "release_using",
                "on_stop_using",
            ] {
                assert!(
                    heard.iter().any(|(h, _)| *h == hook),
                    "{hook} not heard: {heard:?}"
                );
            }
            assert!(
                heard.iter().all(|(_, entity)| *entity == Some(20)),
                "{heard:?}"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn tick_completion_applies_the_finish_listeners_result_stack() {
        with_uses(|game, uses, player| {
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:apple", 2));
            {
                let mut uses = uses.lock().unwrap();
                uses.cancel_tick = true;
                uses.clear_finish_result = true;
            }
            assert!(game.living_start_using_item(player, Hand::Main));
            game.tick_using_server();
            assert_eq!(game.using, None);
            assert!(game.inventory[game.selected].is_none());
            let uses = uses.lock().unwrap();
            assert_eq!(uses.finishes, 1);
            assert!(uses.stops.is_empty());
            assert!(uses.releases.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_zero_start_begins_but_a_cancelled_stop_only_skips_release() {
        with_uses(|game, uses, player| {
            game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:bow", 1));
            uses.lock().unwrap().start_duration = Some(0);
            assert!(game.living_start_using_item(player, Hand::Main));
            assert!(game.using.is_some());
            game.tick_using_server();
            assert_eq!(game.using, None);
            assert_eq!(uses.lock().unwrap().finishes, 1);
            *uses.lock().unwrap() = Uses {
                cancel_stop: true,
                ..Uses::default()
            };
            assert!(game.living_start_using_item(player, Hand::Main));
            assert!(game.living_release_using_item(player));
            assert_eq!(game.using, None);
            let uses = uses.lock().unwrap();
            assert_eq!(uses.stops.len(), 1);
            assert!(uses.releases.is_empty());
            assert_eq!(uses.finishes, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_tick_replaces_the_remaining_counter_rather_than_the_original_duration() {
        with_uses(|game, uses, player| {
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:shield", 1));
            assert!(game.living_start_using_item(player, Hand::Main));
            for _ in 0..10 {
                game.tick_using_server();
            }
            *uses.lock().unwrap() = Uses {
                tick_duration: Some(5),
                ..Uses::default()
            };
            game.tick_using_server();
            uses.lock().unwrap().tick_duration = None;
            assert_eq!(game.living_item_use_remaining(player), Some(4));
            for expected in [3, 2, 1] {
                game.tick_using_server();
                assert_eq!(game.living_item_use_remaining(player), Some(expected));
            }
            game.tick_using_server();
            assert_eq!(game.using, None);
            let uses = uses.lock().unwrap();
            assert_eq!(uses.tick_remaining, [71_990, 4, 3, 2, 1]);
            assert_eq!(uses.effects, 5);
            assert_eq!(uses.finishes, 1);
            assert!(uses.releases.is_empty());
            assert!(uses.stops.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_item_part_can_extend_the_last_tick_or_shorten_a_use() {
        with_uses(|game, uses, player| {
            for (start, replacement, effects) in [(1, 3, 3), (20, 1, 1), (20, 0, 1), (20, -7, 1)] {
                *uses.lock().unwrap() = Uses {
                    start_duration: Some(start),
                    remaining_on_use_tick: Some((player, replacement)),
                    ..Uses::default()
                };
                game.common.inventory[game.common.selected] =
                    Some(ItemStack::new("minecraft:shield", 1));
                assert!(game.living_start_using_item(player, Hand::Main));
                game.tick_using_server();
                if replacement > 1 {
                    assert_eq!(game.living_item_use_remaining(player), Some(2));
                    assert_eq!(uses.lock().unwrap().finishes, 0);
                    game.tick_using_server();
                    assert_eq!(game.living_item_use_remaining(player), Some(1));
                    assert_eq!(uses.lock().unwrap().finishes, 0);
                    game.tick_using_server();
                }
                assert_eq!(game.using, None);
                let uses = uses.lock().unwrap();
                assert_eq!(uses.effects, effects);
                assert_eq!(uses.finishes, 1);
                assert!(uses.stops.is_empty());
                assert!(uses.releases.is_empty());
                assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 1);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_item_used_in_either_hand_is_released_once() {
        with_uses(|game, uses, player| {
            uses.lock().unwrap().release_on_stop = Some(player);
            game.selected = 3;
            game.inventory[5] = Some(ItemStack::new("minecraft:arrow", 8));
            for (round, hand) in [Hand::Main, Hand::Off].into_iter().enumerate() {
                let bow = Some(ItemStack::new("minecraft:bow", 1));
                game.inventory[3] = None;
                game.offhand = None;
                assert!(!game.living_start_using_item(player, hand));
                match hand {
                    Hand::Main => game.inventory[3] = bow,
                    Hand::Off => game.offhand = bow,
                }
                assert_eq!(game.living_used_item_hand(player), None);
                assert!(game.living_start_using_item(player, hand));
                assert!(!game.living_start_using_item(player, hand));
                assert_eq!(game.living_used_item_hand(player), Some(hand));
                assert_eq!(game.living_item_use_remaining(player), Some(72_000));
                for _ in 0..25 {
                    game.tick_using_server();
                }
                assert_eq!(game.living_used_item_hand(player), Some(hand));
                assert_eq!(game.living_item_use_remaining(player), Some(71_975));

                let arrows = game.creatures.projectiles.len();
                assert!(game.living_release_using_item(player));
                assert_eq!(game.creatures.projectiles.len(), arrows + 1);
                assert_eq!(game.living_used_item_hand(player), None);
                assert_eq!(game.living_item_use_remaining(player), Some(0));
                assert!(!game.living_release_using_item(player));
                assert_eq!(game.creatures.projectiles.len(), arrows + 1);
                let uses = uses.lock().unwrap();
                assert_eq!(uses.stops.len(), round + 1);
                assert_eq!(uses.stops[round], (Some(20), hand, 71_975));
                assert_eq!(uses.releases.len(), round + 1);
                assert_eq!(uses.releases[round], (Some(20), 71_975));
                assert_eq!(uses.released_again, vec![false; round + 1]);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_start_a_mod_refuses_starts_nothing() {
        with_uses(|game, uses, player| {
            game.offhand = Some(ItemStack::new("minecraft:bow", 1));
            uses.lock().unwrap().refuse = true;
            assert!(!game.living_start_using_item(player, Hand::Off));
            assert_eq!(game.living_used_item_hand(player), None);
        });
    }

    /// A player whose use button is up lets go on the next tick.
    #[test]
    #[ignore = "daily"]
    fn a_use_with_the_button_up_is_released_on_its_next_tick() {
        with_uses(|game, uses, player| {
            game.server_set_acting_use(false);
            game.offhand = Some(ItemStack::new("minecraft:bow", 1));
            assert!(game.living_start_using_item(player, Hand::Off));
            game.tick_using_server();
            assert_eq!(game.living_used_item_hand(player), None);
            assert_eq!(uses.lock().unwrap().stops, [(Some(20), Hand::Off, 72_000)]);
        });
    }

    /// A use a mod's own tick released stays released, and finishes nothing:
    /// the tick that called it has no use left to go on with.
    #[test]
    #[ignore = "daily"]
    fn a_use_released_from_its_own_tick_stays_released() {
        with_uses(|game, uses, player| {
            let items = ["minecraft:apple", "minecraft:potion", "minecraft:shield"];
            for (item, from_item) in items.into_iter().flat_map(|i| [(i, false), (i, true)]) {
                let stack = ItemStack::new(item, 2);
                game.offhand = Some(stack.clone());
                assert!(game.living_start_using_item(player, Hand::Off));
                {
                    let mut uses = uses.lock().unwrap();
                    uses.release_on_tick = (!from_item).then_some(player);
                    uses.release_on_use_tick = from_item.then_some(player);
                }
                game.tick_using_server();
                assert_eq!(game.living_used_item_hand(player), None);
                {
                    let mut uses = uses.lock().unwrap();
                    (uses.release_on_tick, uses.release_on_use_tick) = (None, None);
                }
                game.tick_using_server();
                assert_eq!(game.living_used_item_hand(player), None);
                assert_eq!(game.offhand, Some(stack));
            }
            assert_eq!(uses.lock().unwrap().stops.len(), 6);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mob_uses_either_hand_until_it_is_released() {
        use sunrise_items::equipment::{MAINHAND, OFFHAND};
        with_uses(|game, uses, _| {
            crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
                game,
                sunrise_core::position::ChunkPos::new(0, 0),
            );
            let kinds = [
                (12, "minecraft:zombie"),
                (13, "minecraft:piglin"),
                (14, "minecraft:witch"),
            ];
            for (id, kind) in kinds {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let piglin = game.living(game.entity(13).unwrap()).unwrap();
            for (round, (hand, slot)) in [(Hand::Main, MAINHAND), (Hand::Off, OFFHAND)]
                .into_iter()
                .enumerate()
            {
                assert!(!game.living_start_using_item(zombie, hand));
                let bow = ItemStack::new("minecraft:bow", 1);
                game.mobs[0].extra.set_slot_stack(slot, Some(bow));
                assert_eq!(game.living_used_item_hand(zombie), None);
                assert!(game.living_start_using_item(zombie, hand));
                assert!(!game.living_start_using_item(zombie, hand));
                assert_eq!(game.living_used_item_hand(zombie), Some(hand));
                // Its clients are told through the counter and the hand.
                assert_eq!(game.mobs[0].extra.using_item, 1);
                assert_eq!(game.mobs[0].extra.using_off_hand, hand == Hand::Off);
                // A bow is used for 72000 ticks, and a write moves the counter
                // to where 100 are left of them.
                assert_eq!(game.living_item_use_remaining(zombie), Some(72_000));
                assert!(game.living_set_item_use_remaining(zombie, 100));
                assert_eq!(game.mobs[0].extra.using_item, 71_901);
                assert_eq!(game.living_item_use_remaining(zombie), Some(100));

                game.mobs[0].extra.using_item = 6;
                let arrows = game.creatures.projectiles.len();
                assert!(game.living_release_using_item(zombie));
                assert_eq!(game.living_used_item_hand(zombie), None);
                assert!(!game.living_release_using_item(zombie));
                // Nothing in use: nothing left, and nothing to move.
                assert_eq!(game.living_item_use_remaining(zombie), Some(0));
                assert!(!game.living_set_item_use_remaining(zombie, 5));
                // `BowItem.releaseUsing` shoots for a player only.
                assert_eq!(game.creatures.projectiles.len(), arrows);
                let uses = uses.lock().unwrap();
                assert_eq!(uses.stops.len(), round + 1);
                assert_eq!(uses.stops[round], (Some(12), hand, 71_995));
                assert_eq!(uses.releases[round], (Some(12), 71_995));
                drop(uses);
                game.mobs[0].extra.set_slot_stack(slot, None);
            }

            // A piglin's off hand makes the counter its admiring timer.
            let gold = ItemStack::new("minecraft:gold_ingot", 1);
            game.mobs[1]
                .extra
                .set_slot_stack(OFFHAND, Some(gold.clone()));
            game.mobs[1].extra.set_slot_stack(MAINHAND, Some(gold));
            assert!(!game.living_start_using_item(piglin, Hand::Main));
            assert_eq!(game.mobs[1].extra.using_item, 0);
            assert_eq!(game.living_item_use_remaining(piglin), Some(0));

            // A witch's drink is its own timer: no use to read, end or replace.
            let witch = game.living(game.entity(14).unwrap()).unwrap();
            let potion = ItemStack::new("minecraft:potion", 1);
            game.mobs[2].extra.set_slot_stack(MAINHAND, Some(potion));
            game.mobs[2].extra.using_item = 10;
            game.mobs[2].extra.flag = true;
            assert_eq!(game.living_used_item_hand(witch), None);
            assert!(!game.living_start_using_item(witch, Hand::Main));
            assert!(!game.living_release_using_item(witch));
            assert_eq!(game.living_item_use_remaining(witch), Some(0));
            assert!(!game.living_set_item_use_remaining(witch, 5));
            assert_eq!(game.mobs[2].extra.using_item, 10);
        });
    }

    #[test]
    fn a_huge_stuck_count_ticks_without_overflow() {
        let mut stuck = Stuck {
            arrows: i32::MAX,
            stingers: i32::MIN + 1,
            ..Stuck::default()
        };
        stuck.tick();
        assert!(stuck.arrows >= i32::MAX - 1);
    }
}
