/// Per-guest state of the gameplay code that lives outside `Guest`.
#[derive(Default)]
pub struct GuestPlay {
    pub mining: Option<Mining>,
    pub destroy_delay: u32,
    pub using: Option<(usize, u32)>,
    pub(in crate::game) use_duration: i32,
    pub(in crate::game) use_start: sunrise_gameplay_common::game::item_use_common::UseStart,
    pub(in crate::game) use_delay: u32,
    /// The shield's ItemCooldowns entry: an axe disabled it for this many ticks.
    pub(in crate::game) shield_cooldown: u32,
    /// `ServerGamePacketListenerImpl.dropSpamThrottler` (`TickThrottler(20, 1480)`): 20 for each
    /// item a creative player drops from the creative screen, one less each tick.
    pub(in crate::game) creative_drops: u32,
    /// `ServerGamePacketListenerImpl.chatSpamThrottler`: 20 for each chat message, one less each tick.
    pub chat_spam: u32,
    /// `commandSpamThrottler`: 20 for each command, one less each tick.
    pub(in crate::game) command_spam: u32,
    /// `sunrise:recipe_check` requests still counting against the burst limit.
    pub(in crate::game) recipe_checks: u32,
    /// `ServerCommandSuggestionsProvider`: the latest suggestion request not answered yet (a
    /// newer one replaces it, which then never gets an answer), and whether this tick's one
    /// answer is still free.
    pub(in crate::game) suggestion: Option<Vec<u8>>,
    pub(in crate::game) suggestion_budget: bool,
    /// The guest's `Player.getCooldowns()` (`Game::cooldowns` for its turn).
    pub cooldowns: sunrise_items::cooldowns::Cooldowns,
    pub(in crate::game) attack_ticks: i32,
    /// The main hand's item at the guest's last tick (`Game::attack_item`).
    pub(in crate::game) attack_item: Option<std::sync::Arc<str>>,
    pub(in crate::game) swing: sunrise_sim::first_person_sim::SwingState,
    /// The block being dug and the face the client aimed at.
    pub digging: Option<([i32; 3], [i32; 3])>,
    /// The server tick the digging started (ServerPlayerGameMode.destroyProgressStart).
    pub dig_start: u64,
    /// The block that clock is for (ServerPlayerGameMode.destroyPos). A START is the only writer
    /// that names one (death, a new ServerPlayerGameMode, clears it), so an abort, or a STOP that
    /// arrives out of reach, leaves it: the STOP of a dig resumed on the same block is still
    /// measured from the first START's ticks (#4124).
    pub(in crate::game) destroy_pos: Option<[i32; 3]>,
    /// A block the client stopped digging before the server's time was up: it breaks once
    /// its progress since `dig_start` is complete (ServerPlayerGameMode.hasDelayedDestroy).
    pub(in crate::game) delayed: Option<([i32; 3], u64)>,
    pub(in crate::game) last_progress: i32,
    /// Swings the guest's actions started this tick, for the players around
    /// (`LivingEntity.swing` sends ClientboundSwingAnimationPacket).
    pub(in crate::game) swings: u32,
    /// The guest's riding jump charge (`LocalPlayer.jumpRidingTicks`).
    pub(in crate::game) mount: sunrise_gameplay_common::game::mounts_common::MountState,
    /// The mount inventory the guest has open (game/network/lan_mount_menu.rs).
    pub(in crate::game) mount_window: Option<super::lan_mount_menu::GuestMount>,
    /// The lectern the guest reads (game/network/lan_stations.rs).
    pub(in crate::game) lectern: Option<super::lan_stations::GuestLectern>,
    /// The equipment `LivingEntity.detectEquipmentUpdates` /
    /// `collectEquipmentChanges` last saw this guest wear (`lastEquipmentItems`),
    /// by `EquipmentSlot` ordinal (`mainhand` through `saddle`). Shared between
    /// equipment attribute refreshes and `living_equipment_change`; allocated
    /// only while non-empty.
    pub last_equipment: Option<Box<sunrise_items::equipment::Equipment>>,
    /// The slots of the guest's inventories the mods last heard (`native_players_inventory_changes`),
    /// in player inventory, ender chest, cursor and crafting order.
    pub(in crate::game) inventory_heard:
        Option<Box<[Vec<Option<sunrise_items::items::ItemStack>>; 4]>>,
    /// What the guest's body was given for its effects and enchantments.
    pub(in crate::game) potions: sunrise_gameplay_common::game::potions_common::BodyPotions,
    /// `ServerPlayer.getStats` and the inventory `inventory_changed` last saw.
    pub stats: sunrise_gameplay_common::stats::Stats,
    /// `Stats.LEAVE_GAME` is counted: a guest is saved more than once on the
    /// way out, the statistic is awarded once (`Game::award_leave_game`).
    pub left_game: bool,
    pub(in crate::game) last_inventory: u64,
    /// `ServerPlayer.levitationStartPos` and `levitationStartTime`.
    pub(in crate::game) levitation: Option<([f64; 3], u64)>,
    /// The fall, lava ride, Nether trip and blast the guest's advancements track.
    pub marks: sunrise_gameplay_common::game::advancements_common::TravelMarks,
    /// `Entity.portalProcess` and `portalCooldown` (game/world/server_travel.rs).
    pub portal: sunrise_sim::nether_portal::PortalState,
    /// `Player.takeXpDelay`: ticks until the next orb can be taken.
    pub(in crate::game) xp_delay: u32,
    /// This player entity's age and level-up sound cooldown.
    pub(in crate::game) level_up_sound: sunrise_sim::entities::LevelUpSound,
    /// What last killed the guest outright (`/kill`): the death message's key.
    pub(in crate::game) death_cause: Option<String>,
    /// The equipment modifiers on the guest's attributes (`apply_equipment`).
    pub(in crate::game) modifiers: Vec<(String, String)>,
    pub(in crate::game) craft_grid: Vec<Option<ItemStack>>,
    /// The guest's cast fishing hook (`Player.fishing`).
    pub(in crate::game) hook:
        sunrise_sim::join_seq::ResidentSlot<sunrise_gameplay_common::game::fishing_common::Hook>,
    pub(in crate::game) sculk: super::sculk_server::PlayerEvents,
    /// What `ServerPlayer` last recorded for the guest's scoreboard criteria
    /// (`lastRecordedHealthAndAbsorption` and the others).
    pub(in crate::game) score_recorded: sunrise_gameplay_common::game::scoreboard_common::Recorded,
}

impl GuestPlay {
    /// The guest's `Player.attackStrengthTicker` outside its turn.
    pub(in crate::game) fn attack_ticks(&self) -> i32 {
        self.attack_ticks
    }
    /// The main-hand item of the guest's last tick (`Game.attack_item`).
    pub(in crate::game) fn attack_item(&self) -> Option<&str> {
        self.attack_item.as_deref()
    }
    /// The item in use: its slot and for how many ticks.
    pub(in crate::game) fn using(&self) -> Option<(usize, u32)> {
        self.using
    }
    /// `LivingEntity.getUsedItemHand` of the item in use, none when none is.
    pub(in crate::game) fn used_hand(&self) -> Option<sunrise_sim::first_person_sim::Hand> {
        sunrise_gameplay_common::game::item_use_common::hand_of(self.using)
    }
    /// `LivingEntity.useItemRemaining` of the item in use, 0 when none is.
    pub(in crate::game) fn use_remaining(&self) -> i32 {
        sunrise_gameplay_common::game::item_use_common::remaining_of(self.using, self.use_duration)
    }
    /// Sets `useItemRemaining`; false when no item is in use.
    pub(in crate::game) fn set_use_remaining(&mut self, remaining: i32) -> bool {
        sunrise_gameplay_common::game::item_use_common::set_remaining(
            self.using,
            &mut self.use_duration,
            remaining,
        )
    }
}

pub(in crate::game) struct ItemRules<'a>(
    pub(in crate::game) &'a sunrise_items::items::Items,
    pub(in crate::game) &'a sunrise_items::enchant::Enchantments,
    pub(in crate::game) Option<&'a dyn sunrise_sim::menu::SlotRules>,
    pub(in crate::game) Option<&'a sunrise_sim::menu::StackedOutcome>,
    pub(in crate::game) Vec<sunrise_level::containers::ModContainerInsertRule>,
);

impl Rules for ItemRules<'_> {
    fn items(&self) -> &sunrise_items::items::Items {
        self.0
    }
    fn enchantments(&self) -> &sunrise_items::enchant::Enchantments {
        self.1
    }
    fn max_stack(&self, item: &str) -> u32 {
        self.0.get(item).map_or(64, |i| i.max_stack)
    }
    fn slot_rules(&self) -> Option<&dyn sunrise_sim::menu::SlotRules> {
        self.2
    }
    fn stacked(&self) -> Option<&sunrise_sim::menu::StackedOutcome> {
        self.3
    }
    fn mod_container_insert_rule(
        &self,
        slot: usize,
        current: Option<&sunrise_items::items::ItemStack>,
        offered: &sunrise_items::items::ItemStack,
    ) -> bool {
        sunrise_level::containers::mod_container_insert_allowed(
            self.4.get(slot).copied().unwrap_or_default(),
            current,
            offered,
        )
    }
}

impl GuestPlay {
    pub(in crate::game) fn decay_recipe_checks(&mut self) {
        self.recipe_checks = self.recipe_checks.saturating_sub(1);
    }
    /// A dig that ends without a break, an abort or a STOP from out of reach: the guest holds
    /// attack no more (`digging` is `as_guest`'s left button and `tick_mining`'s `forced_hit`),
    /// while the clock `destroy_pos` names stays for the STOP that comes when the guest is back
    /// in reach (#4124).
    pub(in crate::game) fn dig_ended(&mut self) {
        self.digging = None;
        self.mining = None;
    }
    /// The `ServerPlayerGameMode.destroyProgressStart` a STOP on `pos` is measured from: the clock
    /// belongs to the block the START named (`destroyPos`), and only a START writes either, so a
    /// dig resumed after a fall still runs on the first START's ticks (#4124).
    pub(in crate::game) fn dig_start_for(&self, pos: [i32; 3]) -> Option<u64> {
        (self.destroy_pos == Some(pos)).then_some(self.dig_start)
    }
    /// The guest can make a recipe check if it has room in its burst budget.
    pub(in crate::game) fn recipe_check_available(&self) -> bool {
        self.recipe_checks < RECIPE_CHECK_LIMIT
    }
}

pub(in crate::game) fn building_player(g: &sunrise_sim::lan::guest::Guest) -> BuildingPlayer {
    BuildingPlayer {
        conn: Some(g.conn),
        dimension: Dim::from_id(&g.dimension),
        bounds: (g.participates_in_player_simulation()
            && g.placed
            && g.loaded
            && !g.dead
            && !g.spectator())
        .then(|| g.bounds()),
        borrowed: false,
    }
}

thread_local! {
    /// The hub's bodies for the turns of one `tick_guests`, built once instead
    /// of for every turn (players x turns a tick). A guest's entry is refreshed
    /// when its turn ends ([`refresh_building_player`]), so the turns after it
    /// see it where it went (a pearl, a death, spectator mode).
    pub(in crate::game) static BUILDING_CACHE: std::cell::RefCell<Option<HubBodies>> =
        const { std::cell::RefCell::new(None) };
}

/// The hub's current bodies, across dimensions. A borrowed ancestor's
/// live body takes precedence over this saved state until its turn returns.
pub(in crate::game) fn building_players(host: &LanHost) -> HubBodies {
    let kept = BUILDING_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|list| {
                list.iter().map(|p| p.get().conn).eq(host
                    .hub
                    .guests
                    .simulation_iter()
                    .map(|g| Some(g.conn)))
            })
            .cloned()
    });
    kept.unwrap_or_else(|| shared(host.hub.guests.simulation_iter().map(building_player)))
}

/// Builds the hub's bodies once for the turns to come (`Some`), or drops them.
pub fn building_cache(host: Option<&LanHost>) {
    let bodies = host.map(|host| shared(host.hub.guests.simulation_iter().map(building_player)));
    BUILDING_CACHE.with(|c| *c.borrow_mut() = bodies);
}

/// The cached bodies as they are now in the hub, after `conn`'s turn (or its
/// death step). Every entry, not only the actor's: a turn can move, kill or
/// change another guest too (`/tp`, an attack, an explosion), and the turns
/// after it must see that guest where it is now, as a list built afresh would.
/// Copies into the shared cells, no allocation.
pub(in crate::game) fn refresh_building_player(host: &LanHost, _conn: u32) {
    BUILDING_CACHE.with(|c| {
        if let Some(list) = c.borrow().as_ref()
            && list.len() == host.hub.guests.simulation_iter().count()
        {
            for (p, g) in list.iter().zip(host.hub.guests.simulation_iter()) {
                if p.get().conn == Some(g.conn) {
                    p.set(building_player(g));
                }
            }
        }
    });
}

/// What `ServerPlayerGameMode.handleBlockBreakAction`'s STOP_DESTROY_BLOCK branch does with a
/// guest's STOP.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::game) enum StopDig {
    /// A block the last START did not name: the STOP is not this dig's.
    Other,
    /// Already broken, by the guest's own tick: only the crack goes.
    Gone,
    /// The dig the START counts is at least 70 % done: the block breaks outright.
    Break,
    /// Less than that: the server finishes it alone from that same START.
    Delay(u64),
}

/// The STOP branch's decision: a STOP is measured from the START that named its block
/// (`destroyPos`, which an abort or a STOP from out of reach leaves alone), whose ticks times
/// `getDestroyProgress` decide between the break and the delayed dig (#4124).
pub(in crate::game) fn stop_dig(
    play: &GuestPlay,
    pos: [i32; 3],
    air: bool,
    ticks: u64,
    per_tick: f32,
) -> StopDig {
    match play.dig_start_for(pos) {
        None => StopDig::Other,
        Some(_) if air => StopDig::Gone,
        Some(start) if per_tick * (ticks - start + 1) as f32 >= 0.7 => StopDig::Break,
        Some(start) => StopDig::Delay(start),
    }
}

#[cfg(test)]
pub(in crate::game) mod recipe_check_budget_tests {
    use super::GuestPlay;
    use sunrise_gameplay_common::game::lan_actions::RECIPE_CHECK_COST;

    #[test]
    fn ten_checks_at_once_then_one_each_tick() {
        let mut play = GuestPlay::default();
        for _ in 0..10 {
            assert!(play.recipe_check_available());
            play.recipe_checks += RECIPE_CHECK_COST;
        }
        assert!(!play.recipe_check_available());
        for _ in 0..20 {
            play.decay_recipe_checks();
            assert!(play.recipe_check_available());
            play.recipe_checks += RECIPE_CHECK_COST;
            assert!(!play.recipe_check_available());
        }
        for _ in 0..20 {
            play.decay_recipe_checks();
        }
        assert_eq!(play.recipe_checks, 0);
    }
}
