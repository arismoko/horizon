//! Writes to a living entity for native server mods, through a `LivingRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles"): `LivingEntity.swing`,
//! the arrows and bee stingers stuck in a player (`setArrowCount`,
//! `setStingerCount`, `removeArrowTime`, `removeStingerTime`),
//! `Mob.setLeftHanded`, `Mob.setCanPickUpLoot`, `useItemRemaining`, and
//! `getUsedItemHand`, `startUsingItem` and `releaseUsingItem`. Reads
//! return None and writes false for a stale handle, on a client, and for a
//! kind Horizon keeps no such state for (each doc says which).
use super::{Game, combat::Stuck, commands::Target};
use horizon_client::first_person::{Hand, SwingAnimation};
use horizon_client::mobs::spear_use;
use mod_api::LivingRef;

impl Game<'_> {
    /// The UUID of the player a living handle names.
    fn native_living_player(&mut self, living: LivingRef) -> Option<u128> {
        match self.native_living_target(living)? {
            Target::Player => Some(self.native_acting_id()),
            Target::Guest(conn) => self.command_guest(conn).map(|g| g.uuid),
            _ => None,
        }
    }
    /// Runs `f` with the player a handle names acting, None when it didn't run.
    fn native_living_as_player<T>(
        &mut self,
        living: LivingRef,
        f: impl FnOnce(&mut Self) -> T,
    ) -> Option<T> {
        let uuid = self.native_living_player(living)?;
        let mut out = None;
        self.native_as_player(uuid, |game| out = Some(f(game)));
        out
    }
    /// What is stuck in the player a handle names.
    fn native_living_stuck(&mut self, living: LivingRef) -> Option<&mut Stuck> {
        match self.native_living_target(living)? {
            Target::Player => Some(&mut self.combat.stuck),
            Target::Guest(conn) => Some(&mut self.guest_combat.entry(conn).or_default().stuck),
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
    pub(super) fn living_swing(
        &mut self,
        living: LivingRef,
        hand: Hand,
        send_to_self: bool,
    ) -> bool {
        match self.native_living_target(living) {
            Some(Target::Mob(id)) => {
                self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                    m.swing_hand_modified(hand == Hand::Off, SwingAnimation::DEFAULT)
                })
            }
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
                    if let (true, true, Some(c), Some(id)) =
                        (started, send_to_self, current, game.lan_acting_entity)
                    {
                        game.guest_packets.push(super::lan_actions::swing_packet(
                            id as i32,
                            c.hand,
                            c.animation,
                        ));
                    }
                });
                started
            }
            _ => false,
        }
    }

    /// `LivingEntity.getArrowCount`: the arrows stuck in a player. None for
    /// the other kinds, which Horizon keeps no count for.
    pub(super) fn living_arrow_count(&mut self, living: LivingRef) -> Option<i32> {
        Some(self.native_living_stuck(living)?.arrows)
    }
    /// `LivingEntity.setArrowCount`: players around see the arrows (vanilla
    /// does not limit the count). False for the other kinds.
    pub(super) fn living_set_arrow_count(&mut self, living: LivingRef, count: i32) -> bool {
        self.native_living_stuck(living)
            .map(|s| s.arrows = count)
            .is_some()
    }
    /// `LivingEntity.getStingerCount`: the bee stingers stuck in a player.
    /// None for the other kinds.
    pub(super) fn living_stinger_count(&mut self, living: LivingRef) -> Option<i32> {
        Some(self.native_living_stuck(living)?.stingers)
    }
    /// `LivingEntity.setStingerCount`: players around see the stingers. False
    /// for the other kinds.
    pub(super) fn living_set_stinger_count(&mut self, living: LivingRef, count: i32) -> bool {
        self.native_living_stuck(living)
            .map(|s| s.stingers = count)
            .is_some()
    }
    /// `LivingEntity.removeArrowTime`: the ticks until the next arrow falls
    /// out of a player; at 0 or less it restarts at `20 * (30 - arrows)`.
    /// False for the other kinds.
    pub(super) fn living_set_arrow_removal_ticks(&mut self, living: LivingRef, ticks: i32) -> bool {
        self.native_living_stuck(living)
            .map(|s| s.remove_arrow_time = ticks)
            .is_some()
    }
    /// `LivingEntity.removeStingerTime`, as `set_arrow_removal_ticks` for the
    /// stingers.
    pub(super) fn living_set_stinger_removal_ticks(
        &mut self,
        living: LivingRef,
        ticks: i32,
    ) -> bool {
        self.native_living_stuck(living)
            .map(|s| s.remove_stinger_time = ticks)
            .is_some()
    }

    /// `LivingEntity.getUseItemRemainingTicks`: the ticks left of the item a
    /// player uses, 0 when they use none. None for the other kinds (a mob's
    /// item use is a counter of its goal, not this timer).
    pub(super) fn living_item_use_remaining(&self, living: LivingRef) -> Option<i32> {
        match self.native_living_target(living)? {
            Target::Player => Some(super::item_use::remaining_of(self.using, self.use_duration)),
            Target::Guest(conn) => Some(self.lan.as_ref()?.play.get(&conn)?.use_remaining()),
            _ => None,
        }
    }
    /// Sets `LivingEntity.useItemRemaining` of the item a player uses (the
    /// way a mod's tick moves the end of the use): at 1 the use completes on
    /// the next tick, at 0 or less it runs until the player lets go, as in
    /// vanilla. It moves the deadline the use-item Tick hook moves, so only
    /// the uses that count it (food, drink, mod items) follow it: shield, bow,
    /// trident, spear, instrument and bundle uses ignore it, and a call from
    /// inside the Tick hook itself is overwritten by that hook's result. False
    /// when the player uses nothing, and for the other kinds.
    pub(super) fn living_set_item_use_remaining(&mut self, living: LivingRef, ticks: i32) -> bool {
        match self.native_living_target(living) {
            Some(Target::Player) => {
                super::item_use::set_remaining(self.using, &mut self.use_duration, ticks)
            }
            Some(Target::Guest(conn)) => self
                .lan
                .as_mut()
                .and_then(|h| h.play.get_mut(&conn))
                .is_some_and(|p| p.set_use_remaining(ticks)),
            _ => false,
        }
    }
    /// `LivingEntity.getUsedItemHand` while `isUsingItem`: the hand of the item
    /// a player or a mob uses, None when it uses none. On a client, the local
    /// player's only.
    pub(in crate::game) fn living_used_item_hand(&self, living: LivingRef) -> Option<Hand> {
        match self.native_living_read_target(living)? {
            Target::Player => self.used_hand(),
            Target::Guest(conn) => self.lan.as_ref()?.play.get(&conn)?.used_hand(),
            Target::Mob(id) => {
                let mob = self.mobs.iter().find(|m| m.id == id)?;
                spear_use::is_using_item(mob).then_some(if mob.extra.using_off_hand {
                    Hand::Off
                } else {
                    Hand::Main
                })
            }
            _ => None,
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
    pub(in crate::game) fn living_start_using_item(
        &mut self,
        living: LivingRef,
        hand: Hand,
    ) -> bool {
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
    pub(in crate::game) fn living_release_using_item(&mut self, living: LivingRef) -> bool {
        match self.native_living_target(living) {
            Some(Target::Mob(id)) => self.release_mob_using_item(id),
            Some(Target::Player | Target::Guest(_)) => self
                .native_living_as_player(living, Self::release_using_item)
                .unwrap_or(false),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::Stuck;
    use horizon_client::items::ItemStack;
    use horizon_client::mobs::Mob;
    use horizon_core::geometry::Vec3;
    use mod_api::first_person::Hand;
    use mod_api::{
        EntityRef, Flow, Hooks, HostSide, ItemPart, LivingEntityUseItemStartEvent,
        LivingEntityUseItemStopEvent, LivingEntityUseItemTickEvent, LivingRef, ModEntry, Mods,
        NativeWorldEpoch, OnUseTickEvent, Registrar, ReleaseUsingEvent,
    };
    use std::sync::{Arc, Mutex};

    /// What a mod heard of item uses, and what it does back.
    #[derive(Default)]
    struct Uses {
        stops: Vec<(Option<i32>, Hand, i32)>,
        releases: Vec<(Option<i32>, i32)>,
        /// Refuses every Start.
        refuse: bool,
        /// Who to release from a Tick hook, and from an `Item.onUseTick`.
        release_on_tick: Option<LivingRef>,
        release_on_use_tick: Option<LivingRef>,
        /// Who to release once more from the Stop hook, and what that answered.
        release_on_stop: Option<LivingRef>,
        released_again: Vec<bool>,
    }
    type Shared = Arc<Mutex<Uses>>;

    #[derive(Clone)]
    struct UsePart(Shared);
    #[mod_api::horizon_part]
    impl ItemPart for UsePart {
        fn on_use_tick(&self, host: HostSide, _: &mut OnUseTickEvent<'_>) {
            let who = self.0.lock().unwrap().release_on_use_tick;
            if let (Some(who), HostSide::Server(host)) = (who, host) {
                assert!(who.release_using_item(host));
            }
        }
        fn release_using(&self, _: HostSide, event: &mut ReleaseUsingEvent<'_>) {
            let mut uses = self.0.lock().unwrap();
            uses.releases.push((event.entity, event.time_left));
        }
    }
    struct UseMod(Shared);
    #[mod_api::__subscriptions]
    impl Hooks for UseMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            for id in [
                "minecraft:bow",
                "minecraft:apple",
                "minecraft:potion",
                "minecraft:shield",
            ] {
                let item = r.item_key(id);
                r.item_part(item, UsePart(self.0.clone()));
            }
        }
        fn living_entity_use_item_start(
            &self,
            _: HostSide,
            _: &mut LivingEntityUseItemStartEvent<'_>,
        ) -> Flow {
            if self.0.lock().unwrap().refuse {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn living_entity_use_item_tick(
            &self,
            host: HostSide,
            _: &mut LivingEntityUseItemTickEvent<'_>,
        ) -> Flow {
            let who = self.0.lock().unwrap().release_on_tick;
            if let (Some(who), HostSide::Server(host)) = (who, host) {
                assert!(who.release_using_item(host));
            }
            Flow::Continue
        }
        fn living_entity_use_item_stop(
            &self,
            host: HostSide,
            event: &mut LivingEntityUseItemStopEvent<'_>,
        ) -> Flow {
            let who = {
                let mut uses = self.0.lock().unwrap();
                uses.stops.push((event.entity, event.hand, event.duration));
                uses.release_on_stop
            };
            if let (Some(who), HostSide::Server(host)) = (who, host) {
                let again = who.release_using_item(host);
                self.0.lock().unwrap().released_again.push(again);
            }
            Flow::Continue
        }
    }

    /// Runs `test` on a game whose acting player stands at the origin, the use
    /// button down, under a mod that fills the returned record.
    fn with_uses(test: impl FnOnce(&mut super::Game<'_>, &Shared, LivingRef)) {
        let uses = Shared::default();
        let mods: Mods = vec![ModEntry::new("uses", Box::new(UseMod(uses.clone())))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::testing::with_mods(mods, || {
            with_game(|game| {
                game.resolve_parts().unwrap();
                game.player = Some(
                    horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap(),
                );
                game.lan_acting_uuid = Some(123);
                game.lan_acting_entity = Some(20);
                game.screen = super::super::Screen::Playing;
                game.right = true;
                let player = game.living(game.entity(20).unwrap()).unwrap();
                test(game, &uses, player);
                crate::game::api_values::install_parts(&[]).unwrap();
                game.resolve_parts().unwrap();
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
                    game.tick_using();
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
                assert_eq!(uses.stops[round], (None, hand, 71_975));
                assert_eq!(uses.releases.len(), round + 1);
                assert_eq!(uses.releases[round], (None, 71_975));
                assert_eq!(uses.released_again, vec![false; round + 1]);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_use_with_the_button_up_is_released_on_its_next_tick() {
        with_uses(|game, uses, player| {
            game.right = false;
            game.offhand = Some(ItemStack::new("minecraft:bow", 1));
            assert!(game.living_start_using_item(player, Hand::Off));
            game.tick_using();
            assert_eq!(game.living_used_item_hand(player), None);
            assert_eq!(uses.lock().unwrap().stops, [(None, Hand::Off, 72_000)]);
        });
    }

    /// A use a mod's own tick released stays released, and finishes nothing:
    /// the tick that called it has no use left to go on with.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
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
                game.tick_using();
                assert_eq!(game.living_used_item_hand(player), None);
                {
                    let mut uses = uses.lock().unwrap();
                    (uses.release_on_tick, uses.release_on_use_tick) = (None, None);
                }
                game.tick_using();
                assert_eq!(game.living_used_item_hand(player), None);
                assert_eq!(game.offhand, Some(stack));
            }
            assert_eq!(uses.lock().unwrap().stops.len(), 6);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mob_uses_either_hand_until_it_is_released() {
        use horizon_items::equipment::{MAINHAND, OFFHAND};
        with_uses(|game, uses, _| {
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

                game.mobs[0].extra.using_item = 6;
                let arrows = game.creatures.projectiles.len();
                assert!(game.living_release_using_item(zombie));
                assert_eq!(game.living_used_item_hand(zombie), None);
                assert!(!game.living_release_using_item(zombie));
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

            // A witch's drink is its own timer: no use to read, end or replace.
            let witch = game.living(game.entity(14).unwrap()).unwrap();
            let potion = ItemStack::new("minecraft:potion", 1);
            game.mobs[2].extra.set_slot_stack(MAINHAND, Some(potion));
            game.mobs[2].extra.using_item = 10;
            game.mobs[2].extra.flag = true;
            assert_eq!(game.living_used_item_hand(witch), None);
            assert!(!game.living_start_using_item(witch, Hand::Main));
            assert!(!game.living_release_using_item(witch));
            assert_eq!(game.mobs[2].extra.using_item, 10);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mob_swings_while_its_hand_and_loot_flags_live_on_the_mob_handle() {
        with_game(|game| {
            for (id, kind) in [(12, "minecraft:zombie"), (13, "minecraft:cow")] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.living(game.entity(12).unwrap()).unwrap();

            assert!(game.living_swing(zombie, Hand::Off, false));
            assert!(game.mobs[0].ai.swinging);
            assert_eq!(game.mobs[0].extra.swing_right_arm, Some(false));
            // In the first half of a swing it doesn't restart, past it does.
            game.mobs[0].ai.swing_time = 1;
            assert!(!game.living_swing(zombie, Hand::Main, false));
            game.mobs[0].ai.swing_time = 3;
            assert!(game.living_swing(zombie, Hand::Main, false));
            assert_eq!(game.mobs[0].ai.swing_time, -1);

            // A mob keeps no stuck arrows, item use timer or player state.
            assert_eq!(game.living_arrow_count(zombie), None);
            assert!(!game.living_set_arrow_count(zombie, 3));
            assert!(!game.living_set_stinger_count(zombie, 3));
            assert!(!game.living_set_arrow_removal_ticks(zombie, 3));
            assert_eq!(game.living_item_use_remaining(zombie), None);
            assert!(!game.living_set_item_use_remaining(zombie, 3));

            let stale =
                EntityRef::new(NativeWorldEpoch::new(zombie.entity().epoch().get() + 1), 12);
            let stale = mod_api::LivingRef::new(stale);
            assert!(!game.living_swing(stale, Hand::Main, true));
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

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_player_keeps_its_stuck_arrows_and_item_use_timer() {
        use super::super::lan::LanSettings;
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "guest12".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                12,
                [3.0, 64.0, 5.0],
                10,
            );
            guest.placed = true;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            // A first turn starts a fresh combat record: take it first.
            game.as_guest(12, |_| {}).unwrap();
            let player = game.living(game.entity(12).unwrap()).unwrap();

            // Between its turns the guest's counts are in its combat record.
            assert_eq!(game.living_arrow_count(player), Some(0));
            assert!(game.living_set_arrow_count(player, 4));
            assert!(game.living_set_stinger_count(player, 2));
            assert!(game.living_set_arrow_removal_ticks(player, 7));
            assert!(game.living_set_stinger_removal_ticks(player, 9));
            let stuck = &game.guest_combat[&12].stuck;
            assert_eq!((stuck.arrows, stuck.stingers), (4, 2));
            assert_eq!((stuck.remove_arrow_time, stuck.remove_stinger_time), (7, 9));
            assert_eq!(game.living_arrow_count(player), Some(4));
            assert_eq!(game.living_stinger_count(player), Some(2));
            assert!(game.living_swing(player, Hand::Off, false));
            // No item in use: nothing to move.
            assert_eq!(game.living_item_use_remaining(player), Some(0));
            assert!(!game.living_set_item_use_remaining(player, 5));

            // Between its turns its use is in its own record too: a bow's is 72000.
            assert_eq!(game.living_used_item_hand(player), None);
            game.as_guest(12, |game| {
                game.offhand = Some(ItemStack::new("minecraft:bow", 1));
            })
            .unwrap();
            assert!(game.living_start_using_item(player, Hand::Off));
            assert_eq!(game.living_used_item_hand(player), Some(Hand::Off));
            assert_eq!(game.living_item_use_remaining(player), Some(72_000));
            assert!(game.living_release_using_item(player));
            assert_eq!(game.living_used_item_hand(player), None);
            assert!(!game.living_release_using_item(player));

            // The acting player's own state: an item in use for 3 of 32 ticks.
            game.as_guest(12, |game| {
                let player = game.living(game.entity(12).unwrap()).unwrap();
                assert_eq!(game.living_arrow_count(player), Some(4));
                game.using = Some((0, 3));
                game.use_duration = 32;
                assert_eq!(game.living_item_use_remaining(player), Some(29));
                assert!(game.living_set_item_use_remaining(player, 1));
                assert_eq!(game.use_duration, 4);
                assert_eq!(game.living_item_use_remaining(player), Some(1));
                game.using = None;
                // Its swing is queued for it when asked: not in the first half.
                game.swing.ticks = 0;
                game.guest_packets.clear();
                assert!(game.living_swing(player, Hand::Main, true));
                assert_eq!(game.guest_packets.len(), 1);
                game.swing.ticks = 1;
                assert!(!game.living_swing(player, Hand::Main, true));
                assert_eq!(game.guest_packets.len(), 1);
            })
            .unwrap();
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mobs_swing_reaches_clients_with_the_hand_asked_for() {
        use super::super::lan::LanSettings;
        use horizon_client::lan::{
            Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig,
        };
        use horizon_protocol::packets::{configuration::ClientInformation, play::Clientbound};
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "guest12".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                12,
                [3.0, 64.0, 5.0],
                10,
            );
            guest.placed = true;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(40, kind, Vec3::new(3.0, 64.0, 5.0), 0.0));
            let zombie = game.living(game.entity(40).unwrap()).unwrap();
            // The hand in the swing packet the flush sends after each swing.
            let mut sent = Vec::new();
            for natural in [false, true] {
                for hand in [Hand::Off, Hand::Main] {
                    game.mobs[0].ai.swinging = false;
                    game.mobs[0].ai.swing_time = 0;
                    if natural {
                        // A swing no hand was named for (a melee attack).
                        game.mobs[0].swing();
                    } else {
                        assert!(game.living_swing(zombie, hand, false));
                    }
                    game.lan_flush_effects();
                    while let Ok(Out::Packet(body)) = rx.try_recv() {
                        let mut reader = horizon_protocol::buf::Reader::new(&body);
                        let id = reader.varint().unwrap();
                        if let Ok(Clientbound::SwingAnimation(p)) =
                            Clientbound::decode(id, reader.rest())
                        {
                            sent.push(p.hand);
                        }
                    }
                    // The swing runs on, so the next one is a new start.
                    game.mobs[0].ai.swing_time = 3;
                    game.lan_flush_effects();
                    if natural {
                        break;
                    }
                }
            }
            assert_eq!(sent, [1, 0, 0]);
        });
    }
}
