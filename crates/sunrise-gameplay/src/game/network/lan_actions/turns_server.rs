use super::*;
use crate::game::server_side::potions_server::swap_player;

type ExitFault = Box<dyn std::any::Any + Send>;

impl crate::game::ServerGame<'_> {
    /// `as_guest`, with the system messages the guest's action earns (the
    /// `player::sendSystemMessage` of a packet handler) sent to that guest.
    pub fn as_guest_messaged<R>(&mut self, conn: u32, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        // The outer capture comes back even when `f` panics (a native callback can), so later
        // feedback isn't trapped in this guest's capture. What the guest was already told still
        // reaches them before the panic goes on, after `as_guest` has restored the swap.
        let (result, lines) = self.as_guest(conn, |game| game.capture_guest_messages(f))?;
        self.send_guest_messages(conn, lines);
        match result {
            Ok(result) => self.server.lan.is_some().then_some(result),
            Err(payload) => {
                // `f`'s panic was caught inside the turn, which ended cleanly: record it here.
                self.record_interrupted_turn(conn);
                std::panic::resume_unwind(payload)
            }
        }
    }

    /// Restore the outer chat capture before the guest swap is unwound.
    pub(in crate::game) fn capture_guest_messages<R>(
        &mut self,
        f: impl FnOnce(&mut Self) -> R,
    ) -> (std::thread::Result<R>, Vec<sunrise_game_data::text::Text>) {
        let outer = self.server.chat_capture.replace(Vec::new());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        let lines = std::mem::replace(&mut self.server.chat_capture, outer).unwrap_or_default();
        (result, lines)
    }
    pub(in crate::game) fn send_guest_messages(
        &self,
        conn: u32,
        lines: Vec<sunrise_game_data::text::Text>,
    ) {
        if let Some(host) = self.server.lan.as_ref() {
            for line in lines {
                host.hub.send_to(
                    conn,
                    &C::SystemChat(play::SystemChat {
                        content: sunrise_sim::lan::data::component_nbt(&line.component()),
                        overlay: false,
                    }),
                );
            }
        }
    }

    /// Resolve a recorded player entity id even when a nested turn has lent
    /// the acting slot to another body. A displaced guest's hub row still
    /// names them, but it does not own their body or stats.
    pub fn combat_player(&self, id: u32) -> Option<CombatPlayer<'_>> {
        if self
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.entity_id as u32 == id)
        {
            return Some(CombatPlayer::InPlace);
        }
        if let Some((depth, body)) = self
            .server
            .lan_displaced_players
            .0
            .iter()
            .enumerate()
            .rev()
            .find_map(|(depth, frame)| {
                let body = frame.body.as_ref().filter(|p| p.entity_id as u32 == id)?;
                Some((depth, body))
            })
        {
            return Some(CombatPlayer::Displaced(depth, body));
        }
        let host = self.server.lan.as_ref()?;
        host.hub
            .guests
            .iter()
            .find(|g| g.entity_id as u32 == id)
            .map(CombatPlayer::Parked)
    }
    /// `ServerLevel.players()` in turn: the host's own player, then each
    /// guest in the level in place with their state swapped in (`as_guest`).
    /// What reaches the players around a point (a beacon's powers, a splash
    /// potion, a warden's darkness, an elder guardian's curse) lands on each
    /// of them, not only on the host. A server has no player of its own.
    pub fn for_each_player(&mut self, mut f: impl FnMut(&mut Self)) {
        if self.common.player.is_some() && !self.server.lan_acting_fake {
            f(self);
        }
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded && !g.dead)
            .map(|g| g.conn)
            .collect();
        for conn in conns {
            self.as_guest(conn, &mut f);
        }
    }
    /// Save and restore the outer player in this stack frame, even for nested turns.
    /// Erase the callback type so the swap code is shared by all callers.
    pub(in crate::game) fn guest_turn(
        &mut self,
        i: usize,
        conn: u32,
        outer_dimension: Dim,
        run: &mut dyn FnMut(&mut Self),
    ) -> Result<Option<ExitFault>, TurnRefused> {
        let spawn_protection = self.server.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .get(i)
                .and_then(|g| self.spawn_protection_for(host, g.uuid))
        });
        let outer = BuildingPlayer {
            conn: self.server.lan_acting,
            dimension: outer_dimension,
            bounds: self
                .common
                .player
                .as_ref()
                .filter(|p| {
                    !self.server.lan_acting_fake
                        && !self.common.spectator()
                        && !p.health.dead_or_dying()
                })
                .map(PlayerMotion::bounds),
            borrowed: true,
        };
        let input_sneak = self.server_acting_sneak();
        let Some(host) = self.server.lan.as_mut() else {
            return Err(TurnRefused::Gone);
        };
        if host.hub.guests.get(i).is_none_or(|g| g.conn != conn) {
            return Err(TurnRefused::Gone);
        }
        // An active guest's record holds an outer player's state, not theirs.
        if self.server.lan_guest_swaps.contains(&conn) {
            return Err(TurnRefused::Nested);
        }
        let other_boxes =
            self.server
                .lan_building_boxes
                .0
                .for_turn(conn, Some(outer), building_players(host));
        let fresh = host.hub.guests[i].motion.is_none();
        let loaded_cooldown = (fresh && host.hub.guests[i].loaded_body.is_some()).then(|| {
            host.hub.guests[i]
                .saved
                .as_ref()
                .and_then(|tag| sunrise_sim::save::number(tag.get("PortalCooldown")))
                .unwrap_or(0) as i32
        });
        if fresh {
            let initialized = host.hub.guests.update(i, |guest| {
                super::guest_body::initialize(self.common.motion, guest)
            });
            if !initialized {
                return Err(TurnRefused::Gone);
            }
        }
        // Refusal must leave canonical records and loan state untouched.
        let Some(mut motion) = host.hub.guests.update(i, |g| g.motion.take()) else {
            return Err(TurnRefused::Gone);
        };
        host.hub.guests[i].residency.extract();
        self.server.lan_guest_swaps.push(conn);
        // `ServerPlayer.permissions()`: the guest's own command level for its
        // turn, so game-master checks see it (not the host's).
        let permission = std::mem::replace(
            &mut self.common.ui.registries.permission,
            host.hub.guests[i].permission,
        );
        let mut play = host.play.remove(&conn).unwrap_or_default();
        play.hook.begin_loan();
        if let Some(cooldown) = loaded_cooldown {
            play.portal.cooldown = cooldown;
        }
        if fresh {
            // A new body (joined, respawned, changed level): nothing on it yet,
            // its effects' modifiers and armor protection included.
            play.last_equipment = None;
            play.potions = Default::default();
            play.sculk = Default::default();
            // A new ServerPlayer has everything to record again.
            play.score_recorded = Default::default();
        }
        let attachments = std::mem::replace(
            &mut self.common.player_state.attachments,
            std::mem::take(host.hub.guests.attachments_mut(conn).unwrap()),
        );
        self.server.sculk.swap_player(&mut play.sculk);
        // The guest's own CombatTracker: damage to them is not the host's.
        // A new body starts with no record of the last one.
        let guest_combat = if fresh {
            Default::default()
        } else {
            self.server.guest_combat.0.remove(&conn).unwrap_or_default()
        };
        let host_combat = std::mem::replace(&mut self.server.combat.0, guest_combat);
        host.hub.guests.update(i, |g| {
            swap_player(&mut self.common.potions, &mut g.effects, &mut play.potions)
        });
        let g = &host.hub.guests[i];
        motion.position = Vec3::new(g.position[0], g.position[1], g.position[2]);
        motion.entity_id = g.entity_id;
        motion.ground.on_ground = g.on_ground;
        motion.yaw = g.yaw;
        motion.pitch = g.pitch;
        motion.sprinting = g.sprinting;
        let abilities = g.abilities;
        let spectator = g.spectator();
        motion.spectator = spectator;
        motion.local_spectator = spectator;
        motion.may_fly = abilities.may_fly;
        motion.flying = abilities.flying;
        motion.flying_speed = abilities.flying_speed;
        // FireworkRocketItem.use boosts only a gliding player.
        motion.fall_flying = g.fall_flying;
        // A player frozen for a hand-over must keep its saved record unchanged.
        motion.invulnerable = abilities.invulnerable || g.handover.is_some();
        // `ServerPlayer.isInvulnerableTo`'s connection term, as of now.
        motion.client_loaded = g.client_loaded();
        // The mods' collision exclusions are the guest's (#3717).
        motion
            .collision_exclusions
            .clone_from(&g.collision_exclusions);
        // The guest mirrors the body's last health. Do not run a health
        // setter just for taking the body on loan: a pending max-health
        // change has not reached LivingEntity.onAttributeUpdated yet.
        if motion.health.health().to_bits() != g.health.to_bits() {
            let _ = motion.set_health(g.health, false);
        }
        motion.health.food.food = g.food;
        motion.health.food.saturation = g.saturation;
        let mut player = Some(*motion);
        match self.server.lan_acting {
            Some(outer) => {
                if let Some(guest) = host.hub.guest(outer) {
                    guest.residency.extract();
                }
            }
            None => self.common.resident_entities.local_player.extract(),
        }
        std::mem::swap(&mut self.common.player, &mut player);
        let eye = (g.eye_height() as f32, self.common.eye);
        self.common.eye = eye.0;
        let (yaw, pitch) = (self.common.yaw, self.common.pitch);
        self.common.yaw = g.yaw;
        self.common.pitch = g.pitch;
        host.hub.guests.update(i, |g| {
            std::mem::swap(&mut self.common.inventory, &mut g.inventory);
            std::mem::swap(&mut self.common.armor, &mut g.armor);
            std::mem::swap(&mut self.server.location_armor, &mut g.location_armor);
            std::mem::swap(&mut self.common.offhand, &mut g.offhand);
            std::mem::swap(&mut self.common.equipment, &mut g.equipment);
            std::mem::swap(&mut self.common.carried, &mut g.carried);
        });
        let g = &host.hub.guests[i];
        let selected = std::mem::replace(&mut self.common.selected, g.selected);
        let (mode, creative) = (self.common.mode, self.common.creative);
        let outer_abilities = self.server.acting_abilities.replace(g.abilities);
        let (entity_reach, block_reach) = (self.common.entity_reach, self.common.block_reach);
        let (game_mode, guest_previous) = (g.game_mode, g.previous_game_mode);
        self.set_mode_raw(game_mode);
        self.common.sync_main_hand();
        let outer_previous = std::mem::replace(
            &mut self.common.previous_mode,
            guest_previous.map(crate::game::game_mode::mode_from_id),
        );
        let Some(host) = self.server.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        let g = &host.hub.guests[i];
        let xp = std::mem::replace(
            &mut self.common.experience,
            Experience {
                level: g.xp_level,
                progress: g.xp_progress,
                total: g.xp_total,
            },
        );
        let player_score = std::mem::replace(&mut self.common.player_score, g.score);
        let level_up_sound =
            std::mem::replace(&mut self.server.level_up_sound, play.level_up_sound);
        // The host's held keys stay theirs: the guest holds only sneak, and a
        // screen the guest opens (`keys.clear()`) leaves the host walking.
        let (sneaking, sprinting, guest_vehicle) = (g.sneaking, g.sprinting, g.vehicle);
        // The window's screens are the host's: a sign, book, command block or
        // mount menu the guest's action opens is not drawn over the host's
        // world (#687). The guest's client opens its own.
        let outer_ui = self.server_replace_pending_guest_ui(
            crate::game::server_side::acting_state::ServerPendingGuestUi::default(),
        );
        // The host's crafting grid stays theirs; a crafting table the guest
        // opens shows as the 3x3 size.
        let craft_size = std::mem::replace(&mut self.common.craft_size, 2);
        let craft_table = self.server.craft_table.take();
        play.craft_grid.clear();
        play.craft_grid.resize(9, None);
        let craft_grid = std::mem::replace(
            &mut self.common.craft_grid,
            std::mem::take(&mut play.craft_grid),
        );
        let container = self.common.container.take();
        let mining = std::mem::replace(&mut self.common.mining, play.mining.take());
        let destroy_delay = std::mem::replace(&mut self.server.destroy_delay, play.destroy_delay);
        let using = std::mem::replace(&mut self.common.using, play.using.take());
        let use_duration = std::mem::replace(&mut self.common.use_duration, play.use_duration);
        let use_start = std::mem::replace(
            &mut self.common.use_start,
            std::mem::take(&mut play.use_start),
        );
        let use_delay = std::mem::replace(&mut self.common.use_delay, play.use_delay);
        let shield_cooldown =
            std::mem::replace(&mut self.server.shield_cooldown, play.shield_cooldown);
        let cooldowns = std::mem::replace(
            &mut self.common.cooldowns,
            std::mem::take(&mut play.cooldowns),
        );
        let xp_delay = std::mem::replace(&mut self.server.xp_delay, play.xp_delay);
        let attack_ticks = std::mem::replace(&mut self.server.attack_ticks, play.attack_ticks);
        let attack_item = std::mem::replace(&mut self.server.attack_item, play.attack_item.take());
        let swing = std::mem::replace(&mut self.common.swing, play.swing);
        std::mem::swap(&mut self.common.mount, &mut play.mount);
        // `Entity.portalCooldown` and `portalProcess` are the player's own.
        std::mem::swap(&mut self.common.portal_player, &mut play.portal);
        let outer_input = self.server_replace_acting_input(
            crate::game::server_side::acting_state::ServerActingInput {
                sneak: sneaking,
                attack: play.digging.is_some(),
                use_item: self.common.using.is_some(),
                sprint: sprinting,
            },
        );
        // The guest's vehicle (Entity.vehicle) for its turn.
        let riding = std::mem::replace(&mut self.common.riding, guest_vehicle);
        // `Entity.isPassenger()` for that turn, from the same vehicle: the
        // guest's body keeps no swim (`Entity.updateSwimming` needs it false).
        // Taken from `g.vehicle` each turn, so boarding and getting out land.
        if let Some(p) = self.common.player.as_mut() {
            p.is_passenger = guest_vehicle.is_some();
        }
        // Entity.collide: the guest's body collides with the level's world
        // border too, as it is this turn (it moves with /worldborder).
        self.common.player_border();
        let Some(host) = self.server.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        let g = &host.hub.guests[i];
        let host_station = self.common.station.take();
        // The guest's bed, sleep counter and respawn point; the host's
        // respawn point, anchor and action bar stay theirs.
        let sleeping = std::mem::replace(&mut self.common.sleeping, g.bed);
        let sleep_timer = std::mem::replace(&mut self.common.sleep_timer, g.sleep_timer);
        let spawn = std::mem::replace(
            &mut self.common.spawn,
            g.spawn.map(|s| Vec3::new(s[0], s[1], s[2])),
        );
        let respawn = (self.server.respawn_config.take(), self.common.level.spawn);
        self.server.respawn_config = g
            .respawn_elsewhere
            .as_ref()
            .and_then(RespawnConfig::from_saved);
        // The guest's own ender inventory; one the host has open goes back
        // into the host's while the guest acts.
        let host_ender = self.server.ender_open;
        self.close_ender_chest();
        let Some(host) = self.server.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        let simulated = host.hub.guests.update(i, |g| {
            std::mem::swap(&mut self.server.ender_items, &mut g.ender_items);
            g.participates_in_player_simulation()
        });
        // The guest's advancements, recipe book and statistics.
        if simulated {
            self.advancement_tree();
        }
        let Some(host) = self.server.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        host.hub.guests.update(i, |g| {
            std::mem::swap(&mut self.common.adv.progress, &mut g.advancements);
            std::mem::swap(&mut self.common.adv.changed, &mut g.advancements_changed);
            std::mem::swap(&mut self.common.adv.unlocked, &mut g.recipes_unlocked);
            std::mem::swap(&mut self.common.level.recipe_book, &mut g.recipe_book);
            std::mem::swap(&mut self.common.level.player_extra, &mut g.saved);
            std::mem::swap(&mut self.common.level.last_death, &mut g.last_death);
        });
        let player_save_projection = (
            self.common.level.player_impulse_grace.take(),
            self.common.level.player_fall_particles.take(),
        );
        let g = &host.hub.guests[i];
        self.common.hook.begin_loan();
        std::mem::swap(&mut self.common.hook, &mut play.hook);
        let adv_loaded = std::mem::replace(&mut self.common.adv.loaded, true);
        let last_inventory =
            std::mem::replace(&mut self.common.adv.last_inventory, play.last_inventory);
        let levitation = std::mem::replace(&mut self.common.adv.levitation, play.levitation);
        let marks = std::mem::replace(&mut self.common.adv.marks, std::mem::take(&mut play.marks));
        let death_cause =
            std::mem::replace(&mut self.common.ui.death_cause, play.death_cause.take());
        let [x, y, z] = g.known_movement;
        let known_movement = std::mem::replace(&mut self.server.known_movement, Vec3::new(x, y, z));
        let acting = (
            self.server.lan_acting.replace(conn),
            self.server.lan_acting_permission.replace(g.permission),
            self.server.lan_acting_entity.replace(g.entity_id as u32),
            std::mem::replace(
                &mut self.server.lan_acting_spawn_protection,
                spawn_protection,
            ),
        );
        self.server.lan_displaced_players.0.push(DisplacedPlayer {
            body: player,
            name: self.server.lan_acting_name.replace(g.name.clone()),
            uuid: self.server.lan_acting_uuid.replace(g.uuid),
            fake: std::mem::replace(&mut self.server.lan_acting_fake, !g.participates_online()),
            mode,
            input_sneak,
            record_conn: conn,
            dimension: outer_dimension,
            selected,
            experience: xp,
            adv_loaded,
            riding,
            yaw,
            pitch,
            stats: std::mem::replace(&mut self.common.stats, std::mem::take(&mut play.stats)),
        });
        g.residency.put_back();
        self.common.hook.finish_loan();
        self.bind_current_hook_residency();
        let packets = std::mem::take(&mut self.server.guest_packets);
        let entity_events = std::mem::take(&mut self.server.guest_entity_events);
        let cooldown_events = std::mem::take(&mut self.server.guest_cooldowns);
        let player_sounds = std::mem::take(&mut self.server.guest_player_sounds);
        let block_sounds = std::mem::take(&mut self.server.guest_block_sounds);
        let equipment_breaks = std::mem::take(&mut self.server.guest_equipment_breaks);
        let swings = self.common.swings_started;
        let modifiers = std::mem::replace(
            &mut self.server.applied_modifiers,
            std::mem::take(&mut play.modifiers),
        );
        // The player's effects are on the new body before anything it does
        // (the movement that brought it, a landing): vanilla keeps the same
        // entity through a level change, with its attributes and effects.
        if fresh {
            self.apply_effect_status();
        }
        self.common.entity_reach =
            self.common
                .interaction_range("minecraft:entity_interaction_range", 3.0, 5.0);
        self.common.block_reach =
            self.common
                .interaction_range("minecraft:block_interaction_range", 4.5, 5.0);
        let building_boxes = std::mem::replace(&mut self.server.lan_building_boxes.0, other_boxes);

        run(self);

        // Commit the guest's changes and put the outer player back.
        let Some(host) = self.server.lan.as_ref() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        debug_assert_eq!(host.hub.guests.get(i).map(|g| g.conn), Some(conn));
        // Recorded before the callbacks run, cleared only after a clean put-back (a record the
        // turn's own panic made stays).
        let already_recorded = self.server.interrupted_turns.contains(&conn);
        self.record_interrupted_turn(conn);
        let turn_events = self
            .common
            .player
            .as_mut()
            .map(|p| std::mem::take(&mut p.events))
            .unwrap_or_default();
        // Catch exit-hook panics so the guest swap is restored before the caller chooses which fault to propagate.
        let callbacks = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !turn_events.is_empty() {
                self.record_combat(&turn_events);
                self.wear_armor(&turn_events);
            }
            // The mods hear the food level the turn changed (eating, hunger,
            // starving...), once for the change it made as a whole, with the
            // guest's state still swapped in; they may change or undo it before it
            // reaches the guest.
            let now = self.common.player.as_ref().map(|p| p.health.food.food);
            let food = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.get(i))
                .filter(|g| now.is_some_and(|now| g.food != now))
                .map(|g| (g.uuid, g.name.clone(), g.food, g.saturation));
            if let Some(now) = now
                && let Some((uuid, name, from, saturation)) = food
            {
                let (mut to, mut cancel) = (now, false);
                self.native_food_level_changed(uuid, &name, from, &mut to, &mut cancel);
                if let Some(p) = self.common.player.as_mut() {
                    if cancel {
                        p.health.food.food = from;
                        p.health.food.saturation = saturation;
                    } else if to != now {
                        p.health.food.food = to.clamp(0, 20);
                        p.health.food.saturation =
                            p.health.food.saturation.min(p.health.food.food as f32);
                    }
                }
            }
            // Player.playSound: everyone near but the guest, in the players' source,
            // through `ServerLevel.playSeededSound` like any level sound.
            for (name, at, volume, pitch) in std::mem::take(&mut self.server.guest_player_sounds) {
                let sound =
                    LevelSound::new(&name, at, volume, pitch).in_source(SoundSource::Players);
                self.send_level_sound(sound, Some(conn));
            }
            // Block-source sounds exclude the acting player, like player sounds.
            for (name, at, volume, pitch) in std::mem::take(&mut self.server.guest_block_sounds) {
                let sound =
                    LevelSound::new(&name, at, volume, pitch).in_source(SoundSource::Blocks);
                self.send_level_sound(sound, Some(conn));
            }
        }));
        let panic = callbacks.err();
        if panic.is_some() {
            self.record_open_turns();
        }
        // `ServerPlayer.permissions()` back to the host's after the turn.
        self.common.ui.registries.permission = permission;
        self.server.sculk.swap_player(&mut play.sculk);
        // The guest's damage record back into their slot, the host's in place.
        let guest_combat = std::mem::replace(&mut self.server.combat.0, host_combat);
        self.server.guest_combat.0.insert(conn, guest_combat);
        play.modifiers = std::mem::replace(&mut self.server.applied_modifiers, modifiers);
        // The guest's swings are theirs: the host's own count stays the host's.
        play.swings += self.common.swings_started.wrapping_sub(swings);
        self.common.swings_started = swings;
        self.server.guest_player_sounds = player_sounds;
        self.server.guest_block_sounds = block_sounds;
        let outgoing = std::mem::replace(&mut self.server.guest_packets, packets);
        let Some(host) = self.server.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        if host.hub.guests[i].participates_online() {
            for packet in outgoing {
                host.hub.send_to(conn, &packet);
            }
        }
        // ServerPlayer.connection.send: the guest's own entity events.
        let entity_id = host.hub.guests[i].entity_id;
        let outgoing_events =
            std::mem::replace(&mut self.server.guest_entity_events, entity_events);
        if host.hub.guests[i].participates_online() {
            for event in outgoing_events {
                host.hub.send_to(
                    conn,
                    &C::EntityEvent(play::EntityEvent {
                        id: entity_id,
                        event,
                    }),
                );
            }
        }
        // LivingEntity.onEquippedItemBroken: `broadcastEntityEvent`, the guest too.
        for event in std::mem::replace(&mut self.server.guest_equipment_breaks, equipment_breaks) {
            host.hub.entity_event(entity_id, event);
        }
        // ServerItemCooldowns.onCooldownStarted: the guest's own cooldown.
        let outgoing_cooldowns =
            std::mem::replace(&mut self.server.guest_cooldowns, cooldown_events);
        if host.hub.guests[i].participates_online() {
            for (group, duration) in outgoing_cooldowns {
                host.hub.send_to(
                    conn,
                    &C::Cooldown(play::Cooldown {
                        group: group.to_string(),
                        duration,
                    }),
                );
            }
        }
        host.hub.guests[i].residency.extract();
        self.common.hook.begin_loan();
        let outer_player = self
            .server
            .lan_displaced_players
            .0
            .pop()
            .expect("open guest turn");
        (
            self.server.lan_acting,
            self.server.lan_acting_permission,
            self.server.lan_acting_entity,
            self.server.lan_acting_spawn_protection,
        ) = acting;
        self.server.lan_acting_name = outer_player.name;
        self.server.lan_acting_uuid = outer_player.uuid;
        self.server.lan_acting_fake = outer_player.fake;
        self.server.known_movement = known_movement;
        self.server.lan_building_boxes.0 = building_boxes;
        play.death_cause = std::mem::replace(&mut self.common.ui.death_cause, death_cause);
        // An ender chest the guest opened: the menu shows their inventory
        // itself (lan_open_container), not a stand-in block entity.
        self.close_ender_chest();

        // What the action opened: handed to the guest instead of the host.
        let opened = self.common.container.take();
        let guest_table = std::mem::replace(&mut self.server.craft_table, craft_table);
        let opened_table = (std::mem::replace(&mut self.common.craft_size, craft_size) == 3)
            .then_some(guest_table)
            .flatten();
        play.craft_grid = std::mem::replace(&mut self.common.craft_grid, craft_grid);
        let opened_station = std::mem::replace(&mut self.common.station, host_station);
        let bed = std::mem::replace(&mut self.common.sleeping, sleeping);
        let guest_sleep_timer = std::mem::replace(&mut self.common.sleep_timer, sleep_timer);
        let guest_spawn = std::mem::replace(&mut self.common.spawn, spawn).map(|s| [s.x, s.y, s.z]);
        let guest_respawn = std::mem::replace(&mut self.server.respawn_config, respawn.0);
        self.common.level.spawn = respawn.1;
        let guest_vehicle = std::mem::replace(&mut self.common.riding, outer_player.riding);
        play.mining = std::mem::replace(&mut self.common.mining, mining);
        play.destroy_delay = std::mem::replace(&mut self.server.destroy_delay, destroy_delay);
        play.using = std::mem::replace(&mut self.common.using, using);
        play.use_duration = std::mem::replace(&mut self.common.use_duration, use_duration);
        play.use_start = std::mem::replace(&mut self.common.use_start, use_start);
        play.use_delay = std::mem::replace(&mut self.common.use_delay, use_delay);
        play.shield_cooldown = std::mem::replace(&mut self.server.shield_cooldown, shield_cooldown);
        play.cooldowns = std::mem::replace(&mut self.common.cooldowns, cooldowns);
        play.xp_delay = std::mem::replace(&mut self.server.xp_delay, xp_delay);
        play.attack_ticks = std::mem::replace(&mut self.server.attack_ticks, attack_ticks);
        play.attack_item = std::mem::replace(&mut self.server.attack_item, attack_item);
        play.swing = std::mem::replace(&mut self.common.swing, swing);
        std::mem::swap(&mut self.common.mount, &mut play.mount);
        std::mem::swap(&mut self.common.portal_player, &mut play.portal);
        self.common.container = container;
        let crate::game::server_side::acting_state::ServerPendingGuestUi {
            sign_editor: guest_sign,
            lectern: guest_lectern,
            command_block: guest_command_block,
            mount: guest_mount,
        } = self.server_replace_pending_guest_ui(outer_ui);
        self.server_set_acting_input(outer_input);
        let new_mode = self.common.mode_id();
        let turn_previous = std::mem::replace(&mut self.common.previous_mode, outer_previous);
        let turn_abilities = std::mem::replace(&mut self.server.acting_abilities, outer_abilities);
        let Some(host) = self.server.lan.as_mut() else {
            unreachable!("LanHost dropped inside a guest turn; closes are deferred");
        };
        enum TurnTeleport {
            Command([f64; 3], [bool; 3]),
            Position([f64; 3]),
            Rider([f64; 3], [bool; 3]),
        }
        let released_vehicle = host.hub.guests[i]
            .vehicle
            .filter(|id| Some(*id) != guest_vehicle);
        let (stat_events, name, teleport) = host.hub.guests.update(i, |g| {
            let mut teleport = None;
            g.attachments =
                std::mem::replace(&mut self.common.player_state.attachments, attachments);
            let left_vehicle = g.vehicle.filter(|_| guest_vehicle.is_none());
            if guest_vehicle.is_none() {
                g.vehicle_good = None;
            }
            g.vehicle = guest_vehicle;
            let fell_asleep = bed.is_some() && g.bed.is_none();
            (g.bed, g.sleep_timer, g.spawn) = (bed, guest_sleep_timer, guest_spawn);
            g.respawn_elsewhere = RespawnConfig::kept(g.respawn_elsewhere.take(), guest_respawn);
            g.sleeping = g.bed.is_some();
            if fell_asleep {
                g.woke_from = None;
            }
            std::mem::swap(&mut self.server.ender_items, &mut g.ender_items);
            std::mem::swap(&mut self.common.adv.progress, &mut g.advancements);
            std::mem::swap(&mut self.common.adv.changed, &mut g.advancements_changed);
            std::mem::swap(&mut self.common.adv.unlocked, &mut g.recipes_unlocked);
            std::mem::swap(&mut self.common.level.recipe_book, &mut g.recipe_book);
            std::mem::swap(&mut self.common.level.player_extra, &mut g.saved);
            std::mem::swap(&mut self.common.level.last_death, &mut g.last_death);
            self.common.level.player_impulse_grace = player_save_projection.0;
            self.common.level.player_fall_particles = player_save_projection.1;
            play.stats = std::mem::replace(&mut self.common.stats, outer_player.stats);
            std::mem::swap(&mut self.common.hook, &mut play.hook);
            self.common.adv.loaded = outer_player.adv_loaded;
            play.last_inventory =
                std::mem::replace(&mut self.common.adv.last_inventory, last_inventory);
            play.levitation = std::mem::replace(&mut self.common.adv.levitation, levitation);
            play.marks = std::mem::replace(&mut self.common.adv.marks, marks);
            let stat_events = std::mem::take(&mut play.stats.events);
            let name = g.name.clone();
            swap_player(&mut self.common.potions, &mut g.effects, &mut play.potions);
            if let Some(key) = host_ender {
                self.common
                    .world
                    .block_entities
                    .insert(key, BlockEntity::Chest(self.server.ender_items.clone()));
                self.server.ender_open = Some(key);
            }
            play.level_up_sound =
                std::mem::replace(&mut self.server.level_up_sound, level_up_sound);
            let new_xp = std::mem::replace(&mut self.common.experience, outer_player.experience);
            g.xp_level = new_xp.level;
            g.xp_progress = new_xp.progress;
            g.xp_total = new_xp.total;
            g.score = std::mem::replace(&mut self.common.player_score, player_score);
            self.common.mode = outer_player.mode;
            self.common.creative = creative;
            self.common.entity_reach = entity_reach;
            self.common.block_reach = block_reach;
            g.selected = std::mem::replace(&mut self.common.selected, outer_player.selected);
            std::mem::swap(&mut self.common.inventory, &mut g.inventory);
            std::mem::swap(&mut self.common.armor, &mut g.armor);
            std::mem::swap(&mut self.server.location_armor, &mut g.location_armor);
            std::mem::swap(&mut self.common.offhand, &mut g.offhand);
            std::mem::swap(&mut self.common.equipment, &mut g.equipment);
            std::mem::swap(&mut self.common.carried, &mut g.carried);
            // A teleport with a rotation (commands) turns the guest. `/rotate`
            // (ServerPlayer.forceSetRotation) sends the rotation alone, rider or not.
            let forced = std::mem::take(&mut self.server.rotation_forced);
            let turned = !forced && [self.common.yaw, self.common.pitch] != [g.yaw, g.pitch];
            [g.yaw, g.pitch] = [self.common.yaw, self.common.pitch];
            if forced {
                g.send(&C::PlayerRotation(play::PlayerRotation {
                    yaw: g.yaw,
                    relative_yaw: false,
                    pitch: g.pitch,
                    relative_pitch: false,
                }));
            }
            self.common.yaw = outer_player.yaw;
            self.common.pitch = outer_player.pitch;
            self.common.eye = eye.1;
            if let Some(mut m) = std::mem::replace(&mut self.common.player, outer_player.body) {
                // Travel can end a glide on a climbable before the next action.
                g.fall_flying = m.fall_flying;
                g.health = m.health.health();
                g.food = m.health.food.food;
                g.saturation = m.health.food.saturation;
                // A teleport (commands, a pearl) moves the guest; a pearl's is
                // sent even to where they stand (ServerPlayer.teleport).
                let at = [m.position.x, m.position.y, m.position.z];
                // A hit with extra knockback stopped the guest's sprint
                // (Player.causeExtraKnockback's setSprinting(false)).
                if !m.sprinting {
                    g.sprinting = false;
                }
                m.sprinting = false;
                // The acting body's glide flag is authoritative for this turn.
                // In a nested turn this is the inner guest, never the suspended
                // outer actor whose body the swap restores above.
                g.fall_flying = m.fall_flying;
                // ServerPlayer.indicateDamage: the hit's direction to the player.
                if let Some(yaw) = m.health.indicated_dir.take() {
                    g.send(&C::HurtAnimation(play::HurtAnimation {
                        id: g.entity_id,
                        yaw,
                    }));
                }
                let teleported = std::mem::take(&mut m.teleported);
                let teleported_delta = m.teleported_delta;
                m.teleported_delta = [false; 3];
                let moved: f64 = (0..3).map(|k| (at[k] - g.position[k]).powi(2)).sum();
                g.motion = Some(Box::new(m));
                // The pearl and the bed took the rider out of its vehicle at their
                // source (`stop_riding`), so `left_vehicle` already says so.
                // ServerPlayer.dismountTo: the client hears it is out of the
                // vehicle before the teleport beside it. A rider sits where its
                // vehicle puts it (lan_vehicles.rs), with no teleport.
                if let Some(vehicle) = left_vehicle {
                    g.send(&C::SetPassengers(play::SetPassengers {
                        vehicle: vehicle as i32,
                        passengers: Vec::new(),
                    }));
                }
                if g.vehicle.is_none() && (moved > 0.0001 || turned || teleported) {
                    // A command's teleport carries its relative axes as the
                    // packet's DELTA flags; any other move is absolute.
                    if teleported {
                        teleport = Some(TurnTeleport::Command(at, teleported_delta));
                    } else {
                        teleport = Some(TurnTeleport::Position(at));
                    }
                } else if teleported {
                    // ServerPlayer.startRiding and an asPassenger transition also
                    // teleport the rider. Retain its mount and publish its body.
                    teleport = Some(TurnTeleport::Rider(at, teleported_delta));
                }
            }
            (stat_events, name, teleport)
        });
        match teleport {
            Some(TurnTeleport::Command(at, delta)) => host.hub.teleport_command(i, at, delta),
            Some(TurnTeleport::Position(at)) => host.hub.teleport(i, Some(at)),
            Some(TurnTeleport::Rider(at, delta)) => host.hub.teleport_rider(i, at, delta, false),
            None => {}
        }
        let g = &host.hub.guests[i];
        if new_mode != g.game_mode {
            let conn = g.conn;
            host.hub.set_game_mode(
                conn,
                new_mode,
                &self.common.world,
                Some(self.common.collision),
            );
        }
        // The turn's previous mode, as `Game::set_mode` kept it: the mode left
        // by the turn's last change, when it changed more than once.
        host.hub.guests.update(i, |g| {
            g.previous_game_mode = turn_previous.map(GameMode::id);
            // The turn's abilities: a mode change made in it set the flags first
            // (`Game::set_mode`), then a mod's writes won, as in vanilla.
            if let Some(now) = turn_abilities
                && now != g.abilities
            {
                g.abilities = now;
                g.send(&g.abilities_packet());
            }
        });
        let ended_digging = host.hub.guests[i].dead
            && (play.digging.is_some() || play.delayed.is_some() || play.destroy_pos.is_some());
        host.play.insert(conn, play);
        let finished = self.server.lan_guest_swaps.pop();
        debug_assert_eq!(finished, Some(conn));
        host.hub.guests[i].residency.put_back();
        host.play[&conn].hook.finish_loan();
        match self.server.lan_acting {
            Some(outer) => {
                if let Some(guest) = host.hub.guest(outer) {
                    guest.residency.put_back();
                }
            }
            None => self.common.resident_entities.local_player.put_back(),
        }
        self.common.hook.finish_loan();
        if let Some(vehicle) = released_vehicle {
            self.release_removed_vehicle(vehicle);
        }
        self.bind_current_hook_residency();
        if ended_digging {
            self.guest_forget_digging(conn);
        }
        for event in turn_events {
            match event {
                MotionEvent::Health(sunrise_sim::health::HealthEvent::DamageBroadcast {
                    source,
                }) => self.guest_damage_broadcast(conn, source),
                MotionEvent::HurtSound(sound) => self.guest_hurt_sound(conn, sound),
                _ => {}
            }
        }
        self.stat_criteria_changed(&name, stat_events);
        if let Some(pos) = opened {
            self.lan_open_container(conn, pos);
        }
        if let Some(station) = opened_station {
            self.lan_station_opened(conn, station);
        }
        if let Some(edit) = guest_sign {
            // `edit: PendingSignEditor`; callee moved to `sign_edit_server.rs`
            self.lan_open_sign_editor(conn, edit);
        }
        if let Some(key) = guest_lectern {
            self.lan_open_lectern(conn, key);
        }
        if let Some(pos) = guest_command_block {
            self.lan_open_command_block(conn, pos);
        }
        if let Some(pos) = opened_table {
            self.lan_open_crafting(conn, pos);
        }
        if let Some(mob) = guest_mount {
            self.lan_open_mount(conn, mob);
        }
        // An ender chest a mod opened for them in the turn (`player_open_ender_chest`).
        self.open_turn_ender_menus(conn);
        self.drain_menu_opens(conn);
        // The turn's teleports are in the hub now: the mods hear them.
        self.flush_teleport_notes();
        if let Some(host) = self.server.lan.as_ref() {
            refresh_building_player(host, conn);
        }
        // An inner turn can move, kill, or change the mode of a hub-owned body.
        if let (Some(actor), Some(host)) = (self.server.lan_acting, self.server.lan.as_ref()) {
            self.server.lan_building_boxes.0 =
                self.server
                    .lan_building_boxes
                    .0
                    .for_turn(actor, None, building_players(host));
        }
        if panic.is_none() && !already_recorded {
            self.server.interrupted_turns.retain(|&c| c != conn);
        }
        Ok(panic)
    }
    /// The host can be parked below a projectile victim's guest turn.
    pub(in crate::game) fn projectile_player_owner_id(&self, conn: Option<u32>) -> Option<u32> {
        self.player_entity_id_by_connection(conn)
            .map(|id| id as u32)
            .or_else(|| {
                conn.is_none().then_some(())?;
                Some(
                    self.server
                        .lan_displaced_players
                        .0
                        .first()?
                        .body
                        .as_ref()?
                        .entity_id as u32,
                )
            })
    }
    /// Record that a panic interrupted `conn`'s turn: the crash save keeps that player's last
    /// saved file. The record clears at the start of the next server tick.
    pub(in crate::game::server_side) fn record_interrupted_turn(&mut self, conn: u32) {
        if !self.server.interrupted_turns.contains(&conn) {
            self.server.interrupted_turns.push(conn);
        }
    }
    /// Record every swap still open: a panic going through them leaves each of their players'
    /// state displaced or half-done.
    pub(in crate::game) fn record_open_turns(&mut self) {
        for i in 0..self.server.lan_guest_swaps.len() {
            self.record_interrupted_turn(self.server.lan_guest_swaps[i]);
        }
    }
    /// Record the shot on its shooter, not on the victim whose damage turn is in place.
    pub(in crate::game) fn remember_player_projectile_target(
        &mut self,
        shooter: u32,
        target: Option<u32>,
    ) {
        match self.combat_player(shooter) {
            Some(CombatPlayer::InPlace) => self.remember_player_target(target),
            Some(CombatPlayer::Displaced(depth, _)) => {
                if let Some(body) = self.server.lan_displaced_players.0[depth].body.as_mut() {
                    body.health.last_hurt_mob = target.map(|id| (id, body.health.tick_count));
                }
            }
            Some(CombatPlayer::Parked(guest)) => {
                let conn = guest.conn;
                if let Some(host) = self.server.lan.as_mut() {
                    host.hub.update_guest(conn, |guest| {
                        if let Some(body) = guest.motion.as_mut() {
                            body.health.last_hurt_mob =
                                target.map(|id| (id, body.health.tick_count));
                        } else if let Some(tick) = guest.carried_tick_count {
                            guest.carried_last_hurt_mob = target.map(|id| (id, tick));
                        }
                    });
                }
            }
            None => {}
        }
    }
    /// Run `f` with the player's own inventory in their hub record. The live
    /// actor's inventory is in `Game`; a suspended actor's is in the next turn's
    /// guest record. The closure reads the item table and world while ownership is lent.
    pub fn with_own_record<R>(
        &mut self,
        gi: usize,
        f: impl FnOnce(
            &mut LanHost,
            &sunrise_items::items::Items,
            &sunrise_level::world::World,
        ) -> Option<R>,
    ) -> Option<R> {
        let host = self.server.lan.as_ref()?;
        let conn = host.hub.guests[gi].conn;
        let swap = match self.server.lan_guest_swaps.iter().position(|&c| c == conn) {
            Some(loan) => match self.server.lan_guest_swaps.get(loan + 1) {
                Some(&inner) => Some(Some(host.hub.guests.connection_index(inner)?)),
                None => Some(None),
            },
            None => None,
        };
        let swap_inventory = |game: &mut Self| {
            let host = game
                .server
                .lan
                .as_mut()
                .expect("the guest turn owns the LAN host");
            match swap {
                Some(Some(inner)) => host.hub.guests.swap_loaned_inventory(gi, inner),
                Some(None) => host.hub.guests.update(gi, |g| {
                    // The fields `guest_turn` swaps.
                    std::mem::swap(&mut game.common.inventory, &mut g.inventory);
                    std::mem::swap(&mut game.common.armor, &mut g.armor);
                    std::mem::swap(&mut game.server.location_armor, &mut g.location_armor);
                    std::mem::swap(&mut game.common.offhand, &mut g.offhand);
                    std::mem::swap(&mut game.common.equipment, &mut g.equipment);
                    std::mem::swap(&mut game.common.carried, &mut g.carried);
                    std::mem::swap(&mut game.server.ender_items, &mut g.ender_items);
                }),
                None => {}
            }
        };
        swap_inventory(self);
        let result = f(
            self.server.lan.as_mut()?,
            server_world_items!(self),
            &self.common.world,
        );
        swap_inventory(self);
        result
    }

    /// Run `f` as the guest `conn`: the host's player state is swapped for theirs.
    /// Returns `None` if the guest is missing or already active in a nested turn.
    pub fn as_guest<R>(&mut self, conn: u32, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let gi = self.guest_index(conn)?;
        self.as_guest_at(gi, conn, f).ok()
    }

    /// `as_guest` for the guest at `gi` in the hub's list, which a caller that already found it
    /// passes with its `conn`: the turn starts without another scan, and is refused when the
    /// guest there isn't `conn`.
    pub fn as_guest_at<R>(
        &mut self,
        gi: usize,
        conn: u32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Result<R, TurnRefused> {
        let (result, exit_fault) = self.with_guest_swap_at(gi, conn, self.common.dim, |game| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let dimension = (game.server.serving.is_some() || game.options.dedicated.is_some())
                    .then_some(game.dim.id());
                let result = sunrise_sim::block_contact::in_level(game.tick, dimension, || f(game));
                game.drain_entity_inside();
                result
            }));
            // The restore clears the acting marker, so teardown learns of all interrupted turns
            // from the record (#2704).
            if result.is_err() {
                game.record_open_turns();
            }
            result
        })?;
        // The guest's progress and the outer player were restored before propagating a panic.
        // When the body and the turn's exit work both unwound, the body's fault is the one
        // that propagates, as it would without the exit work.
        match (result, exit_fault) {
            (Ok(result), None) => Ok(result),
            (Err(payload), _) | (Ok(_), Some(payload)) => std::panic::resume_unwind(payload),
        }
    }

    /// Where guest `conn` is in the hub's list.
    pub fn guest_index(&self, conn: u32) -> Option<usize> {
        self.server.lan.as_ref()?.hub.guests.connection_index(conn)
    }

    /// Run a command or fixture with the guest's player state in place, without the
    /// packet/tick turn's block-contact scope or handling a panic from `f`. When the caller
    /// has changed levels, the outer body is still in `outer_dimension`.
    pub fn with_guest_swap<R>(
        &mut self,
        conn: u32,
        outer_dimension: Dim,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let i = self.guest_index(conn)?;
        let (result, exit_fault) = self.with_guest_swap_at(i, conn, outer_dimension, f).ok()?;
        if let Some(payload) = exit_fault {
            std::panic::resume_unwind(payload);
        }
        Some(result)
    }

    pub(in crate::game) fn with_guest_swap_at<R>(
        &mut self,
        i: usize,
        conn: u32,
        outer_dimension: Dim,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Result<(R, Option<ExitFault>), TurnRefused> {
        let mut f = Some(f);
        let mut result = None;
        let exit_fault = self.guest_turn(i, conn, outer_dimension, &mut |game| {
            result = Some(f.take().unwrap()(game));
        })?;
        Ok((result.unwrap(), exit_fault))
    }

    /// `ServerLevel.broadcastDamageEvent` for a guest that took an unblocked hit.
    pub(in crate::game) fn guest_damage_broadcast(
        &mut self,
        conn: u32,
        source: sunrise_world::damage_source::DamageTypeId,
    ) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (id, at) = (g.entity_id, g.position);
        let kind = self
            .common
            .health_resources
            .damage
            .definition(source)
            .map_or("minecraft:generic", |d| d.name.as_str());
        host.hub.damage_event(id, kind, None, None, at);
    }

    /// The game mode fields without `set_mode`'s side effects on the host.
    pub(in crate::game) fn set_mode_raw(&mut self, id: i32) {
        self.common.mode = crate::game::game_mode::mode_from_id(id);
        self.common.creative = self.common.mode == GameMode::Creative;
    }

    /// `LivingEntity.playHurtSound` for a guest: the server's `Entity.playSound`
    /// reaches every player around but the hurt one, whose own client plays it
    /// from `handleDamageEvent` (#4149).
    pub(in crate::game) fn guest_hurt_sound(&mut self, conn: u32, sound: &str) {
        let Some(at) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.position)
        else {
            return;
        };
        self.send_level_sound(LevelSound::new(sound, at, 1.0, 1.0), Some(conn));
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
