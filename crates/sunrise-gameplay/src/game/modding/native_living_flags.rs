//! State flags of a living entity for native server mods, through a
//! `LivingRef` (docs/modding/src/how-to/native-mods.md, "Entity and level
//! handles"): the glide (`LivingEntity.isFallFlying` and `stopFallFlying`),
//! the sleep of a player or a villager (`isSleeping`, `getSleepingPos`,
//! `startSleeping`, `stopSleeping`, `setSleepingPos` and `clearSleepingPos`),
//! the jump of the player in place (`isJumping`, `setJumping`), climbing
//! (`LivingEntity.onClimbable`), the baby age (`LivingEntity.isBaby` and
//! `Mob.setBaby`) and the mob flags `Mob.isNoAi`/`setNoAi`,
//! `isPersistenceRequired`/`setPersistenceRequired` and
//! `requiresCustomPersistence`. Reads return `None` and writes `false` for a
//! stale handle, and for a kind Sunrise keeps no such state for (each doc
//! says which).
use super::commands::Target;
use mod_api::LivingRef;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::native_living_body::FlagClimbing;
use sunrise_sim::mobs::villager;
use sunrise_world::climbing;

impl crate::game::ServerGame<'_> {
    /// Run a player sleep write in the authoritative body. An acting player's
    /// hub record is displaced until its turn ends; only inactive guests enter
    /// another turn, in the level already in place. Fake players never sleep.
    pub(in crate::game) fn native_player_sleep_write(
        &mut self,
        target: Target,
        write: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        match target {
            Target::Player => !self.server.lan_acting_fake && write(self),
            Target::Guest(conn) => {
                let Some(gi) = self.guest_index(conn) else {
                    return false;
                };
                // The entity resolver includes players in other levels, but a
                // body swap does not swap the world a bed write would touch.
                if self.server.lan.as_ref().unwrap().hub.guests[gi].dimension
                    != self.common.dim.id()
                {
                    return false;
                }
                self.as_guest_at(gi, conn, |game| !game.server.lan_acting_fake && write(game))
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    /// `LivingEntity.startFallFlying`: sets the glide flag, so the next tick glides.
    /// The raw flag write: the caller owns the canGlide checks. True for a player, a
    /// guest or a mob; false for an armor stand or the dragon.
    pub fn living_start_gliding(&mut self, living: LivingRef) -> bool {
        match self.native_living_target(living) {
            Some(Target::Player) => {
                let Some(p) = self.common.player.as_mut() else {
                    return false;
                };
                p.fall_flying = true;
                true
            }
            Some(Target::Guest(conn)) => self
                .server
                .lan
                .as_mut()
                .and_then(|h| h.hub.update_guest(conn, |g| g.fall_flying = true))
                .is_some(),
            Some(Target::Mob(id)) => {
                let Some(m) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                else {
                    return false;
                };
                m.extra.fall_flying = true;
                true
            }
            _ => false,
        }
    }

    /// `LivingEntity.stopFallFlying`: clears the glide flag, so the glide's
    /// wear and sound stop and the next fall is a fall. The players around
    /// learn it with the entity's next flag sync, as they do when a glide
    /// ends on its own. True for a player, a guest or a mob; false for an
    /// armor stand or the dragon.
    pub fn living_stop_gliding(&mut self, living: LivingRef) -> bool {
        match self.native_living_target(living) {
            Some(Target::Player) => {
                let Some(p) = self.common.player.as_mut() else {
                    return false;
                };
                p.fall_flying = false;
                true
            }
            Some(Target::Guest(conn)) => self
                .server
                .lan
                .as_mut()
                .and_then(|h| h.hub.update_guest(conn, |g| g.fall_flying = false))
                .is_some(),
            Some(Target::Mob(id)) => {
                let Some(m) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                else {
                    return false;
                };
                m.extra.fall_flying = false;
                true
            }
            _ => false,
        }
    }

    /// `LivingEntity.startSleeping`: the entity lies in the bed whose head
    /// `pos` names, with the effect the game's own sleep has. The player in
    /// place is moved onto the bed, put to sleep and occupies it; a
    /// villager or wandering trader takes a valid, unoccupied bed. Normal
    /// integrated-owner, LAN and dedicated guests use their server body,
    /// inside or outside their own turn, in the level in place. False for
    /// a guest in another level, a fake player, another
    /// mob, an armor stand or the dragon, or a bed without a sleep shape.
    pub fn living_start_sleeping(&mut self, living: LivingRef, pos: [i32; 3]) -> bool {
        match self.native_living_target(living) {
            Some(target @ (Target::Player | Target::Guest(_))) => {
                self.native_player_sleep_write(target, |game| game.lie_in_bed(pos))
            }
            Some(Target::Mob(id)) => {
                let Some(m) = self.common.mobs.iter().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                if !matches!(
                    m.species(),
                    sunrise_sim::mobs::Species::Villager
                        | sunrise_sim::mobs::Species::WanderingTrader
                ) {
                    return false;
                }
                // The internal brain helper assumes its target is a bed. Validate
                // the public target and its sleep shape before any mutation.
                let Some(at) = self.sleeping_position(pos) else {
                    return false;
                };
                let mut mobs = self.common.mobs.lift();
                #[cfg(any(test, feature = "test-support"))]
                crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
                let w = self.mob_world_base(&[]);
                let mut events = Vec::new();
                let slept = mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                    let slept = villager::start_sleeping(m, pos, &w, &mut events);
                    if slept {
                        sunrise_sim::mobs::living::teleports::set_pos(m, at);
                    }
                    slept
                });
                self.common
                    .mobs
                    .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
                let _ = self.handle_mob_events(events);
                slept
            }
            _ => false,
        }
    }
    /// `LivingEntity.stopSleeping`: the sleeper stands up beside the bed, as
    /// the game's own waking does, with its pose and packets. True when the
    /// entity slept; false for a fake player, another mob, an armor stand
    /// or the dragon. Normal server guests in the level in place use the
    /// same wake-up path; guests in another level are refused.
    pub fn living_stop_sleeping(&mut self, living: LivingRef) -> bool {
        match self.native_living_target(living) {
            Some(target @ (Target::Player | Target::Guest(_))) => {
                self.native_player_sleep_write(target, |game| {
                    let slept = game.sleeping.is_some();
                    if slept {
                        game.wake_up(true, true);
                    }
                    slept
                })
            }
            Some(Target::Mob(id)) => {
                let Some(m) = self.common.mobs.iter().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                if !villager::sleeping(m) {
                    return false;
                }
                let mut mobs = self.common.mobs.lift();
                #[cfg(any(test, feature = "test-support"))]
                crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
                let w = self.mob_world_base(&[]);
                let mut events = Vec::new();
                if let Some(m) = mobs.iter_mut().find(|m| m.id == id) {
                    villager::stop_sleeping(m, &w, &mut events);
                }
                self.common
                    .mobs
                    .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
                let _ = self.handle_mob_events(events);
                true
            }
            _ => false,
        }
    }
    /// `LivingEntity.setSleepingPos`: the bed `pos` names becomes the one
    /// the player sleeps in, without moving them; the next entity sync
    /// tells the players around they sleep there, as it tells them the
    /// game's own sleep. The villagers' beds belong to the evening's bed
    /// path (`start_sleeping`). Normal server guests in the level in place
    /// are supported; guests in another level, fake players, mobs, armor
    /// stands and the dragon are refused. This only
    /// changes the sleeping position/pose, not bed occupancy.
    pub fn living_set_sleeping_pos(&mut self, living: LivingRef, pos: [i32; 3]) -> bool {
        let Some(target) = self.native_living_target(living) else {
            return false;
        };
        self.native_player_sleep_write(target, |game| {
            game.sleeping = Some(pos);
            if let Some(p) = game.player.as_mut() {
                let _ = p.set_sleeping(true);
            }
            true
        })
    }
    /// `LivingEntity.clearSleepingPos`: the player no longer names a bed, so
    /// `is_sleeping` is false and the next entity sync stands them up.
    /// Normal server guests in the level in place are supported; guests in
    /// another level, fake players, mobs, armor stands and the dragon are
    /// refused. This does not run the wake-up animation
    /// or move the player; use `stop_sleeping` to stand beside the bed.
    pub fn living_clear_sleeping_pos(&mut self, living: LivingRef) -> bool {
        let Some(target) = self.native_living_target(living) else {
            return false;
        };
        self.native_player_sleep_write(target, |game| {
            game.sleeping = None;
            if let Some(p) = game.player.as_mut() {
                let _ = p.set_sleeping(false);
            }
            true
        })
    }
    /// `LivingEntity.setJumping`: whether the player in place's next step
    /// jumps, as their own key does. False for a guest outside its turn,
    /// whose client holds its own jump, a mob, an armor stand or the
    /// dragon.
    pub fn living_set_is_jumping(&mut self, living: LivingRef, jumping: bool) -> bool {
        match self.native_living_target(living) {
            Some(Target::Player) => {
                let Some(p) = self.common.player.as_mut() else {
                    return false;
                };
                p.jumping = jumping;
                true
            }
            _ => false,
        }
    }
    /// The climbing decision for a mob currently lent out of the game's list.
    pub(in crate::game::server_side) fn native_mob_is_climbing(
        &self,
        mob: &sunrise_sim::mobs::Mob,
    ) -> bool {
        let mut world = FlagClimbing {
            world: &self.common.world,
            context: &self.common.loop_context,
            entity: mob.id,
            position: BlockPos::containing(mob.position),
            spectator: false,
            gliding: mob.extra.fall_flying,
        };
        climbing::on_climbable(&mut world, &self.common.motion.climbing).unwrap_or(false)
    }
}

use sunrise_gameplay_common::game::block_props::BlockPropsGameExt1 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
