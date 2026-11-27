use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `C::DamageEvent`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_damage_event_packet(&mut self, p: play::DamageEvent) {
        // ClientPacketListener.handleDamageEvent: LivingEntity.handleDamageEvent
        // jolts the walk animation and restarts the hurt flash, and the
        // hurt player's own client plays its hurt sound (`Player.getHurtSound`:
        // the damage type's effects) at `getSoundVolume` and
        // `Player.getVoicePitch`, which is 1.0 for every player. The
        // server's sound packet reaches every other player around, never
        // this one (#4149), and that copy is played at 1.0.
        // This player's own hurt tilt and flash come from the server too
        // (sent to the player itself): its own moves never hurt it.
        let me = self.client.remote.as_ref().map(|r| r.entity_id);
        let own = me == Some(p.id);
        self.pvp_hurt(p.id, own, p.direct.map(|d| Some(d) == me));
        if own {
            // For `events`: this client has no combat tracker of its own here.
            let cause = self.client.remote.as_ref().and_then(|r| {
                r.registries
                    .name("minecraft:damage_type", p.source_type)
                    .map(str::to_owned)
            });
            let by = p
                .cause
                .and_then(|id| u32::try_from(id).ok())
                .and_then(|id| {
                    self.common
                        .mob_index
                        .get(&self.common.mobs, id)
                        .map(|m| m.kind.id.to_string())
                });
            self.common
                .events
                .note_hit(cause.unwrap_or_else(|| "unknown".into()), by);
            if let Some(pl) = self.common.player.as_mut() {
                pl.health.animate_hurt();
                let sound = self.remote_hurt_sound(p.source_type);
                let pitch = player_voice_pitch();
                self.play_client_player_sound(sound, 1.0, pitch);
            }
        } else if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, p.id as u32)
        {
            m.limb_speed = 1.5;
            m.hurt_time = 10;
            // LivingEntity.handleDamageEvent.
            m.invulnerable = 20;
        } else {
            self.remote_dragon_hurt(p.id);
        }
    }
    /// `C::EntityEvent`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_entity_event_packet(
        &mut self,
        p: play::EntityEvent,
    ) -> Result<()> {
        if p.event == super::super::fireworks::FIREWORK_EXPLODE_EVENT {
            self.remote_firework_explosion(p.id);
        }
        self.remote_fangs_event(p.id as u32, p.event);
        self.remote_boat_event(p.id, p.event);
        // MinecartTNT.handleEntityEvent 70: prime the fuse the window draws.
        self.remote_tnt_cart_event(p.id, p.event);
        // Arrow.handleEntityEvent 0: a tipped arrow's burst of particles.
        if p.event == 0 {
            self.remote_arrow_puff(p.id as u32);
        }
        // LivingEntity.handleEntityEvent 2: a spear's hit felt.
        if p.event == 2 {
            self.remote_kinetic_hit(p.id);
        }
        // LocalPlayer.setPermissions also refreshes an open World Options screen.
        if (24..=28).contains(&p.event)
            && self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.entity_id == p.id)
        {
            self.common.ui.registries.permission = (p.event - 24) as u8;
            self.remote_world_options_permission(p.event >= 26);
        }
        // Player.handleEntityEvent 22 / 23: reduced debug info on / off.
        if (22..=23).contains(&p.event)
            && let Some(r) = self.client.remote.as_mut().filter(|r| r.entity_id == p.id)
        {
            r.reduced_debug_info = p.event == 22;
            self.client.debug_entries.0.invalidate();
        }
        // Ravager.handleEntityEvent 69: the roar's puff and push.
        if p.event == 69 {
            self.remote_ravager_roar(p.id);
        }
        // ClientPacketListener.handleEntityEvent 63: a sniffer's digging sound.
        if p.event == 63 {
            self.start_sniffer_digging_sound(p.id as u32);
        }
        // ClientPacketListener.handleEntityEvent 21: a guardian's hum.
        if p.event == 21 {
            self.start_guardian_sound(p.id as u32);
        }
        // ZombieVillager.handleEntityEvent 16: the cure starts.
        if p.event == 16 {
            self.zombie_villager_cure_sound(p.id as u32);
        }
        // Witch.handleEntityEvent 15: the magic puffs over its hat.
        if p.event == 15 {
            self.witch_magic_particles(p.id as u32);
        }
        // MinecartTNT.handleEntityEvent 70: primeFuse on the client.
        if p.event == 70 {
            self.remote_cart_primed(p.id as u32);
        }
        // Fox.handleEntityEvent 45: the crumbs of the food in its mouth.
        if p.event == 45 {
            self.fox_eat_particles(p.id as u32);
        }
        // Rabbit.handleEntityEvent(1): the hop clocks, and
        // Entity.spawnSprintParticle (one block-dust crumb).
        if p.event == 1 {
            let hop = {
                let m = self
                    .common
                    .mob_index
                    .get_mut(&mut self.common.mobs, p.id as u32);
                if let Some(m) = m
                    && m.species() == sunrise_sim::mobs::Species::Rabbit
                {
                    let sample = (
                        m.position,
                        m.velocity,
                        f64::from(m.width()),
                        m.ground.supporting_block.map(|p| [p.x, p.y, p.z]),
                    );
                    sunrise_sim::mobs::rabbit::hop_event(m);
                    Some(sample)
                } else {
                    None
                }
            };
            if let Some((position, movement, width, support)) = hop {
                let id = p.id as u32;
                self.sprint_particle(position, movement, support, width, |game| {
                    // this.random: the rabbit's own, x then z.
                    game.common
                        .mob_index
                        .get_mut(&mut game.common.mobs, id)
                        .map(|m| (m.random.next_double(), m.random.next_double()))
                });
            }
        }
        if p.event == 19
            && let Some(m) = self
                .common
                .mob_index
                .get_mut(&mut self.common.mobs, p.id as u32)
        {
            sunrise_sim::mobs::aquatic::squid_cycle_event(m);
        }
        // LivingEntity.handleEntityEvent 55: another player swapped
        // its main and off hand (`swapHandItems`).
        if p.event == 55
            && let Some(m) = self
                .common
                .mob_index
                .get_mut(&mut self.common.mobs, p.id as u32)
        {
            m.extra.swap_hands();
        }
        // LivingEntity.handleEntityEvent 47 to 52, 65 and 68: an
        // equipped item broke (onEquippedItemBroken).
        if let Some(slot) = sunrise_sim::mobs::item_break::equipment_break_slot(p.event) {
            self.remote_equipment_break(p.id, slot);
        }
        // EntityEvent.DEATH: start the death animation.
        if p.event == 3
            && let Some(m) = self
                .common
                .mob_index
                .get_mut(&mut self.common.mobs, p.id as u32)
        {
            m.health = 0.0;
        }
        // LivingEntity.handleEntityEvent 3 on this player: its death
        // sound, which Player.playSound plays for this client alone
        // (the server's goes to everyone else).
        if p.event == 3
            && self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.entity_id == p.id)
            && self.common.player.is_some()
        {
            let sound = "minecraft:entity.player.death";
            let pitch = player_voice_pitch();
            self.play_client_player_sound(sound, 1.0, pitch);
        }
        // The events that start a mob's animation (the warden's).
        if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, p.id as u32)
        {
            sunrise_sim::mobs::sync::entity_event(m, p.event as u8);
        }
        // LivingEntity.handleEntityEvent 46: portal particles from the
        // old position to the new one (chorus fruit, an enderman).
        if p.event == 46 {
            self.entity_teleport_particles(p.id);
        }
        // LivingEntity.handleEntityEvent 60: the poof as the dead body goes.
        if p.event == 60
            && let Some(m) = self.common.mob_index.get(&self.common.mobs, p.id as u32)
        {
            let (at, w, h) = (m.position, f64::from(m.width()), f64::from(m.height()));
            self.death_poof([at.x, at.y, at.z], w, h);
        }
        if p.event == sunrise_gameplay_common::game::totem_common::TOTEM_EVENT {
            self.remote_totem(p.id);
        }
        // Mob.handleEntityEvent 20: the spawn poof.
        if p.event == 20 && self.common.mobs.iter().any(|m| m.id == p.id as u32) {
            self.poof_particles(p.id as u32);
        }
        // The particle bursts of Villager 12 to 14 and 42, TamableAnimal
        // and AbstractHorse 6 and 7, Animal and Allay 18, Ocelot 40 and
        // 41, Dolphin 38: the table in `entity_event_burst` is the one
        // list of them; any other event draws nothing here.
        self.entity_event_particles(p.id as u32, p.event);
        // Snowball and ThrownEgg.handleEntityEvent 3: the crumbs where
        // the projectile is (the server removes it right after).
        if p.event == 3 {
            let crumbs = self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|pr| pr.id == p.id as u32)
                .and_then(|pr| {
                    let item = match pr.kind {
                        sunrise_sim::projectiles::ProjectileKind::Snowball => "minecraft:snowball",
                        sunrise_sim::projectiles::ProjectileKind::Egg => "minecraft:egg",
                        sunrise_sim::projectiles::ProjectileKind::Thrown(t) => t.item,
                        _ => return None,
                    };
                    Some(([pr.position.x, pr.position.y, pr.position.z], item))
                });
            if let Some((at, item)) = crumbs {
                self.item_burst(at, item);
            }
        }
        if p.event == sunrise_gameplay_common::game::combat_common::DROWN_EVENT {
            self.remote_drown(p.id);
        }
        // Entity events 53 and 54: honey crumbs while sliding, and the
        // ten from a landing (`HoneyBlock.showSlideParticles` /
        // `showJumpParticles`).
        if sunrise_sim::landing::honey_particle_count(p.event).is_some() {
            let at = if self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.entity_id == p.id)
            {
                self.common
                    .player
                    .as_ref()
                    .map(|q| [q.position.x, q.position.y, q.position.z])
            } else {
                self.common
                    .mobs
                    .iter()
                    .find(|m| m.id == p.id as u32)
                    .map(|m| [m.position.x, m.position.y, m.position.z])
            };
            if let Some(at) = at {
                self.honey_entity_particles(at, p.event);
            }
        }
        if p.event == sunrise_gameplay_common::game::item_use_common::USE_ITEM_COMPLETE_EVENT
            && self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.entity_id == p.id)
        {
            self.remote_complete_using();
        }
        Ok(())
    }
    /// `C::RemoveMobEffect` (`LivingEntity.removeEffectNoUpdate`): from the player itself or
    /// from another living entity whose effects this client keeps.
    pub(in crate::game) fn apply_remove_mob_effect_packet(&mut self, p: play::RemoveMobEffect) {
        let Some(id) = self.remote_effect_id(p.effect) else {
            return;
        };
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.id)
        {
            self.common.potions.effects.list.retain(|e| e.id != id);
        } else if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, p.id as u32)
        {
            m.extra.client_effects.list.retain(|e| e.id != id);
        } else if let Some(a) = self
            .common
            .armor_stands
            .iter_mut()
            .find(|a| a.id == p.id as u32)
        {
            a.client_effects.list.retain(|e| e.id != id);
        }
    }
    /// Apply one validated `SynchedEntityData` entry (`assignValue` + `onSyncedDataUpdated`).
    pub(in crate::game::client_side) fn apply_set_entity_data_entry(
        &mut self,
        id: i32,
        entry: &(u8, i32, Value),
    ) {
        let (index, _, value) = entry;
        let single = std::slice::from_ref(entry);
        let size_key = self.synched_size_key(id as u32);
        if let Some(r) = self.client.remote.as_mut() {
            if let (0, Value::Byte(flags)) = (index, value) {
                if let Some(t) = r.entities.get_mut(&id) {
                    t.on_fire = flags & 1 != 0;
                }
                if flags & 0x40 != 0 {
                    r.glowing.insert(id);
                } else {
                    r.glowing.remove(&id);
                }
            }
            if let (4, Value::Bool(silent)) = (index, value) {
                if id == r.entity_id {
                    r.local_entity_silent = *silent;
                }
                if let Some(t) = r.entities.get_mut(&id) {
                    t.silent = *silent;
                }
            }
            // Entity.DATA_CUSTOM_NAME and DATA_CUSTOM_NAME_VISIBLE of a general row.
            if let Some(row) = r.general.rows.get_mut(&id) {
                match (index, value) {
                    (2, Value::OptText(name)) => {
                        row.custom_name = name.as_ref().map(sunrise_client::entity_nbt::name_text);
                    }
                    (3, Value::Bool(visible)) => row.custom_name_visible = *visible,
                    (5, Value::Bool(no_gravity)) => row.no_gravity = *no_gravity,
                    _ => {}
                }
            }
        }
        // Entity.DATA_SHARED_FLAGS_ID, DATA_CUSTOM_NAME and
        // DATA_CUSTOM_NAME_VISIBLE: server NPCs and holograms rely on
        // these instead of saved NBT.
        if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, id as u32)
        {
            match (index, value) {
                (0, Value::Byte(flags)) => {
                    m.invisible = flags & 0x20 != 0;
                    m.glowing = flags & 0x40 != 0;
                }
                (2, Value::OptText(name)) => {
                    let text = name.as_ref().map(sunrise_sim::entity_nbt::name_text);
                    m.custom_name = text.as_ref().map(sunrise_game_data::text::Text::plain);
                    m.custom_name_text = text;
                }
                (2, Value::Text(name)) => {
                    let text = sunrise_sim::entity_nbt::name_text(name);
                    m.custom_name = Some(text.plain());
                    m.custom_name_text = Some(text);
                }
                (3, Value::Bool(visible)) => m.name_visible = *visible,
                // Entity.DATA_NO_GRAVITY: `isNoGravity` in the client's own travel.
                (5, Value::Bool(no_gravity)) => m.no_gravity = *no_gravity,
                _ => {}
            }
        }
        // Entity.DATA_NO_GRAVITY of an item, which may come before its stack
        // (`DATA_ITEM`, 8) and so before the item is made.
        if let (5, Value::Bool(no_gravity)) = (index, value) {
            if let Some(item) = self.common.dropped.iter_mut().find(|d| d.id == id as u32) {
                item.no_gravity = *no_gravity;
            } else if let Some(pending) = self
                .client
                .remote
                .as_mut()
                .and_then(|r| r.pending_items.get_mut(&id))
            {
                pending.no_gravity = *no_gravity;
            }
        }
        self.remote_mob_data(id, single);
        if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, id as u32)
            && m.species() == sunrise_sim::mobs::Species::Creeper
        {
            match (index, value) {
                // Creeper.DATA_SWELL_DIR, DATA_IS_POWERED, DATA_IS_IGNITED.
                (16, Value::Int(dir)) => m.extra.counter = *dir,
                (17, Value::Bool(powered)) => m.extra.charged = *powered,
                (18, Value::Bool(ignited)) => m.extra.flag = *ignited,
                _ => {}
            }
        }
        if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, id as u32)
        {
            match (index, value) {
                // Entity.DATA_TICKS_FROZEN: isFullyFrozen shakes it.
                (7, Value::Int(ticks)) => m.ticks_frozen = *ticks,
                // Strider.DATA_SUFFOCATING (after AgeableMob's two
                // and DATA_BOOST_TIME): a cold strider shakes.
                (19, Value::Bool(cold)) if m.species() == sunrise_sim::mobs::Species::Strider => {
                    m.extra.flag = *cold;
                }
                _ => {}
            }
        }
        // LivingEntity.DATA_LIVING_ENTITY_FLAGS: for another player
        // too, remote_mob_data's apply reads index 8 above.
        let mannequin = self
            .common
            .mobs
            .iter()
            .any(|m| m.id == id as u32 && &*m.kind.id == "minecraft:mannequin");
        let other = mannequin
            || self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.level_players.contains_key(&id) && r.entity_id != id);
        // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): another
        // player lies down in their bed (drawn lying in it), or gets up.
        if other {
            if let (14, Value::OptPos(bed)) = (index, value) {
                let code = bed.map_or(0, |b| self.common.bed_facing(b));
                let at = bed.and_then(|head| self.sleeping_position(head));
                if let Some(m) = self
                    .common
                    .mob_index
                    .get_mut(&mut self.common.mobs, id as u32)
                {
                    m.extra.ints[1] = code;
                    if bed.is_some() {
                        // setPosToBed leaves the server's position alone
                        // when the client's bed has no sleep shape.
                        if let Some(at) = at {
                            m.position = at;
                            m.previous = at;
                        }
                        m.head_pitch = 0.0;
                        m.previous_head_pitch = 0.0;
                    }
                }
            }
            // Avatar.DATA_PLAYER_MAIN_HAND (HumanoidArm: 0 left, 1
            // right): the arm a swing of the main hand moves.
            if let (15, Value::Id(arm)) = (index, value)
                && let Some(m) = self
                    .common
                    .mob_index
                    .get_mut(&mut self.common.mobs, id as u32)
            {
                m.extra.main_hand_left = *arm == 0;
            }
            // Avatar.DATA_PLAYER_MODE_CUSTOMISATION: the skin layers
            // the player shows (PlayerModelPart bits).
            if let (16, Value::Byte(bits)) = (index, value)
                && let Some(m) = self
                    .common
                    .mob_index
                    .get_mut(&mut self.common.mobs, id as u32)
            {
                m.extra.hidden_model_parts = !(*bits as u8) & 0x7F;
            }
        }
        // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): this
        // player lies down in the bed, or gets up.
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == id)
        {
            match (index, value) {
                // Player.DATA_SCORE_ID: DeathScreen reads this, not the XP packet.
                (18, Value::Int(score)) => self.player_score = *score,
                (14, Value::OptPos(bed)) => {
                    if bed.is_some() && self.common.sleeping.is_none() {
                        self.common.sleep_timer = 0;
                    }
                    self.common.sleeping = *bed;
                    if let Some(head) = *bed {
                        // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): setPosToBed.
                        let at = self.sleeping_position(head);
                        if let Some(pl) = self.common.player.as_mut() {
                            if let Some(at) = at {
                                pl.position = at;
                            }
                            pl.velocity = Vec3::ZERO;
                            pl.pitch = 0.0;
                        }
                        if let Some(at) = at {
                            self.common.previous_position = at;
                        }
                        self.common.pitch = 0.0;
                        self.snap_player_model_pitch();
                    }
                }
                // Entity.DATA_SHARED_FLAGS_ID's sprinting bit: the
                // server stops the sprint after a hit's extra
                // knockback (the flag only, as assignValues sets
                // it: the speed modifier and wasSprinting stay).
                // Its invisible bit (Entity.isInvisible): the
                // Invisibility effect's, or a plugin's that hides
                // the real body under a pose (#3788).
                (0, Value::Byte(flags)) => {
                    if let Some(pl) = self.common.player.as_mut() {
                        pl.sprinting = flags & 0x08 != 0;
                        // Entity.DATA_SHARED_FLAGS_ID's fall-flying
                        // bit (0x80, `Entity.setSharedFlag(7, ...)`):
                        // `LivingEntity.updateFallFlying` clears the
                        // glide on the server only and `LocalPlayer`
                        // never ends one itself, so this packet is
                        // the glide's own end. Applied when it is
                        // handled — the frame's poll, before its
                        // ticks — as ClientPacketListener does, so a
                        // clear that arrives mid-tick lets that tick
                        // keep its glide travel (#7616).
                        pl.fall_flying = flags & -128 != 0;
                    }
                    let on_fire = flags & 0x01 != 0;
                    if let Some(r) = self.client.remote.as_mut() {
                        r.own_invisible = flags & 0x20 != 0;
                        let was_on_fire = std::mem::replace(&mut r.own_on_fire, on_fire);
                        // The server put the fire out: the
                        // window's countdown ends with it.
                        if was_on_fire
                            && !on_fire
                            && let Some(pl) = self.common.player.as_mut()
                        {
                            pl.fire_ticks = pl.fire_ticks.min(0);
                        }
                    }
                }
                // Entity.DATA_AIR_SUPPLY_ID and
                // Player.DATA_PLAYER_ABSORPTION_ID: the bubbles
                // and golden hearts the server's player has.
                (1, Value::Int(air)) => {
                    if let Some(pl) = self.common.player.as_mut() {
                        pl.air = *air;
                    }
                }
                // Entity.DATA_TICKS_FROZEN: the server's count of the
                // powder snow freeze, which the client's own copy reads
                // for the frost overlay, the frozen hearts and the
                // shiver of its third-person model (#11217).
                (7, Value::Int(ticks)) => {
                    if let Some(pl) = self.common.player.as_mut() {
                        pl.ticks_frozen = *ticks;
                    }
                }
                // LivingEntity.DATA_ARROW_COUNT_ID and DATA_STINGER_COUNT_ID.
                (12, Value::Int(count)) => {
                    if let Some(r) = self.client.remote.as_mut() {
                        r.own_arrows = *count;
                    }
                }
                (13, Value::Int(count)) => {
                    if let Some(r) = self.client.remote.as_mut() {
                        r.own_stingers = *count;
                    }
                }
                (19 | 20, Value::Id(v)) => {
                    if let Some(r) = self.client.remote.as_mut() {
                        r.own_shoulders[usize::from(*index == 20)] =
                            (*v > 0).then(|| (*v - 1).clamp(0, 4));
                    }
                }
                (17, Value::Float(amount)) => {
                    if let Some(pl) = self.common.player.as_mut() {
                        let _ = pl.internal_set_absorption(*amount);
                    }
                }
                // LivingEntity.DATA_LIVING_ENTITY_FLAGS: the server
                // starts or stops this player's use, as an axe that
                // disables the raised shield does (Player.disableShield's
                // stopUsingItem). LocalPlayer.onSyncedDataUpdated.
                (8, Value::Byte(flags)) => {
                    let selected = self.common.selected;
                    let using = self.common.using;
                    self.common.using = synced_local_use(using, *flags, selected, |slot| {
                        self.common.stack_in(slot).is_some()
                    });
                    // onSyncedDataUpdated's useItemRemaining: getUseDuration,
                    // without startUsingItem's event.
                    if using.is_none()
                        && let Some((slot, _)) = self.common.using
                        && let Some(stack) = self.common.stack_in(slot).cloned()
                    {
                        self.common.use_duration = self.use_duration(&stack);
                        self.common.use_start =
                            sunrise_gameplay_common::game::item_use_common::UseStart::of(stack);
                        self.side_reset_client_item_use();
                    }
                }
                _ => {}
            }
        }
        // Entity.DATA_SHARED_FLAGS_ID's crouching and sprinting bits:
        // another player's or a mannequin's crouch (their model and
        // Entity.isDiscrete's name tag), and the dust a runner raises.
        if (mannequin
            || self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.level_players.contains_key(&id)))
            && let Some(m) = self
                .common
                .mob_index
                .get_mut(&mut self.common.mobs, id as u32)
            && let (0, Value::Byte(flags)) = (index, value)
        {
            m.extra.crouching = flags & 2 != 0;
            m.extra.sprinting = flags & 8 != 0;
        }
        if *index == ITEM_ENTITY_ITEM
            && let Value::Item(stack) = value
        {
            self.remote_item_data(id, stack);
        }
        // ExperienceOrb.DATA_VALUE (the icon's size).
        if *index == 8
            && let Value::Int(v) = value
            && let Some(o) = self.common.orbs.iter_mut().find(|o| o.id == id as u32)
        {
            o.value = *v;
        }
        self.remote_object_data(id, single);
        self.remote_projectile_data(id, single);
        self.remote_dragon_data(id, single);
        // `onSyncedDataUpdated` → `refreshDimensions`: `EntityEvent.Size` when
        // this value changed the copy's pose or the size it follows.
        self.native_synched_size(id as u32, size_key, *index, value);
    }
    /// `ClientPacketListener.handleSetEntityData`: require an active level, ignore absent entities,
    /// and validate/assign each entry in wire order (`SynchedEntityData.assignValues`).
    pub fn apply_set_entity_data_packet(&mut self, p: play::SetEntityData) -> Result<()> {
        if !self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.logged_in && r.reconfiguring.is_none())
        {
            return Err(raw_ingress::PacketError(
                "Cannot apply entity data without a client level.",
            )
            .into());
        }
        let Some(kind) = self.remote_entity_kind(p.id) else {
            return Ok(());
        };
        let schema = entity_data_schema::serializers(&kind);
        for entry in &p.values {
            let (index, serializer_id, _) = entry;
            if let Some(schema) = schema {
                let Some(&expected) = schema.get(usize::from(*index)) else {
                    return Err(raw_ingress::PacketError(
                        "Entity data accessor index out of range.",
                    )
                    .into());
                };
                if expected != *serializer_id {
                    return Err(raw_ingress::PacketError(
                        "Invalid entity data item type for field.",
                    )
                    .into());
                }
            } else if let Some(&expected) = entity_data_schema::ENTITY_BASE.get(usize::from(*index))
                && expected != *serializer_id
            {
                return Err(
                    raw_ingress::PacketError("Invalid entity data item type for field.").into(),
                );
            }
            self.apply_set_entity_data_entry(p.id, entry);
        }
        Ok(())
    }
    /// `C::SetEquipment`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game) fn apply_set_equipment_packet(&mut self, p: play::SetEquipment) {
        // ClientPacketListener.handleSetEquipment: what a mob or another
        // player holds and wears (EquipmentSlot ordinals).
        self.remote_object_equipment(p.id, &p.slots);
        let items: Vec<(u8, Option<Arc<str>>, Option<ItemStack>)> = p
            .slots
            .iter()
            .map(|(slot, stack)| {
                (
                    *slot,
                    stack.as_ref().map(|s| self.item_name(s.item)),
                    self.to_stack(stack),
                )
            })
            .collect();
        // CrossbowItem.isCharged: the main hand's `charged_projectiles`
        // (AvatarRenderer.getArmPose holds a loaded crossbow up).
        let charged = p.slots.iter().find(|(slot, _)| *slot == 0).map(|(_, s)| {
            s.as_ref().is_some_and(|s| {
                matches!(
                    s.component(sunrise_protocol::item::component::CHARGED_PROJECTILES),
                    Some(sunrise_protocol::item::Component::Stacks(list))
                        if !list.is_empty()
                )
            })
        });
        if let Some(m) = self
            .common
            .mob_index
            .get_mut(&mut self.common.mobs, p.id as u32)
        {
            if let Some(charged) = charged
                && &*m.kind.id == "minecraft:player"
            {
                m.extra.charged = charged;
            }
            for (slot, item, stack) in items {
                let stack = stack.or_else(|| item.map(|item| ItemStack::new(&item, 1)));
                m.set_equipment_stack(slot as usize, stack);
            }
        }
    }
    /// `C::UpdateAttributes`: its arms of `apply_packet`, guard and order kept.
    pub(in crate::game::client_side) fn apply_update_attributes_packet(
        &mut self,
        p: play::UpdateAttributes,
    ) -> Result<()> {
        // The player's own instances: its maximum health (the hearts the
        // HUD draws and the health it may hold), reach, speed and so on.
        // Then the received health again, which the old maximum clamped.
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == p.id)
        {
            let nameplate = self.remote_nameplate_values(&p.values);
            self.apply_own_attributes(p.values);
            if let Some(m) = self.client.player_model.as_mut() {
                if let Some(value) = nameplate.name_tag {
                    m.set_name_tag_distance(value);
                }
                if let Some(value) = nameplate.below_name {
                    m.set_below_name_distance(value);
                }
            }
            self.apply_remote_health();
            return Ok(());
        }
        self.remote_mob_attributes(p.id, &p.values);
        Ok(())
    }
    /// The entity type of a resident remote entity (`ClientLevel.getEntity`), if present.
    pub(in crate::game::client_side) fn remote_entity_kind(&self, id: i32) -> Option<String> {
        let r = self.client.remote.as_ref()?;
        if id == r.entity_id {
            return Some("minecraft:player".to_owned());
        }
        if let Some(t) = r.entities.get(&id) {
            return Some(t.kind.clone());
        }
        if let Some(row) = r.general.rows.get(&id) {
            return Some(row.kind().to_owned());
        }
        self.common
            .mobs
            .iter()
            .find(|m| m.id == id as u32)
            .map(|m| m.kind.id.to_string())
    }
}

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::block_props::BlockPropsGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
