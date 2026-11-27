//! The entities beyond mobs and items that a server tracks to its clients
//! (`ChunkMap.TrackedEntity`, `ServerEntity`): boats, minecarts, armor
//! stands, item frames, paintings and the entities Sunrise keeps without
//! simulating them, each as vanilla pairs it (`getAddEntityPacket` and the
//! synched data that differs from the defaults), and who rides what.
use crate::game::ServerGame;
use sunrise_protocol::entity_data::{PaintingHolder, PaintingVariant, Value, serializer};
use sunrise_sim::lan::{
    entity::{DisplayTracking, EntityState, Riding, States},
    playerdata::ints_uuid,
};
use {sunrise_core::nbt::Tag, sunrise_world::registry::BlockStateId};

/// Allocate once: a native summon may need its id before the tracking tick.
pub(in crate::game::server_side) fn assign_entity_id(next: &mut u32, id: &mut u32) -> u32 {
    if *id == 0 {
        *next += 1;
        *id = *next;
    }
    *id
}

impl ServerGame<'_> {
    /// `Entity.nextEntityId`: the id a new entity takes from the one counter.
    pub fn take_entity_id(&mut self) -> u32 {
        self.server.next_entity += 1;
        self.server.next_entity
    }
    /// Whether the id counter is within [`ENTITY_ID_HEADROOM`] of the end: then a mod's spawn is
    /// refused and the tick spawns nothing of its own, named once in the log. Ids already handed
    /// out are never taken back, so a stale handle never names a later entity.
    pub(in crate::game) fn entity_ids_exhausted(&mut self) -> bool {
        let exhausted = self.server.next_entity >= u32::MAX - ENTITY_ID_HEADROOM;
        if exhausted && !self.common.native_lifecycle.ids_exhausted_told {
            self.common.native_lifecycle.ids_exhausted_told = true;
            log::error!(
                "entity ids exhausted: {} of {} handed out; no entity spawns until the world reopens",
                self.server.next_entity,
                u32::MAX
            );
        }
        exhausted
    }
}

/// The ids kept back below `u32::MAX`: far more than one tick hands out (a whole ender dragon
/// takes nine in a row) and the provisional ids a natural spawn pass counts down from the end.
pub(in crate::game) const ENTITY_ID_HEADROOM: u32 = 1 << 24;

fn add_state<'a>(
    out: &'a mut States,
    id: u32,
    kind: &str,
    position: [f64; 3],
    yaw: f32,
    on_ground: bool,
) -> &'a mut EntityState {
    let e = out.add(id as i32, super::lan::entity_uuid(id), kind);
    e.position = position;
    e.yaw = yaw;
    e.on_ground = on_ground;
    e
}

fn state(id: u32, kind: &str, position: [f64; 3], yaw: f32, on_ground: bool) -> EntityState {
    let mut e = EntityState::default();
    e.reset(id as i32, super::lan::entity_uuid(id), kind);
    e.position = position;
    e.yaw = yaw;
    e.on_ground = on_ground;
    e
}

/// `VehicleEntity`'s DATA_ID_HURT, DATA_ID_HURTDIR and DATA_ID_DAMAGE
/// (entries 8 to 10) when they differ from their defaults (0, 1, 0).
fn vehicle_meta(e: &mut EntityState, hurt_time: i32, hurt_dir: i32, damage: f32) {
    if hurt_time != 0 {
        e.meta.push((8, serializer::INT, Value::Int(hurt_time)));
    }
    if hurt_dir != 1 {
        e.meta.push((9, serializer::INT, Value::Int(hurt_dir)));
    }
    if damage != 0.0 {
        e.meta.push((10, serializer::FLOAT, Value::Float(damage)));
    }
    e.defaults.push((8, serializer::INT, Value::Int(0)));
    e.defaults.push((9, serializer::INT, Value::Int(1)));
    e.defaults.push((10, serializer::FLOAT, Value::Float(0.0)));
}

/// `AbstractMinecart`'s DATA_ID_CUSTOM_DISPLAY_BLOCK (entry 11, an optional
/// block state, empty by default) and DATA_ID_DISPLAY_OFFSET (entry 12, the
/// kind's `getDefaultDisplayOffset` by default).
fn cart_display_data(
    e: &mut EntityState,
    shown: Option<BlockStateId>,
    offset: Option<i32>,
    default_offset: i32,
) {
    e.defaults
        .push((11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(0)));
    e.defaults
        .push((12, serializer::INT, Value::Int(default_offset)));
    if let Some(s) = shown {
        e.meta
            .push((11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(s.0 as i32)));
    }
    if let Some(o) = offset.filter(|&o| o != default_offset) {
        e.meta.push((12, serializer::INT, Value::Int(o)));
    }
}

/// An armor stand's pose (`ArmorStandPose`), each part that differs from the
/// default, as DATA_HEAD_POSE to DATA_RIGHT_LEG_POSE (entries 16 to 21).
fn push_stand_poses(meta: &mut Vec<(u8, i32, Value)>, a: &sunrise_sim::entities::ArmorStand) {
    for (i, (rotation, default)) in a
        .pose
        .iter()
        .zip(sunrise_sim::entities::ArmorStand::DEFAULT_POSE)
        .enumerate()
    {
        if *rotation != default {
            meta.push((
                16 + i as u8,
                serializer::ROTATIONS,
                Value::Floats(rotation.to_vec()),
            ));
        }
    }
}

#[cfg(test)]
fn stand_poses(a: &sunrise_sim::entities::ArmorStand) -> Vec<(u8, i32, Value)> {
    let mut meta = Vec::new();
    push_stand_poses(&mut meta, a);
    meta
}

impl ServerGame<'_> {
    /// The entities players started and stopped tracking, told to the mods in
    /// each player's order with the hub in `self.lan` and no player's state
    /// swapped in (`ServerEntity.addPairing` and `removePairing`). Output a
    /// tracking pass kept back (`TrackingBatch::held`) is sent up to each
    /// start's bundle or stop's removal before the mods hear it, so what a
    /// listener sends the player follows that packet; the rest after the last.
    pub(in crate::game::server_side) fn run_native_tracking(&mut self) {
        let batches = match self.server.lan.as_mut() {
            Some(host) => std::mem::take(&mut host.hub.native_tracking),
            None => return,
        };
        let tell = self.common.is_logical_server();
        for mut batch in batches {
            // Packet order; the stops a respawn or a level change made without one come first.
            batch.events.sort_by_key(|e| e.at.unwrap_or(0));
            let mut held = std::mem::take(&mut batch.held).into_iter();
            let mut sent = 0;
            // Removals a captured sound still needs go out after it (`finish_captured_sounds`):
            // their stops are told then, from `lan_broadcast_effects`.
            let mut waiting = Vec::new();
            let mut after_sounds = Vec::new();
            for mut e in std::mem::take(&mut batch.events) {
                if let Some(at) = e.at.filter(|&at| at > sent) {
                    let part = held.by_ref().take(at - sent).collect();
                    waiting.extend(self.send_tracking_out(batch.conn, part));
                    sent = at;
                }
                if !e.start && waiting.contains(&e.entity) {
                    e.at = None;
                    after_sounds.push(e);
                } else if tell {
                    self.tell_tracking(batch.player, &batch.name, &e);
                }
            }
            let _ = self.send_tracking_out(batch.conn, held.collect());
            if !after_sounds.is_empty()
                && let Some(host) = self.server.lan.as_mut()
            {
                host.hub
                    .tracking_after_sounds
                    .push(sunrise_sim::lan::hub::TrackingBatch {
                        events: after_sounds,
                        held: Vec::new(),
                        ..batch
                    });
            }
        }
    }

    /// The stops whose removal waited for a captured sound, told once `finish_captured_sounds`
    /// has sent that removal.
    pub(in crate::game::server_side) fn run_tracking_after_sounds(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if host.hub.tracking_after_sounds.is_empty() {
            return;
        }
        let later = std::mem::take(&mut host.hub.tracking_after_sounds);
        host.hub.native_tracking.extend(later);
        self.run_native_tracking();
    }

    fn send_tracking_out(&self, conn: u32, part: Vec<sunrise_sim::lan::Body>) -> Vec<i32> {
        match self.server.lan.as_ref() {
            Some(host) => host.hub.send_tracking_out(conn, part),
            None => Vec::new(),
        }
    }

    /// One start or stop of tracking, told to the mods.
    pub(in crate::game::server_side) fn tell_tracking(
        &mut self,
        player: u128,
        name: &str,
        e: &sunrise_sim::lan::entity::TrackEvent,
    ) {
        let (id, kind) = (e.entity as u32, &*e.tracked.kind);
        if e.start {
            self.native_player_start_tracking(player, name, id, e.tracked.uuid, kind);
        } else {
            self.native_player_stop_tracking(player, name, id, e.tracked.uuid, kind);
        }
    }

    /// Who rides what (`Entity.getPassengers`): the players in their vehicles and
    /// mobs on mobs.
    pub fn lan_riding(&mut self, guests: &[(u32, i32)], riding: &mut Riding) {
        riding.clear();
        // Players arrive in boarding order; the first rider drives.
        for &(vehicle, guest) in guests {
            riding.add(vehicle as i32, guest);
        }
        for m in self.common.mobs.iter().filter(|m| !m.removed) {
            if let Some(vehicle) = m.extra.mount {
                riding.add(vehicle as i32, m.id as i32);
            }
        }
        for (&vehicle, pass) in &mut riding.passengers {
            // Players occupy the leading seats, as in `position_riders`.
            // `extra.rider` orders mob passengers, not a player's driver seat.
            if guests.iter().any(|&(id, _)| id as i32 == vehicle) {
                continue;
            }
            if let Some(driver) = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == vehicle as u32)
                .and_then(|m| m.extra.rider)
                && let Some(pos) = pass.iter().position(|&p| p == driver as i32)
                && pos != 0
            {
                let d = pass.remove(pos);
                pass.insert(0, d);
            }
        }
        let mut mixed = sunrise_level::fast_hash::FxHashSet::default();
        for row in &self.common.resident_entities.general.rows {
            if let Some(vehicle) = row.mount
                && !self
                    .common
                    .persist
                    .removals
                    .contains(&super::commands::Target::General(row.id))
            {
                riding.add(vehicle as i32, row.id as i32);
                mixed.insert(vehicle);
            }
        }
        for b in &self.common.bolts {
            if let Some(vehicle) = b.mount
                && !self
                    .common
                    .persist
                    .removals
                    .contains(&super::commands::Target::Bolt(b.id))
            {
                riding.add(vehicle as i32, b.id as i32);
                mixed.insert(vehicle);
            }
        }
        for vehicle in mixed {
            let seats = self.seated_passengers(vehicle);
            if let Some(passengers) = riding.passengers.get_mut(&(vehicle as i32)) {
                passengers
                    .sort_by_key(|&id| seats.iter().position(|&(_, rider, _)| rider == id as u32));
            }
        }
        // Players carry passengers only when a mod seated a mob on one.
        self.tell_vehicle_players(riding);
    }

    /// Boats, minecarts, armor stands, item frames, paintings and the
    /// entities kept unsimulated, as guests track them.
    pub fn lan_object_states(&mut self, out: &mut States, drain_minecart_steps: bool, after: u32) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let hub = &host.hub;
        let guests: Vec<(u128, i32)> = hub
            .guests
            .iter()
            .filter(|g| hub.here(g))
            .map(|g| (g.uuid, g.entity_id))
            .collect();
        for b in self.common.boats.iter().filter(|b| b.id > after) {
            let p = b.position;
            let leash = self.boat_leash_link(b, &guests);
            let e = add_state(
                out,
                b.id,
                &b.entity_type(),
                [p.x, p.y, p.z],
                b.yaw,
                b.ground.on_ground,
            );
            // Leashable.setLeashedTo / dropLeash: who holds the boat's lead.
            e.leash = leash;
            e.velocity = [b.velocity.x, b.velocity.y, b.velocity.z];
            vehicle_meta(e, b.hurt_time, b.hurt_dir, b.damage);
            if b.fire_ticks > 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }

            // AbstractBoat.DATA_ID_PADDLE_LEFT and DATA_ID_PADDLE_RIGHT.
            // Omitted false values must reset clients that saw a true value,
            // including when the controlling passenger dismounts mid-stroke.
            for (k, paddling) in b.paddling.iter().enumerate() {
                e.defaults
                    .push((11 + k as u8, serializer::BOOLEAN, Value::Bool(false)));
                if *paddling {
                    e.meta
                        .push((11 + k as u8, serializer::BOOLEAN, Value::Bool(true)));
                }
            }
            // AbstractBoat.DATA_ID_BUBBLE_TIME, back to 0 once the boat is off
            // the column.
            if b.bubble_time != 0 {
                e.meta
                    .push((13, serializer::INT, Value::Int(b.bubble_time)));
            }
            e.defaults.push((13, serializer::INT, Value::Int(0)));
        }
        let improvements = self.minecart_improvements();
        self.common.carts.for_each_mut(|c| {
            if c.id <= after {
                return;
            }
            let p = c.position;
            let shown = c.custom_display.as_ref().and_then(|t| {
                sunrise_sim::entity_nbt::state_from_nbt(self.common.pack.assets.registry, t)
            });
            let default = super::minecarts::default_display_offset(c.kind) as i32;
            let e = add_state(
                out,
                c.id,
                c.kind.id(),
                [p.x, p.y, p.z],
                c.yaw,
                c.on_ground(),
            );
            e.pitch = c.pitch;
            e.velocity = [c.velocity.x, c.velocity.y, c.velocity.z];
            if improvements {
                // Joining another dimension only takes a spawn snapshot; its
                // regular tracking tick still owns the unsent steps.
                e.minecart_steps = Some(if drain_minecart_steps {
                    std::mem::take(&mut c.lerp_steps)
                } else {
                    Vec::new()
                });
            }
            vehicle_meta(e, c.hurt_time, c.hurt_dir, c.damage);
            cart_display_data(e, shown, c.custom_offset, default);
            match c.kind {
                // MinecartFurnace.DATA_ID_FUEL.
                super::minecarts::CartKind::Furnace => {
                    e.defaults
                        .push((13, serializer::BOOLEAN, Value::Bool(false)));
                    if c.fuel > 0 {
                        e.meta.push((13, serializer::BOOLEAN, Value::Bool(true)));
                    }
                }
                // MinecartCommandBlock.DATA_ID_COMMAND_NAME.
                super::minecarts::CartKind::CommandBlock => {
                    // and DATA_ID_LAST_OUTPUT (a component, defaults empty).
                    let empty = Tag::String("".into());
                    e.defaults
                        .push((13, serializer::STRING, Value::String(String::new())));
                    if !c.synced_command.is_empty() {
                        e.meta.push((
                            13,
                            serializer::STRING,
                            Value::String(c.synced_command.clone()),
                        ));
                    }
                    if let Some(output) = &c.synced_output
                        && *output != empty
                    {
                        e.meta
                            .push((14, serializer::COMPONENT, Value::Text(output.clone())));
                    }
                    e.defaults
                        .push((14, serializer::COMPONENT, Value::Text(empty)));
                }
                _ => {}
            }
        });
        for a in self.common.armor_stands.iter().filter(|a| a.id > after) {
            let p = a.position;
            let e = add_state(
                out,
                a.id,
                "minecraft:armor_stand",
                [p.x, p.y, p.z],
                a.yaw,
                a.ground.on_ground,
            );
            e.head_yaw = a.head_yaw;
            e.velocity = [a.velocity.x, a.velocity.y, a.velocity.z];
            let mut flags = 0i8;
            if a.is_on_fire() {
                flags |= 1;
            }
            if a.invisible {
                flags |= 0x20;
            }
            if flags != 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(flags)));
            }
            // LivingEntity.DATA_HEALTH_ID: 1 until the stand's max health is set.
            e.meta.push((9, serializer::FLOAT, Value::Float(a.health)));
            // ArmorStand.DATA_CLIENT_FLAGS: small 1, arms 4, no base plate 8, marker 16.
            let flags = i8::from(a.small)
                | i8::from(a.show_arms) << 2
                | i8::from(a.no_base_plate) << 3
                | i8::from(a.marker) << 4;
            if flags != 0 {
                e.meta.push((15, serializer::BYTE, Value::Byte(flags)));
            }
            push_stand_poses(&mut e.meta, a);
            // Entity.DATA_CUSTOM_NAME and DATA_CUSTOM_NAME_VISIBLE: a hologram's text.
            if let Some(name) = &a.custom_name {
                e.meta.push((
                    2,
                    serializer::OPTIONAL_COMPONENT,
                    Value::OptText(Some(sunrise_game_data::scoreboard::json_to_tag(
                        &name.component(),
                    ))),
                ));
                if a.name_visible {
                    e.meta.push((3, serializer::BOOLEAN, Value::Bool(true)));
                }
            }
            // A name taken away or hidden again (`/data`) is sent as none / false.
            e.defaults
                .push((2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)));
            e.defaults
                .push((3, serializer::BOOLEAN, Value::Bool(false)));
            // A pose put back to its default (`/data`) is sent as that default.
            e.defaults.push((15, serializer::BYTE, Value::Byte(0)));
            for (i, default) in sunrise_sim::entities::ArmorStand::DEFAULT_POSE
                .iter()
                .enumerate()
            {
                e.defaults.push((
                    16 + i as u8,
                    serializer::ROTATIONS,
                    Value::Floats(default.to_vec()),
                ));
            }
            // Every stored slot, by `EquipmentSlot` ordinal: the wire's slot numbers.
            for (slot, item) in a.equipment.iter().enumerate() {
                if item.is_some() {
                    let Some(host) = self.server.lan.as_mut() else {
                        return;
                    };
                    e.equipment.push((slot as u8, host.hub.stack(item)));
                }
            }
            self.stand_attributes(&a.live, &mut e.attributes);
        }
        for f in self.common.item_frames.iter().filter(|f| f.id > after) {
            // ItemFrame.getAddEntityPacket: the block it hangs in and its direction.
            let e = add_state(out, f.id, f.kind(), f.pos.map(f64::from), 0.0, false);
            e.pitch = match f.facing {
                0 => 90.0,
                1 => -90.0,
                _ => 0.0,
            };
            e.yaw = match f.facing {
                2 => 180.0,
                4 => 90.0,
                5 => 270.0,
                _ => 0.0,
            };
            e.data = i32::from(f.facing);
            if f.invisible {
                e.meta.push((0, serializer::BYTE, Value::Byte(0x20)));
            }
            // HangingEntity.DATA_DIRECTION, south by default: a frame turned back to south
            // (`entity_set_hanging_direction`) clears the clients' value.
            if f.facing != 3 {
                e.meta
                    .push((8, serializer::DIRECTION, Value::Id(i32::from(f.facing))));
            }
            e.defaults.push((8, serializer::DIRECTION, Value::Id(3)));
            if f.item.is_some() {
                let Some(host) = self.server.lan.as_mut() else {
                    return;
                };
                e.meta.push((
                    9,
                    serializer::ITEM_STACK,
                    Value::Item(host.hub.stack(&f.item)),
                ));
            }
            if f.rotation != 0 {
                e.meta
                    .push((10, serializer::INT, Value::Int(i32::from(f.rotation))));
            }
        }
        for (p, &id) in self.server.knot_ids.iter().filter(|&(_, &id)| id > after) {
            // LeashFenceKnotEntity.getAddEntityPacket: the fence's block.
            add_state(
                out,
                id,
                "minecraft:leash_knot",
                p.map(f64::from),
                0.0,
                false,
            );
        }
        for p in self.common.paintings.iter().filter(|p| p.id > after) {
            let e = add_state(
                out,
                p.id,
                "minecraft:painting",
                p.pos.map(f64::from),
                0.0,
                false,
            );
            // Direction.get2DDataValue (south 0, west 1, north 2, east 3) to its
            // 3D data value and yaw.
            let facing = [3, 4, 2, 5][usize::from(p.facing & 3)];
            e.yaw = f32::from(p.facing & 3) * 90.0;
            e.data = facing;
            if facing != 3 {
                e.meta.push((8, serializer::DIRECTION, Value::Id(facing)));
            }
            e.defaults.push((8, serializer::DIRECTION, Value::Id(3)));
            // Painting.DATA_PAINTING_VARIANT_ID, a registry holder (id + 1),
            // set unless it is the registry's first variant (VariantUtils.getAny).
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            if let Some(id) = host
                .hub
                .registry_id("minecraft:painting_variant", &p.variant)
                && id != 0
            {
                let asset_id = p
                    .asset
                    .as_deref()
                    .map(|a| {
                        if a.contains(':') {
                            a.to_owned()
                        } else {
                            format!("minecraft:{a}")
                        }
                    })
                    .unwrap_or_else(|| p.variant.to_string());
                e.meta.push((
                    9,
                    serializer::PAINTING_VARIANT,
                    Value::PaintingVariant(PaintingHolder::Reference {
                        id,
                        name: p.variant.to_string(),
                        variant: PaintingVariant {
                            width: p.width,
                            height: p.height,
                            asset_id,
                            title: None,
                            author: None,
                        },
                    }),
                ));
            }
        }
        for c in self.common.cushions.iter().filter(|c| c.id > after) {
            let p = c.position;
            let e = add_state(
                out,
                c.id,
                "minecraft:cushion",
                [p.x, p.y, p.z],
                c.yaw,
                false,
            );
            // Cushion.DATA_COLOR, set unless it is the default white.
            if c.color != 0 {
                e.meta
                    .push((8, serializer::DYE_COLOR, Value::Id(i32::from(c.color))));
            }
        }
        // DATA_ITEM, `ItemStack.EMPTY` by default.
        for o in self.common.ominous_spawners.iter().filter(|o| o.id > after) {
            let e = add_state(
                out,
                o.id,
                "minecraft:ominous_item_spawner",
                o.position,
                0.0,
                false,
            );
            if o.synced_fire {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }
            e.meta.push((
                8,
                serializer::ITEM_STACK,
                Value::Item(self.server.lan.as_mut().and_then(|host| {
                    host.hub
                        .stack(&Some(o.item.clone()).filter(|i| &*i.item != "minecraft:air"))
                })),
            ));
        }
        self.lan_projectile_states(out, after);
        self.collect_lan_general_states(out, after);
        for o in self.server.others.0.iter().filter(|o| o.id > after) {
            let p = o.position;
            let e = add_state(out, o.id, &o.kind, [p.x, p.y, p.z], o.yaw, false);
            e.pitch = o.pitch;
            e.head_yaw = o.yaw;
            if o.eye.as_ref().is_some_and(|eye| eye.synced_fire) {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }
            e.display = o.display.as_ref().map(|display| {
                DisplayTracking::new(display, self.server.lan.as_ref().map(|lan| &lan.hub))
            });
        }
    }
}

impl ServerGame<'_> {
    /// `ClientboundAddEntityPacket(Entity, ServerEntity, int)`: the entity's
    /// own UUID (`Entity.getUUID`), the one `/data`, saves and UUID
    /// selectors use, so a client can name it back (the crosshair UUID that
    /// `ClientSuggestionProvider.getSelectedEntities` suggests).
    pub fn lan_entity_uuids(&mut self, states: &mut States) {
        // A bare state is never paired: its UUID is never read.
        for e in states.as_mut_slice().iter_mut().filter(|e| !e.bare) {
            e.uuid = ints_uuid(self.uuid_for(e.id as u32));
            if !self.common.persist.sync_body_position.is_empty() {
                e.sync_position |= self
                    .common
                    .persist
                    .sync_body_position
                    .contains(&(e.id as u32));
            }
            if let Some(bits) = self.common.persist.forced_meta.get(&(e.id as u32)) {
                e.forced_meta = *bits;
            }
        }
    }

    /// ServerEntity.sendChanges clears needsSync once it has looked, and
    /// SteppedInterpolationTracker.updateTracking clears syncPosition: once the states for the
    /// tracking pass are taken.
    pub(in crate::game::server_side) fn lan_syncs_looked(&mut self) {
        self.common.persist.sync_body_position.clear();
        for m in &mut self.common.mobs {
            m.needs_sync = false;
            m.sync_position = false;
        }
        self.common.creatures.projectiles.for_each_mut(|p| {
            p.needs_sync = false;
            p.sync_velocity = false;
        });
        self.common
            .falling
            .for_each_mut(|f| f.sync_velocity = false);
        for d in &mut self.common.dropped {
            d.needs_sync = false;
        }
        self.common
            .tridents
            .iter_mut()
            .for_each(|t| t.needs_sync = false);
    }

    /// Gives the entities Sunrise keeps without an id one (`Entity.id`, from
    /// the same counter as the rest) before they are first tracked.
    pub fn lan_assign_ids(&mut self) {
        // LeashFenceKnotEntity: the tie made the knot of a fence post
        // (game/leads.rs `tie_knot`); the tracker shows that same entity.
        // `notifyLeasheeRemoved` discards it once the last lead is gone.
        self.forget_untied_knots();
        let next = &mut self.server.next_entity;
        use super::entity_residency::assign_resident_ids;
        assign_resident_ids!(self.common.tnts, t, next, None);
        assign_resident_ids!(self.common.falling, f, next, Some(&f.attachment_lifetime));
        assign_resident_ids!(
            self.common.creatures.projectiles,
            p,
            next,
            Some(&p.attachment_lifetime)
        );
        assign_resident_ids!(self.common.tridents, t, next, None);
        assign_resident_ids!(self.common.rockets, r, next, None);
        assign_resident_ids!(self.common.crystals, c, next, None);
        assign_resident_ids!(self.common.bolts, b, next, Some(&b.attachment_lifetime));
        assign_resident_ids!(self.common.ominous_spawners, o, next, None);
        crate::game::server_side::potions_server::thrown_ids(&mut self.common.potions).for_each(
            |id| {
                assign_entity_id(next, id);
            },
        );
        self.common.end.eye_ids().for_each(|id| {
            assign_entity_id(next, id);
        });
        assign_resident_ids!(self.common.breath_clouds, b, next, None);
    }

    /// A mod's general entity rows (#792), as the tracker reads them. Their type key is no
    /// entity type a client knows, so pairing introduces them on `sunrise:general_entity`
    /// (`lan/entity.rs`), to Sunrise clients only; their own data are their synced attachments.
    /// Only these states are marked general: a quarantined record, which keeps its type's key,
    /// is no row and is tracked as an unknown kind (never paired).
    fn collect_lan_general_states(&self, out: &mut States, after: u32) {
        let catalog = super::general::general_type_catalog();
        let rows = self.common.resident_entities.general.rows.iter();
        for row in rows.filter(|row| row.id > after) {
            let kind = catalog.key(row.type_slot);
            let e = out.add(
                row.id as i32,
                sunrise_sim::lan::playerdata::ints_uuid(row.uuid),
                kind,
            );
            let (p, v) = (row.position, row.velocity);
            e.position = [p.x, p.y, p.z];
            e.velocity = [v.x, v.y, v.z];
            [e.yaw, e.pitch] = row.rotation;
            e.head_yaw = row.rotation[0];
            e.general = Some(catalog.tracking(row.type_slot));
            if let Some(name) = &row.custom_name {
                e.meta.push((
                    2,
                    serializer::OPTIONAL_COMPONENT,
                    Value::OptText(Some(sunrise_game_data::scoreboard::json_to_tag(
                        &name.component(),
                    ))),
                ));
            }
            if row.custom_name_visible {
                e.meta.push((3, serializer::BOOLEAN, Value::Bool(true)));
            }
            // `Entity.DATA_SHARED_FLAGS_ID`: bit 0 is on fire (`Entity.isOnFire`), bit 6 glowing.
            let on_fire = row.base.fire_ticks > 0 && !catalog.fire_immune(row.type_slot);
            let flags = u8::from(on_fire) | (u8::from(row.base.glowing) << 6);
            if flags != 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(flags as i8)));
            }
            // `Entity.DATA_SILENT` (4) and `DATA_NO_GRAVITY` (5), sent while set.
            if row.base.silent {
                e.meta.push((4, serializer::BOOLEAN, Value::Bool(true)));
            }
            if row.base.no_gravity {
                e.meta.push((5, serializer::BOOLEAN, Value::Bool(true)));
            }
            e.defaults.push((0, serializer::BYTE, Value::Byte(0)));
            // A name taken away or hidden again is sent as none / false.
            e.defaults
                .push((2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)));
            e.defaults
                .push((3, serializer::BOOLEAN, Value::Bool(false)));
            e.defaults
                .push((4, serializer::BOOLEAN, Value::Bool(false)));
            e.defaults
                .push((5, serializer::BOOLEAN, Value::Bool(false)));
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn lan_general_states(&self, out: &mut States, after: u32) {
        self.collect_lan_general_states(out, after);
    }

    /// Primed TNT, falling blocks, projectiles, tridents, firework rockets and
    /// end crystals, as guests track them.
    fn lan_projectile_states(&mut self, out: &mut States, after: u32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let hub = &mut host.hub;
        let chunks = &self.common.world.chunks;
        let owner = |who: Option<sunrise_sim::mobs::Who>| match who {
            Some(sunrise_sim::mobs::Who::Mob(id)) => id as i32,
            Some(sunrise_sim::mobs::Who::Player) | None => 0,
        };
        let at = |p: sunrise_core::geometry::Vec3| [p.x, p.y, p.z];
        for f in self.common.fangs.list.iter().filter(|f| f.id > after) {
            let e = add_state(out, f.id, "minecraft:evoker_fangs", f.pos, f.yaw, false);
            if f.synced_fire {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }
        }
        // LightningBolt: each client draws and flashes its own; sync inherited Entity data.
        for b in self.common.bolts.iter().filter(|b| b.id > after) {
            let e = add_state(
                out,
                b.id,
                "minecraft:lightning_bolt",
                b.at,
                b.rotation[0],
                b.on_ground,
            );
            e.pitch = b.rotation[1];
            e.velocity = at(b.velocity);
            let flags =
                i8::from(b.fire_ticks > 0 || b.has_visual_fire) | (i8::from(b.glowing) << 6);
            if flags != 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(flags)));
            }
            if b.air != 300 {
                e.meta.push((1, serializer::INT, Value::Int(b.air)));
            }
            if let Some(name) = &b.custom_name {
                e.meta.push((
                    2,
                    serializer::OPTIONAL_COMPONENT,
                    Value::OptText(Some(name.clone())),
                ));
            }
            if b.custom_name_visible {
                e.meta.push((3, serializer::BOOLEAN, Value::Bool(true)));
            }
            if b.silent {
                e.meta.push((4, serializer::BOOLEAN, Value::Bool(true)));
            }
            if b.no_gravity {
                e.meta.push((5, serializer::BOOLEAN, Value::Bool(true)));
            }
            if b.ticks_frozen != 0 {
                e.meta
                    .push((7, serializer::INT, Value::Int(b.ticks_frozen)));
            }
            e.defaults.push((0, serializer::BYTE, Value::Byte(0)));
            e.defaults.push((1, serializer::INT, Value::Int(300)));
            e.defaults
                .push((2, serializer::OPTIONAL_COMPONENT, Value::OptText(None)));
            e.defaults
                .push((3, serializer::BOOLEAN, Value::Bool(false)));
            e.defaults
                .push((4, serializer::BOOLEAN, Value::Bool(false)));
            e.defaults
                .push((5, serializer::BOOLEAN, Value::Bool(false)));
            e.defaults.push((7, serializer::INT, Value::Int(0)));
        }
        for t in self.common.tnts.iter().filter(|t| t.id > after) {
            let e = add_state(
                out,
                t.id,
                "minecraft:tnt",
                at(t.position),
                0.0,
                t.ground.on_ground,
            );
            e.velocity = at(t.velocity);
            // PrimedTnt.DATA_FUSE_ID, 80 by default; requiresPrecisePosition
            // while it touches a block.
            if t.fuse != 80 {
                e.meta.push((8, serializer::INT, Value::Int(t.fuse)));
            }
            e.precise = t.ground.on_ground;
        }
        for f in self.common.falling.iter().filter(|f| f.id > after) {
            let e = add_state(
                out,
                f.id,
                "minecraft:falling_block",
                at(f.position),
                0.0,
                f.ground.on_ground,
            );
            e.velocity = at(f.velocity);
            e.sync_velocity = f.sync_velocity;
            // getAddEntityPacket: the block state; DATA_START_POS.
            e.data = f.state.0 as i32;
            if f.no_gravity {
                e.meta.push((5, serializer::BOOLEAN, Value::Bool(true)));
            }
            let start = [f.start_pos.x, f.start_pos.y, f.start_pos.z];
            if start != [0; 3] {
                e.meta.push((8, serializer::BLOCK_POS, Value::Pos(start)));
            }
        }
        for p in self
            .common
            .creatures
            .projectiles
            .iter()
            .filter(|p| p.id > after)
        {
            use sunrise_sim::projectiles::ProjectileKind as K;
            // A spectral arrow is its own entity type.
            let kind = p.entity_type();
            let data = match p.guest.and_then(|conn| hub.guest(conn)) {
                Some(g) => g.entity_id,
                None => owner(p.owner),
            };
            let e = add_state(out, p.id, kind, at(p.position), p.yaw, p.ground.on_ground);
            e.pitch = p.pitch;
            e.velocity = at(p.velocity);
            e.needs_sync = p.needs_sync;
            e.sync_velocity = p.sync_velocity;
            // Projectile.getAddEntityPacket: the owner's id.
            e.data = data;
            let uuid = e.uuid;
            // Entity.DATA_SHARED_FLAGS_ID's FLAG_ONFIRE: Entity.baseTick's
            // `setSharedFlagOnFire(remainingFireTicks > 0)`, which a client's
            // isOnFire reads to draw the flames (a Flame bow's arrow).
            let fire = p.arrow.as_ref().map_or(p.synced_fire, |a| a.on_fire());
            if fire {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }
            if p.air_supply != 300 {
                e.meta.push((1, serializer::INT, Value::Int(p.air_supply)));
            }
            match &p.kind {
                K::Arrow { critical, .. } => {
                    let a = p.arrow.as_deref();
                    push_arrow_meta(
                        &mut e.meta,
                        *critical,
                        a.is_some_and(|a| a.no_physics),
                        a.map_or(0, |a| a.pierce),
                        a.is_some_and(|a| a.in_ground) || p.stuck,
                    );
                    // Arrow.ID_EFFECT_COLOR, -1 by default.
                    let color = p.arrow_effect_color();
                    if color != -1 {
                        e.meta.push((11, serializer::INT, Value::Int(color)));
                    }
                }
                // ThrownTrident.ID_LOYALTY (none for a mob's) and ID_FOIL, on
                // AbstractArrow's flags.
                K::Trident => {
                    let a = p.arrow.as_deref();
                    push_arrow_meta(
                        &mut e.meta,
                        false,
                        a.is_some_and(|a| a.no_physics),
                        0,
                        a.is_some_and(|a| a.in_ground) || p.stuck,
                    );
                    if a.is_some_and(|a| a.pickup_item.has_any_enchantments()) {
                        e.meta.push((12, serializer::BOOLEAN, Value::Bool(true)));
                    }
                }
                // WitherSkull.DATA_DANGEROUS.
                K::WitherSkull { dangerous: true } => {
                    e.meta.push((8, serializer::BOOLEAN, Value::Bool(true)));
                }
                // ThrowableItemProjectile.DATA_ITEM_STACK: the potion's
                // `potion_contents`, whose effects the guest's splash and
                // whose colour its drawing need. A plain splash potion is the
                // client's default and is not sent.
                K::Potion(stack, _) if stack.extra.is_some() || stack.count != 1 => {
                    e.meta.push((
                        8,
                        serializer::ITEM_STACK,
                        Value::Item(hub.stack(&Some((**stack).clone()))),
                    ));
                }
                // Fireball.DATA_ITEM_STACK: the item it is drawn as, which a
                // guest has no other way of learning. A plain fire charge is
                // the client's default and is not sent.
                K::SmallFireball | K::LargeFireball { .. }
                    if p.item.as_ref().is_some_and(|i| {
                        &*i.item != "minecraft:fire_charge" || i.count != 1 || i.extra.is_some()
                    }) =>
                {
                    e.meta
                        .push((8, serializer::ITEM_STACK, Value::Item(hub.stack(&p.item))));
                }
                _ => {}
            }
            // ChunkMap.addEntity (#7461): a fresh arrow joined before its first tick
            // is paired as it joined with IN_GROUND=false, and its first update is deferred
            // to the next tick's send_changes so impact updates (e.g. point-blank wall
            // collision) arrive after the client's first tick has completed.
            if !hub.tracks(p.id as i32)
                && crate::game::native_entity_refs::entity_section_accessible(
                    chunks,
                    at(p.position),
                )
                && let Some(a) = p.arrow.as_deref()
                && let (Some(spawn_pos), Some(spawn_vel), Some((pitch, yaw))) =
                    (a.spawn_pos, a.spawn_velocity, a.spawn_rot)
            {
                let mut joined = state(p.id, kind, at(spawn_pos), yaw, false);
                joined.pitch = pitch;
                joined.velocity = at(spawn_vel);
                joined.needs_sync = p.needs_sync;
                joined.data = data;
                joined.uuid = uuid;
                if a.on_fire() {
                    joined.meta.push((0, serializer::BYTE, Value::Byte(1)));
                }
                match &p.kind {
                    K::Arrow { critical, .. } => {
                        push_arrow_meta(&mut joined.meta, *critical, a.no_physics, a.pierce, false);
                        let color = p.arrow_effect_color();
                        if color != -1 {
                            joined.meta.push((11, serializer::INT, Value::Int(color)));
                        }
                    }
                    K::Trident => {
                        push_arrow_meta(&mut joined.meta, false, a.no_physics, 0, false);
                        if a.pickup_item.has_any_enchantments() {
                            joined
                                .meta
                                .push((12, serializer::BOOLEAN, Value::Bool(true)));
                        }
                    }
                    _ => {}
                }
                hub.note_fresh_joined(&[joined]);
            }
        }
        // Thrown potions and bottles: ThrowableItemProjectile.DATA_ITEM_STACK
        // (the potion's colour), which a plain bottle's type already has.
        for (id, kind, stack, position, velocity) in self
            .common
            .potions
            .thrown_tracked()
            .filter(|(id, ..)| *id > after)
        {
            let e = add_state(out, id, kind, at(position), 0.0, false);
            e.velocity = at(velocity);
            if stack.extra.is_some() || stack.count != 1 {
                e.meta.push((
                    8,
                    serializer::ITEM_STACK,
                    Value::Item(hub.stack(&Some(stack.clone()))),
                ));
            }
        }
        // EyeOfEnder: its item is the plain eye (DATA_ITEM_STACK's default).
        for (id, position, velocity) in self
            .common
            .end
            .eyes_tracked()
            .filter(|(id, ..)| *id > after)
        {
            let e = add_state(out, id, "minecraft:eye_of_ender", position, 0.0, false);
            e.velocity = velocity;
        }
        for t in self.common.tridents.iter().filter(|t| t.id > after) {
            let data = match t.guest.and_then(|conn| hub.guest(conn)) {
                Some(g) => g.entity_id,
                None => owner(Some(sunrise_sim::mobs::Who::Player)),
            };
            let e = add_state(
                out,
                t.id,
                "minecraft:trident",
                at(t.position),
                t.yaw,
                t.ground.on_ground,
            );
            e.pitch = t.pitch;
            e.velocity = at(t.velocity);
            e.needs_sync = t.needs_sync;
            // Projectile.getAddEntityPacket: the owner's id, the guest's that
            // threw it.
            e.data = data;
            push_arrow_meta(&mut e.meta, false, t.no_physics, 0, t.in_ground);
            // Entity.DATA_SHARED_FLAGS_ID: FLAG_ONFIRE.
            if t.fire_ticks > 0 {
                e.meta.push((0, serializer::BYTE, Value::Byte(1)));
            }
            // ThrownTrident.ID_LOYALTY and ID_FOIL.
            if t.loyalty != 0 {
                e.meta
                    .push((11, serializer::BYTE, Value::Byte(t.loyalty as i8)));
            }
            if t.stack.has_any_enchantments() {
                e.meta.push((12, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        for r in self.common.rockets.iter().filter(|r| r.id > after) {
            let plain = &*r.item.item == "minecraft:firework_rocket"
                && r.item.count == 1
                && r.item.extra.is_none();
            let item = (!plain).then(|| hub.stack(&Some(r.item.as_ref().clone())));
            let boosted = match r.guest {
                Some(conn) => hub
                    .guests
                    .iter()
                    .find(|g| g.conn == conn)
                    .map(|g| g.entity_id),
                None => None,
            };
            let e = add_state(
                out,
                r.id,
                "minecraft:firework_rocket",
                at(r.position),
                0.0,
                r.ground.on_ground,
            );
            e.velocity = at(r.velocity);
            // Entity.DATA_SILENT (index 4): default listed so clearing Silent:1b syncs too.
            if r.silent {
                e.meta.push((4, serializer::BOOLEAN, Value::Bool(true)));
            }
            e.defaults
                .push((4, serializer::BOOLEAN, Value::Bool(false)));
            // DATA_ID_FIREWORKS_ITEM once set from a stack, DATA_ATTACHED_TO_TARGET,
            // DATA_SHOT_AT_ANGLE.
            if let Some(item) = item {
                e.meta.push((8, serializer::ITEM_STACK, Value::Item(item)));
            }
            if r.attached
                && let Some(player) = boosted
            {
                e.meta
                    .push((9, serializer::OPTIONAL_UNSIGNED_INT, Value::Id(player + 1)));
            }
            if r.angled {
                e.meta.push((10, serializer::BOOLEAN, Value::Bool(true)));
            }
        }
        for c in self.common.crystals.iter().filter(|c| c.id > after) {
            let e = add_state(
                out,
                c.id,
                "minecraft:end_crystal",
                at(c.position),
                0.0,
                false,
            );
            // EndCrystal.DATA_BEAM_TARGET and DATA_SHOW_BOTTOM (true by default).
            // The beam's default is listed so that clearing it (the respawn
            // sequence ending) is sent to the guests instead of leaving the old beam.
            e.defaults
                .push((8, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(None)));
            if c.beam.is_some() {
                e.meta
                    .push((8, serializer::OPTIONAL_BLOCK_POS, Value::OptPos(c.beam)));
            }
            if !c.show_bottom {
                e.meta.push((9, serializer::BOOLEAN, Value::Bool(false)));
            }
        }
        // The dragon's breath and lingering potion clouds (AreaEffectCloud's DATA_RADIUS,
        // DATA_WAITING for its 20-tick wait, and DATA_PARTICLE): the
        // client draws them from these (`tick_remote_clouds`).
        let breath = hub.particle(
            &sunrise_level::particle_options::ParticleOptions::simple(
                sunrise_level::particle_options::ParticleType::DragonBreath,
            ),
            None,
        );
        for b in self.common.breath_clouds.iter().filter(|b| b.id > after) {
            // The hub is out of `self.lan` while these states are built, so
            // the particle must be encoded through the hub passed in: going
            // through `network_particle` would drop it and leave the client
            // on the default white particle.
            let particle = match &b.potion {
                Some(p) => hub.particle(&p.particle(), None),
                None => breath.clone(),
            };
            let e = add_state(out, b.id, "minecraft:area_effect_cloud", b.at, 0.0, false);
            e.meta.push((8, serializer::FLOAT, Value::Float(b.radius)));
            if b.waiting() {
                e.meta.push((9, serializer::BOOLEAN, Value::Bool(true)));
            }
            if let Some(p) = &particle {
                e.meta
                    .push((10, serializer::PARTICLE, Value::Particle(p.clone())));
            }
            e.defaults
                .push((9, serializer::BOOLEAN, Value::Bool(false)));
        }
        out.extend(self.lan_dragon_state());
    }
}

/// `AbstractArrow`'s ID_FLAGS (critical 1, no physics 2), PIERCE_LEVEL and
/// IN_GROUND, when they differ from the defaults.
fn push_arrow_meta(
    meta: &mut Vec<(u8, i32, Value)>,
    critical: bool,
    no_physics: bool,
    pierce: u8,
    in_ground: bool,
) {
    let flags = i8::from(critical) | i8::from(no_physics) << 1;
    if flags != 0 {
        meta.push((8, serializer::BYTE, Value::Byte(flags)));
    }
    if pierce != 0 {
        meta.push((9, serializer::BYTE, Value::Byte(pierce as i8)));
    }
    if in_ground {
        meta.push((10, serializer::BOOLEAN, Value::Bool(true)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NBT constructors assign ids. Reinstall unbound owners to exercise delayed tracking.

    #[test]
    fn summoned_ids_are_unique_and_survive_the_tracking_pass() {
        let mut next = 40;
        let mut roots = [0; 3];
        let returned = roots.each_mut().map(|id| assign_entity_id(&mut next, id));
        assert_eq!(returned, [41, 42, 43]);
        // Tracking sees the same roots plus an unrelated deferred entity.
        for id in &mut roots {
            assign_entity_id(&mut next, id);
        }
        let mut later = 0;
        assert_eq!(assign_entity_id(&mut next, &mut later), 44);
        assert_eq!(roots, returned);
        assert_eq!(next, 44);
    }

    #[test]
    fn armor_stand_poses_send_only_what_differs() {
        let mut stand = sunrise_sim::entities::ArmorStand::new(1, Default::default(), 0.0);
        stand.pose[0] = [10.0, 20.0, 0.0];
        let meta = stand_poses(&stand);
        assert_eq!(
            meta,
            vec![(
                16,
                serializer::ROTATIONS,
                Value::Floats(vec![10.0, 20.0, 0.0])
            )]
        );
        let plain = sunrise_sim::entities::ArmorStand::new(1, Default::default(), 0.0);
        assert!(stand_poses(&plain).is_empty());
    }

    #[test]
    #[ignore = "daily"]
    fn a_stands_add_entity_packet_keeps_head_yaw_separate_from_view_yaw() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use sunrise_core::geometry::Vec3;
        use sunrise_protocol::packets::play::Clientbound;
        use sunrise_sim::entities::ArmorStand;
        use sunrise_sim::lan::entity::{Entities, Tracker};

        if run_fresh_cases(
            "game::server_side::lan_entities::tests::a_stands_add_entity_packet_keeps_head_yaw_separate_from_view_yaw",
            "SUNRISE_STAND_PAIR_TEST",
            &["pair"],
        ) {
            return;
        }
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let at = [3.0, 64.0, 5.0];
            let mut stand = ArmorStand::new(40, Vec3::new(at[0], at[1], at[2]), 45.0);
            stand.head_yaw = 90.0;
            game.armor_stands.push(stand);
            let packet = track_stands(game, &mut Entities::default(), &mut Tracker::default())
                .into_iter()
                .find_map(|packet| match packet {
                    Clientbound::AddEntity(p) if p.id == 40 => Some(p),
                    _ => None,
                })
                .expect("the tracker sent the stand's AddEntity");
            assert_eq!([packet.yaw, packet.head_yaw], [32, 64]);
        });
    }

    /// Track from a guest's chunk for one stand update interval. This lets the
    /// test observe both initial pairing and updates to that same viewer.
    fn track_stands(
        game: &mut crate::game::ServerGame<'_>,
        level: &mut sunrise_sim::lan::entity::Entities,
        tracker: &mut sunrise_sim::lan::entity::Tracker,
    ) -> Vec<sunrise_protocol::packets::play::Clientbound> {
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        use sunrise_sim::lan::entity::Viewer;

        let mut rows = States::default();
        game.lan_object_states(&mut rows, false, 0);
        let rows: Vec<_> = rows.as_slice().iter().collect();
        let mut out = Vec::new();
        let chunks = [(0, 0)].into_iter().collect();
        let sync = game.server.lan.as_ref().unwrap().hub.sync_data();
        let mut viewers = [Viewer {
            entity_id: 12,
            spectator: false,
            permission: 0,
            sunrise: false,
            general_entity: false,
            moved: false,
            position: [3.0, 64.0, 5.0],
            view_distance: 2,
            chunks: &chunks,
            sources: None,
            far: &[],
            attachment_sender: None,
            tracker,
            out: &mut out,
        }];
        let riding = Riding::default();
        for _ in 0..sunrise_sim::lan::tracking::settings("minecraft:armor_stand").1 {
            level.tick(&rows, &riding, &mut viewers, None, |kind| {
                sync.id("minecraft:entity_type", kind)
            });
        }
        out.iter()
            .map(|body| {
                let mut reader = Reader::new(body);
                Clientbound::decode(reader.varint().unwrap(), reader.rest()).unwrap()
            })
            .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn stand_rotation_commands_update_the_head_for_an_already_paired_viewer() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use sunrise_protocol::packets::play::Clientbound;
        use sunrise_sim::lan::entity::{Entities, Tracker};

        if run_fresh_cases(
            "game::server_side::lan_entities::tests::stand_rotation_commands_update_the_head_for_an_already_paired_viewer",
            "SUNRISE_STAND_COMMAND_YAW_TEST",
            &["rotate", "teleport", "teleport_without_rotation"],
        ) {
            return;
        }
        let case = std::env::var("SUNRISE_STAND_COMMAND_YAW_TEST").unwrap();
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let run = |game: &mut crate::game::ServerGame<'_>, command| {
                game.server.chat_capture = Some(Vec::new());
                let ok = game.as_console_in_spawn_level(|g| g.run_command(command));
                let feedback = game.server.chat_capture.take().unwrap();
                assert!(
                    ok,
                    "{command}: {:?}",
                    feedback.iter().map(|m| m.plain()).collect::<Vec<_>>()
                );
            };
            let stand = game
                .summon_with_nbt(
                    "minecraft:armor_stand",
                    [3.0, 64.0, 5.0],
                    r#"{Tags:["turn_me"],Rotation:[45f,0f]}"#,
                )
                .unwrap();
            let Some(crate::game::commands::Target::Stand(id)) = stand else {
                panic!("the loaded entity was not an armor stand");
            };
            let id = id as i32;
            let (command, expected) = match case.as_str() {
                "rotate" => ("rotate @e[tag=turn_me,limit=1] 90 0", [64, 64]),
                "teleport" => ("teleport @e[tag=turn_me,limit=1] 4 64 5 135 0", [96, 96]),
                "teleport_without_rotation" => {
                    // The head can be turned independently of view before /tp.
                    game.armor_stands.iter_mut().next().unwrap().head_yaw = 90.0;
                    ("teleport @e[tag=turn_me,limit=1] 4 64 5", [32, 32])
                }
                _ => unreachable!(),
            };
            let mut level = Entities::default();
            let mut tracker = Tracker::default();
            let paired = track_stands(game, &mut level, &mut tracker);
            assert!(
                paired
                    .iter()
                    .any(|p| matches!(p, Clientbound::AddEntity(a) if a.id == id))
            );
            run(game, command);
            let packets = track_stands(game, &mut level, &mut tracker);
            assert!(
                packets.iter().any(|p| {
                    matches!(p, Clientbound::RotateHead(r) if r.id == id && r.head_yaw == expected[1])
                }),
                "no RotateHead with yaw {} after {command}",
                expected[1]
            );
            let late = track_stands(game, &mut Entities::default(), &mut Tracker::default());
            assert!(
                late.iter().any(|p| {
                    matches!(p, Clientbound::AddEntity(a) if a.id == id && [a.yaw, a.head_yaw] == expected)
                }),
                "no AddEntity with yaw {expected:?} after {command}"
            );
        });
    }

    /// Issue #8050: a cart's custom display block and offset reach clients
    /// as entries 11 and 12, the offset only off the kind's default.
    #[test]
    fn cart_display_block_and_offset_are_synched() {
        let mut e = state(1, "minecraft:minecart", [0.0; 3], 0.0, false);
        cart_display_data(&mut e, Some(BlockStateId(42)), Some(14), 6);
        assert_eq!(
            e.meta,
            vec![
                (11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(42)),
                (12, serializer::INT, Value::Int(14)),
            ]
        );
        assert_eq!(
            e.defaults,
            vec![
                (11, serializer::OPTIONAL_BLOCK_STATE, Value::Id(0)),
                (12, serializer::INT, Value::Int(6)),
            ]
        );
        let mut plain = state(2, "minecraft:hopper_minecart", [0.0; 3], 0.0, false);
        cart_display_data(&mut plain, None, Some(1), 1);
        assert!(plain.meta.is_empty());
        assert_eq!(plain.defaults[1], (12, serializer::INT, Value::Int(1)));
    }

    #[test]
    fn add_state_and_meta_helpers_reuse_pooled_capacity() {
        let mut states = States::default();
        let (kind_ptr, meta_ptr, meta_cap) = {
            let e = add_state(
                &mut states,
                10,
                "minecraft:armor_stand",
                [1.0, 2.0, 3.0],
                45.0,
                true,
            );
            let mut s = sunrise_sim::entities::ArmorStand::new(10, Default::default(), 0.0);
            s.pose[0] = [5.0, 0.0, 0.0];
            push_stand_poses(&mut e.meta, &s);
            vehicle_meta(e, 2, 1, 3.5);
            (e.kind.as_ptr(), e.meta.as_ptr(), e.meta.capacity())
        };
        assert!(meta_cap >= 2);

        states.clear();
        let (kind_ptr2, meta_ptr2, meta_cap2) = {
            let e = add_state(
                &mut states,
                11,
                "minecraft:boat",
                [4.0, 5.0, 6.0],
                0.0,
                false,
            );
            vehicle_meta(e, 0, 1, 0.0);
            (e.kind.as_ptr(), e.meta.as_ptr(), e.meta.capacity())
        };
        assert_eq!(kind_ptr2, kind_ptr);
        assert_eq!(meta_ptr2, meta_ptr);
        assert_eq!(meta_cap2, meta_cap);
    }
}

#[cfg(test)]
mod vehicle_meta_tests {
    use super::*;

    #[test]
    fn vehicle_wobble_state_carries_its_defaults() {
        let mut e = state(5, "minecraft:minecart", [0.0; 3], 0.0, false);
        vehicle_meta(&mut e, 0, 1, 0.0);
        assert!(e.meta.is_empty());
        assert_eq!(
            e.defaults,
            vec![
                (8, serializer::INT, Value::Int(0)),
                (9, serializer::INT, Value::Int(1)),
                (10, serializer::FLOAT, Value::Float(0.0)),
            ]
        );
        vehicle_meta(&mut e, 4, -1, 3.0);
        assert_eq!(e.meta.len(), 3);
    }
}

use sunrise_gameplay_common::game::minecarts::MinecartRailGameExt as _;
