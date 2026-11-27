//! Client-side implementations of `side_ops::world` and client-only world/harness helpers.
use super::*;
use sunrise_gameplay_common::game::end_common::EndGameExt as _;
use sunrise_gameplay_common::game::piston_carry_common::PositionChange;
use sunrise_gameplay_common::game::residency_tick::ResidencyTickGameExt as _;
use sunrise_sim::join_seq::{
    Joined as _,
    residency::{Kind, Owner, Source},
};

impl crate::game::ClientGame<'_> {
    #[inline(always)]
    pub(in crate::game) fn client_game_side_meta(
        &self,
    ) -> (Option<u128>, Option<mod_api::LevelRef>, bool) {
        let level = self.common.native_lifecycle.epoch().and_then(|epoch| {
            let key = self.common.dim.key();
            mod_api::__private::dimension_id(key)?;
            Some(mod_api::LevelRef::with_generation(epoch, key, 0))
        });
        (None, level, false)
    }

    pub(in crate::game) fn client_with_level_access<R>(
        &mut self,
        placement_preview: bool,
        f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        let darken = self.sky_darken();
        let (rain, thunder) = self.weather_levels(1.0);
        let (
            player_box,
            player_collision,
            cart_boxes,
            arrow_boxes,
            fire_players,
            trigger_hooks,
            trigger_extras,
            open_containers,
            golem_chests,
        ) = if placement_preview {
            Default::default()
        } else {
            let spectator = self.common.spectator();
            let player_box = self.common.player.as_ref().filter(|_| !spectator).map(|p| {
                let w = f64::from(p.dimensions.width) / 2.0;
                (
                    [p.position.x - w, p.position.y, p.position.z - w],
                    [
                        p.position.x + w,
                        p.position.y + f64::from(p.dimensions.height),
                        p.position.z + w,
                    ],
                )
            });
            let player_collision = self.common.player.as_ref().filter(|_| !spectator).map(|p| {
                let main_hand = self
                    .common
                    .inventory
                    .get(self.common.selected)
                    .and_then(Option::as_ref);
                PlayerCollision {
                    y: p.position.y,
                    descending: p.velocity.y < 0.0,
                    fall_distance: p.fall_distance,
                    main_hand_item: main_hand.map_or_else(
                        || Arc::from("minecraft:air"),
                        |stack| Arc::clone(&stack.item),
                    ),
                    main_hand_count: main_hand.map_or(0, |stack| stack.count as i32),
                    leather_boots: worn_boots::leather_boots(&self.common.armor),
                }
            });
            let cart_boxes = self
                .common
                .carts
                .iter()
                .map(|c| {
                    let p = c.position;
                    (
                        [p.x - 0.49, p.y, p.z - 0.49],
                        [p.x + 0.49, p.y + 0.7, p.z + 0.49],
                    )
                })
                .collect();
            let arrow_boxes = self
                .common
                .creatures
                .projectiles
                .iter()
                .filter(|p| p.arrow.is_some())
                .map(|p| p.position)
                .chain(self.common.tridents.iter().map(|t| t.position))
                .map(|at| {
                    let b = sunrise_sim::entities::arrows::arrow_bounds(at);
                    (b.min_position().components(), b.max_position().components())
                })
                .collect();
            let fire_players: Vec<[f64; 3]> = self
                .common
                .player
                .iter()
                .filter(|_| !spectator)
                .map(|p| [p.position.x, p.position.y, p.position.z])
                .collect();
            let trigger_hooks: Vec<(Aabb, bool)> = self
                .common
                .hook
                .iter()
                .map(|h| (h.bounds(), h.applies_block_effects()))
                .collect();
            let trigger_extras: Vec<Aabb> = self
                .common
                .bolts
                .iter()
                .map(|b| {
                    let at = Vec3::new(b.at[0], b.at[1], b.at[2]);
                    Aabb::new(at, at)
                })
                .collect();
            let mut open_containers: Vec<(i32, i32, i32)> = Vec::new();
            for key in self.common.container.into_iter().filter(|_| !spectator) {
                open_containers.push(key);
                if let Some(other) = self.common.chest_other_half(key) {
                    open_containers.push(other);
                }
            }
            let mut golem_chests: Vec<(i32, i32, i32)> = Vec::new();
            for &key in self.common.world.chest_openers.keys() {
                golem_chests.push(key);
                golem_chests.extend(self.common.chest_other_half(key));
            }
            (
                player_box,
                player_collision,
                cart_boxes,
                arrow_boxes,
                fire_players,
                trigger_hooks,
                trigger_extras,
                open_containers,
                golem_chests,
            )
        };
        let dimension = self.common.dim.level();
        let rules = self.common.level_rules();
        let api_level = self.native_client_level();
        let liquid_transaction_open = self.has_level_transaction();
        let environment = Self::level_environment(
            self.client
                .remote
                .as_ref()
                .and_then(|r| r.level_environment.as_deref()),
            &self.common.res,
            self.common.dim.id(),
            &self.dimension_type().id,
        );
        let side = client_game_side!(self, self.client_game_side_meta());

        let mut scratch_gateways = Default::default();
        let mut scratch_hoppers = HashMap::new();
        let mut scratch_rand_sequences =
            sunrise_items::random_sequences::RandomSequences::default();
        let mut client_spill = sunrise_gameplay_common::game::spill_common::Spill::default();
        let mut scratch_random_tick_sections =
            sunrise_sim::random_tick_sections::RandomTickSections::default();
        let mut scratch_lid_openers = HashMap::new();
        let mut scratch_native_sounds = Vec::new();
        let mut scratch_native_particles = Vec::new();

        let items = self
            .common
            .server_items
            .as_deref()
            .unwrap_or_else(|| self.common.res.items());
        let recipes = self
            .common
            .recipe_tables
            .synced
            .as_ref()
            .unwrap_or_else(|| self.common.res.recipes());
        let enabled_features = match &self.client.remote {
            Some(remote) => &remote.features,
            None => sunrise_gameplay_common::game::tables::default_client_features(),
        };
        let redstone_experiments = enabled_features
            .iter()
            .any(|feature| feature == "minecraft:redstone_experiments");
        let other_entities = entity::entity_lists_of!(self.common, without_carts);
        let loot_clocks = self.common.loot_clock_ticks();

        let mut level = GameLevel {
            cake_probe: false,
            cake_requested: std::cell::Cell::new(false),
            cake_answer: None,
            api_level,
            part_state_index: self.common.attachment_indexes.get(&self.common.dim),
            attachment_query_scopes: &self.common.transactions.attachment_query_scopes,
            command_block_transition: None,
            positions: sunrise_gameplay_common::game::piston_carry_common::PositionState {
                gateways: &mut scratch_gateways,
                hoppers: &mut scratch_hoppers,
            },
            border: self.common.border.geometry(),
            redstone_experiments,
            cart_boxes,
            item_frames: &self.common.item_frames,
            carts: &mut self.common.carts,
            arrow_boxes,
            other_entities,
            crystals: &self.common.crystals,
            trigger_hooks,
            trigger_extras,
            world: &mut self.common.world,
            facts: self.common.res.facts(),
            tables: native_block_tables::BlockTables::new(&self.common.res, &self.common.pack),
            collision: self.common.collision,
            events: &mut self.common.level_events,
            dimension,
            darken,
            environment,
            raining: rain > 0.2,
            thundering: thunder > 0.9,
            enabled_features,
            player_box,
            guest_boxes: Vec::new(),
            building_player_boxes: None,
            player_collision,
            biomes: world_biomes!(self.common),
            biome_keys: None,
            biome_zoom: self.client.streaming.biome_zoom,
            seed: self.common.level.seed,
            behaviours: self.common.res.behaviours(),
            items,
            recipes,
            loot: None,
            loot_storage: self.common.score.storage.snapshot(),
            loot_clocks,
            registry: self.common.pack.assets.registry,
            block_registries: &self.common.pack.block_registries,
            rules,
            difficulty: i32::from(self.common.ui.difficulty),
            fire_players,
            rand_sequences: &mut scratch_rand_sequences,
            pending: &mut self.common.pending,
            spill: &mut client_spill,
            tickets: sunrise_sim::ticket_levels::ChunkTickets::default().levels(),
            random_tick_sections: &mut scratch_random_tick_sections,
            lid_openers: &mut scratch_lid_openers,
            open_containers: &open_containers,
            golem_chests: &golem_chests,
            client_side: true,
            native_authoritative: false,
            native_player: None,
            native_player_entity: None,
            liquid_transaction_open,
            debug: false,
            native_sounds: &mut scratch_native_sounds,
            loop_context: &self.common.loop_context,
            side,
            native_particles: &mut scratch_native_particles,
            sound_seeds: None,
            drop_tool: None,
            drop_entity: None,
            interacting: None,
            interacting_tool: None,
            location: location_world::LocationRefs {
                dimension: self.common.dim.id(),
                end: &self.common.end,
                tags: sunrise_gameplay_common::game::advancements_common::TagSources {
                    pack_tags: None,
                    items,
                    mob_data: self.common.res.mob_data(),
                    damage: &self.common.health_resources.damage,
                    resources: &self.common.options.resources,
                },
            },
            drop_breaker_at: None,
            drop_into: None,
        };
        let result = f(
            self.common.res.behaviours(),
            &mut level,
            &mut self.common.block_ticks,
        );
        drop(level);
        if !placement_preview
            && (!client_spill.piston_positions.is_empty() || !client_spill.removed.is_empty())
        {
            self.client_finish_spill(client_spill);
        }
        result
    }

    /// Client-visible spill cleanup (moving-piston animation/container state and removed block
    /// entity container reset); omits server drops, hopper caches, guest menus, and LAN broadcasts.
    fn client_finish_spill(&mut self, spill: sunrise_gameplay_common::game::spill_common::Spill) {
        for change in spill.piston_positions {
            match change {
                PositionChange::Carry { from, to, partner } => {
                    for at in [from, to] {
                        let p = [at.0, at.1, at.2];
                        self.client.be_anims.remove_position(p);
                        self.common.world.shulkers.remove(&p);
                        self.common.world.chest_openers.remove(&at);
                    }
                    if self
                        .common
                        .container
                        .is_some_and(|at| at == from || Some(at) == partner)
                    {
                        self.side_close_container();
                    }
                }
                PositionChange::Land { at } => {
                    if self.common.world.block_entities.get(&at).is_none() {
                        self.common
                            .world
                            .set_block_entity_extras(at, Default::default());
                        self.common.world.block_names.remove(&at);
                    }
                }
            }
        }
        for (pos, _) in spill.removed {
            if self.common.container == Some((pos[0], pos[1], pos[2])) {
                self.common.container = None;
            }
        }
    }

    pub(in crate::game) fn client_after_with_level(&mut self) {}

    pub(in crate::game) fn client_set_block_live(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
        create_entity: bool,
    ) -> bool {
        let [x, y, z] = pos;
        let at = BlockPos::new(x, y, z);
        let previous = self.common.world.block(x, y, z);
        self.with_level(|b, level, ticks| {
            let changed = b.set_block(level, ticks, at, state, flags);
            if create_entity {
                level.finish_native_block(pos, previous, state, changed);
            }
            changed
        })
    }

    pub(in crate::game) fn client_update_chunk_tickets(&mut self) {}

    pub(in crate::game) fn client_section_mesh_pending_in(&self, section: (i32, i32, i32)) -> bool {
        self.section_mesh_pending_in(section)
    }

    pub(in crate::game) fn client_landing_change_cancelled(
        &mut self,
        _player: Option<u128>,
        _position: BlockPos,
        _from: BlockStateId,
        _to: BlockStateId,
    ) -> bool {
        false
    }

    pub(in crate::game) fn client_tick(&mut self) -> Result<()> {
        let client_level = self.client.remote.is_some();
        // Held through the carry too: a carried body's move asks the parts as its own does.
        let _parts = client_level
            .then(|| self.common.collision_parts())
            .flatten();
        // A client's tick of the level it shows holds the colliders as it began, then carries
        // its own player with the one it stands on.
        let _scope = client_level.then(|| {
            sunrise_world::moving_colliders::Scope::enter_level(
                self.client_collider_level(),
                self.client_placed_moving_colliders(),
            )
        });
        if self.client.agent.hold_server {
            return Ok(());
        }
        self.abort_open_transactions();
        self.common.update_interaction_ranges();
        self.tick_client_lead()?;
        if client_level {
            self.with_residency_tick(Self::tick_remote)
        } else {
            Ok(())
        }
    }

    pub(in crate::game) fn client_drop_stack_from_eyes(
        &mut self,
        _stack: ItemStack,
        _thrown_from_hand: bool,
    ) -> Option<u32> {
        None
    }

    pub(in crate::game) fn client_give_experience_points(&mut self, _points: i32) {}

    pub(in crate::game) fn client_journal_entity(&mut self, _id: u32) {
        self.common.transactions.writes = self.common.transactions.writes.wrapping_add(1);
    }

    pub(in crate::game) fn client_active_level_ref(
        &self,
        _epoch: mod_api::WorldEpoch,
    ) -> Option<mod_api::LevelRef> {
        self.native_client_level()
    }

    /// Reset and rebind client-visible entity residency without touching server parked levels
    /// or LAN guest tables.
    pub(in crate::game) fn client_reset_entity_residency(&mut self) {
        let dimension = self.common.dim.key();
        let table = self.common.resident_entities.table.get_mut();
        table.reset();
        let journal = table.producer();
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Mob,
            self.mobs,
            &journal,
            Owner {
                dimension,
                kind: Kind::Mob
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Item,
            self.dropped,
            &journal,
            Owner {
                dimension,
                kind: Kind::Item
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Orb,
            self.orbs,
            &journal,
            Owner {
                dimension,
                kind: Kind::Orb
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Stand,
            self.armor_stands,
            &journal,
            Owner {
                dimension,
                kind: Kind::Stand
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Frame,
            self.item_frames,
            &journal,
            Owner {
                dimension,
                kind: Kind::Frame
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Painting,
            self.paintings,
            &journal,
            Owner {
                dimension,
                kind: Kind::Painting
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Boat,
            self.boats,
            &journal,
            Owner {
                dimension,
                kind: Kind::Boat
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Cart,
            self.carts,
            &journal,
            Owner {
                dimension,
                kind: Kind::Cart
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Fangs,
            self.fangs.list,
            &journal,
            Owner {
                dimension,
                kind: Kind::Fangs
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Crystal,
            self.crystals,
            &journal,
            Owner {
                dimension,
                kind: Kind::Crystal
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Tnt,
            self.tnts,
            &journal,
            Owner {
                dimension,
                kind: Kind::Tnt
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Cushion,
            self.cushions,
            &journal,
            Owner {
                dimension,
                kind: Kind::Cushion
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Knot,
            self.resident_entities.knots,
            &journal,
            Owner {
                dimension,
                kind: Kind::Knot
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Falling,
            self.falling,
            &journal,
            Owner {
                dimension,
                kind: Kind::Falling
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Rocket,
            self.rockets,
            &journal,
            Owner {
                dimension,
                kind: Kind::Rocket
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Trident,
            self.tridents,
            &journal,
            Owner {
                dimension,
                kind: Kind::Trident
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Spawner,
            self.ominous_spawners,
            &journal,
            Owner {
                dimension,
                kind: Kind::Spawner
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Cloud,
            self.breath_clouds,
            &journal,
            Owner {
                dimension,
                kind: Kind::Cloud
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            Bolt,
            self.bolts,
            &journal,
            Owner {
                dimension,
                kind: Kind::Bolt
            }
        );
        sunrise_gameplay_common::game::resident_state::bind_resident_list!(
            General,
            self.resident_entities.general.rows,
            &journal,
            Owner {
                dimension,
                kind: Kind::General
            }
        );
        self.common.creatures.projectiles.bind_residency(
            &journal,
            Owner {
                dimension,
                kind: Kind::Projectile,
            },
            |entity| (entity.id, entity.join_seq()),
        );
        self.common.dragon.bind_residency_specialized(
            &journal,
            Owner {
                dimension,
                kind: Kind::Dragon,
            },
            |entity| (entity.id, entity.join_seq()),
            Source::Ordinary,
            Some((Kind::DragonPart, 8, |dragon, offset| {
                dragon.parts.get(usize::from(offset)).map(|part| part.3)
            })),
            None,
        );
        self.common.resident_entities.local_player.rebind(&journal);
        let body_id = self
            .common
            .player
            .as_ref()
            .map(|player| player.entity_id as u32);
        if self
            .common
            .resident_entities
            .local_player
            .identity()
            .is_some_and(|identity| Some(identity.id.get()) != body_id)
        {
            self.common.resident_entities.local_player.extract();
        }
        self.common.hook.bind_residency_specialized(
            &journal,
            Owner {
                dimension,
                kind: Kind::Hook,
            },
            |hook| (hook.id(), hook.join_seq()),
            Source::Hook { caster: None },
            None,
            Some(|hook| Some(hook.uuid)),
        );
    }

    /// Forget client-side records for a removed remote entity (omitting server hurt tracking).
    pub(in crate::game) fn client_forget_entity(&mut self, id: u32) {
        self.common.persist.transient.remove(&id);
        self.common.persist.uuids.remove(&id);
        self.common.persist.score_destroyed.remove(&id);
        self.common
            .forget_entity_initialization(mod_api::Holder::Entity(id));
        self.common.entity_attachments.remove(&id);
        self.common.entity_attachments.remove_collider(id);
        self.common.persist.raw.remove(&id);
        self.common.persist.no_physics.remove(&id);
    }

    /// The reach of the block entity a block state holds (see [`Self::block_entity_reach_entry`]).
    pub fn block_entity_reach(&self, state: u32) -> Reach {
        self.block_entity_reach_entry(state).0
    }

    pub(in crate::game) fn orb_geometry<'a>(
        &self,
        camera: glam::DVec3,
        frame: &OrbFrame,
        orbs: impl Iterator<Item = (&'a sunrise_sim::entities::ExperienceOrb, [f64; 3], f32)>,
        orb_view: Option<&sunrise_client::gfx::JoinedFrusta>,
        out: &mut Vec<Vertex>,
    ) {
        let OrbFrame {
            sprite,
            forward,
            right,
            up,
        } = *frame;
        for (o, at, partial) in orbs {
            let p = glam::DVec3::new(at[0], at[1] + 0.1, at[2]) - camera;
            let c = p.as_vec3();
            if orb_view
                .is_some_and(|view| forward.dot(c) < 0.0 && !entity_culling::orb_in_view(view, c))
            {
                continue;
            }
            if let Some(resolved) = native_entity_renderers::experience_orb_renderer() {
                let [width, height, eye_height] = o
                    .size_answer
                    .filter(|s| s.vanilla == sunrise_sim::entities::ExperienceOrb::DIMENSIONS)
                    .and_then(|s| s.size)
                    .unwrap_or(sunrise_sim::entities::ExperienceOrb::DIMENSIONS);
                let entity: mod_api::EntityRenderState =
                    mod_api::__private::built::EntityRenderState {
                        id: o.id,
                        partial,
                        age: o.age as f32 + partial,
                        yaw: o.rotation[0],
                        pitch: o.rotation[1],
                        position: at,
                        size: [width, height],
                        eye_height,
                        distance_to_camera_sq: (glam::DVec3::from_array(at) - camera)
                            .length_squared(),
                        living: None,
                        tnt_fuse: None,
                        orb_value: Some(o.value),
                        painting: None,
                        item_frame: None,
                        attachments: None,
                    }
                    .into();
                let light = self.common.world.light(
                    o.position.x.floor() as i32,
                    o.light_probe_y().floor() as i32,
                    o.position.z.floor() as i32,
                );
                let packed = o.render_light(i32::from(light));
                let block = [
                    o.position.x.floor() as i32,
                    o.light_probe_y().floor() as i32,
                    o.position.z.floor() as i32,
                ];
                if self.mod_entity_renderer_geometry(
                    &resolved,
                    &entity,
                    "minecraft:experience_orb",
                    glam::DVec3::from_array(at) - camera,
                    Some(glam::DVec3::from_array(at)),
                    block,
                    packed,
                    out,
                ) {
                    continue;
                }
            }
            let Some(r) = sprite else {
                continue;
            };
            let icon = o.icon();
            let (du, dv) = ((r.u1 - r.u0) / 4.0, (r.v1 - r.v0) / 4.0);
            let (u0, v0) = (r.u0 + (icon % 4) as f32 * du, r.v0 + (icon / 4) as f32 * dv);
            let t = (o.age as f32 + partial) / 2.0;
            let red = ((t.sin() + 1.0) * 0.5 * 255.0) as u32;
            let blue = (((t + 4.188_79).sin() + 1.0) * 0.1 * 255.0) as u32;
            let color = 0x8000_0000 | blue << 16 | 255 << 8 | red;
            let light = self.common.world.light(
                o.position.x.floor() as i32,
                o.light_probe_y().floor() as i32,
                o.position.z.floor() as i32,
            );
            let packed = o.render_light(i32::from(light));
            let s = sunrise_sim::entities::ORB_QUAD_SCALE;
            for (k, uv) in sunrise_sim::entities::ORB_QUAD.iter().zip([
                [u0, v0 + dv],
                [u0 + du, v0 + dv],
                [u0 + du, v0],
                [u0, v0],
            ]) {
                let v = c + right * (k[0] * s) + up * (k[1] * s);
                out.push(Vertex {
                    normal: [0; 4],
                    pos: v.to_array(),
                    uv,
                    layer: sunrise_client::gfx::OIT_ENTITY
                        | u32::from(sunrise_client::mesher::VERY_BRIGHT) << 16,
                    color,
                    light: packed,
                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }

    pub(in crate::game) fn run_component_harness(
        &self,
        fixture: sunrise_gameplay_common::game::integrated_link::ComponentHarness,
    ) -> Result<()> {
        (self.client.run_component_harness)(
            self.common.res.clone(),
            self.common.pack.clone(),
            self.common.options.clone(),
            self.client.settings.sim_settings(),
            fixture,
            self.client.profile_resolver,
        )
    }

    /// The server thread has already delivered its report: written to disk, or printed
    /// when it couldn't be written. Join through the normal halt path and exit.
    pub(in crate::game) fn exit_after_server_crash(&mut self) {
        if !sunrise_level::crash::server_crashed() {
            return;
        }
        let starting = self
            .client
            .integrated_starting
            .take()
            .map(|starting| starting.handle);
        for mut server in self.client.integrated.take().into_iter().chain(starting) {
            if let Err(error) = server.halt() {
                log::error!("Sunrise: crashed integrated server: {error}");
            }
        }
        std::process::exit(101);
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
