//! The client side of animated block entities (`sunrise_client::block_entity_anim`):
//! their client tickers (`Level.tickBlockEntities` on the client level) and
//! the block events that drive them (`ClientboundBlockEventPacket` reaching
//! `BlockEntity.triggerEvent`). The renderer reads the resulting state through
//! [`Game::block_anim`] and [`Game::chest_lid`].
use crate::game::ServerGame;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_gameplay_common::game::block_entity_anims_common::PLAYER;
use sunrise_level::{
    block_entity_ticks::{BellEntity, ConduitServer, Ticker, closer_to_center_than},
    shulker_box::{Shulker, ShulkerStatus},
};
use sunrise_level::{
    collisions::{SceneCollisions, no_collision_empty},
    containers::BlockEntity,
    effects::EffectInstance,
};
use sunrise_sim::mobs::{Damage, DamageKind};

impl ServerGame<'_> {
    /// A player opened (`open`) or closed the container at `key`: the opener
    /// count block event (`ContainerOpenersCounter` / `ShulkerBoxBlockEntity`
    /// `startOpen`/`stopOpen`) for chests, ender chests and shulker boxes,
    /// on both halves of a double chest (`CompoundContainer`). True when the
    /// count went from 0 to 1 or back to 0: the only moments the container
    /// plays its open or close sound.
    pub(in crate::game::server_side) fn container_opener_event(
        &mut self,
        key: (i32, i32, i32),
        open: bool,
    ) -> bool {
        // Spectators never count as openers.
        !self.common.spectator() && self.container_openers_changed(key, open)
    }
    /// `container_opener_event` for a player who is not a spectator (a guest).
    pub fn container_openers_changed(&mut self, key: (i32, i32, i32), open: bool) -> bool {
        // Declared containers use their own counter, even on a lid-bearing template.
        if self.common.mod_menu_at(key).is_some() {
            return false;
        }
        let pos = [key.0, key.1, key.2];
        let state = self.common.world.block(pos[0], pos[1], pos[2]).0;
        match self.server.be_anims.ticker(state) {
            Some(Ticker::Lid) => {
                let edge = self.lid_openers_changed(key, open);
                if let Some(other) = self.common.chest_other_half(key) {
                    self.lid_openers_changed(other, open);
                }
                edge
            }
            _ if self
                .common
                .res
                .facts()
                .get(self.common.world.block(pos[0], pos[1], pos[2]))
                .name
                .as_ref()
                == "minecraft:barrel" =>
            {
                self.barrel_openers_changed(key, open)
            }
            Some(Ticker::Shulker { .. }) => {
                let shulker = self.server.be_anims.shulkers.entry(pos).or_default();
                let (count, edge) = if open {
                    shulker.start_open()
                } else {
                    shulker.stop_open()
                };
                self.block_entity_event(pos, 1, count);
                self.lan_block_event(pos, 1, count);
                edge
            }
            _ => false,
        }
    }
    /// `ContainerOpenersCounter.incrementOpeners` / `decrementOpeners` of the
    /// chest at `key` (the host and the guests are counted), then
    /// `ChestBlockEntity.signalOpenCount`: block event 1 with the count, here
    /// and for the guests. True when the count crossed zero.
    pub(in crate::game::server_side) fn lid_openers_changed(
        &mut self,
        key: (i32, i32, i32),
        open: bool,
    ) -> bool {
        let (before, count) = self.count_opener(key, open);
        let pos = [key.0, key.1, key.2];
        self.block_entity_event(pos, 1, count);
        self.lan_block_event(pos, 1, count);
        // TrappedChestBlockEntity.signalOpenCount: its signal is the count
        // (`TrappedChestBlock.getSignal`), so the blocks around it and
        // below it hear of a change.
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if before != count
            && self.common.res.facts().get(state).name.as_ref() == "minecraft:trapped_chest"
        {
            let p = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.trapped_chest_update_neighbors(level, ticks, p, state);
            });
        }
        (before == 0) != (count == 0)
    }
    /// `BarrelBlockEntity.openersCounter`: the first opener opens the barrel
    /// (`open=true`), the last one to leave closes it (`updateBlockState`).
    /// True when the count crossed zero.
    pub(in crate::game::server_side) fn barrel_openers_changed(
        &mut self,
        key: (i32, i32, i32),
        open: bool,
    ) -> bool {
        let (before, count) = self.count_opener(key, open);
        let edge = (before == 0) != (count == 0);
        if edge {
            self.set_bool_prop(
                key,
                "open",
                count > 0,
                sunrise_world::level::flags::UPDATE_ALL,
            );
        }
        edge
    }
    /// One opener more (`open`) or less at `key`: the count before and after
    /// (`ContainerOpenersCounter.incrementOpeners` / `decrementOpeners`).
    pub(in crate::game::server_side) fn count_opener(
        &mut self,
        key: (i32, i32, i32),
        open: bool,
    ) -> (i32, i32) {
        let count = self.server.lid_openers.entry(key).or_default();
        let before = *count;
        *count = (before + if open { 1 } else { -1 }).max(0);
        let after = *count;
        if after == 0 {
            self.server.lid_openers.remove(&key);
        }
        // `ContainerOpenersCounter.incrementOpeners`: the first opener
        // schedules the five-tick recheck of who still has the container open
        // (`scheduleRecheck`, `CHECK_TICK_DELAY`).
        if open && before == 0 {
            let pos = BlockPos::new(key.0, key.1, key.2);
            self.with_level(|b, level, ticks| b.schedule_tick(level, ticks, pos, 5));
        }
        (before, after)
    }
    /// `BellBlockEntity.updateEntities`: the living entities whose bounding
    /// box meets the bell's block box inflated by 48.
    pub(in crate::game) fn server_bell_gather(&self, pos: [i32; 3]) -> Vec<u64> {
        let area = Aabb::new(
            Vec3::new(
                f64::from(pos[0]) - 48.0,
                f64::from(pos[1]) - 48.0,
                f64::from(pos[2]) - 48.0,
            ),
            Vec3::new(
                f64::from(pos[0]) + 49.0,
                f64::from(pos[1]) + 49.0,
                f64::from(pos[2]) + 49.0,
            ),
        );
        let mut out = Vec::new();
        if self
            .player_view()
            .is_some_and(|p| p.bounds.intersects(area))
        {
            out.push(PLAYER);
        }
        for m in &self.common.mobs {
            if !m.removed && m.bounds().intersects(area) {
                out.push(u64::from(m.id));
            }
        }
        out
    }
    /// `BellBlockEntity.updateEntities`: every gathered entity alive within 32
    /// blocks of the bell's centre gets `HEARD_BELL_TIME` (only villagers
    /// have a brain that reads it).
    pub(in crate::game) fn server_bell_heard(&mut self, pos: [i32; 3], now: i64) {
        let nearby = self
            .server
            .be_anims
            .bells
            .get(&pos)
            .and_then(|b| b.nearby.clone())
            .unwrap_or_default();
        for m in &mut self.common.mobs {
            if m.alive()
                && !m.removed
                && nearby.contains(&u64::from(m.id))
                && closer_to_center_than(pos, [m.position.x, m.position.y, m.position.z], 32.0)
            {
                sunrise_sim::mobs::villager_brain::heard_bell(m, now);
            }
        }
    }
    /// `BellBlockEntity.serverTick` and `makeRaidersGlow` on the server.
    pub(in crate::game) fn server_tick_bell(
        &mut self,
        pos: [i32; 3],
        tags: &sunrise_items::items::EntityTypeTags,
    ) {
        let player = self.player_view().map(|p| BellEntity {
            position: [p.position.x, p.position.y, p.position.z],
            alive: p.alive,
            raider: false,
        });
        let mobs = &self.common.mobs;
        let entity = |h: u64| {
            if h == PLAYER {
                return player;
            }
            let m = mobs.iter().find(|m| u64::from(m.id) == h)?;
            Some(BellEntity {
                position: [m.position.x, m.position.y, m.position.z],
                alive: m.alive() && !m.removed,
                raider: tags.has_bit(
                    m.kind.flags.entity_tag_slot,
                    sunrise_items::items::EntityTypeTags::RAIDERS,
                ),
            })
        };
        let tick = self
            .server
            .be_anims
            .bells
            .entry(pos)
            .or_default()
            .tick(pos, entity);
        let centre = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if tick.resonate {
            self.server_play_sound("minecraft:block.bell.resonate", centre, 1.0, 1.0);
        }
        if let Some((_, glow)) = tick.ended {
            // makeRaidersGlow (the server's effect).
            let targets: Vec<u32> = self
                .common
                .mobs
                .iter()
                .filter(|m| glow.contains(&u64::from(m.id)))
                .map(|m| m.id)
                .collect();
            for mob_id in targets {
                self.add_mob_effect(mob_id, EffectInstance::new("glowing", 60, 0), None);
            }
        }
    }
    /// `ShulkerBoxBlock.canOpen`: a closed box opens only when its lid has
    /// room to rise (`level.noCollision` of the half-open lid box).
    pub(in crate::game::server_side) fn shulker_can_open(&self, pos: [i32; 3]) -> bool {
        let state = self.common.world.block(pos[0], pos[1], pos[2]).0;
        let Some(Ticker::Shulker { facing }) = self.server.be_anims.ticker(state) else {
            return true;
        };
        let closed = self
            .server
            .be_anims
            .shulkers
            .get(&pos)
            .is_none_or(|s| s.status == ShulkerStatus::Closed);
        if !closed {
            return true;
        }
        let b = Shulker::lid_box(pos, facing);
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        no_collision_empty(
            &mut scene,
            Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])),
        )
    }
    /// `BellBlockEntity.serverTick`: ringing server bells belong to the world,
    /// including when no renderer is attached.
    pub(in crate::game::server_side) fn tick_bell_servers(&mut self) {
        if self.server.be_anims.bells.is_empty() {
            return;
        }
        let mut positions: Vec<_> = self.server.be_anims.bells.keys().copied().collect();
        positions.sort_unstable();
        let tags = self.items().entity_type_tags.clone();
        for pos in positions {
            if self
                .server
                .be_anims
                .ticker(self.common.world.block(pos[0], pos[1], pos[2]).0)
                != Some(Ticker::Bell)
            {
                self.server.be_anims.bells.remove(&pos);
            } else if self.block_ticking_area()(pos[0], pos[2]) {
                self.server_tick_bell(pos, &tags);
            }
        }
    }

    /// The server tickers of the bells and conduits, which a server runs in its block-entity pass.
    pub(in crate::game::server_side) fn tick_block_entity_servers(&mut self) {
        self.tick_conduit_servers();
        self.tick_bell_servers();
    }
    /// `ConduitBlockEntity.serverTick` of every conduit in the chunks that
    /// tick blocks, with or without a renderer. The conduits are looked for
    /// when the shape is checked, every 40 ticks (they have no block entity
    /// of their own here).
    pub(in crate::game::server_side) fn tick_conduit_servers(&mut self) {
        if self.common.world.game_time % 40 == 0 {
            self.find_conduits();
        }
        let ticking = self.block_ticking_area();
        let mut positions: Vec<[i32; 3]> = self
            .server
            .be_anims
            .conduit_servers
            .keys()
            .copied()
            .filter(|p| ticking(p[0], p[2]))
            .collect();
        positions.sort_unstable();
        for pos in positions {
            self.tick_conduit_server(pos);
        }
    }
    /// Sets `be_anims.conduit_servers` to the conduits in the
    /// chunks that tick blocks, keeping the state of those already known.
    pub(in crate::game::server_side) fn find_conduits(&mut self) {
        let levels = self.server.tickets.0.levels();
        let kinds = &self.server.be_anims.kinds;
        let is_conduit = |id: u32| matches!(kinds.get(id as usize), Some(Some(Ticker::Conduit)));
        let mut found = Vec::new();
        for (&(cx, cz), chunk) in &self.common.world.chunks {
            if !levels.block_ticking_at(cx * 16, cz * 16) {
                continue;
            }
            for (si, section) in chunk.sections.iter().enumerate() {
                if !section.blocks.maybe_has(is_conduit) {
                    continue;
                }
                for i in 0..4096 {
                    if is_conduit(section.get(i)) {
                        found.push([
                            cx * 16 + (i & 15) as i32,
                            self.common.world.bottom_y() + si as i32 * 16 + (i >> 8) as i32,
                            cz * 16 + ((i >> 4) & 15) as i32,
                        ]);
                    }
                }
            }
        }
        let mut old = std::mem::take(&mut self.server.be_anims.conduit_servers);
        for pos in found {
            let known = old.remove(&pos).unwrap_or_default();
            self.server.be_anims.conduit_servers.insert(pos, known);
        }
    }
    /// `ConduitBlockEntity.serverTick` of the conduit at `pos`, in a chunk
    /// that ticks blocks: the shape check every 40 ticks with its sounds, the
    /// power for players in range, the hurt for an enemy, then the ambient
    /// sounds.
    pub(in crate::game::server_side) fn tick_conduit_server(&mut self, pos: [i32; 3]) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]).0;
        if self.server.be_anims.ticker(state) != Some(Ticker::Conduit) {
            // Broken since the chunks were searched.
            self.server.be_anims.conduit_servers.remove(&pos);
            return;
        }
        if !self
            .server
            .tickets
            .0
            .levels()
            .block_ticking_at(pos[0], pos[2])
            || !self
                .common
                .border
                .contains(f64::from(pos[0]), f64::from(pos[2]))
        {
            return;
        }
        let time = self.common.world.game_time;
        let mut conduit = self
            .server
            .be_anims
            .conduit_servers
            .remove(&pos)
            .unwrap_or_default();
        let check = {
            let world = &self.common.world;
            let facts = self.common.res.facts();
            let registry = self.common.pack.assets.registry;
            let frames = self
                .common
                .res
                .items()
                .block_tags
                .get("minecraft:conduit_effect_block");
            // Level.isWaterAt: the fluid at the position is water.
            let is_water = |p: [i32; 3]| {
                facts.get(world.block(p[0], p[1], p[2])).fluid
                    == sunrise_items::blocks::Fluid::Water
            };
            let is_frame = |p: [i32; 3]| {
                registry
                    .state(world.block(p[0], p[1], p[2]))
                    .is_some_and(|s| frames.is_some_and(|f| f.contains(&s.name)))
            };
            conduit.check(pos, time, is_water, is_frame)
        };
        let centre = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if let Some(check) = check {
            if let Some(active) = check.toggled {
                let sound = if active {
                    "minecraft:block.conduit.activate"
                } else {
                    "minecraft:block.conduit.deactivate"
                };
                self.server_play_sound(sound, centre, 1.0, 1.0);
            }
            if let Some(frames) = check.frames {
                self.conduit_apply_effects(pos, frames);
                self.conduit_attack(&mut conduit, pos, frames >= 42);
            }
        }
        let (ambient, short) = conduit.ambient(time, &mut self.common.world.random);
        if ambient {
            self.server_play_sound("minecraft:block.conduit.ambient", centre, 1.0, 1.0);
        }
        if short {
            self.server_play_sound("minecraft:block.conduit.ambient.short", centre, 1.0, 1.0);
        }
        self.server.be_anims.conduit_servers.insert(pos, conduit);
    }
    /// `ConduitBlockEntity.applyEffects`: conduit power for every player
    /// in water or rain within `frames / 7 * 16` blocks.
    pub(in crate::game::server_side) fn conduit_apply_effects(
        &mut self,
        pos: [i32; 3],
        frames: usize,
    ) {
        let range = (frames / 7 * 16) as i64;
        self.for_each_player(|g| {
            let Some(p) = g.player.as_ref().filter(|p| !p.health.dead_or_dying()) else {
                return;
            };
            if g.spectator() {
                return;
            }
            let at = p.position;
            let in_water = p.fluids.was_touching_water;
            let top = at.y + f64::from(p.dimensions.height);
            // BlockPos.closerThan: the squared distance under range squared.
            let d = [at.x, at.y, at.z]
                .map(|v| v.floor() as i64)
                .iter()
                .zip(pos)
                .map(|(a, b)| (a - i64::from(b)).pow(2))
                .sum::<i64>();
            if d >= range * range || !(in_water || g.in_rain(at, top)) {
                return;
            }
            let power = EffectInstance {
                ambient: true,
                ..EffectInstance::new("conduit_power", 260, 0)
            };
            g.add_player_effect(power);
        });
    }
    /// `Entity.isInRain`: rain at the block position or at the top of the box.
    pub(in crate::game) fn in_rain(&mut self, at: Vec3, top: f64) -> bool {
        // A server's client has no live biome features to ask: it answers from
        // the chunks and the weather it was sent.
        if !self.common.is_logical_server() {
            return self.rain_falls_on(at, top);
        }
        let (p, up) = rain_positions(at, top);
        self.with_level(|b, level, _| b.is_raining_at(level, p) || b.is_raining_at(level, up))
    }
    /// [`Self::in_rain`] read without the level access (no snapshot, no flushes): a server's
    /// client asks the chunks and weather it was sent, single player the side's read-only
    /// `Level.isRainingAt`.
    pub(in crate::game) fn rain_falls_on(&self, at: Vec3, top: f64) -> bool {
        use sunrise_sim::motion::SideLevel;
        let (p, up) = rain_positions(at, top);
        if !self.common.is_logical_server() {
            return self.raining_at(p) || self.raining_at(up);
        }
        self.is_raining_at(&self.common.world, p) || self.is_raining_at(&self.common.world, up)
    }
    /// `ConduitBlockEntity.updateAndAttackTarget`: while the conduit hunts
    /// (42 frame blocks) its target stays until it dies or leaves the 8
    /// blocks, else a random enemy in water or rain is chosen; the target
    /// takes 4 magic damage. The target is the block entity's (`Target`), so
    /// a saved or `/data` one is kept or dropped as vanilla's reference is.
    pub(in crate::game::server_side) fn conduit_attack(
        &mut self,
        conduit: &mut ConduitServer,
        pos: [i32; 3],
        hunting: bool,
    ) {
        let key = (pos[0], pos[1], pos[2]);
        let saved = match self.common.world.block_entities.get(&key) {
            Some(BlockEntity::Conduit(target)) => Some(*target),
            _ => None,
        };
        // EntityReference.getLivingEntity: a reference no loaded mob answers
        // is no target, and no new one is chosen in its place this time.
        let current = match saved {
            Some(Some(uuid)) => Some(self.mob_by_uuid(uuid)),
            Some(None) => None,
            None => conduit.target.map(Some),
        };
        conduit.target = match current {
            _ if !hunting => None,
            None => self.conduit_new_target(pos),
            Some(None) => None,
            Some(Some(id)) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .filter(|m| {
                    let at = [m.position.x, m.position.y, m.position.z].map(|v| v.floor() as i64);
                    m.alive()
                        && at
                            .iter()
                            .zip(pos)
                            .map(|(a, b)| (a - i64::from(b)).pow(2))
                            .sum::<i64>()
                            < 64
                })
                .map(|m| m.id),
        };
        if saved.is_some() {
            let target = conduit.target.map(|id| self.mob_uuid(id));
            if saved != Some(target)
                && let Some(BlockEntity::Conduit(saved)) =
                    self.common.world.block_entities.get_mut(&key)
            {
                // `sendBlockUpdated`: the new target reaches the clients.
                *saved = target;
                self.mark_block_entity_updated(key);
            }
        }
        let Some(index) = conduit
            .target
            .and_then(|id| self.common.mob_index.find(&self.common.mobs, id))
        else {
            return;
        };
        let m = &mut self.common.mobs[index];
        let at = [m.position.x, m.position.y, m.position.z];
        m.damage(
            crate::game::server_side::entity_server::damage_level_of!(
                self,
                &sunrise_sim::health::NO_ATTACK,
                no_push,
                None
            ),
            4.0,
            Damage::of(DamageKind::Magic),
        );
        self.finish_mob_damage_at(index);
        self.server_play_sound("minecraft:block.conduit.attack.target", at, 1.0, 1.0);
    }
    /// `ConduitBlockEntity.selectNewTarget`: one of the enemies in water or
    /// rain whose box meets the conduit's block inflated by 8.
    pub fn conduit_new_target(&mut self, pos: [i32; 3]) -> Option<u32> {
        let [x, y, z] = pos.map(f64::from);
        let area = Aabb::new(
            Vec3::new(x - 8.0, y - 8.0, z - 8.0),
            Vec3::new(x + 9.0, y + 9.0, z + 9.0),
        );
        let near: Vec<(u32, bool, Vec3, f64)> = self
            .common
            .mobs
            .iter()
            // `instanceof Enemy`, not the monster category: the zombie horse,
            // camel husk and zombie nautilus are `Animal`s.
            .filter(|m| m.is_enemy() && !m.removed && m.bounds().intersects(area))
            .map(|m| {
                let top = m.position.y + f64::from(m.height());
                (m.id, m.fluids.in_water, m.position, top)
            })
            .collect();
        let candidates: Vec<u32> = near
            .into_iter()
            .filter(|&(_, in_water, at, top)| in_water || self.in_rain(at, top))
            .map(|(id, ..)| id)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let pick = self
            .common
            .world
            .random
            .next_int_bound(candidates.len() as i32)
            .unwrap_or(0);
        Some(candidates[pick as usize])
    }
    /// `TheEndGatewayBlockEntity.portalTick` (the server ticker), from
    /// `Level.tickBlockEntities`: the age counts up in the block entity itself,
    /// which `/data get block` and the chunk save read; the cooldown an entry
    /// started runs out, and every 2400 ticks `triggerCooldown` sends its
    /// block event.
    pub fn tick_end_gateway(&mut self, pos: (i32, i32, i32)) {
        let key = [pos.0, pos.1, pos.2];
        let initial = self.common.gateway_loaded(key);
        let server = self
            .server
            .be_anims
            .gateway_servers
            .entry(key)
            .or_insert(initial);
        let triggered = server.portal_tick();
        let age = server.age;
        let mut aged = false;
        if let Some(BlockEntity::EndGateway(g)) = self.common.world.block_entities.get_mut(&pos)
            && g.age != age
        {
            g.age = age;
            aged = true;
        }
        if aged {
            // `TheEndGatewayBlockEntity.portalTick` ends with `setChanged`: the
            // age the beam and the chunk save read moved.
            self.common.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        }
        if triggered {
            self.gateway_cooldown_event(key);
        }
    }
    /// Whether the gateway at `pos` can be entered (`!isCoolingDown`).
    pub fn gateway_available(&self, pos: [i32; 3]) -> bool {
        self.server
            .be_anims
            .gateway_servers
            .get(&pos)
            .copied()
            .unwrap_or_else(|| self.common.gateway_loaded(pos))
            .teleport_cooldown
            <= 0
    }

    /// `EndGatewayBlock.entityInside` on the server: whether the gateway at
    /// `pos` let the entity in (`triggerCooldown`, with its block event).
    pub fn gateway_enter(&mut self, pos: [i32; 3]) -> bool {
        let initial = self.common.gateway_loaded(pos);
        let entered = self
            .server
            .be_anims
            .gateway_servers
            .entry(pos)
            .or_insert(initial)
            .enter();
        if entered {
            // `TheEndGatewayBlockEntity.triggerCooldown`: event 1 here, and
            // `Level.blockEvent` so the window's copy starts the beam.
            self.gateway_cooldown_event(pos);
        }
        entered
    }
    /// `triggerCooldown`'s block event 1, on this level and for the window
    /// (`ClientboundBlockEventPacket` within 64 blocks).
    pub(in crate::game::server_side) fn gateway_cooldown_event(&mut self, pos: [i32; 3]) {
        self.block_entity_event(pos, 1, 0);
        self.lan_block_event(pos, 1, 0);
    }

    /// `BlockEntity.triggerEvent(a, b)` on the block entity at `pos`.
    pub(in crate::game) fn block_entity_event(&mut self, pos: [i32; 3], a: i32, b: i32) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]).0;
        let time = self.common.world.game_time;
        // PotentSulfurBlock.triggerEvent.
        if self
            .common
            .potent_sulfur_eruption_event((pos[0], pos[1], pos[2]))
        {
            return;
        }
        match self.server.be_anims.ticker(state) {
            Some(Ticker::Shulker { .. }) => {
                let shulker = self.server.be_anims.shulkers.entry(pos).or_default();
                if shulker.trigger_event(a, b) {
                    // Publish shulker lid status change to the collision snapshot immediately.
                    self.common.world.shulkers.insert(pos, shulker.clone());
                }
            }
            Some(Ticker::Bell) => {
                let gathered = self.server_bell_gather(pos);
                let rung = self
                    .server
                    .be_anims
                    .bells
                    .entry(pos)
                    .or_default()
                    .trigger_event(a, b, time, || gathered);
                if rung {
                    self.server_bell_heard(pos, time);
                }
            }
            _ => {}
        }
    }

    /// Copies the shulker box lids into the world, where the collision queries
    /// read them (`SceneCollisions::shulker_collision_state`), as
    /// `sync_moving_pistons` does for pistons.
    pub(in crate::game) fn sync_shulkers(&mut self) {
        self.common.world.shulkers.clear();
        self.common.world.shulkers.extend(
            self.server
                .be_anims
                .shulkers
                .iter()
                .map(|(p, s)| (*p, s.clone())),
        );
    }
}

/// `Entity.isInRain`'s two positions: `blockPosition()` and the same column at the box's top.
pub(in crate::game::server_side) fn rain_positions(at: Vec3, top: f64) -> (BlockPos, BlockPos) {
    let p = BlockPos::new(
        at.x.floor() as i32,
        at.y.floor() as i32,
        at.z.floor() as i32,
    );
    (p, BlockPos::new(p.x, top.floor() as i32, p.z))
}

#[cfg(any(test, feature = "test-support"))]
pub mod conduit_target_tests {
    //! `ConduitBlockEntity.selectNewTarget` picks `Enemy`s only, and its
    //! `Target` reaches every window through `sendBlockUpdated`; the window
    //! then spits a nautilus particle at the mob.

    /// A conduit in a loaded chunk.
    pub fn setup(game: &mut crate::game::ServerGame<'_>) {
        for command in [
            "difficulty normal",
            "forceload add 0 0",
            "setblock 8 100 8 minecraft:conduit",
        ] {
            assert!(game.run_command(command), "{command}");
        }
    }

    /// A mob of `kind` beside the conduit, in the water.
    pub fn summon(game: &mut crate::game::ServerGame<'_>, kind: &str) -> u32 {
        assert!(game.run_command(&format!("summon minecraft:{kind} 4.5 100 8.5")));
        let id = game.mobs.last().unwrap().id;
        game.mobs.last_mut().unwrap().fluids.in_water = true;
        id
    }

    pub fn attack_conduit_for_test(
        game: &mut crate::game::ServerGame<'_>,
        pos: [i32; 3],
        hunting: bool,
    ) {
        game.conduit_attack(&mut super::ConduitServer::default(), pos, hunting);
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
