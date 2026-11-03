//! Entity and level handles for native server mods (docs/native-mods.md,
//! "Entity and level handles", #7387): `native_entity(id)` turns an event's
//! entity id into an `EntityRef`, and each `native_entity_*` helper reads the
//! entity a handle names. They read where the world is simulated: on a client,
//! with no world open, or for a handle of an earlier open, they return `None`.
//! The entities are those of the level in place (the one a hook runs in) and
//! the server's players, whatever their level.
use super::{
    Dim, Game,
    commands::Target,
    entity::{Entity as _, entity_lists_of, for_each_entity},
};
use horizon_client::mod_api::{
    EntityRef, LevelRef, NativeBlockFace, NativeBlockShapeKind, NativeBlockUpdateFlags,
    NativeClipBlock, NativeClipFluid, NativeClipHit, NativeSupportType,
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// What a handle names now, or None for a stale or foreign one.
    pub(super) fn native_entity_target(&self, entity: EntityRef) -> Option<Target> {
        if !self.is_logical_server() || self.native_lifecycle.epoch() != Some(entity.epoch()) {
            return None;
        }
        let id = i32::try_from(entity.id()).ok()?;
        if id == 0 {
            return None;
        }
        // A guest action can take the hub out of the game while its player
        // state is in place. Resolve that player before looking in the hub.
        if self.lan_acting_uuid.is_some()
            && self.lan_acting_entity == Some(entity.id())
            && self.player.is_some()
        {
            return Some(Target::Player);
        }
        let player = self
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .find(|g| g.placed && g.entity_id == id);
        if let Some(g) = player {
            // An outer guest in a nested swap has a displaced record.
            return (!self.lan_guest_swaps.contains(&g.conn)).then_some(Target::Guest(g.conn));
        }
        match self.level_entity(id)? {
            Target::Mob(id) if !self.mobs.iter().any(|m| m.id == id && !m.removed) => None,
            // Removed by a mod and waiting for the sweep: `isRemoved` is
            // true from the call on.
            t if self.persist.removals.contains(&t) => None,
            t => Some(t),
        }
    }
    /// The entity `id` (the id events carry) as a handle for the `entity_*`
    /// methods: None on a client, with no world open, or for no such entity
    /// in the level in place or among the server's players. During a nested
    /// player swap, an outer player's displaced record is unavailable. API
    /// @API:entity_handles@.
    pub(super) fn native_entity(&self, id: u32) -> Option<EntityRef> {
        let entity = EntityRef::new(self.native_lifecycle.epoch()?, id);
        self.native_entity_target(entity).map(|_| entity)
    }
    /// The level `dimension` (`minecraft:the_nether`) as a handle: None on a
    /// client, with no world open, or for a dimension the server hasn't
    /// loaded. API @API:entity_handles@.
    pub(super) fn native_level(&self, dimension: &str) -> Option<LevelRef> {
        if !self.is_logical_server() {
            return None;
        }
        let epoch = self.native_lifecycle.epoch()?;
        let dim = std::iter::once(self.dim)
            .chain(self.levels.parked_dims())
            .find(|d| d.id() == dimension)?;
        Some(LevelRef::new(epoch, dim.id()))
    }
    /// Entities whose bounding boxes intersect this box (`Level.getEntities`).
    /// Face-only contact does not intersect; spectator players are excluded.
    /// Returns None on a client, for a stale handle, or for a parked level:
    /// its entity handles cannot resolve until the level is in place. The walk
    /// is in entity-list order followed by players; each overlap requires a
    /// handle lookup, so cost grows with the box's contents. Tridents cannot
    /// yet resolve; frames, paintings, other special entities and the dragon
    /// are not on the entity-list walk.
    /// During a player's action with the hub taken out, players including the
    /// acting player are not listed: their handles cannot resolve there yet.
    /// During `hears_projectile_impacts`, the projectile list is taken out of
    /// the game and is not visible; it is restored for `projectile_impact`.
    /// API @API:level_entities_in@.
    pub(super) fn native_level_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Option<Vec<EntityRef>> {
        if !self.is_logical_server()
            || self.native_lifecycle.epoch() != Some(level.epoch())
            || level.dimension() != self.dim.id()
        {
            return None;
        }
        let bounds = Aabb::new(
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        );
        let mut ids = Vec::new();
        for_each_entity!(entity_lists_of!(self), entity => {
            // The join flush assigns zero IDs later; a handle could alias another entity.
            if entity.id() != 0 && !entity.is_removed() && bounds.intersects(entity.bounds()) {
                ids.push(entity.id());
            }
        });
        for guest in self.lan.iter().flat_map(|h| &h.hub.guests) {
            if !guest.participates_in_player_simulation()
                || !guest.placed
                || guest.dimension != level.dimension()
            {
                continue;
            }
            let body = if self.lan_acting == Some(guest.conn)
                && let Some(player) = &self.player
            {
                if self.mode_id() == 3 {
                    continue;
                }
                player.bounds()
            } else {
                if guest.game_mode == 3 {
                    continue;
                }
                guest.bounds()
            };
            if bounds.intersects(body)
                && let Ok(id) = u32::try_from(guest.entity_id)
            {
                ids.push(id);
            }
        }
        Some(
            ids.into_iter()
                .map(|id| EntityRef::new(level.epoch(), id))
                .filter(|handle| self.native_entity_target(*handle).is_some())
                .collect(),
        )
    }
    /// Set a block in the level in place (`Level.setBlock`). Parked levels,
    /// stale handles, clients and unloaded or out-of-height positions return
    /// false. Client-update and client-render flags are not supported; Horizon
    /// sends every change to clients. API @API:level_block_writes@.
    pub(super) fn native_level_set_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        flags: NativeBlockUpdateFlags,
    ) -> bool {
        if !self.native_level_write_allowed(level, pos) {
            return false;
        }
        self.native_set_block(level.dimension(), pos, state, flags.bits())
    }
    /// Destroy a block in the level in place (`Level.destroyBlock`), optionally
    /// dropping its resources. Parked levels and invalid positions return false.
    /// API @API:level_block_writes@.
    pub(super) fn native_level_destroy_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        drop: bool,
    ) -> bool {
        if !self.native_level_write_allowed(level, pos)
            || !self.is_logical_server()
            || self.dim.id() != level.dimension()
        {
            return false;
        }
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let changed = self.with_level(|b, level, ticks| b.destroy_block(level, ticks, at, drop));
        let _ = self.process_level_events();
        changed
    }
    fn native_level_write_allowed(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.is_logical_server()
            && self.native_lifecycle.epoch() == Some(level.epoch())
            && self.world.loaded(pos[0], pos[2])
            && self.world.shape.contains_y(pos[1])
    }
    /// Block-local outline or collision boxes with an empty context. Interaction,
    /// visual, occlusion and support shapes are not exposed. Returns None on a
    /// client, for a stale handle, or for a closed level. Unloaded blocks read
    /// as air; collision errors return no boxes, like `native_block_collision`.
    /// API @API:level_shapes@.
    pub(super) fn native_level_block_shape(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        kind: NativeBlockShapeKind,
    ) -> Option<Vec<[f64; 6]>> {
        if !self.is_logical_server() || self.native_lifecycle.epoch() != Some(level.epoch()) {
            return None;
        }
        let (world, anims) = if self.dim.id() == level.dimension() {
            (&self.world, &self.be_anims)
        } else {
            let dim = self
                .levels
                .parked_dims()
                .into_iter()
                .find(|dim| dim.id() == level.dimension())?;
            (
                self.levels.parked_world(dim)?,
                self.levels.parked_anims(dim)?,
            )
        };
        Some(match kind {
            NativeBlockShapeKind::Outline => self
                .outline_boxes_in_context(world, anims, pos, false)
                .into_iter()
                .map(|b| std::array::from_fn(|i| b[i] - f64::from(pos[i % 3])))
                .collect(),
            NativeBlockShapeKind::Collision => self.native_block_collision_in(world, pos),
        })
    }

    /// Gameplay's cached face support in the current or a parked level. Returns
    /// None for clients, stale or closed levels, unloaded or out-of-height blocks,
    /// dynamic-shape states (including moving pistons and shulker boxes), or an
    /// exporter error. Dynamic support is a follow-up on #29. False means not
    /// sturdy. API @API:level_shapes@.
    pub(super) fn native_level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: NativeBlockFace,
        support: NativeSupportType,
    ) -> Option<bool> {
        let state = self.native_level_state(level, pos)?;
        use horizon_core::geometry::Direction;
        let direction = match face {
            NativeBlockFace::Down => Direction::Down,
            NativeBlockFace::Up => Direction::Up,
            NativeBlockFace::North => Direction::North,
            NativeBlockFace::South => Direction::South,
            NativeBlockFace::West => Direction::West,
            NativeBlockFace::East => Direction::East,
        };
        let b = self.res.behaviours();
        if !b.has_cached_support(state) {
            return None;
        }
        Some(match support {
            NativeSupportType::Full => b.is_face_sturdy(state, direction),
            NativeSupportType::Center => b.is_face_sturdy_center(state, direction),
            NativeSupportType::Rigid => b.is_face_sturdy_rigid(state, direction),
        })
    }

    /// Clip an empty-context ray against collider or outline shapes, without fluids.
    /// None on failed reads, dynamic shapes, invalid rays or traversal limits.
    /// API @API:level_shapes@.
    pub(super) fn native_level_clip(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: NativeClipBlock,
        fluids: NativeClipFluid,
    ) -> Option<NativeClipHit> {
        use horizon_client::collisions::SceneCollisions;
        use horizon_core::geometry::Direction;
        use horizon_world::{
            collision_context::CollisionContext,
            world_ray::{RayBlockMode, RayFluidMode, clip_world},
        };
        let world = self.native_level_world(level)?;
        let mut rays = super::explosion::ColliderRays {
            scene: SceneCollisions {
                world,
                binding: self.collision,
                subject: None,
            },
            context: CollisionContext::empty(false),
            clip_data: Some(super::explosion::ClipData {
                facts: self.res.facts(),
                behaviours: self.res.behaviours(),
                tables: &self.pack.context.tables,
                registry: self.pack.assets.registry,
            }),
        };
        let mode = match blocks {
            NativeClipBlock::Collider => RayBlockMode::Collider,
            NativeClipBlock::Outline => RayBlockMode::Outline,
        };
        let fluid = match fluids {
            NativeClipFluid::None => RayFluidMode::None,
        };
        let result = clip_world(
            &mut rays,
            Vec3::new(from[0], from[1], from[2]),
            Vec3::new(to[0], to[1], to[2]),
            mode,
            fluid,
            4096,
        )
        .ok()?;
        let h = result.hit;
        let face = match h.direction {
            Direction::Down => NativeBlockFace::Down,
            Direction::Up => NativeBlockFace::Up,
            Direction::North => NativeBlockFace::North,
            Direction::South => NativeBlockFace::South,
            Direction::West => NativeBlockFace::West,
            Direction::East => NativeBlockFace::East,
        };
        Some(NativeClipHit {
            miss: result.miss,
            block: [h.block.x, h.block.y, h.block.z],
            face,
            location: [h.location.x, h.location.y, h.location.z],
            inside: h.inside,
            world_border: false,
        })
    }

    /// The entity's position (`Entity.position`): its feet, the middle of
    /// its box for a painting or an item frame. API @API:entity_handles@.
    pub(super) fn native_entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        self.target_position(self.native_entity_target(entity)?)
    }
    /// The block the entity's position is in (`Entity.blockPosition`). API
    /// @API:entity_handles@.
    pub(super) fn native_entity_block_position(&self, entity: EntityRef) -> Option<[i32; 3]> {
        self.native_entity_position(entity)
            .map(|p| p.map(|v| v.floor() as i32))
    }
    /// The entity's yaw and pitch in degrees (`Entity.getYRot`, `getXRot`); 0
    /// for the kinds that don't turn. API @API:entity_handles@.
    pub(super) fn native_entity_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        Some(self.target_rotation(self.native_entity_target(entity)?))
    }
    /// The entity's type (`Entity.getType`): `minecraft:zombie`. API
    /// @API:entity_handles@.
    pub(super) fn native_entity_kind(&self, entity: EntityRef) -> Option<String> {
        Some(self.target_type(self.native_entity_target(entity)?))
    }
    /// The entity's UUID (`Entity.getUUID`), the one it is saved with; an
    /// entity that has none yet gets it now, as a new vanilla entity does.
    /// API @API:entity_handles@.
    pub(super) fn native_entity_uuid(&mut self, entity: EntityRef) -> Option<u128> {
        let words = match self.native_entity_target(entity)? {
            Target::Player => return self.lan_acting_uuid,
            Target::Guest(conn) => return self.command_guest(conn).map(|g| g.uuid),
            _ => self.uuid_for(entity.id()),
        };
        Some(uuid_from_words(words))
    }
    /// The level the entity is in (`Entity.level`). API @API:entity_handles@.
    pub(super) fn native_entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
        let dim: Dim = self.target_level(self.native_entity_target(entity)?);
        Some(LevelRef::new(entity.epoch(), dim.id()))
    }
}

/// A saved UUID's four ints (`UUIDUtil.uuidFromIntArray`) as one number.
fn uuid_from_words(words: [i32; 4]) -> u128 {
    words
        .iter()
        .fold(0, |uuid, w| (uuid << 32) | u128::from(*w as u32))
}

#[cfg(test)]
pub(super) mod tests {
    pub(in crate::game) fn with_game(test: impl FnOnce(&mut super::Game<'_>)) {
        use super::super::{
            Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        };
        use horizon_client::mod_api::NativeAuthority;

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        game.native_lifecycle
            .begin_open(NativeAuthority::IntegratedServer);
        test(&mut game);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn side_guards_agree_across_integrated_dedicated_and_client() {
        use super::super::execution_role::ExecutionRole;
        use horizon_client::chunk::Chunk;
        use std::sync::Arc;
        with_game(|game| {
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            let level = game.native_level(game.dim.id()).unwrap();
            // The integrated server thread (game/integrated.rs sets the role and
            // `dedicated` on the serving Game) and a dedicated server simulate;
            // a client never does. A remote client runs with the `Client` role
            // (connecting changes `remote`, not the role), and the guards read
            // only the role, so it answers like the title-screen game here.
            for (role, dedicated) in [
                (ExecutionRole::IntegratedServer, true),
                (ExecutionRole::DedicatedServer, true),
            ] {
                game.options.execution_role = role;
                game.options.dedicated = dedicated.then(Vec::new);
                assert!(game.native_simulates());
                assert!(game.native_level(game.dim.id()).is_some());
                assert!(game.native_level_in_place(level));
                assert!(game.native_level_write_allowed(level, [-32, 64, 48]));
            }
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            assert!(!game.native_simulates());
            assert!(game.native_level(game.dim.id()).is_none());
            assert!(!game.native_level_in_place(level));
            assert!(!game.native_level_write_allowed(level, [-32, 64, 48]));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn chunk_handles_resolve_loaded_parked_and_stale_levels() {
        use horizon_client::{
            chunk::Chunk,
            mod_api::{ChunkStatus, Host, NativeAuthority},
        };
        use std::sync::Arc;
        with_game(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            let chunk = game.chunk(level, [-2, 3]).unwrap();
            assert_eq!(chunk.pos(), [-2, 3]);
            assert_eq!(chunk.level(), level);
            assert!(chunk.loaded(game));
            assert_eq!(chunk.status(game), Some(ChunkStatus::Full));
            assert!(game.chunk(level, [99, 99]).is_none());
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(chunk.status(game), Some(ChunkStatus::Full));
            assert_eq!(game.chunk(level, [-2, 3]), Some(chunk));
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            game.world.chunks.remove(&(-2, 3));
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
            game.world
                .chunks
                .insert((-2, 3), Arc::new(Chunk::empty(-2, 3)));
            game.native_lifecycle
                .begin_open(NativeAuthority::IntegratedServer);
            assert!(game.chunk(level, [-2, 3]).is_none());
            assert!(!chunk.loaded(game));
            assert_eq!(chunk.status(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn level_reads_and_writes_use_dimension_build_height() {
        use horizon_client::{
            chunk::{Chunk, LevelShape},
            mod_api::NativeBlockUpdateFlags,
        };
        use std::sync::Arc;
        with_game(|game| {
            game.world.shape = LevelShape::new(0, 128, 128, true, false).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.native_level(game.dim.id()).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 127, 0, stone).unwrap();
            game.world.set(0, 128, 0, stone).unwrap();
            assert!(game.native_level_is_loaded(level, [0, 127, 0]));
            assert_eq!(game.native_level_state(level, [0, 127, 0]), Some(stone));
            assert_eq!(
                game.native_level_block(level, [0, 127, 0]).as_deref(),
                Some("minecraft:stone")
            );
            assert!(game.native_level_write_allowed(level, [0, 127, 0]));
            assert!(!game.native_level_is_loaded(level, [0, 128, 0]));
            assert_eq!(game.native_level_state(level, [0, 128, 0]), None);
            assert_eq!(game.native_level_block(level, [0, 128, 0]), None);
            assert!(!game.native_level_is_loaded(level, [0, -1, 0]));
            assert_eq!(game.native_level_state(level, [0, -1, 0]), None);
            assert!(!game.native_level_set_block(
                level,
                [0, 128, 0],
                stone,
                NativeBlockUpdateFlags::NEIGHBORS
            ));
            assert!(!game.native_level_destroy_block(level, [0, 128, 0], false));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn equipping_mobs_by_hand_keeps_one_complete_stack() {
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            game.creative = false;
            for (id, kind, item) in [
                (12, "minecraft:horse", "minecraft:iron_horse_armor"),
                (13, "minecraft:llama", "minecraft:red_carpet"),
                (14, "minecraft:happy_ghast", "minecraft:red_harness"),
                (15, "minecraft:nautilus", "minecraft:iron_nautilus_armor"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                mob.extra.tame = true;
                let mut held = ItemStack::new(item, 3);
                held.damage = 7;
                held.data_mut().custom_name = Some(horizon_world::nbt::Tag::String("kept".into()));
                game.inventory[game.selected] = Some(held.clone());
                let index = game.mobs.len();
                game.mobs.push(mob);
                let used = match id {
                    12 => game.interact_mount(index, item),
                    13 => game.interact_llama(index, item),
                    14 => game.interact_happy_ghast(index, item),
                    _ => game.interact_nautilus(index, item),
                };
                assert!(used, "{item}");
                assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
                held.count = 1;
                assert_eq!(game.mobs[index].equipment_stacks()[6], Some(held), "{item}");
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn sulfur_cube_absorbs_one_complete_stack_by_hand_and_pickup() {
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            game.creative = false;
            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            let item = kind.tempt.first().expect("a swallowable item").to_string();
            let mut held = ItemStack::new(&item, 3);
            held.data_mut().custom_name = Some(horizon_world::nbt::Tag::String("kept".into()));
            game.inventory[game.selected] = Some(held.clone());
            game.mobs.push(Mob::new(12, kind.clone(), Vec3::ZERO, 0.0));
            assert!(game.interact_sulfur_cube(0, &item));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
            held.count = 1;
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(held.clone()));
            game.mobs.clear();
            game.mobs.push(Mob::new(13, kind, Vec3::ZERO, 0.0));
            game.spawn_item(
                ItemStack {
                    count: 3,
                    ..held.clone()
                },
                [0.0, 0.0, 0.0],
                [0.0; 3],
                0,
            );
            game.sulfur_cube_pickups();
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(held));
            assert_eq!(game.dropped[0].stack.count, 2);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn shearing_llama_and_happy_ghast_returns_full_body_stacks() {
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            for (id, kind, item) in [
                (12, "minecraft:llama", "minecraft:red_carpet"),
                (13, "minecraft:happy_ghast", "minecraft:red_harness"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
                mob.extra.tame = true;
                let mut stack = ItemStack::new(item, 3);
                stack.damage = 7;
                stack.data_mut().custom_name = Some(horizon_world::nbt::Tag::String("kept".into()));
                mob.extra.set_body_stack(Some(stack.clone()));
                let index = game.mobs.len();
                game.mobs.push(mob);
                let used = if item.ends_with("carpet") {
                    game.interact_llama(index, "minecraft:shears")
                } else {
                    game.interact_happy_ghast(index, "minecraft:shears")
                };
                assert!(used);
                assert!(game.dropped.iter().any(|drop| drop.stack == stack));
                assert!(game.mobs[index].extra.body.is_none());
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn acting_guest_handles_resolve_while_hub_is_out() {
        use super::super::lan::LanSettings;
        use horizon_client::{
            items::ItemStack,
            lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
            mod_api::{EquipmentSlot, GameMode},
        };
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
            let mut host = game.lan.take().unwrap();
            for id in [12, 13] {
                let (tx, _rx) = std::sync::mpsc::channel();
                let mut guest = Guest::new(
                    Joined {
                        conn: id,
                        uuid: id as u128,
                        name: format!("guest{id}"),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                        latency: Arc::new(AtomicI32::new(0)),
                        address: "127.0.0.1:1".parse().unwrap(),
                    },
                    id as i32,
                    [3.0, 64.0, 5.0],
                    10,
                );
                guest.placed = true;
                if id == 12 {
                    // A player loaded from a file saved with a walking speed.
                    guest.abilities.walking_speed = 0.25;
                    guest.loaded_body = Some(horizon_client::lan::guest::SavedBody {
                        fire_ticks: 0,
                        air: 300,
                        fall_distance: 0.0,
                    });
                }
                guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
                host.hub.guests.push(guest);
            }
            game.lan = Some(host);
            let player = game.native_player_ref(12).unwrap();
            assert!(!game.native_player_set_game_mode(player, GameMode::Survival));
            assert!(game.native_player_set_game_mode(player, GameMode::Creative));
            assert_eq!(
                game.native_player_game_mode(player),
                Some(GameMode::Creative)
            );
            let abilities = game.native_player_abilities(player).unwrap();
            assert!(abilities.invulnerable && abilities.may_fly && abilities.instabuild);
            assert!(game.native_player_set_game_mode(player, GameMode::Survival));
            assert_eq!(
                game.native_player_game_mode(player),
                Some(GameMode::Survival)
            );
            let abilities = game.native_player_abilities(player).unwrap();
            assert!(!abilities.invulnerable && !abilities.may_fly && !abilities.instabuild);
            // A write sticks, is refused for a speed that is not a number, and a
            // mode change sets the flags again (the speeds stay).
            let mut written = abilities;
            written.may_build = false;
            written.walk_speed = 0.2;
            written.fly_speed = 0.1;
            assert!(game.native_player_set_abilities(player, written));
            assert_eq!(game.native_player_abilities(player), Some(written));
            assert_eq!(game.native_player_may_build(player), Some(false));
            let mut bad = written;
            bad.walk_speed = f32::NAN;
            assert!(!game.native_player_set_abilities(player, bad));
            assert_eq!(game.native_player_abilities(player), Some(written));
            assert!(game.native_player_set_game_mode(player, GameMode::Creative));
            let abilities = game.native_player_abilities(player).unwrap();
            assert!(abilities.may_build && abilities.instabuild && abilities.may_fly);
            assert_eq!((abilities.walk_speed, abilities.fly_speed), (0.2, 0.1));
            assert!(game.native_player_set_game_mode(player, GameMode::Survival));
            let mut stale = player;
            stale.entity = horizon_client::mod_api::EntityRef::new(
                horizon_client::mod_api::NativeWorldEpoch::new(0),
                player.entity.id(),
            );
            assert!(!game.native_player_set_game_mode(stale, GameMode::Creative));
            host = game.lan.take().unwrap();
            let other = game
                .native_lifecycle
                .epoch()
                .map(|epoch| horizon_client::mod_api::EntityRef::new(epoch, 13))
                .unwrap();
            game.as_guest(&mut host, 12, |game| {
                let player = game.native_player_ref(12).unwrap();
                let entity = game.native_entity(12).unwrap();
                assert_eq!(player.entity, entity);
                assert_eq!(entity.position(game), Some([3.0, 64.0, 5.0]));
                assert_eq!(entity.uuid(game), Some(12));
                let living = game.native_living(entity).unwrap();
                assert_eq!(
                    living
                        .equipment(game, EquipmentSlot::MainHand)
                        .unwrap()
                        .item
                        .as_ref(),
                    "minecraft:stone"
                );
                assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
                // The loaded walking speed is the movement speed's base.
                assert_eq!(
                    game.player
                        .as_mut()
                        .unwrap()
                        .attribute("minecraft:movement_speed")
                        .unwrap(),
                    f64::from(0.25f32)
                );
                // In the player's own turn the write reaches the body as well.
                let mut a = game.native_player_abilities(player).unwrap();
                a.may_fly = true;
                a.walk_speed = 0.3;
                assert!(game.native_player_set_abilities(player, a));
                assert_eq!(game.native_player_abilities(player), Some(a));
                assert!(game.player.as_ref().unwrap().may_fly);
                assert!(other.position(game).is_none());
                assert!(game.native_living(other).is_none());
            })
            .unwrap();
            // A mode change in a turn, then a write in the same turn: the write wins.
            game.as_guest(&mut host, 12, |game| {
                let player = game.native_player_ref(12).unwrap();
                assert!(game.native_player_set_game_mode(player, GameMode::Creative));
                let mut a = game.native_player_abilities(player).unwrap();
                assert!(a.instabuild && a.may_fly);
                a.fly_speed = 0.4;
                a.walk_speed = 0.3;
                assert!(game.native_player_set_abilities(player, a));
            })
            .unwrap();
            let kept = host.hub.guest(12).unwrap();
            assert_eq!(kept.game_mode, 1);
            assert!(kept.abilities.instabuild);
            assert_eq!(kept.abilities.flying_speed, 0.4);
            assert_eq!(kept.abilities.walking_speed, 0.3);
            // The turn's write is stored in the guest's record.
            let stored = host.hub.guest(12).unwrap().abilities;
            assert!(stored.may_fly && stored.walking_speed == 0.3);
            let swap = game.enter_guest(&mut host, 12).unwrap();
            game.lan = Some(host);
            assert!(game.native_entity(12).is_some());
            let other = game.native_entity(13).unwrap();
            assert_eq!(other.position(game), Some([3.0, 64.0, 5.0]));
            assert_eq!(other.uuid(game), Some(13));
            assert_eq!(other.kind(game).as_deref(), Some("minecraft:player"));
            let living = game.native_living(other).unwrap();
            assert_eq!(
                living
                    .equipment(game, EquipmentSlot::MainHand)
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:stone"
            );
            let player = game.native_player_ref(13).unwrap();
            assert_eq!(player.entity, other);
            assert_eq!(player.game_mode(game), Some(GameMode::Survival));
            assert!(!game.native_player_set_game_mode(player, GameMode::Creative));
            let acting = game.native_player_ref(12).unwrap();
            assert!(game.native_player_set_game_mode(acting, GameMode::Adventure));
            assert_eq!(
                game.native_player_game_mode(acting),
                Some(GameMode::Adventure)
            );
            host = game.lan.take().unwrap();
            let nested = game.enter_guest(&mut host, 13).unwrap();
            game.lan = Some(host);
            assert!(game.native_entity(12).is_none());
            assert!(game.native_player_ref(12).is_none());
            assert!(
                game.native_living(horizon_client::mod_api::EntityRef::new(other.epoch(), 12))
                    .is_none()
            );
            assert!(game.native_entity(13).is_some());
            host = game.lan.take().unwrap();
            game.leave_guest(&mut host, nested);
            game.leave_guest(&mut host, swap);
            assert!(game.lan_guest_swaps.is_empty());
            // A player saved in creative (game mode 1, mayfly and instabuild)
            // joins again: without force they stay creative; forced into the
            // default mode (survival) they get its abilities.
            let root = std::env::temp_dir().join(format!("abilities-force-{}", std::process::id()));
            game.world_root = root.clone();
            let g = host.hub.guest_mut(13).unwrap();
            g.game_mode = 1;
            g.abilities.update_for_mode(1);
            assert!(g.abilities.may_fly && g.abilities.instabuild);
            horizon_client::lan::playerdata::save(g, &root);
            let joined = |game: &mut super::super::Game<'_>,
                          host: &mut super::super::lan::LanHost,
                          force| {
                host.hub.config.default_game_mode = 0;
                host.hub.config.force_game_mode = force;
                let g = host.hub.guest_mut(13).unwrap();
                g.game_mode = 0;
                g.abilities = Default::default();
                game.lan_guest_joined(host, 13, &[], [0.0, 64.0, 0.0]);
                let g = host.hub.guest(13).unwrap();
                (
                    g.game_mode,
                    g.abilities.may_fly,
                    g.abilities.instabuild,
                    g.abilities.may_build,
                )
            };
            assert_eq!(joined(game, &mut host, false), (1, true, true, true));
            assert_eq!(joined(game, &mut host, true), (0, false, false, true));
            let _ = std::fs::remove_dir_all(root);
            game.lan = Some(host);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn entities_in_returns_readable_handles_and_intersects_boxes() {
        use horizon_client::entities::ArmorStand;
        use horizon_client::mod_api::{LevelRef, NativeWorldEpoch};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            let level = game.native_level(game.dim.id()).unwrap();
            game.armor_stands
                .push(ArmorStand::new(0, Vec3::new(0.5, 0.0, 0.0), 0.0));
            assert!(game.native_entity(0).is_none());
            game.armor_stands
                .push(ArmorStand::new(12, Vec3::new(1.0, 0.0, 0.0), 0.0));
            let found = game
                .native_level_entities_in(level, [0.0, 0.0, -1.0], [0.8, 1.0, 1.0])
                .unwrap();
            assert_eq!(found.len(), 1);
            assert_eq!(game.native_entity_position(found[0]), Some([1.0, 0.0, 0.0]));
            assert_eq!(
                game.native_entity_kind(found[0]).as_deref(),
                Some("minecraft:armor_stand")
            );
            assert!(
                game.native_level_entities_in(level, [0.0, 0.0, -1.0], [0.75, 1.0, 1.0])
                    .unwrap()
                    .is_empty()
            );
            assert!(
                game.native_level_entities_in(
                    LevelRef::new(NativeWorldEpoch::new(0), game.dim.id()),
                    [0.0; 3],
                    [2.0; 3]
                )
                .is_none()
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn connectionless_guest_has_no_transport() {
        use horizon_client::{
            lan::{
                Joined, Lan, NetConfig, PlayerSender,
                chunks::ChunkEncoder,
                conn::{Holding, flush_all},
                data::SyncData,
                guest::Guest,
                hub::{Hub, HubConfig},
            },
            motion::PlayerMotion,
        };
        use horizon_core::geometry::Vec3;
        use horizon_protocol::packets::{
            configuration::ClientInformation,
            play::{Clientbound, Distance},
        };
        use std::sync::{Arc, atomic::Ordering};
        with_game(|game| {
            let sync = Arc::new(SyncData::empty());
            let net = NetConfig {
                announce: false,
                ..NetConfig::lan(0, "test".into(), 2)
            };
            let lan = Lan::start_with(sync.clone(), net).unwrap();
            let real_id = lan.shared.next_id().unwrap();
            let fake_id = lan.shared.next_id().unwrap();
            assert_ne!(real_id, fake_id);
            let encoder = ChunkEncoder::new(
                [] as [(Arc<str>, bool); 0],
                &horizon_protocol::registry::Builtin::default(),
                &[],
                &[],
            );
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 10,
                simulation_distance: 10,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 20,
                integrated: false,
                levels: Default::default(),
            };
            let mut hub = Hub::new(lan, sync, encoder, config.clone());
            let motion = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            let fake = Guest::connectionless(fake_id, 2, "fake".into(), 2, [0.0; 3], motion);
            assert!(fake.placed && fake.motion.is_some());
            assert!(fake.loaded && fake.client_loaded());
            assert!(!fake.participates_online() && !fake.participates_in_player_simulation());
            assert_eq!((fake.game_mode, fake.permission, fake.input), (0, 0, 0));
            assert!(fake.vehicle.is_none() && fake.bed.is_none() && fake.menu.is_none());
            let (tx, rx) = std::sync::mpsc::channel();
            let mut real = Guest::new(
                Joined {
                    conn: real_id,
                    uuid: 1,
                    name: "real".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 1).into(),
                },
                1,
                [0.0; 3],
                10,
            );
            real.placed = true;
            hub.guests.push(real);
            hub.guests.push(fake);
            assert!(hub.here(&hub.guests[0]));
            assert!(!hub.here(&hub.guests[1]));
            let packet = Clientbound::SetSimulationDistance(Distance { distance: 8 });
            let hold = Holding::start();
            hub.send_to(fake_id, &packet);
            hub.broadcast(&packet);
            hub.guests[1].sender.send_many([vec![1, 2, 3]]);
            flush_all(hub.guests.iter().map(|g| &g.sender));
            drop(hold);
            assert_eq!(hub.guests[1].sender.pending.load(Ordering::Relaxed), 0);
            assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).is_ok());
            game.lan_host(
                super::super::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::lan(0, "swap".into(), 2)
                },
                config,
            )
            .unwrap();
            let mut host = game.lan.take().unwrap();
            host.hub.guests = std::mem::take(&mut hub.guests);
            host.hub.guests[1].position = [1600.0, 64.0, 0.0];
            game.lan_update_streaming(&host);
            assert!(
                game.streaming
                    .extra
                    .iter()
                    .any(|(chunk, _)| *chunk == (0, 0))
            );
            assert!(
                !game
                    .streaming
                    .extra
                    .iter()
                    .any(|(chunk, _)| *chunk == (100, 0))
            );
            game.lan = Some(host);
            let players = game.native_server_players();
            assert_eq!(players.len(), 1);
            assert_eq!(players[0].uuid, 1);
            // A fake record in the Nether does not keep that level ticking.
            game.lan.as_mut().unwrap().hub.guests[1].dimension = "minecraft:the_nether".into();
            let ticking = game.levels_to_tick();
            assert!(!ticking.contains(&super::super::Dim::Nether));
            game.lan.as_mut().unwrap().hub.guests[0].dimension = "minecraft:the_nether".into();
            assert!(game.levels_to_tick().contains(&super::super::Dim::Nether));
            game.lan.as_mut().unwrap().hub.guests[0].dimension = "minecraft:overworld".into();
            game.lan_acting = Some(real_id);
            let selector = horizon_client::command::args::Selector {
                kind: 'a',
                limit: usize::MAX,
                ..Default::default()
            };
            assert_eq!(game.select(&selector).len(), 1);
            let named = horizon_client::command::args::Selector {
                player_name: Some("fake".into()),
                ..Default::default()
            };
            assert!(game.select(&named).is_empty());
            assert!(game.run_command_quietly("list"));
            assert!(!game.run_command_quietly("tp fake 0 64 0"));
            game.lan_acting = None;
            let mut host = game.lan.take().unwrap();
            for (conn, learns) in [(fake_id, false), (real_id, true)] {
                game.as_guest(&mut host, conn, |game| {
                    let mut book =
                        horizon_client::items::ItemStack::new("minecraft:knowledge_book", 1);
                    book.data_mut().set_other(
                        "minecraft:recipes",
                        Some(horizon_world::nbt::Tag::List(vec![
                            horizon_world::nbt::Tag::String("minecraft:stick".into()),
                        ])),
                    );
                    let slot = game.held_slot();
                    *game.stack_in_mut(slot) = Some(book.clone());
                    assert!(game.use_knowledge_book(&book));
                    assert!(game.stack_in(slot).is_none());
                    assert_eq!(game.level.recipe_book.knows("minecraft:stick"), learns);
                })
                .unwrap();
            }
            game.guest_packets
                .push(Clientbound::SetSimulationDistance(Distance { distance: 4 }));
            game.as_guest(&mut host, fake_id, |game| {
                assert!(game.lan_acting_fake);
                assert!(game.player.as_ref().is_some_and(|p| p.client_loaded));
                assert!(game.player_view().is_none());
                let mut visited = 0;
                game.for_each_player(|_| visited += 1);
                assert_eq!(visited, 0);
                let before = game.stats.get("custom", "minecraft:jump");
                game.award_stat("custom", "minecraft:jump", 1);
                assert_eq!(game.stats.get("custom", "minecraft:jump"), before);
                game.trigger("minecraft:tick", |_, _| panic!("fake criterion evaluated"));
                assert_eq!(game.award_recipes(&["minecraft:test".into()]), 0);
                let source = horizon_client::command::args::Selector {
                    kind: 's',
                    limit: 1,
                    ..Default::default()
                };
                assert_eq!(
                    game.select(&source),
                    vec![super::super::commands::Target::Player]
                );
                let named = horizon_client::command::args::Selector {
                    player_name: Some("fake".into()),
                    ..Default::default()
                };
                assert!(game.select(&named).is_empty());
                let uuid = horizon_client::command::args::Selector {
                    uuid: Some(2),
                    ..Default::default()
                };
                assert!(game.select(&uuid).is_empty());
                game.guest_packets
                    .push(Clientbound::SetSimulationDistance(Distance {
                        distance: 12,
                    }));
                game.guest_entity_events.push(3);
                game.guest_cooldowns.push(("minecraft:test".into(), 20));
            })
            .unwrap();
            assert_eq!(game.guest_packets.len(), 1);
            assert!(game.guest_entity_events.is_empty() && game.guest_cooldowns.is_empty());
            assert_eq!(host.hub.guests[1].sender.pending.load(Ordering::Relaxed), 0);
            assert!(rx.try_recv().is_err());
            let outer = game.enter_guest(&mut host, real_id).unwrap();
            assert!(!game.lan_acting_fake);
            game.guest_cooldowns.push(("minecraft:real".into(), 7));
            game.guest_player_sounds
                .push(("outer".into(), [0.0; 3], 1.0, 1.0));
            game.as_guest(&mut host, fake_id, |game| {
                assert!(game.lan_acting_fake);
                game.guest_cooldowns.push(("minecraft:fake".into(), 20));
            })
            .unwrap();
            assert!(!game.lan_acting_fake);
            assert_eq!(game.guest_cooldowns.len(), 1);
            assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:real");
            assert_eq!(game.guest_player_sounds[0].0, "outer");
            game.guest_player_sounds.clear();
            game.leave_guest(&mut host, outer);
            let out = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
            let horizon_client::lan::Out::Packet(body) = out else {
                panic!("expected cooldown packet")
            };
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let packet = Clientbound::decode(id, reader.rest()).unwrap();
            let Clientbound::Cooldown(cooldown) = packet else {
                panic!("expected cooldown")
            };
            assert_eq!(cooldown.group, "minecraft:real");
            assert_eq!(cooldown.duration, 7);
            assert!(rx.try_recv().is_err());

            // A real turn nested in a fake must not receive the fake's cooldown.
            let outer = game.enter_guest(&mut host, fake_id).unwrap();
            assert!(game.lan_acting_fake);
            game.guest_cooldowns
                .push(("minecraft:fake_outer".into(), 8));
            game.guest_player_sounds
                .push(("fake_outer".into(), [0.0; 3], 1.0, 1.0));
            game.as_guest(&mut host, real_id, |game| {
                assert!(!game.lan_acting_fake);
                let before = game.stats.get("custom", "minecraft:jump");
                game.award_stat("custom", "minecraft:jump", 1);
                assert_eq!(game.stats.get("custom", "minecraft:jump"), before + 1);
                game.guest_cooldowns
                    .push(("minecraft:real_inner".into(), 9));
            })
            .unwrap();
            assert!(game.lan_acting_fake);
            assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:fake_outer");
            assert_eq!(game.guest_player_sounds[0].0, "fake_outer");
            game.guest_player_sounds.clear();
            game.leave_guest(&mut host, outer);
            let horizon_client::lan::Out::Packet(body) =
                rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()
            else {
                panic!("expected cooldown")
            };
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            let Clientbound::Cooldown(cooldown) = Clientbound::decode(id, reader.rest()).unwrap()
            else {
                panic!("expected cooldown")
            };
            assert_eq!(cooldown.group, "minecraft:real_inner");
            assert!(rx.try_recv().is_err());

            // Two real nested turns deliver each queue to its own connection.
            let (tx2, rx2) = std::sync::mpsc::channel();
            let mut second = Guest::new(
                Joined {
                    conn: 3,
                    uuid: 3,
                    name: "second".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx2, Arc::default()),
                    latency: Arc::default(),
                    address: ([127, 0, 0, 1], 3).into(),
                },
                3,
                [0.0; 3],
                10,
            );
            second.placed = true;
            host.hub.guests.push(second);
            let outer = game.enter_guest(&mut host, real_id).unwrap();
            game.guest_cooldowns
                .push(("minecraft:real_outer".into(), 10));
            game.guest_player_sounds
                .push(("real_outer".into(), [0.0; 3], 1.0, 1.0));
            game.as_guest(&mut host, 3, |game| {
                game.guest_cooldowns
                    .push(("minecraft:real_second".into(), 11))
            })
            .unwrap();
            assert_eq!(&*game.guest_cooldowns[0].0, "minecraft:real_outer");
            assert_eq!(game.guest_player_sounds[0].0, "real_outer");
            game.guest_player_sounds.clear();
            game.leave_guest(&mut host, outer);
            for (receiver, expected) in [
                (&rx, "minecraft:real_outer"),
                (&rx2, "minecraft:real_second"),
            ] {
                let horizon_client::lan::Out::Packet(body) = receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap()
                else {
                    panic!("expected cooldown")
                };
                let mut reader = horizon_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                let Clientbound::Cooldown(cooldown) =
                    Clientbound::decode(id, reader.rest()).unwrap()
                else {
                    panic!("expected cooldown")
                };
                assert_eq!(cooldown.group, expected);
                assert!(receiver.try_recv().is_err());
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn level_sturdy_uses_gameplay_support() {
        use horizon_client::{
            chunk::Chunk,
            mod_api::{
                LevelRef, NativeBlockFace as Face, NativeSupportType as Support, NativeWorldEpoch,
            },
        };
        use std::sync::Arc;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.native_level(game.dim.id()).unwrap();
            let slab = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone_slab")
                .unwrap();
            game.world.set(0, 64, 0, slab).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Down, Support::Full),
                Some(true)
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::North, Support::Full),
                Some(false)
            );
            let wall = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cobblestone_wall")
                .unwrap();
            game.world.set(0, 64, 0, wall).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Center),
                Some(true)
            );
            assert_eq!(
                level.block_face_sturdy(game, [16, 64, 0], Face::Down, Support::Full),
                None
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 10000, 0], Face::Down, Support::Full),
                None
            );
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Center),
                Some(true)
            );
            assert!(
                game.enter_command_level(super::super::Dim::Overworld)
                    .is_some()
            );
            let shulker = game
                .pack
                .assets
                .registry
                .default_state("minecraft:shulker_box")
                .unwrap();
            game.world.set(0, 64, 0, shulker).unwrap();
            assert_eq!(
                level.block_face_sturdy(game, [0, 64, 0], Face::Up, Support::Full),
                None
            );
            let stale = LevelRef::new(
                NativeWorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            assert_eq!(
                stale.block_face_sturdy(game, [0, 64, 0], Face::Down, Support::Full),
                None
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn level_clip_reads_parked_world_and_refuses_unloaded() {
        use horizon_client::{
            chunk::Chunk,
            mod_api::{
                NativeBlockFace as Face, NativeClipBlock as Block, NativeClipFluid as Fluid,
            },
        };
        use std::sync::Arc;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.native_level(game.dim.id()).unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 64, 0, stone).unwrap();
            let from = [0.5, 64.5, 0.5];
            let to = [2.5, 64.5, 0.5];
            for mode in [Block::Collider, Block::Outline] {
                let hit = level.clip(game, from, to, mode, Fluid::None).unwrap();
                assert!(!hit.miss);
                assert_eq!(hit.block, [1, 64, 0]);
                assert_eq!(hit.face, Face::West);
                assert_eq!(hit.location, [1.0, 64.5, 0.5]);
                assert!(!hit.inside && !hit.world_border);
            }
            assert!(
                level
                    .clip(
                        game,
                        [15.5, 64.5, 0.5],
                        [17.5, 64.5, 0.5],
                        Block::Collider,
                        Fluid::None
                    )
                    .is_none()
            );
            let miss = level
                .clip(
                    game,
                    [0.5, 67.5, 0.5],
                    [2.5, 67.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                )
                .unwrap();
            assert!(miss.miss);
            let inside = level
                .clip(
                    game,
                    [1.5, 64.5, 0.5],
                    [2.5, 64.5, 0.5],
                    Block::Collider,
                    Fluid::None,
                )
                .unwrap();
            assert!(inside.inside && !inside.miss);
            assert!(
                level
                    .clip(
                        game,
                        [f64::NAN, 64.0, 0.0],
                        to,
                        Block::Collider,
                        Fluid::None
                    )
                    .is_none()
            );
            let stale = horizon_client::mod_api::LevelRef::new(
                horizon_client::mod_api::NativeWorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            assert!(
                stale
                    .clip(game, from, to, Block::Collider, Fluid::None)
                    .is_none()
            );
            use horizon_world::world_ray::RayWorld;
            let mut ordinary = super::super::explosion::ColliderRays {
                scene: horizon_client::collisions::SceneCollisions {
                    world: &game.world,
                    binding: game.collision,
                    subject: None,
                },
                context: horizon_world::collision_context::CollisionContext::empty(false),
                clip_data: None,
            };
            assert!(
                ordinary
                    .interaction_shape(
                        horizon_world::registry::BlockStateId(u32::MAX),
                        horizon_core::position::BlockPos::new(0, 0, 0)
                    )
                    .unwrap()
                    .is_empty()
            );
            let plant = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dandelion")
                .unwrap();
            game.world.set(4, 64, 4, plant).unwrap();
            let facts = game.res.facts().get(plant);
            assert!(facts.outline_offset);
            let offset = game.pack.context.tables.offset(plant, 4, 4).unwrap();
            assert_ne!(offset[0], 0.0);
            let b = facts.outline[0];
            let y = 64.0 + offset[1] + (b[1] + b[4]) / 2.0;
            let z = 4.0 + offset[2] + (b[2] + b[5]) / 2.0;
            let plant_hit = level
                .clip(game, [3.0, y, z], [5.0, y, z], Block::Outline, Fluid::None)
                .unwrap();
            assert_eq!(plant_hit.block, [4, 64, 4]);
            assert!((plant_hit.location[0] - (4.0 + offset[0] + b[0])).abs() < 1e-9);
            let cauldron = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            game.world.set(0, 64, 2, cauldron).unwrap();
            let bowl = level
                .clip(
                    game,
                    [0.5, 65.27, 0.5],
                    [0.5, 64.85, 3.5],
                    Block::Outline,
                    Fluid::None,
                )
                .unwrap();
            assert!(!bowl.miss);
            assert_eq!(bowl.block, [0, 64, 2]);
            assert_eq!(bowl.face, Face::Up);
            assert!(
                game.enter_command_level(super::super::Dim::Nether)
                    .is_some()
            );
            assert_eq!(
                level
                    .clip(game, from, to, Block::Collider, Fluid::None)
                    .unwrap()
                    .block,
                [1, 64, 0]
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn living_attributes_are_unavailable_for_nonplayers() {
        use horizon_client::{entities::ArmorStand, mobs::Mob};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 14;
            game.dragon = Some(dragon);
            for id in [12, 13, 14] {
                let living = game.native_living(game.native_entity(id).unwrap()).unwrap();
                assert_eq!(living.attribute_base(game, "minecraft:max_health"), None);
                assert_eq!(living.attribute_value(game, "minecraft:max_health"), None);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn level_shapes_use_empty_context() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use horizon_client::{
            chunk::Chunk,
            mod_api::{NativeAuthority, NativeBlockShapeKind},
        };
        use std::{sync::Arc, time::Instant};

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(NativeAuthority::IntegratedServer);
        let level = game.native_level("minecraft:overworld").unwrap();
        assert!(
            game.enter_command_level(super::super::Dim::Nether)
                .is_some()
        );
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 0, stone).unwrap();
        assert!(
            game.enter_command_level(super::super::Dim::Overworld)
                .is_some()
        );
        let nether = game.native_level("minecraft:the_nether").unwrap();
        let cube = vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]];
        assert_eq!(
            game.native_level_block_shape(nether, [0, 64, 0], NativeBlockShapeKind::Outline),
            Some(cube.clone())
        );
        assert_eq!(
            game.native_level_block_shape(nether, [0, 64, 0], NativeBlockShapeKind::Collision),
            Some(cube)
        );
        let shape = |game: &mut Game<'_>, name: &str, kind| {
            let state = game.pack.assets.registry.default_state(name).unwrap();
            game.world.set(0, 64, 0, state).unwrap();
            game.native_level_block_shape(level, [0, 64, 0], kind)
                .unwrap()
        };
        game.inventory[game.selected] = Some(horizon_client::items::ItemStack {
            item: "minecraft:light".into(),
            count: 1,
            damage: 0,
            extra: None,
        });
        assert!(shape(&mut game, "minecraft:light", NativeBlockShapeKind::Outline).is_empty());
        assert_eq!(
            game.outline_boxes([0, 64, 0]),
            vec![[0.0, 64.0, 0.0, 1.0, 65.0, 1.0]]
        );
        game.inventory[game.selected].as_mut().unwrap().item = "minecraft:scaffolding".into();
        let scaffolding = shape(
            &mut game,
            "minecraft:scaffolding",
            NativeBlockShapeKind::Outline,
        );
        assert!(!scaffolding.is_empty());
        assert_ne!(scaffolding, vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]);
        assert_eq!(
            game.outline_boxes([0, 64, 0]),
            vec![[0.0, 64.0, 0.0, 1.0, 65.0, 1.0]]
        );
        assert_eq!(
            shape(
                &mut game,
                "minecraft:stone_slab",
                NativeBlockShapeKind::Outline
            ),
            vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
        );
        assert_eq!(
            shape(
                &mut game,
                "minecraft:stone_slab",
                NativeBlockShapeKind::Collision
            ),
            vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
        );
        let contains = |boxes: &[[f64; 6]], p: [f64; 3]| {
            boxes
                .iter()
                .any(|b| (0..3).all(|i| b[i] < p[i] && p[i] < b[i + 3]))
        };
        let stair = shape(
            &mut game,
            "minecraft:oak_stairs",
            NativeBlockShapeKind::Outline,
        );
        assert!(contains(&stair, [0.25, 0.25, 0.25]));
        let upper = [
            [0.25, 0.75, 0.25],
            [0.75, 0.75, 0.25],
            [0.25, 0.75, 0.75],
            [0.75, 0.75, 0.75],
        ];
        assert_eq!(
            upper.into_iter().filter(|p| contains(&stair, *p)).count(),
            2
        );
        let fence = shape(
            &mut game,
            "minecraft:oak_fence",
            NativeBlockShapeKind::Outline,
        );
        assert!(contains(&fence, [0.5, 0.5, 0.5]));
        assert!(!contains(&fence, [0.1, 0.5, 0.1]));
        let lectern = shape(
            &mut game,
            "minecraft:lectern",
            NativeBlockShapeKind::Collision,
        );
        assert!(contains(&lectern, [0.5, 0.5, 0.5]));
        assert!(!contains(&lectern, [0.1, 0.5, 0.1]));
    }

    #[test]
    fn uuid_words_are_most_significant_first() {
        let words = [0x0102_0304, -1, 0, 0x7fff_ffff];
        assert_eq!(
            super::uuid_from_words(words),
            0x0102_0304_ffff_ffff_0000_0000_7fff_ffff
        );
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn level_block_writes_validate_and_change_world() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use horizon_client::{
            chunk::Chunk,
            mod_api::{NativeAuthority, NativeBlockUpdateFlags},
        };
        use std::{sync::Arc, time::Instant};

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(NativeAuthority::IntegratedServer);
        let level = game.native_level("minecraft:overworld").unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        let flags = NativeBlockUpdateFlags::NEIGHBORS;
        assert!(game.native_level_set_block(level, [0, 64, 0], stone, flags));
        assert_eq!(game.world.block(0, 64, 0), stone);
        assert!(!game.native_level_set_block(level, [16, 64, 0], stone, flags));
        assert!(!game.native_level_set_block(level, [0, 10000, 0], stone, flags));
        let stale = horizon_client::mod_api::LevelRef::new(
            horizon_client::mod_api::NativeWorldEpoch::new(level.epoch().get() + 1),
            level.dimension(),
        );
        assert!(!game.native_level_set_block(stale, [0, 64, 0], stone, flags));
        assert!(!game.native_level_destroy_block(stale, [0, 64, 0], true));
        let fill_chest = |game: &mut Game<'_>| {
            let Some(super::super::BlockEntity::Chest(items)) =
                game.world.block_entities.get_mut(&(0, 64, 0))
            else {
                panic!("chest block entity missing")
            };
            items[0] = Some(horizon_client::items::ItemStack {
                item: "minecraft:diamond".into(),
                count: 3,
                damage: 0,
                extra: None,
            });
        };
        assert!(game.native_level_set_block(level, [0, 64, 0], chest, flags));
        fill_chest(&mut game);
        assert!(game.native_level_set_block(level, [0, 64, 0], air, flags));
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:diamond")
                .map(|item| item.stack.count)
                .sum::<u32>(),
            3
        );
        game.dropped.clear();
        assert!(game.native_level_set_block(level, [0, 64, 0], chest, flags));
        fill_chest(&mut game);
        assert!(game.native_level_set_block(
            level,
            [0, 64, 0],
            air,
            NativeBlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS
        ));
        assert!(!game.world.block_entities.contains_key(&(0, 64, 0)));
        assert!(game.dropped.is_empty());
        assert!(game.native_level_set_block(level, [0, 64, 0], stone, flags));
        assert!(game.native_level_destroy_block(level, [0, 64, 0], false));
        assert!(game.dropped.is_empty());
        assert_eq!(game.world.block(0, 64, 0), air);
        assert!(game.native_level_set_block(level, [0, 64, 0], stone, flags));
        assert!(game.native_level_destroy_block(level, [0, 64, 0], true));
        assert!(
            game.dropped
                .iter()
                .any(|item| item.stack.item.as_ref() == "minecraft:cobblestone")
        );
        assert!(!game.native_level_destroy_block(level, [0, 64, 0], true));
        game.dropped.clear();
        let copper = game
            .pack
            .assets
            .registry
            .default_state("minecraft:copper_chest")
            .unwrap();
        let exposed = game
            .pack
            .assets
            .registry
            .default_state("minecraft:exposed_copper_chest")
            .unwrap();
        assert!(game.native_level_set_block(level, [0, 64, 0], copper, flags));
        fill_chest(&mut game);
        assert!(game.native_level_set_block(level, [0, 64, 0], exposed, flags));
        let Some(super::super::BlockEntity::Chest(items)) =
            game.world.block_entities.get(&(0, 64, 0))
        else {
            panic!("copper chest entity missing")
        };
        assert_eq!(items[0].as_ref().unwrap().count, 3);
        assert!(game.dropped.is_empty());
        let sensor = game
            .pack
            .assets
            .registry
            .default_state("minecraft:sculk_sensor")
            .unwrap();
        assert!(game.native_level_set_block(level, [0, 64, 0], sensor, flags));
        assert!(matches!(
            game.world.block_entities.get(&(0, 64, 0)),
            Some(super::super::BlockEntity::Sculk(_))
        ));
        assert!(game.native_level_set_block(level, [0, 64, 0], chest, flags));
        assert!(matches!(
            game.world.block_entities.get(&(0, 64, 0)),
            Some(super::super::BlockEntity::Chest(_))
        ));
        assert!(
            game.enter_command_level(super::super::Dim::Nether)
                .is_some()
        );
        assert!(!game.native_level_set_block(level, [0, 64, 0], stone, flags));
        assert!(!game.native_level_destroy_block(level, [0, 64, 0], true));
    }
}
