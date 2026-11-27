use super::*;
use crate::game::{commands::Target, server_travel::SpawnAction};

impl crate::game::ServerGame<'_> {
    /// `handleClientCommand(PERFORM_RESPAWN)` calls PlayerList, not ServerPlayer.teleport.
    pub(in crate::game) fn commit_player_respawn(&mut self, conn: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let Some((end_conquered, dead)) = host.hub.update_guest(conn, |guest| {
            (
                std::mem::take(&mut guest.won_game),
                guest.dead && !guest.respawning,
            )
        }) else {
            return;
        };
        if end_conquered || dead {
            self.queue_spawn_trip(
                conn,
                SpawnAction::Replace {
                    was_death: !end_conquered,
                },
            );
        }
    }
    /// `SculkCatalystBlockEntity.CatalystListener`'s trigger on a player's
    /// death: for the player the dying guest's combat record credits
    /// (`LivingEntity.getKillCredit`), with the death's `killing_blow`.
    pub(in crate::game::server_side) fn guest_catalyst_trigger(
        &mut self,
        gi: usize,
        conn: u32,
        at: [f64; 3],
    ) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let now = self.common.world.game_time;
        let combat = self.guest_combat_record(conn);
        let player_credit = combat.and_then(|c| c.player_credit(now));
        let who = if let Some(credit) = player_credit {
            self.resolve_player_credit(credit)
        } else {
            combat.and_then(|c| c.kill_credit(now)).and_then(|id| {
                if self.server.lan_acting.is_none()
                    && self
                        .common
                        .player
                        .as_ref()
                        .is_some_and(|p| p.entity_id as u32 == id)
                {
                    Some(None)
                } else {
                    host.hub
                        .guests
                        .iter()
                        .find(|g| g.entity_id as u32 == id)
                        .map(|g| Some(g.conn))
                }
            })
        };
        let Some(who) = who else { return };
        let (damage, attacker) = combat
            .and_then(|c| c.killing_blow())
            .unwrap_or_else(|| ("minecraft:player_attack".into(), None));
        let g = &host.hub.guests[gi];
        let view = sunrise_sim::advancements::EntityView {
            kind: "minecraft:player".into(),
            position: at,
            sneaking: g.sneaking,
            input_sneak: g.input & 32 != 0,
            gamemode: Some(
                match g.game_mode {
                    1 => "creative",
                    2 => "adventure",
                    3 => "spectator",
                    _ => "survival",
                }
                .into(),
            ),
            ..Default::default()
        };
        // The damage's causing entity: the player in place, another guest, or
        // a mob (`DamageSource.getEntity`).
        let source = attacker.and_then(|id| {
            if self.server.lan_acting.is_none()
                && self
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| p.entity_id as u32 == id)
            {
                Some(self.player_entity_for_any(&["minecraft:kill_mob_near_sculk_catalyst"]))
            } else {
                host.hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == id)
                    .map(|g| sunrise_sim::advancements::EntityView {
                        kind: "minecraft:player".into(),
                        position: g.position,
                        sneaking: g.sneaking,
                        input_sneak: g.input & 32 != 0,
                        ..Default::default()
                    })
                    .or_else(|| {
                        self.common
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| self.mob_entity(m))
                    })
            }
        });
        let projectile = match damage.as_str() {
            "minecraft:arrow" => Some("minecraft:arrow"),
            "minecraft:fireball" | "minecraft:small_fireball" => Some("minecraft:fireball"),
            "minecraft:wither_skull" => Some("minecraft:wither_skull"),
            "minecraft:fireworks" => Some("minecraft:firework_rocket"),
            _ => None,
        };
        let blow = sunrise_sim::advancements::DamageView {
            kind: damage,
            // DamageSource.isDirect: the causing entity is the direct one.
            is_direct: projectile.is_none(),
            direct: match projectile {
                Some(kind) => Some(sunrise_sim::advancements::EntityView {
                    kind: kind.into(),
                    position: at,
                    ..Default::default()
                }),
                None => source.clone(),
            },
            source,
            ..Default::default()
        };
        // CatalystListener: trigger(player, killedEntity, damageSource).
        match who {
            None => {
                self.trigger_killed_as("minecraft:kill_mob_near_sculk_catalyst", &view, Some(&blow))
            }
            Some(kconn) => {
                self.as_guest(kconn, |g| {
                    g.trigger_killed_as(
                        "minecraft:kill_mob_near_sculk_catalyst",
                        &view,
                        Some(&blow),
                    );
                });
            }
        }
    }
    /// The combat history follows the body into and out of an acting turn.
    pub(in crate::game) fn guest_combat_record(
        &self,
        conn: u32,
    ) -> Option<&crate::game::combat::CombatTracker> {
        if self.server.lan_acting == Some(conn) {
            Some(&self.server.combat.0)
        } else {
            self.server.guest_combat.0.get(&conn)
        }
    }
    /// The scalar death-tail/death-message source. `living_death` uses the
    /// full accepted hit retained on the body instead.
    pub(in crate::game::server_side) fn guest_damage_source(&self, conn: u32) -> (String, u32) {
        let cause = if self.server.lan_acting == Some(conn) {
            self.common.ui.death_cause.as_deref()
        } else {
            self.server
                .lan
                .as_ref()
                .and_then(|host| host.play.get(&conn))
                .and_then(|p| p.death_cause.as_deref())
        };
        let (damage_type, attacker) = match cause {
            Some("death.attack.genericKill") => ("minecraft:generic_kill", 0),
            _ => self
                .guest_combat_record(conn)
                .and_then(|tracker| tracker.last_blow())
                .unwrap_or(("minecraft:generic", 0)),
        };
        (damage_type.to_owned(), attacker)
    }
    /// `ServerPlayer.die`'s message from a guest's own `CombatTracker`, which
    /// sits in `guest_combat` between turns.
    pub(in crate::game::server_side) fn guest_death_message(
        &mut self,
        conn: u32,
        me: sunrise_game_data::text::Text,
    ) -> sunrise_game_data::text::Text {
        if self.server.lan_acting == Some(conn) {
            return self.combat_death_message_for(me);
        }
        let mut tracker = self.server.guest_combat.0.remove(&conn).unwrap_or_default();
        std::mem::swap(&mut self.server.combat.0, &mut tracker);
        let text = self.combat_death_message_for(me);
        std::mem::swap(&mut self.server.combat.0, &mut tracker);
        self.server.guest_combat.0.insert(conn, tracker);
        text
    }
    /// Use the killed player's random when computing their experience reward.
    fn guest_kill_experience(&mut self, conn: u32, killer: Option<u32>, amount: i32) -> i32 {
        let Some(killer) = killer else { return amount };
        let mut value = amount;
        self.with_dying_guest(conn, |game| {
            let Some(victim) = game.common.player.as_ref() else {
                return;
            };
            let ctx = sunrise_items::enchant::EffectContext {
                this: Some("minecraft:player"),
                this_motion: Some((victim.fall_distance, victim.fall_flying || victim.flying)),
                ..Default::default()
            };
            let mut random = victim.random.clone();
            value = game.kill_experience(killer, ctx, &mut random, amount);
            if let Some(victim) = game.common.player.as_mut() {
                victim.random = random;
            }
        });
        value
    }

    /// `ServerPlayer.die` for a guest whose health has run out: the death
    /// message, the loot and experience, the death screen.
    pub fn guest_die(&mut self, gi: usize, conn: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let guest = &host.hub.guests[gi];
        let id = guest.entity_id;
        let acting = self.server.lan_acting == Some(conn);
        let dying = |game: &Self, guest: &sunrise_sim::lan::guest::Guest| {
            !guest.dead
                && if acting {
                    game.player
                        .as_ref()
                        .is_some_and(|body| body.health.health() <= 0.0)
                } else {
                    guest.health <= 0.0
                }
        };
        if !dying(self, guest) {
            return;
        }
        // The fatal source, for the death hooks.
        let fatal =
            (!mod_api::__private::mods().is_empty()).then(|| self.guest_fatal_source(gi, conn));
        if self.guest_death_canceled(gi, conn, fatal.as_ref()) {
            return;
        }
        // A dead player carries no mob a mod seated on it.
        self.eject_player_riders(Some(id as u32));
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let g = &host.hub.guests[gi];
        if dying(self, g) {
            let (uuid, name) = (g.uuid, g.name.clone());
            let here = host.hub.here(g);
            let (at, eye, xp, spectator) = if acting {
                let Some(body) = self.common.player.as_ref() else {
                    return;
                };
                (
                    [body.position.x, body.position.y, body.position.z],
                    f64::from(body.dimensions.eye_height),
                    self.common.experience,
                    self.common.spectator(),
                )
            } else {
                (
                    g.position,
                    g.eye_height(),
                    Experience {
                        level: g.xp_level,
                        progress: g.xp_progress,
                        total: g.xp_total,
                    },
                    g.game_mode == SPECTATOR,
                )
            };
            let last_death = Some((g.dimension.clone(), at.map(|v| v.floor() as i32)));
            let reward = xp.death_reward(
                self.common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::KEEP_INVENTORY)
                    || spectator,
            );
            let drops = (!spectator).then(|| [at[0], at[1] + eye - f64::from(0.3f32), at[2]]);
            let cause = if acting {
                self.common.mining = None;
                self.server.destroy_delay = 0;
                self.common.using = None;
                self.common.hook.discard();
                self.common.level.last_death = last_death;
                self.common.ui.death_cause.take()
            } else {
                self.guest_forget_digging(conn);
                self.server.lan.as_mut().and_then(|host| {
                    host.hub.guests.update(gi, |g| g.last_death = last_death);
                    host.play.get_mut(&conn).and_then(|play| {
                        play.hook.discard();
                        play.death_cause.take()
                    })
                })
            };
            if let Some(host) = self.server.lan.as_mut() {
                host.hub.guests.update(gi, |g| g.dead = true);
            }
            // ServerPlayer.die opens with `gameEvent(ENTITY_DIE)` at the player:
            // sculk sensors and wardens hear the death.
            if here {
                self.with_dying_guest(conn, |game| {
                    let ctx = game.player_context(None);
                    game.post_game_event("minecraft:entity_die", at, ctx);
                });
            }
            // `getKillCredit`, as the death finds it.
            let killer = self.guest_kill_credit_type(conn);
            // ServerPlayer.die: the neutral mobs near a guest in this level
            // forget their anger at them.
            if here {
                self.tell_neutral_mobs_player_died(at, Some(conn));
            }
            // The death's last blow, read before the death message empties the
            // combat record: ServerPlayer.die's triggers read it from a record
            // that CombatTracker.recheckStatus clears only at the end of die.
            let killing = self
                .guest_combat_record(conn)
                .and_then(|c| c.killing_blow());
            // ServerPlayer.die: with showDeathMessages, the message goes to
            // the dying player's death screen and to everyone the team lets
            // see it; without it, the death screen gets an empty one.
            let message = if self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SHOW_DEATH_MESSAGES)
            {
                let me = self.common.player_text(&name, uuid);
                let text = match cause.as_deref() {
                    Some(key) => sunrise_game_data::text::Text::translate(key, &[me]),
                    None if at[1] < self.common.world.below_world_y() => {
                        sunrise_game_data::text::Text::translate("death.attack.outOfWorld", &[me])
                    }
                    None => self.guest_death_message(conn, me),
                };
                let message = sunrise_sim::lan::data::component_nbt(&text.component());
                let board = &self.common.score.board;
                let Some(host) = self.server.lan.as_ref() else {
                    return;
                };
                host.hub.system_message_to(message.clone(), |other| {
                    sunrise_game_data::scoreboard::death_message_reaches(board, &name, other)
                });
                if self.common.is_dedicated_server() {
                    crate::game::dedicated::events::record(
                        "death",
                        &name,
                        uuid,
                        serde_json::json!({ "message": text.plain() }),
                    );
                    // Only a message for everyone is logged (`broadcastSystemMessage`).
                    if board.players_team(&name).is_none_or(|t| {
                        t.death_message_visibility
                            == sunrise_game_data::scoreboard::Visibility::Always
                    }) {
                        crate::game::dedicated::log(
                            "INFO",
                            &format!("System chat: {}", text.plain()),
                        );
                    }
                }
                message
            } else {
                literal("")
            };
            let Some(host) = self.server.lan.as_mut() else {
                return;
            };
            host.hub.guests.update(gi, |guest| guest.sync_score());
            host.hub.send_to(
                conn,
                &C::PlayerCombatKill(play::PlayerCombatKill { id, message }),
            );
            // LivingEntity.die: the death animation, the dying player included.
            host.hub.entity_event(id, 3);
            // LivingDropsEvent: the source living_death heard, even with no
            // combat entry (`/kill`), and the last-hurt-by-player memory.
            let recently_hit = self
                .guest_combat_record(conn)
                .is_some_and(|c| c.recently_hit_by_player(self.common.world.game_time));
            let dropping = sunrise_gameplay_common::game::creatures::DyingDrops {
                entity: id as u32,
                kind: "minecraft:player",
                player: Some(uuid),
                source: fatal.as_ref().map(|fatal| fatal.source()),
                recently_hit,
            };
            // LivingEntity.die: the death is heard, and a sculk catalyst
            // nearby takes the experience (before the loot is dropped), as it
            // does for a mob.
            let killing_entity = killing.as_ref().and_then(|(_, attacker)| *attacker);
            let mut catalyst_xp = 0;
            let consumed = self.catalyst_takes_experience(at, |game| {
                catalyst_xp = game
                    .guest_kill_experience(conn, killing_entity, reward as i32)
                    .max(0) as u32;
                catalyst_xp
            });
            // CatalystListener: with experience to take, the player who last
            // hurt them gets KILL_MOB_NEAR_SCULK_CATALYST.
            if consumed && catalyst_xp > 0 {
                self.guest_catalyst_trigger(gi, conn, at);
            }
            // ServerPlayer.die: a spectator never reaches dropAllDeathLoot. As
            // NeoForge's `dropAllDeathLoot`, the mods hear the experience
            // (`living_experience_drop`) before the item list (`living_drops`);
            // the orbs still join the level after the items.
            let orbs = drops.is_some().then(|| {
                // LivingEntity.dropExperience: Player.isAlwaysExperienceDropper.
                let now = self.common.world.game_time;
                let attacker = if mod_api::__private::mods().is_empty() {
                    None
                } else {
                    let credit = if acting {
                        self.server.combat.0.take_hurt_by_player(now)
                    } else {
                        self.server
                            .guest_combat
                            .0
                            .get_mut(&conn)
                            .and_then(|c| c.take_hurt_by_player(now))
                    };
                    credit.and_then(|c| self.resolve_player_credit(c).map(|_| c.0))
                };
                let reward = self.guest_kill_experience(conn, killing_entity, reward as i32);
                self.living_experience_drop(id as u32, "minecraft:player", attacker, reward)
            });
            // ServerPlayer.die: the drops, the deathCount objectives, the
            // experience (`dropAllDeathLoot`), then a kill credit's `killed_by`,
            // awardKillScore and wither rose, then `deaths` and the two timers.
            self.with_dying_guest(conn, |game| {
                if let Some(eyes) = drops {
                    game.drop_inventory(eyes, &dropping);
                }
                game.score_player_died();
                if let Some(kind) = &killer {
                    game.award_stat("killed_by", kind, 1);
                }
            });
            // The experience the catalyst took does not drop
            // (`Player.skipDropExperience`), and `dropAllDeathLoot` drops it
            // before the rose.
            if let Some(reward) = orbs
                && !consumed
            {
                self.spawn_orbs(at, reward as u32);
            }
            self.guest_killed_player_trigger(conn, at, killing);
            self.guest_award_kill_score(conn, at);
            self.with_dying_guest(conn, |game| {
                game.award_custom_stat("deaths", 1);
                game.stats.reset_death_timers();
            });
            // `ServerPlayer.die`'s `broadcastEntityEvent`, after the message,
            // the loot and the experience: Fabric's AFTER_DEATH. The source is
            // the one `living_death` heard, so a death it cancelled returns
            // above and is never told here.
            if let Some(fatal) = &fatal
                && mod_api::__private::listened(mod_api::HookId::LivingDeathPost)
            {
                self.native_living_death_post(mod_api::__private::LivingDeathPostCall {
                    entity: id as u32,
                    kind: "minecraft:player",
                    damage_type: &fatal.damage_type,
                    attacker: fatal.causing,
                    player: Some(uuid),
                    credited_player: None,
                    message_key: cause.as_deref(),
                });
            }
        }
    }
    /// The entity behind a combat record's id, as `trigger`'s predicates see
    /// it: an installed, displaced or parked player, or a mob.
    pub(in crate::game) fn guest_entity_view(
        &self,
        id: u32,
        trigger: &str,
    ) -> Option<sunrise_sim::advancements::EntityView> {
        use sunrise_sim::advancements::EntityView;
        match self.combat_player(id) {
            // `player_entity_for` takes `&mut self` (the compound for `nbt`);
            // this view of the killer has the player's records without it.
            Some(CombatPlayer::InPlace) => Some(self.player_entity_for_any(&[trigger])),
            Some(CombatPlayer::Displaced(depth, body)) => self.displaced_player_entity_for(
                &self.server.lan_displaced_players.0[depth],
                body,
                trigger,
            ),
            Some(CombatPlayer::Parked(g)) => Some(EntityView {
                kind: "minecraft:player".into(),
                position: g.position,
                sneaking: g.sneaking,
                input_sneak: g.input & 32 != 0,
                ..Default::default()
            }),
            None => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| self.mob_entity(m)),
        }
    }
    /// The type of the entity a dying guest's combat record credits with the
    /// kill (`LivingEntity.getKillCredit`): a player (the one in place or a
    /// guest), a mob (one gone already credits nobody, as vanilla's reference
    /// no longer resolves), a mob whose own blast this tick killed the guest,
    /// or the ender dragon.
    pub(in crate::game::server_side) fn guest_kill_credit_type(&self, conn: u32) -> Option<String> {
        let id = self
            .guest_combat_record(conn)?
            .kill_credit(self.common.world.game_time)?;
        if self.combat_player(id).is_some() {
            return Some("minecraft:player".into());
        }
        if let Some(m) = self.common.mobs.iter().find(|m| m.id == id) {
            return Some(m.kind.id.clone());
        }
        // A mob that blew up this tick has left the list, but vanilla's player
        // dies inside that blast, before the source is discarded.
        if let Some((_, kind)) = self
            .server
            .combat
            .0
            .exploded_kinds
            .iter()
            .rfind(|(m, _)| *m == id)
        {
            return Some(kind.clone());
        }
        self.common
            .dragon
            .as_ref()
            .filter(|d| d.id == id)
            .map(|_| "minecraft:ender_dragon".into())
    }
    pub fn guest_respawn(&mut self, conn: u32) {
        self.after_commit(sunrise_gameplay_common::game::transactions::After::PlayerRespawn(conn));
    }
    /// PlayerList.respawn, after its TeleportTransition and NeoForge's redirect are final.
    pub(in crate::game::server_side) fn place_cloned_player(
        &mut self,
        conn: u32,
        to: crate::game::respawn_position::Respawn,
        was_death: bool,
        copy_spawn: bool,
        missing: bool,
    ) {
        let Some(old) = self.command_guest(conn) else {
            return;
        };
        let old_spawn = old.spawn;
        let old_saved = old.respawn_elsewhere.clone();
        let old_config = old_saved.as_ref().and_then(RespawnConfig::from_saved);
        if let Some(host) = self.server.lan.as_mut()
            && let Some(play) = host.play.get_mut(&conn)
        {
            // The replacement ServerPlayer's lastEquipmentItems starts empty.
            play.last_equipment = None;
            play.inventory_heard = None;
            play.level_up_sound = Default::default();
        }
        let keep_inventory_rule = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::KEEP_INVENTORY);
        let motion = self.common.motion;
        if !self.restore_cloned_player(
            conn,
            to.dim,
            was_death,
            |g| {
                if let Some(previous) = g.new_body(!was_death) {
                    let previous = *previous;
                    g.logs_kept = Some((previous.hurt_log, previous.attack_log));
                }
                // Even a credits return constructs a new entity. Keep its restored attributes
                // and health, not the old body's tick count or body yaw.
                g.carried_tick_count = None;
                g.carried_body_yaw = Some(0.0);
                g.carried_last_hurt_mob = None;
                // Entity's constructor starts it at the origin; snapTo follows clone and copy.
                g.place([0.0; 3]);
                (g.yaw, g.pitch) = (0.0, 0.0);
                g.spawn = None;
                g.respawn_elsewhere = None;
                if !was_death {
                    return;
                }
                g.dead = false;
                g.death_time = 0;
                g.food = 20;
                g.saturation = 5.0;
                g.health = g.respawn_health(motion);
                let mut xp = Experience {
                    level: g.xp_level,
                    progress: g.xp_progress,
                    total: g.xp_total,
                };
                let keep_inventory = keep_inventory_rule || g.game_mode == SPECTATOR;
                if !keep_inventory {
                    g.inventory.fill(None);
                    g.armor.fill(None);
                    g.offhand = None;
                    g.equipment = Default::default();
                    g.selected = 0;
                    // ServerPlayer.restoreFrom copies Score with the inventory and XP.
                    g.score = 0;
                }
                xp.respawn(keep_inventory);
                g.effects = Default::default();
                (g.xp_level, g.xp_progress, g.xp_total) = (xp.level, xp.progress, xp.total);
            },
            |game| {
                if !copy_spawn {
                    return;
                }
                // NeoForge's setRespawnPosition event can veto the copy without
                // undoing a point already set by a clone listener.
                let legacy = old_config.is_none().then_some(old_spawn).flatten();
                let config = old_config.or_else(|| {
                    legacy.map(|at| {
                        RespawnConfig::forced(
                            Dim::Overworld,
                            at.map(|c| c.floor() as i32),
                            0.0,
                            0.0,
                        )
                    })
                });
                let accepted = game.as_guest(conn, |g| {
                    if !g.set_respawn_point(config) {
                        return false;
                    }
                    // Keep the save's legacy point as a literal position, not a block lookup.
                    if let Some([x, y, z]) = legacy {
                        g.spawn = Some(Vec3::new(x, y, z));
                        g.server.respawn_config = None;
                    }
                    true
                });
                if accepted == Some(true)
                    && config.is_none()
                    && let Some(host) = game.server.lan.as_mut()
                {
                    // A config in a missing dimension is preserved in the save.
                    host.hub
                        .update_guest(conn, |g| g.respawn_elsewhere = old_saved);
                }
            },
        ) {
            return;
        }
        let Some(i) = self
            .server
            .lan
            .as_ref()
            .and_then(|h| h.hub.guests.connection_index(conn))
        else {
            return;
        };
        let clock_rate = self.network_clock_rate();
        let world_spawn = self.world_spawn_position();
        let ctx = TickCtx {
            world: &mut self.common.world,
            entities: &[],
            next_entity: &mut self.server.next_entity,
            spawn: world_spawn,
            spawn_angle: self.common.level.world_spawn_angle,
            spawn_dimension: sunrise_gameplay_common::game::Dim::of_world_spawn(&self.common.level)
                .id(),
            sea_level: self
                .common
                .end
                .locator()
                .map_or(63, |terrain| terrain.sea_level()),
            difficulty: i32::from(self.common.ui.difficulty),
            difficulty_locked: self.common.level.difficulty_locked,
            clock_rate,
            tick_rate: self.common.clock.rate(),
            tick_frozen: self.common.clock.frozen(),
            tick_steps: self.common.clock.step_state(),
            collision: Some(self.common.collision),
            border: &self.common.border,
            streaming: Some(&self.server.streaming),
        };
        if let Some(host) = self.server.lan.as_mut() {
            if missing {
                host.hub.missing_respawn_block(conn);
            }
            host.hub.begin_respawn(
                i,
                [to.at.x, to.at.y, to.at.z],
                Some([to.yaw, to.pitch]),
                !was_death,
                &ctx,
            );
        }
        self.send_player_command_tree(conn);
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.finish_respawn(i);
        }
        self.bind_player_attachments(conn, false);
        if mod_api::__private::listened(mod_api::HookId::PlayerRespawned)
            && let Some(guest) = self.command_guest(conn)
        {
            let (uuid, name) = (guest.uuid, guest.name.clone());
            self.as_guest(conn, |g| g.native_player_respawned(uuid, &name, !was_death));
        }
        if !was_death {
            self.as_guest(conn, |g| {
                g.trigger_changed_dimension(
                    sunrise_gameplay_common::game::Dim::End,
                    sunrise_gameplay_common::game::Dim::Overworld,
                )
            });
        } else {
            if let Some(config) = self
                .command_guest(conn)
                .and_then(|g| g.respawn_elsewhere.as_ref())
                .and_then(RespawnConfig::from_saved)
            {
                // PlayerList.respawn uses Level.getBlockState, which loads the saved point's
                // column to FULL (unless it is outside the level's addressable bounds).
                let placement = self.common.dim;
                let anchor = match self.enter_level(config.dim) {
                    Ok(()) => {
                        let [x, y, z] = config.pos;
                        let chunk = (x >> 4, z >> 4);
                        if !self.common.world.shape.contains_y(y) {
                            false
                        } else if self.load_full_chunk(None, chunk).is_ok() {
                            self.common.is_anchor(self.common.world.block(x, y, z))
                        } else {
                            log::error!(
                                "could not load the respawn sound's saved point at {config:?}"
                            );
                            false
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "could not open {} for the respawn sound: {e}",
                            config.dim.id()
                        );
                        false
                    }
                };
                if let Err(e) = self.enter_level(placement) {
                    log::error!("could not reopen {}: {e}", placement.id());
                    return;
                }
                // Send only to this connection, using the placement level's RNG.
                if anchor {
                    let seed = self.common.world.random.next_long();
                    self.play_sound_to(
                        Target::Guest(conn),
                        "minecraft:block.respawn_anchor.deplete",
                        config.pos.map(f64::from),
                        1.0,
                        1.0,
                        mod_api::SoundSource::Blocks.index(),
                        seed,
                    );
                }
            }
            let Some(guest) = self.command_guest(conn) else {
                return;
            };
            let from = crate::game::game_mode::mode_from_id(guest.game_mode);
            if !self.common.level.hardcore || from == GameMode::Spectator {
                return;
            }
            let (uuid, name) = (guest.uuid, guest.name.clone());
            let (mut cancel, mut to) = (false, GameMode::Spectator);
            self.native_player_game_mode_changed(uuid, &name, from, &mut to, &mut cancel);
            if !cancel
                && to != from
                && let Some(host) = self.server.lan.as_mut()
            {
                host.hub.set_game_mode(
                    conn,
                    to.id(),
                    &self.common.world,
                    Some(self.common.collision),
                );
            }
        }
    }
    /// Player.remove in the source level. The close hook still writes the old player's map.
    pub fn remove_old_player_for_clone(&mut self, conn: u32, was_death: bool) {
        let Some(entity) = self
            .server
            .lan
            .as_ref()
            .and_then(|h| h.hub.guest(conn))
            .filter(|g| g.placed)
            .map(|g| g.entity_id as u32)
        else {
            return;
        };
        self.eject_player_riders(Some(entity));
        self.guest_close_menus(
            conn,
            if was_death {
                MenuRemoval::Killed
            } else {
                MenuRemoval::ChangedDimension
            },
        );
        if let Some(host) = self.server.lan.as_mut()
            && let Some(i) = host.hub.guests.connection_index(conn)
        {
            host.hub.remove_player_for_respawn(i);
            if let Some(map) = host.hub.guests.attachments_mut(conn) {
                map.retire();
                map.unbind_index();
            }
        }
        self.entity_section_forget(entity);
    }
    /// Run part of `ServerPlayer.die` on the victim's body. A hit that already
    /// has the body installed must not try to enter the same guest again.
    pub(in crate::game) fn with_dying_guest(&mut self, conn: u32, f: impl FnOnce(&mut Self)) {
        if self.server.lan_acting == Some(conn) {
            f(self);
        } else {
            self.as_guest(conn, f);
        }
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;
