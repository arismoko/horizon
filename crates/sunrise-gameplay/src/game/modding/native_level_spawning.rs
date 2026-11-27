//! Entity spawning and a level's natural spawning policy through a `LevelRef`
//! (docs/modding/src/how-to/world/spawning.md, "Spawning from a level"): an entity made
//! as its family's own spawn makes one, the per-category limits and the flags
//! natural spawning reads (`natural_spawning::SpawnPolicy`), and the chunk
//! question it asks (`ServerLevel.canSpawnEntitiesInChunk`).
use super::{
    commands::Target,
    general::{GeneralEntityRecord, GeneralTypeEnvelope, general_type_catalog},
    native_projectile_launch::thrown_projectile_kind,
    natural_spawning,
};
use mod_api::{EntityRef, EntitySpawn, LevelRef, SpawnBody, SpawnFlags, SpawnLimits};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::creatures::SpawnSite;
use sunrise_items::items::ItemStack;
use sunrise_sim::{lan::playerdata::uuid_ints, projectiles::Projectile};

impl crate::game::ServerGame<'_> {
    /// `LevelRef::spawn_entity`: a thrown projectile, a mob or a general row of
    /// `kind` at `pos`, each through its family's own route (`Projectile.spawnProjectile`,
    /// `EntityType.spawn`, the general insertion `/summon` reaches), with the
    /// velocity, rotation and owner `spawn` gives. None, with nothing spawned,
    /// for every refusal `level_spawn_entity`'s declaration lists.
    pub fn level_spawn_entity(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        spawn: &EntitySpawn,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        let finite = pos.iter().chain(&spawn.velocity).all(|v| v.is_finite())
            && spawn
                .rotation
                .is_none_or(|r| r.iter().all(|v| v.is_finite()));
        if !self.level_in_place(level) || !finite {
            return None;
        }
        // BlockPos.containing. `as` saturates, and a value past `i32` is out of bounds.
        let block = pos.map(|v| v.floor() as i32);
        if self.level_in_spawnable_bounds(level, block) != Some(true)
            || !self.common.world.loaded(block[0], block[2])
        {
            return None;
        }
        // The owner is resolved before anything is made: one that is stale or
        // in another level refuses the spawn.
        let owner = match spawn.owner {
            Some(owner) if self.entity_level(owner) != Some(level) => return None,
            Some(owner) => Some((owner, self.native_entity_target(owner)?)),
            None => None,
        };
        let velocity = Vec3::new(spawn.velocity[0], spawn.velocity[1], spawn.velocity[2]);
        let id = match &spawn.body {
            SpawnBody::Thrown { item } => {
                let stack = sunrise_gameplay_common::game::api_values::stack_value(item).ok()?;
                self.spawn_thrown_entity(kind, pos, velocity, spawn.rotation, owner, &stack)?
            }
            SpawnBody::Living { reason } => {
                // A mob takes no owner; a kind that isn't a mob makes nothing.
                if owner.is_some() {
                    return None;
                }
                let kind = sunrise_command::args::normalized(kind);
                if self.difficulty_id() == 0
                    && self
                        .common
                        .res
                        .mob_types()
                        .get(kind.as_ref())
                        .is_some_and(|t| !t.allowed_in_peaceful())
                {
                    return None;
                }
                let site = SpawnSite::Finalizing(*reason);
                self.spawn_mob_spawned_moving(&kind, pos, site, velocity, spawn.rotation)?
            }
            SpawnBody::General => {
                let owner = match owner {
                    Some((owner, _)) => Some(uuid_ints(self.entity_uuid(owner)?)),
                    None => None,
                };
                self.spawn_general_entity(kind, pos, spawn.velocity, spawn.rotation, owner)?
            }
            _ => return None,
        };
        Some(EntityRef::new(level.epoch(), id))
    }
    /// `LevelRef::spawn_mob`: [`level_spawn_entity`](Self::level_spawn_entity)'s
    /// living route for a mob at rest, with no owner.
    pub fn level_spawn_mob(
        &mut self,
        level: LevelRef,
        kind: &str,
        pos: [f64; 3],
        reason: mod_api::SpawnReason,
    ) -> Option<EntityRef> {
        let spawn = EntitySpawn::new(SpawnBody::Living { reason });
        self.level_spawn_entity(level, kind, pos, &spawn)
    }
    /// `Projectile.spawnProjectile` of a thrown item projectile at `pos`: its
    /// velocity set (`setDeltaMovement`) and its rotation given or taken from
    /// the velocity (`Projectile.shoot`), its owner set before it joins
    /// (`setOwner`), carrying one of `stack`. None for a type that isn't a
    /// thrown one or a stack whose spawn effects the engine can't all apply.
    fn spawn_thrown_entity(
        &mut self,
        kind: &str,
        pos: [f64; 3],
        velocity: Vec3,
        rotation: Option<[f32; 2]>,
        owner: Option<(EntityRef, Target)>,
        stack: &ItemStack,
    ) -> Option<u32> {
        // As `spawn_projectile_from_rotation`: a stack the engine can't fully
        // apply is refused before anything is made or drawn.
        if !self.projectile_spawned_effects_applied(stack) {
            return None;
        }
        // The owner must have a collision identity the shot's hit filter
        // knows (`Who`, or the vehicle it leaves): a player, a mob, a boat or
        // a minecart. Any other owner would get damage credit but no
        // identity, so the shot could hit it before it left; it is refused.
        if !matches!(
            owner,
            None | Some((
                _,
                Target::Player
                    | Target::Guest(_)
                    | Target::Mob(_)
                    | Target::Boat(_)
                    | Target::Cart(_)
            ))
        ) {
            return None;
        }
        let mut carried = stack.clone();
        carried.count = 1;
        let kind = thrown_projectile_kind(kind, &carried)?;
        let ((who, uuid, guest), vehicle) =
            self.native_projectile_owner_fields(owner.map(|(owner, _)| owner))?;
        let mut shot = Projectile::new(kind, Vec3::new(pos[0], pos[1], pos[2]), who);
        (shot.owner_uuid, shot.guest, shot.owner_vehicle) = (uuid, guest, vehicle);
        shot.velocity = velocity;
        match rotation {
            Some([yaw, pitch]) => {
                (shot.yaw, shot.pitch) = (yaw, pitch);
                (shot.previous_yaw, shot.previous_pitch) = (yaw, pitch);
            }
            None => shot.update_rotation(),
        }
        shot.item = Some(carried);
        self.spawn_owned_projectile(owner.map(|(_, target)| target), shot, stack)
    }
    /// A new row of the general type `kind` at `pos`: a fresh UUID, the type's
    /// declared payload version and no payload bytes, through the insertion
    /// `/summon` and chunk loading share (`spawn_entity_from_nbt`), then told
    /// to mods as it joins. None for a type the registry doesn't hold, or a row
    /// a mod kept out.
    fn spawn_general_entity(
        &mut self,
        kind: &str,
        pos: [f64; 3],
        velocity: [f64; 3],
        rotation: Option<[f32; 2]>,
        owner: Option<[i32; 4]>,
    ) -> Option<u32> {
        let catalog = general_type_catalog();
        let slot = catalog.slot(kind)?;
        // `EntityType.canSpawn`: refuse types whose required Minecraft feature flags are not
        // enabled in this world.
        let features = mod_api::FeatureFlags::new(self.level.enabled_features.as_arc().clone());
        if !catalog.is_enabled(slot, &features) {
            return None;
        }
        let record = GeneralEntityRecord {
            position: pos,
            velocity,
            rotation: rotation.unwrap_or([0.0; 2]),
            uuid: sunrise_sim::save::insecure_uuid(&mut sunrise_level::random_source::entity()),
            owner,
            custom_name: None,
            custom_name_visible: false,
            base: Default::default(),
            on_ground: false,
            fall_distance: 0.0,
            envelope: GeneralTypeEnvelope {
                type_key: kind.to_owned(),
                payload_version: catalog.payload_version(slot),
                payload_bytes: Vec::new(),
            },
            passengers: Vec::new(),
        };
        // The single-record admission chunk loading uses, without its
        // `entities_load` (nothing was read from a save), then the join as a
        // fresh spawn's: told now, or at the root commit inside a transaction.
        let tag = record.to_tag();
        let id = self.admit_general_record(record, &tag, None).ok()??;
        self.native_joins_flush(false, id).then_some(id)
    }
    /// The level's natural spawning limits. None for a stale handle or on a client.
    pub fn level_spawn_limits(&self, level: LevelRef) -> Option<SpawnLimits> {
        self.native_level_world(level)?;
        Some(self.spawn_policy(level.dimension()).limits)
    }
    /// The level's natural spawning flags. None for a stale handle or on a client.
    pub fn level_spawn_flags(&self, level: LevelRef) -> Option<SpawnFlags> {
        self.native_level_world(level)?;
        Some(self.spawn_policy(level.dimension()).flags)
    }
    /// Sets the level's natural spawning limits for the rest of the open. False for a
    /// stale handle or on a client.
    pub fn level_set_spawn_limits(&mut self, level: LevelRef, limits: SpawnLimits) -> bool {
        self.native_set_spawn_policy(level, |policy| policy.limits = limits)
    }
    /// Sets the level's natural spawning flags for the rest of the open. False as for
    /// `level_set_spawn_limits`.
    pub fn level_set_spawn_flags(&mut self, level: LevelRef, flags: SpawnFlags) -> bool {
        self.native_set_spawn_policy(level, |policy| policy.flags = flags)
    }
    fn native_set_spawn_policy(
        &mut self,
        level: LevelRef,
        set: impl FnOnce(&mut natural_spawning::SpawnPolicy),
    ) -> bool {
        if self.native_level_world(level).is_none() {
            return false;
        }
        set(self
            .server
            .spawn_policies
            .entry(level.dimension())
            .or_default());
        true
    }
    /// `ServerLevel.canSpawnEntitiesInChunk` as natural spawning answers it. None for a
    /// stale handle or on a client.
    pub fn level_can_spawn_entities_in_chunk(
        &self,
        level: LevelRef,
        pos: [i32; 2],
    ) -> Option<bool> {
        let world = self.native_level_world(level)?;
        Some(natural_spawning::can_spawn_entities_in_chunk(
            world, pos[0], pos[1],
        ))
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
