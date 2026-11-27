//! Object-owned passenger memory and typed transfer loading.
use super::*;

impl crate::game::ServerGame<'_> {
    pub(super) fn mob_at(&self, object: MobObject) -> &Mob {
        match object {
            MobObject::Resident(index) => &self.common.mobs[index],
            MobObject::OffLevel(id) => self.common.persist.off_level[&id]
                .as_mob()
                .expect("mob object slot"),
        }
    }

    pub(super) fn mob_at_mut(&mut self, object: MobObject) -> &mut Mob {
        match object {
            MobObject::Resident(index) => &mut self.common.mobs[index],
            MobObject::OffLevel(id) => self
                .common
                .persist
                .off_level
                .get_mut(&id)
                .and_then(OffLevel::as_mob_mut)
                .expect("mob object slot"),
        }
    }

    /// Mutate an extant mob object without making it resident.
    pub fn mob_object_mut(&mut self, id: u32) -> Option<&mut Mob> {
        if let Some(index) = self.common.mobs.iter().position(|mob| mob.id == id) {
            return Some(&mut self.common.mobs[index]);
        }
        self.common
            .persist
            .off_level
            .get_mut(&id)
            .and_then(OffLevel::as_mob_mut)
    }

    /// A boat object, resident or retained as the root of a passenger graph.
    /// Selectors, tracking and ticking keep iterating `boats` instead.
    pub fn boat_object(
        &self,
        id: u32,
    ) -> Option<&sunrise_gameplay_common::game::boats_common::Boat> {
        self.common
            .boats
            .iter()
            .find(|boat| boat.id == id)
            .or_else(|| {
                self.common
                    .persist
                    .off_level
                    .get(&id)
                    .and_then(OffLevel::as_boat)
            })
    }

    /// A minecart object, resident or retained as the root of a passenger graph.
    /// Selectors, tracking and ticking keep iterating `carts` instead.
    pub fn cart_object(
        &self,
        id: u32,
    ) -> Option<&sunrise_gameplay_common::game::minecarts::Minecart> {
        self.common
            .carts
            .iter()
            .find(|cart| cart.id == id)
            .or_else(|| {
                self.common
                    .persist
                    .off_level
                    .get(&id)
                    .and_then(OffLevel::as_cart)
            })
    }

    /// Internal topology only: resident objects followed by retained passengers.
    /// Callers needing level membership must continue to iterate `mobs` instead.
    pub fn mob_objects(&self) -> impl Iterator<Item = &Mob> {
        self.common.mobs.iter().chain(
            self.common
                .persist
                .off_level
                .values()
                .filter_map(OffLevel::as_mob),
        )
    }

    pub(in crate::game::server_side) fn load_item_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        self.server.next_entity += 1;
        // `ItemEntity(EntityType, Level)` first: its own random draws UUID, bobOffs and yaw.
        let mut own = random_source::entity();
        let fresh = sunrise_sim::save::insecure_uuid(&mut own);
        let bob = own.next_float() * std::f32::consts::PI * 2.0;
        own.next_float();
        if let Some(mut e) = entity_nbt::item_entity_from_nbt(&tag, self.server.next_entity, bob) {
            e.random = own;
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(e.id, state);
            }
            self.remember(e.id, Some(uuid.unwrap_or(fresh)), tag);
            self.bind_saved_entity_attachments(e.id, &mut e.attachment_lifetime);
            self.common.dropped.push(e);
        }
    }

    pub(in crate::game::server_side) fn load_orb_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        mut transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        for (i, mut orb) in entity_nbt::orbs_from_nbt(&tag).into_iter().enumerate() {
            self.server.next_entity += 1;
            orb.id = self.server.next_entity;
            let id = orb.id;
            // Sunrise splits a saved `count` into orbs: the first is the saved one.
            if i == 0 {
                if let Some(state) = transferred.take() {
                    self.adopt_transferred_entity_state(id, state);
                }
                let uuid = with_constructor_uuid(uuid);
                self.remember_uuid(id, uuid, &tag);
                self.bind_saved_entity_attachments(id, &mut orb.attachment_lifetime);
            }
            // An orb that came through a portal keeps its cooldown.
            self.restore_portal_cooldown(id, &tag);
            self.common.orbs.push(orb);
            self.remember_tags(id, &tag);
        }
    }

    pub(in crate::game::server_side) fn load_arrow_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some((mut p, in_block)) =
            entity_nbt::arrow_from_nbt(&tag, player_uuid, crate::game::dispenser::tipped_effect)
        {
            if let Some(a) = p.arrow.as_mut() {
                a.last_state = in_block.and_then(|t| self.state_from_nbt(&t));
            }
            self.claim_owner(&mut p);
            // An arrow that came through a portal keeps its
            // cooldown: it does not go straight back through.
            self.server.next_entity += 1;
            p.id = self.server.next_entity;
            self.restore_portal_cooldown(p.id, &tag);
            self.remember_tags(p.id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(p.id, state);
            }
            let own_uuid = p.own_uuid(uuid);
            self.remember_uuid(p.id, Some(own_uuid), &tag);
            self.bind_saved_entity_attachments(p.id, &mut p.attachment_lifetime);
            self.add_projectile(p);
        }
    }

    pub(in crate::game::server_side) fn load_spit_or_bullet_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        let target = self.bullet_target(&tag);
        if let Some(mut p) = entity_nbt::spit_or_bullet_from_nbt(&tag, player_uuid, target) {
            self.claim_owner(&mut p);
            self.server.next_entity += 1;
            p.id = self.server.next_entity;
            self.restore_portal_cooldown(p.id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(p.id, state);
            }
            let own_uuid = p.own_uuid(uuid);
            self.remember_uuid(p.id, Some(own_uuid), &tag);
            self.bind_saved_entity_attachments(p.id, &mut p.attachment_lifetime);
            self.add_projectile(p);
            self.remember_last_projectile_tags(&tag);
        }
    }

    pub(in crate::game::server_side) fn load_fireball_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some(mut p) = entity_nbt::fireball_from_nbt(&tag, player_uuid) {
            self.claim_owner(&mut p);
            // A projectile that came through a portal keeps its cooldown.
            self.server.next_entity += 1;
            p.id = self.server.next_entity;
            self.restore_portal_cooldown(p.id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(p.id, state);
            }
            let own_uuid = p.own_uuid(uuid);
            self.remember_uuid(p.id, Some(own_uuid), &tag);
            self.bind_saved_entity_attachments(p.id, &mut p.attachment_lifetime);
            self.add_projectile(p);
            self.remember_last_projectile_tags(&tag);
        }
    }

    pub(in crate::game::server_side) fn load_throwable_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some(mut p) =
            entity_nbt::throwable_from_nbt_with_items(&tag, player_uuid, self.common.res.items())
        {
            self.claim_owner(&mut p);
            // A projectile that came through a portal keeps its cooldown.
            self.server.next_entity += 1;
            p.id = self.server.next_entity;
            self.restore_portal_cooldown(p.id, &tag);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(p.id, state);
            }
            let own_uuid = p.own_uuid(uuid);
            self.remember_uuid(p.id, Some(own_uuid), &tag);
            self.bind_saved_entity_attachments(p.id, &mut p.attachment_lifetime);
            self.add_projectile(p);
            self.remember_last_projectile_tags(&tag);
        }
    }

    pub(in crate::game::server_side) fn load_falling_transferred(
        &mut self,
        mut tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if transferred.is_none() && uuid.is_some_and(|u| self.target_with_uuid(u).is_some()) {
            return;
        }
        entity_nbt::sanitize_custom_name(&mut tag);
        if let Some((state, position, velocity, age, fall_distance)) =
            entity_nbt::falling_block_from_nbt(&tag)
            && let Some(state) =
                entity_nbt::falling_state_from_nbt(self.common.pack.assets.registry, &state)
        {
            let (no_gravity, drop_item) = entity_nbt::falling_block_flags(&tag);
            self.server.next_entity += 1;
            // A summoned one has no UUID yet: `Entity` makes a random one.
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(self.server.next_entity, state);
            }
            self.remember_uuid(self.server.next_entity, uuid, &tag);
            let uuid = self.uuid_for(self.server.next_entity);
            self.common.falling.push(FallingBlock {
                size_answer: None,
                attachment_lifetime: Default::default(),
                // Entity.load's rotation, which nothing turns afterwards.
                rotation: entity_nbt::loaded_rotation(&tag),
                join: Default::default(),
                id: self.server.next_entity,
                state,
                blocks_building: false,
                // DATA_START_POS is not persisted by vanilla.
                start_pos: sunrise_core::position::BlockPos::default(),
                position,
                previous: position,
                velocity,
                age,
                no_gravity,
                portal: sunrise_sim::nether_portal::PortalState {
                    cooldown: sunrise_sim::save::number(tag.get("PortalCooldown")).unwrap_or(0)
                        as i32,
                    process: None,
                },
                drop_item,
                fall_distance,
                ground: sunrise_world::ground_support::GroundState::from_ground_flag(
                    entity_nbt::flag(&tag, "OnGround"),
                ),
                fire_ticks: crate::game::boats::saved_fire(&tag),
                sync_velocity: false,
                stuck: Vec3::ZERO,
                damage: sunrise_sim::falling_block::Damage::from_nbt(
                    &tag,
                    self.common.res.facts().get(state).name.ends_with("anvil"),
                ),
                saved: Some(Box::new((uuid, tag.clone()))),
            });
        }
    }

    pub(in crate::game::server_side) fn load_trident_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some(n) = entity_nbt::trident_from_nbt(&tag) {
            let mut t = sunrise_sim::thrown_trident::ThrownTrident::from_nbt(
                n,
                &tag,
                self.common.pack.assets.registry,
            );
            t.id = self.take_entity_id();
            self.resolve_trident_loyalty(&mut t);
            // One that came through a portal keeps its cooldown.
            let id = t.id;
            self.common.tridents.push(t);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
        }
    }

    pub(in crate::game::server_side) fn load_rocket_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        if let Some(r) = entity_nbt::firework_rocket_from_nbt(&tag, player_uuid) {
            let id = self.take_entity_id();
            let mut rocket = sunrise_gameplay_common::game::fireworks::Rocket::from_nbt(r);
            rocket.id = id;
            // One that came through a portal keeps its cooldown.
            self.restore_portal_cooldown(id, &tag);
            self.common.rockets.push(rocket);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
        }
    }

    pub(in crate::game::server_side) fn load_tnt_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        self.load_tnt_with_portal(tag, uuid, transferred, false);
    }

    pub(in crate::game::server_side) fn load_tnt_with_portal(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
        used_portal: bool,
    ) {
        if uuid.is_some_and(|u| self.target_with_uuid(u).is_some()) {
            return;
        }
        let mut tag = tag;
        let riders = self.take_passengers(&mut tag);
        if let Some(position) = entity_nbt::position(&tag) {
            self.server.next_entity += 1;
            let id = self.server.next_entity;
            // getShortOr uses NumericTag.shortValue: floor floating values
            // to int before short narrowing; integer tags narrow directly.
            let fuse = tnt_fuse(&tag);
            self.common
                .tnts
                .push(sunrise_gameplay_common::game::tnt_common::PrimedTnt {
                    size_answer: None,
                    used_portal,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: entity_nbt::flag(&tag, "NoGravity"),
                    id,
                    position,
                    previous: position,
                    velocity: entity_nbt::motion(&tag),
                    fuse,
                    ticks: 0,
                    ground: sunrise_world::ground_support::GroundState::from_ground_flag(
                        entity_nbt::flag(&tag, "OnGround"),
                    ),
                    fall_distance: sunrise_sim::save::float(tag.get("fall_distance"))
                        .unwrap_or(0.0),
                    stuck: Vec3::ZERO,
                    in_water: false,
                    owner: None,
                    guest: None,
                    // PrimedTnt.readAdditionalSaveData: 4 by default,
                    // kept to 0..128.
                    power: entity_nbt::tnt_power(&tag),
                    block_state: tag.get("block_state").and_then(|v| {
                        entity_nbt::state_from_nbt(self.common.pack.assets.registry, v)
                    }),
                });
            // `Entity.load`: the `PortalCooldown` of one that just came through.
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
            self.load_passengers(id, riders);
        }
    }

    pub fn load_boat_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        k: &str,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) -> Option<u32> {
        // A runtime transfer owns a distinct live object: like a Mob, the copy
        // refused residency stays below the passengers admitted before it.
        // A disk copy with a taken UUID is still not constructed.
        if transferred.is_none() && uuid.is_some_and(|u| self.target_with_uuid(u).is_some()) {
            return None;
        }
        let refused = transferred.is_some() && self.uuid_admission_refused(uuid);
        if let Some((position, velocity, yaw)) = entity_nbt::boat_from_nbt(&tag) {
            let mut tag = tag;
            let riders = self.take_passengers(&mut tag);
            self.server.next_entity += 1;
            let id = self.server.next_entity;
            let (wood, chest) = sunrise_gameplay_common::game::boats_common::any_boat_wood(k)
                .unwrap_or(("oak", false));
            let mut b = sunrise_gameplay_common::game::boats_common::Boat::new(
                id,
                std::sync::Arc::from(wood),
                position,
                yaw,
            );
            b.no_gravity = entity_nbt::flag(&tag, "NoGravity");
            b.ground.on_ground = entity_nbt::flag(&tag, "OnGround");
            b.fire_ticks = crate::game::boats::saved_fire(&tag);
            b.ticks_frozen = sunrise_gameplay_common::game::boats_common::saved_ticks_frozen(&tag);
            b.pitch = entity_nbt::rotation(&tag)[1];
            // Entity.load: OnGround and fall_distance, as saved.
            (b.ground.on_ground, b.fall_distance) = entity_nbt::ground_from_nbt(&tag);
            if chest {
                b = b.with_chest();
                b.items = sunrise_sim::save::decode_items(
                    tag.get("Items"),
                    sunrise_gameplay_common::game::boats_common::CHEST_SLOTS,
                );
            }
            b.velocity = velocity;
            // AbstractBoat.readAdditionalSaveData: the lead's holder.
            b.leash_pending = sunrise_sim::mobs::leash::pending_from_nbt(tag.get("leash"));
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
            if refused {
                self.common.persist.off_level.insert(id, OffLevel::Boat(b));
            } else {
                self.bind_saved_entity_attachments(id, &mut b.attachment_lifetime);
                self.common.boats.push(b);
            }
            self.load_passengers(id, riders);
            return Some(id);
        }
        None
    }

    pub fn load_cart_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        k: &str,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) -> Option<u32> {
        // As `load_boat_transferred`: only a runtime transfer retains a refused copy.
        if transferred.is_none() && uuid.is_some_and(|u| self.target_with_uuid(u).is_some()) {
            return None;
        }
        let refused = transferred.is_some() && self.uuid_admission_refused(uuid);
        let kind = sunrise_gameplay_common::game::minecarts::CartKind::from_id(k).unwrap();
        let mut tag = tag;
        let riders = self.take_passengers(&mut tag);
        let id = self.server.next_entity + 1;
        if let Some(mut c) =
            sunrise_gameplay_common::game::minecarts::Minecart::from_nbt(&tag, id, kind)
        {
            sunrise_gameplay_common::game::minecarts::forget_unreadable_display(&mut c, |t| {
                self.state_from_nbt(t).is_some()
            });
            self.server.next_entity = id;
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
            if refused {
                self.common.persist.off_level.insert(id, OffLevel::Cart(c));
            } else {
                self.bind_saved_entity_attachments(id, &mut c.attachment_lifetime);
                self.common.carts.push(c);
            }
            self.load_passengers(id, riders);
            return Some(id);
        }
        None
    }

    #[cfg(not(feature = "test-support"))]
    pub(in crate::game::server_side) fn uuid_admission_refused(
        &self,
        uuid: Option<[i32; 4]>,
    ) -> bool {
        self.resident_uuid_conflicts(uuid)
    }

    #[cfg(feature = "test-support")]
    pub fn uuid_admission_refused(&self, uuid: Option<[i32; 4]>) -> bool {
        self.resident_uuid_conflicts(uuid)
    }

    /// `PersistentEntitySectionManager.addEntityUuid` refuses a UUID held by another resident object.
    fn resident_uuid_conflicts(&self, uuid: Option<[i32; 4]>) -> bool {
        uuid.is_some_and(|uuid| {
            self.target_with_uuid(uuid)
                .is_some_and(|target| target != crate::game::commands::Target::Player)
        })
    }

    pub fn load_mob_transferred(
        &mut self,
        tag: Tag,
        mount: Option<u32>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) -> Option<u32> {
        let uuid = entity_nbt::uuid(&tag);
        // Only disk reconstruction coalesces the player's saved copy with a
        // chunk copy. An explicit runtime token owns a distinct live object.
        if transferred.is_none()
            && let Some(id) = uuid.and_then(|u| self.restored_vehicle(u))
        {
            return Some(id);
        }
        self.server.next_entity += 1;
        // Structure mobs that generation left to finish: saves from before
        // they were finalized at placement, or a placement that could not.
        let mut tag = tag;
        Self::normalize_loaded_mob_tags(&mut tag);
        let mut finalize = false;
        if let Tag::Compound(c) = &mut tag {
            finalize = c.remove(&"sunrise:finalize".into()).is_some();
        }
        let riders = self.take_passengers(&mut tag);
        // The mob's own random makes its UUID, kept unless the tag has one.
        let (own, fresh) = sunrise_sim::mobs::living_random();
        let uuid = uuid.unwrap_or(fresh);
        let mob = entity_nbt::mob_from_nbt_with_random(
            &tag,
            self.common.res.mob_types(),
            self.server.next_entity,
            Some(self.common.pack.assets.registry),
            own,
        )?;
        let id = self.install_loaded_mob((mob, uuid), tag, mount, transferred, finalize, None);
        self.load_passengers(id, riders);
        Some(id)
    }

    pub(super) fn install_loaded_mob(
        &mut self,
        (mut m, uuid): (Mob, [i32; 4]),
        tag: Tag,
        mount: Option<u32>,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
        finalize: bool,
        admitted: Option<bool>,
    ) -> u32 {
        self.resolve_loaded_profession(&mut m);
        entity_nbt::read_mob_variants(&mut m, &tag, self.world_variants());
        if sunrise_sim::mobs::camel::is_camel(m.species()) {
            sunrise_sim::mobs::camel::finish_load(&mut m, self.common.world.game_time);
        }
        // `NeutralMob.readPersistentAngerSaveData`: the anger runs until its
        // end time, and the mob is angry at the target again, through
        // `Mob.setTarget` -> `asValidTarget`: a player the golem cannot attack
        // (`IronGolem.canAttack`) comes back as no target.
        m.extra.anger =
            sunrise_sim::mobs::anger_left(m.extra.anger_end, self.common.world.game_time);
        let anger_uuid = match tag.get("angry_at") {
            Some(Tag::IntArray(words)) if words.len() == 4 => {
                Some([words[0], words[1], words[2], words[3]])
            }
            _ => None,
        };
        // These are the vanilla NeutralMob implementors; each calls
        // readPersistentAngerSaveData, which invokes setTarget even when
        // `angry_at` is absent or cannot resolve to a living entity.
        let neutral = matches!(
            m.species(),
            sunrise_sim::mobs::Species::Wolf
                | sunrise_sim::mobs::Species::IronGolem
                | sunrise_sim::mobs::Species::Bee
                | sunrise_sim::mobs::Species::PolarBear
                | sunrise_sim::mobs::Species::Enderman
                | sunrise_sim::mobs::Species::ZombifiedPiglin
        );
        // Restore on the server whether or not a mod listens. The saved UUID
        // can name an unloaded entity, an online player, or another mob;
        // substituting the local player when there is no listener both changes
        // vanilla behavior and makes loading depend on mod subscriptions.
        let restore_target = neutral
            && self.common.is_logical_server()
            && self.common.native_lifecycle.epoch().is_some();
        m.extra.mount = mount;
        m.extra.passenger = mount.is_some();
        // The object id preserves load order even when some passengers are
        // retained in the off-level store rather than the resident list.
        m.extra.vehicle_order = i64::from(m.id);
        if finalize {
            let at = [m.position.x, m.position.y, m.position.z];
            let difficulty = self.common.local_difficulty(at);
            self.finalize_plain(
                &mut m,
                &difficulty,
                sunrise_sim::mobs::spawn_rules::Reason::Structure,
            );
        }
        sunrise_sim::mobs::armor::prepare(&m, &self.common.motion.attributes);
        let id = m.id;
        // Refuse residency, not the already-constructed object or its links.
        // A spawner tree has already run this decision in preorder.
        let resident = admitted.unwrap_or_else(|| !self.uuid_admission_refused(Some(uuid)));
        if let Some(mut state) = transferred {
            state.restore_mob(&mut m);
            self.adopt_transferred_entity_state(id, state);
        } else if m.health <= 0.0 {
            // A saved dying body has already passed die; loading only resumes tickDeath.
            self.server.died.insert(id);
        }
        self.remember(id, Some(uuid), tag);
        if resident {
            self.bind_saved_entity_attachments(id, &mut m.attachment_lifetime);
            self.common.mobs.push(m);
            let join = self.common.mobs.last().unwrap().join;
            self.flush_resident_mob_parts(id, join);
            if restore_target {
                self.native_restore_loaded_target(id, anger_uuid);
            }
        } else {
            self.flush_unjoined_mob_parts(&mut m);
            self.common.persist.off_level.insert(id, OffLevel::Mob(m));
        }
        id
    }

    /// The supported records under an entity's `Passengers`, taken out of it; the
    /// passengers Sunrise doesn't simulate stay. An entry no entity loads
    /// from goes (`EntityType.loadPassengersRecursive` mounts only a loaded
    /// one): not a compound, or no registered type (`EntityType.by`).
    pub(super) fn take_passengers(&mut self, tag: &mut Tag) -> Vec<Tag> {
        // Without the registry nothing can be told apart: keep every entry.
        let registry = self.entity_types_loaded();
        let mut riders = Vec::new();
        if let Tag::Compound(c) = tag
            && let Some(Tag::List(list)) = c.remove(&"Passengers".into())
        {
            let mut unknown = Vec::new();
            for p in list {
                match entity_nbt::kind(&p) {
                    Some(kind) if self.loads_as_passenger(&kind) => riders.push(p),
                    Some(kind)
                        if self.entity_type_exists(&kind)
                            || (sunrise_gameplay_common::game::general::quarantined_kind(
                                &kind,
                            ) && entity_nbt::position(&p).is_some_and(|at| {
                                self.general_quarantine_admits(&p, &kind, at)
                            })) =>
                    {
                        unknown.push(p);
                    }
                    _ if !registry => unknown.push(p),
                    _ => {}
                }
            }
            if !unknown.is_empty() {
                c.insert("Passengers".into(), Tag::List(unknown));
            }
        }
        riders
    }

    /// `Entity.shouldBeSaved`: the mobs riding entity `id` under `tag`'s
    /// `Passengers`, after the passengers it kept from loading. Discarded
    /// passengers cannot save even before their list entry is swept.
    pub(super) fn mob_passenger_objects(&self, id: u32) -> Vec<MobObject> {
        let mut riders: Vec<(i64, u32, MobObject)> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, mob)| mob.extra.mount == Some(id) && !mob.removed)
            .map(|(index, mob)| (mob.extra.vehicle_order, mob.id, MobObject::Resident(index)))
            .chain(
                self.common
                    .persist
                    .off_level
                    .iter()
                    .filter_map(|(&object_id, object)| {
                        let mob = object.as_mob()?;
                        (mob.extra.mount == Some(id) && !mob.removed).then_some((
                            mob.extra.vehicle_order,
                            mob.id,
                            MobObject::OffLevel(object_id),
                        ))
                    }),
            )
            .collect();
        riders.sort_by_key(|&(order, id, _)| (order, id));
        if let Some(driver) = self.common.mob_object(id).and_then(|mob| mob.extra.rider)
            && let Some(position) = riders.iter().position(|&(_, id, _)| id == driver)
            && position != 0
        {
            let driver = riders.remove(position);
            riders.insert(0, driver);
        }
        riders.into_iter().map(|(_, _, object)| object).collect()
    }

    pub fn mob_passenger_ids(&self, root: u32) -> Vec<u32> {
        self.mob_passenger_objects(root)
            .into_iter()
            .map(|object| self.mob_at(object).id)
            .collect()
    }

    /// Capture after tree serialization and before source removal. This consumes
    /// each object's runtime ownership once, independently of duplicate UUIDs.
    pub fn capture_vehicle_tree_state(&mut self, root: u32) -> Option<TransferredVehicleTree> {
        let (kind, name) = if let Some(mob) = self.common.mob_object(root) {
            (
                VehicleTreeKind::Mob,
                sunrise_game_data::identifier::normalize_id(&mob.kind.id)?,
            )
        } else if let Some(boat) = self.boat_object(root) {
            (VehicleTreeKind::Boat, boat.entity_type())
        } else if let Some(row) = self.common.general_row(root) {
            (
                VehicleTreeKind::General,
                super::super::general::general_type_catalog()
                    .key(row.type_slot)
                    .to_owned(),
            )
        } else if let Some(cart) = self.cart_object(root) {
            (VehicleTreeKind::Cart, cart.kind.id().to_owned())
        } else if self
            .common
            .armor_stands
            .iter()
            .any(|stand| stand.id == root)
        {
            (VehicleTreeKind::Stand, "minecraft:armor_stand".to_owned())
        } else {
            let other = self.server.others.0.iter().find(|other| other.id == root)?;
            (VehicleTreeKind::Other, other.kind.clone())
        };
        let passengers = self
            .vehicle_passenger_ids(root)
            .into_iter()
            .map(|id| self.capture_vehicle_tree_state(id))
            .collect::<Option<Vec<_>>>()?;
        Some(TransferredVehicleTree {
            source_id: root,
            kind,
            name,
            state: self.take_transferred_entity_state(root),
            passengers,
        })
    }

    pub fn prepare_vehicle_transfer(
        &mut self,
        mut tag: Tag,
        tree: TransferredVehicleTree,
    ) -> Option<PreparedVehicleTransfer> {
        if entity_nbt::kind(&tag).as_deref() != Some(tree.name.as_str()) {
            log::warn!(
                "Sunrise: transferred vehicle type does not match its captured object state"
            );
            return None;
        }
        let tags = self.take_passengers(&mut tag);
        if tags.len() != tree.passengers.len() {
            log::warn!(
                "Sunrise: transferred vehicle topology does not match its captured object state"
            );
            return None;
        }
        let passengers = tags
            .into_iter()
            .zip(tree.passengers)
            .map(|(tag, child)| self.prepare_vehicle_transfer(tag, child))
            .collect::<Option<Vec<_>>>()?;
        Some(PreparedVehicleTransfer {
            source_id: tree.source_id,
            tag,
            kind: tree.kind,
            name: tree.name,
            state: tree.state,
            passengers,
        })
    }

    /// Reconstruct using actual object IDs, even when the copied root is refused
    /// residency. `Entity.teleportCrossDimension` admits every passenger before
    /// its copied vehicle, whether that root is a mob, boat or minecart.
    pub fn restore_vehicle_tree_state(
        &mut self,
        tag: Tag,
        tree: TransferredVehicleTree,
    ) -> Option<RestoredVehicleTree> {
        let prepared = self.prepare_vehicle_transfer(tag, tree)?;
        let mut by_source = HashMap::new();
        let root = self.restore_vehicle_transfer(prepared, &mut by_source)?;
        // A natural-spawn candidate is still the argument of the spawn callback after
        // crossing levels in a join listener. Follow the exact restored object, not its UUID.
        use crate::game::natural_spawning::NaturalAdmission;
        if let Some(NaturalAdmission::Detached(candidate)) = &self.server.natural_admission
            && let Some(&id) = by_source.get(&candidate.id)
            && let Some(mob) = self.mob_object(id)
        {
            self.server.natural_admission = Some(NaturalAdmission::Placed {
                level: self.common.dim,
                id,
                join: mob.join,
            });
        }
        Some(RestoredVehicleTree { root, by_source })
    }

    pub(super) fn restore_vehicle_transfer(
        &mut self,
        tree: PreparedVehicleTransfer,
        by_source: &mut HashMap<u32, u32>,
    ) -> Option<u32> {
        let PreparedVehicleTransfer {
            source_id,
            tag,
            kind,
            name,
            state,
            passengers,
        } = tree;
        let riders = passengers
            .into_iter()
            .map(|child| self.restore_vehicle_transfer(child, by_source))
            .collect::<Option<Vec<_>>>()?;
        let root = self.restore_vehicle_object(tag, kind, &name, state)?;
        for (order, &id) in riders.iter().enumerate() {
            if let Some(mob) = self.mob_object_mut(id) {
                mob.extra.mount = Some(root);
                mob.extra.passenger = true;
                mob.extra.vehicle_order = order as i64;
            } else {
                self.set_general_mount(id, Some((root, order as i64)));
            }
        }
        let mob_rider = riders
            .into_iter()
            .find(|&id| self.common.mob_object(id).is_some());
        if let Some(mob) = self.mob_object_mut(root) {
            mob.extra.rider = mob_rider;
        }
        by_source.insert(source_id, root);
        Some(root)
    }

    pub(super) fn restore_vehicle_object(
        &mut self,
        tag: Tag,
        kind: VehicleTreeKind,
        name: &str,
        state: crate::game::entity_initialization::TransferredEntityState,
    ) -> Option<u32> {
        let uuid = entity_nbt::uuid(&tag);
        match kind {
            VehicleTreeKind::Mob => self.load_mob_transferred(tag, None, Some(state)),
            VehicleTreeKind::Boat => self.load_boat_transferred(tag, uuid, name, Some(state)),
            VehicleTreeKind::Cart => self.load_cart_transferred(tag, uuid, name, Some(state)),
            VehicleTreeKind::General => self
                .load_general_passenger(&tag, None, Some(state))
                .ok()
                .flatten(),
            VehicleTreeKind::Stand => {
                let id = self.server.next_entity + 1;
                self.load_armor_stand_transferred(tag, uuid, Some(state));
                self.common
                    .armor_stands
                    .iter()
                    .any(|stand| stand.id == id)
                    .then_some(id)
            }
            VehicleTreeKind::Other => self.load_other_transferred(tag, name, Some(state)),
        }
    }

    pub(super) fn mob_object_slot(&self, id: u32) -> Option<MobObject> {
        self.common
            .mobs
            .iter()
            .position(|mob| mob.id == id)
            .map(MobObject::Resident)
            .or_else(|| {
                self.common
                    .persist
                    .off_level
                    .get(&id)
                    .and_then(OffLevel::as_mob)
                    .map(|_| MobObject::OffLevel(id))
            })
    }

    pub(super) fn mob_object_tree_nbt(&mut self, object: MobObject, transferring: bool) -> Tag {
        let id = self.mob_at(object).id;
        let mut tag = self.mob_object_nbt(object);
        self.put_passengers_for(&mut tag, id, transferring);
        tag
    }

    pub(super) fn mob_object_nbt(&mut self, object: MobObject) -> Tag {
        self.mob_object_tag(object, true)
    }

    pub(super) fn mob_object_tag(&mut self, object: MobObject, include_id: bool) -> Tag {
        let id = self.mob_at(object).id;
        let uuid = self.uuid_for(id);
        // A warden's anger names each suspect by UUID. A mob that has never been
        // loaded or saved has no UUID yet, so give it one, or the save would
        // drop the anger at it (`who_uuid` writes nothing for a mob the map does
        // not know).
        let suspects: Vec<u32> = self
            .mob_at(object)
            .extra
            .warden
            .as_ref()
            .map(|st| {
                st.anger
                    .suspects()
                    .iter()
                    .filter_map(|who| match who {
                        Who::Mob(id) => Some(*id),
                        Who::Player => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for suspect in suspects {
            self.uuid_for(suspect);
        }
        // `NeutralMob.addPersistentAngerSaveData`: the anger ends at an
        // absolute game time, so it goes on while the chunk is unloaded, and
        // the target is the entity the mob is angry at (`settle_anger_end`).
        let anger = self.mob_at(object).extra.anger;
        if sunrise_sim::mobs::persistent_anger(self.mob_at(object).species()) {
            let game_time = self.common.world.game_time;
            sunrise_sim::mobs::settle_anger_end(self.mob_at_mut(object), game_time);
        } else {
            self.mob_at_mut(object).extra.anger_end = if anger > 0 {
                self.common.world.game_time + i64::from(anger)
            } else {
                -1
            };
        }
        self.mob_at_mut(object).extra.angry_at = if anger > 0 {
            // A wolf and an iron golem keep their anger's target themselves,
            // also once the target is gone (`tame::update_persistent_anger`).
            if matches!(
                self.mob_at(object).species(),
                sunrise_sim::mobs::Species::Wolf | sunrise_sim::mobs::Species::IronGolem
            ) {
                self.mob_at(object).extra.angry_at
            } else {
                self.mob_at(object).ai.target
            }
        } else {
            None
        };
        // A mob gets its UUID when it is first saved (`uuid_for`): the anger
        // target still in the world gets it now, so `uuid_of` can name it.
        if let Some(sunrise_sim::mobs::Who::Mob(target)) = self.mob_at(object).extra.angry_at
            && target != 0
            && self.common.mobs.iter().any(|m| m.id == target)
        {
            self.uuid_for(target);
        }
        // The mob that last hurt it, still resident, gets its UUID now, as the
        // anger target does, so `put_last_hurt_by_mob` can name it.
        if let Some(sunrise_sim::mobs::Who::Mob(attacker)) = self.mob_at(object).ai.last_hurt_by
            && self.common.mobs.iter().any(|m| m.id == attacker)
        {
            self.uuid_for(attacker);
        }
        use sunrise_sim::mobs::leash::Holder;
        let mut retained = self
            .common
            .persist
            .raw
            .get(&id)
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Default::default()));
        self.put_last_hurt_by_mob(self.mob_at(object), &mut retained);
        self.save_portal_cooldown(id, &mut retained);
        // `write_leash` may assign the holder a UUID. Other references retain
        // the lookup they had before that assignment.
        let holder = match self.mob_at(object).extra.leash {
            Some(Holder::Mob(id) | Holder::Boat(id)) => Some((id, self.common.persist.uuid_of(id))),
            _ => None,
        };
        self.write_leash(&mut retained, id);
        // Keep the mod's AI pause after attachment projection but before the
        // final passenger list, as in the persistent mob tree.
        let paused = self.mob_at(object).ai.ai_paused;
        entity_nbt::write_ai_paused(paused, &mut retained);
        self.write_entity_attachments(&mut retained, id);
        entity_nbt::write_ai_paused(paused, &mut retained);
        let mut out = entity_nbt::EntityTagWriter::from_saved(retained);
        entity_nbt::write_mob(
            self.mob_at(object),
            uuid,
            &mut out,
            Some(self.common.pack.assets.registry),
            |other| match holder {
                Some((id, uuid)) if other == id => uuid,
                _ => self.common.persist.uuid_of(other),
            },
            self.common.res.items(),
            entity_nbt::MobSave {
                include_id,
                game_time: self.common.world.game_time,
                variants: Some(self.world_variants()),
            },
        );
        out.finish()
    }

    /// Trace only edges touching off-level objects. A rejected leaf is owned
    /// by its resident vehicle; an orphaned rejected ancestor can instead be
    /// owned by a resident descendant or a player riding it.
    pub(super) fn retain_off_level_objects(&mut self, retained: &mut HashSet<u32>) {
        let mut links: HashMap<u32, Vec<u32>> = HashMap::new();
        for mob in self.mob_objects() {
            if let Some(vehicle) = mob.extra.mount
                && (self.common.persist.off_level.contains_key(&mob.id)
                    || self.common.persist.off_level.contains_key(&vehicle)
                    || self.common.persist.removed_vehicles.contains_key(&vehicle))
            {
                links.entry(mob.id).or_default().push(vehicle);
                links.entry(vehicle).or_default().push(mob.id);
            }
        }
        for vehicle in self.common.riding.into_iter().chain(
            self.server
                .lan
                .iter()
                .flat_map(|host| host.hub.guests.iter().filter_map(|guest| guest.vehicle)),
        ) {
            if self.common.persist.off_level.contains_key(&vehicle)
                || self.common.persist.removed_vehicles.contains_key(&vehicle)
            {
                retained.insert(vehicle);
            }
        }
        retained.extend(
            self.common
                .persist
                .removed_vehicles
                .keys()
                .copied()
                .filter(|&id| self.common.general_passengers(id).next().is_some()),
        );
        let mut pending: Vec<_> = links
            .keys()
            .copied()
            .filter(|id| retained.contains(id))
            .collect();
        while let Some(id) = pending.pop() {
            for &neighbor in links.get(&id).into_iter().flatten() {
                if (self.common.persist.off_level.contains_key(&neighbor)
                    || self.common.persist.removed_vehicles.contains_key(&neighbor))
                    && retained.insert(neighbor)
                {
                    pending.push(neighbor);
                }
            }
        }
        let gone: Vec<_> = self
            .common
            .persist
            .off_level
            .keys()
            .copied()
            .filter(|id| !retained.contains(id))
            .collect();
        for id in gone {
            self.common.persist.off_level.remove(&id);
            self.forget_entity(id);
        }
        let gone: Vec<_> = self
            .common
            .persist
            .removed_vehicles
            .keys()
            .copied()
            .filter(|id| !retained.contains(id))
            .collect();
        for id in gone {
            self.common.persist.removed_vehicles.remove(&id);
            self.forget_entity(id);
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
