//! Landing impacts for falling blocks, before their placement/drop decision.
use super::{FallingBlock, Landing};
use sunrise_core::{geometry::Vec3, position::BlockPos, random::RandomSource};
use sunrise_sim::falling_block;
use sunrise_world::level::LevelEvent;

impl crate::game::ServerGame<'_> {
    /// `FallingBlockEntity.causeFallDamage`: impact damage and anvil wear,
    /// before block effects and portal handling can transfer the entity.
    pub fn falling_block_impact(&mut self, block: &mut FallingBlock) {
        let name = self.common.res.facts().get(block.state).name.clone();
        let anvil = name.ends_with("anvil");
        let stalactite = matches!(
            &*name,
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
        );
        if let Some(impact) = block.damage.impact(block.fall_distance) {
            if impact.amount > 0.0 {
                let mob_kind = if stalactite {
                    sunrise_sim::mobs::DamageKind::FallingStalactite
                } else if anvil {
                    sunrise_sim::mobs::DamageKind::FallingAnvil
                } else {
                    sunrise_sim::mobs::DamageKind::FallingBlock
                };
                let attack = sunrise_sim::health::Attack {
                    attacker: block.id as i32,
                    direct: Some(block.id as i32),
                    position: Some([block.position.x, block.position.y, block.position.z]),
                    ..Default::default()
                };
                // FallingBlockEntity.causeFallDamage selects the entities first, then hurts
                // them in order. Death listeners can spawn or remove rows between hits.
                let bounds = block.bounds();
                let victims: Vec<_> = self
                    .common
                    .mobs
                    .iter()
                    .filter(|mob| mob.alive() && !mob.removed && bounds.intersects(mob.bounds()))
                    .map(|mob| mob.join)
                    .collect();
                let source = sunrise_sim::mobs::Damage {
                    from: attack.position,
                    ..sunrise_sim::mobs::Damage::of(mob_kind)
                };
                for join in victims {
                    let Some(index) = self.common.mobs.locate(join) else {
                        continue;
                    };
                    if self.common.mobs[index].removed {
                        continue;
                    }
                    let mob_id = self.common.mobs[index].id;
                    let plan = self.prepare_knockback_credit(
                        mob_id as i32,
                        None,
                        self.server.combat.0.attack(),
                    );
                    let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                    self.common.mobs[index].damage(
                        crate::game::server_side::entity_server::damage_level_of!(
                            self,
                            &attack,
                            &mut credit,
                            None
                        ),
                        impact.amount,
                        source,
                    );
                    self.finish_mob_damage_at(index);
                }
                // Enderman.hurtServer: a falling block is no living attacker.
                if let Err(e) = self.settle_hurt_teleports() {
                    log::warn!("Sunrise: falling block: enderman teleport: {e}");
                }
                let kind = if stalactite {
                    "minecraft:falling_stalactite"
                } else if anvil {
                    "minecraft:falling_anvil"
                } else {
                    "minecraft:falling_block"
                };
                let bounds = block.bounds();
                self.falling_hurt_player(block.id, block.position, bounds, impact.amount, kind);
                if let Some(host) = self.server.lan.as_ref() {
                    let conns = host
                        .hub
                        .guests
                        .iter()
                        .filter(|g| {
                            host.hub.here(g)
                                && g.loaded
                                && !g.dead
                                && !g.creative()
                                && !g.spectator()
                                && bounds.intersects(super::lan_mobs::guest_bounds(g))
                        })
                        .map(|g| g.conn)
                        .collect::<Vec<_>>();
                    for conn in conns {
                        self.guest_hit_by(conn, |g| {
                            g.falling_hurt_player(
                                block.id,
                                block.position,
                                bounds,
                                impact.amount,
                                kind,
                            )
                        });
                    }
                }
            }
            if anvil
                && impact.wear_chance > 0.0
                && self.common.world.random.next_float() < impact.wear_chance
                && let Some(mut state) =
                    self.common.pack.assets.registry.state(block.state).cloned()
            {
                if falling_block::wear_anvil(&mut state) {
                    block.state = self
                        .common
                        .pack
                        .assets
                        .registry
                        .resolve(&state)
                        .unwrap_or(block.state);
                } else {
                    block.damage.cancel_drop = true;
                }
            }
        }
    }

    /// `FallingBlockEntity.tick`: finish a landing, including the placed block
    /// entity and the carried `TileEntityData` applied through loadWithComponents.
    pub fn finish_falling_block(
        &mut self,
        block: FallingBlock,
        pos: [i32; 3],
    ) -> Option<FallingBlock> {
        // A moving piston's cell skips the place, drop and discard branch
        // before cancelDrop is even read: the block stays and is pushed on.
        let at = self.common.world.block(pos[0], pos[1], pos[2]);
        if &*self.common.res.facts().get(at).name == "minecraft:moving_piston" {
            return Some(block);
        }
        let name = self.common.res.facts().get(block.state).name.clone();
        let anvil = name.ends_with("anvil");
        // Fallable.onBrokenAfterFall: the anvil's, the pointed dripstone's and
        // the sulfur spike's level event.
        let sound_event = match &*name {
            "minecraft:pointed_dripstone" => Some(1045),
            "minecraft:sulfur_spike" => Some(1052),
            _ if anvil => Some(1029),
            _ => None,
        };
        if block.damage.cancel_drop {
            if let Some(id) = sound_event {
                self.common.level_events.push(LevelEvent::LevelEvent {
                    id,
                    pos: BlockPos::new(pos[0], pos[1], pos[2]),
                    data: 0,
                    source: None,
                });
            } else if self
                .common
                .res
                .facts()
                .get(block.state)
                .name
                .starts_with("minecraft:suspicious_")
            {
                // BrushableBlock.onBrokenAfterFall.
                let at = BlockPos::new(pos[0], pos[1], pos[2]);
                self.common.level_events.push(LevelEvent::LevelEvent {
                    id: 2001,
                    pos: at,
                    data: block.state.0 as i32,
                    source: None,
                });
                self.common.level_events.push(LevelEvent::GameEvent {
                    pos: at,
                    event: "minecraft:block_destroy",
                    state: None,
                });
            }
            return None;
        }
        let landing = self.with_level(|b, level, ticks| {
            b.falling_block_landed(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                block.state,
                false,
            )
        });
        if landing == Landing::Continue {
            return Some(block);
        }
        if let Landing::Placed(state) = landing {
            let name = self.common.res.facts().get(state).name.clone();
            // LevelChunk.setBlockState creates the new block entity even when
            // the falling entity has no supplied data.
            let key = (pos[0], pos[1], pos[2]);
            if !self.common.world.block_entities.stateful_entity(state.0)
                && let Some(entity) = self.new_block_entity_for_state(state)
            {
                self.common.world.block_entities.insert(key, entity);
            }
            if let Some(sunrise_core::nbt::Tag::Compound(carried)) = block
                .saved
                .as_ref()
                .and_then(|saved| saved.1.get("TileEntityData"))
                && let Some(entity) = self.common.world.block_entities.get(&key)
            {
                let mut tag =
                    self.encode_live_block_entity(key, entity, &name, self.common.res.items());
                // FallingBlockEntity.tick copies each supplied field over the
                // new block entity's saved defaults before loadWithComponents.
                if let sunrise_core::nbt::Tag::Compound(fields) = &mut tag {
                    fields.extend(carried.clone());
                    self.load_block_entity_nbt(pos, &sunrise_game_data::snbt::to_snbt(&tag));
                }
            }
        }
        // FallingBlockEntity.tick: the players tracking the entity are told to
        // show the block for a moment, after the block update.
        if let Landing::Placed(state) = landing
            && block.id != 0
            && let Some(host) = &self.server.lan
        {
            host.hub.send_to_tracking(
                block.id as i32,
                &sunrise_protocol::packets::play::Clientbound::AddTransientBlock(
                    sunrise_protocol::packets::play::AddTransientBlock {
                        pos,
                        state: state.0 as i32,
                    },
                ),
            );
        }
        if matches!(landing, Landing::Broken)
            && block.drop_item
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
        {
            if let Some(id) = sound_event {
                self.common.level_events.push(LevelEvent::LevelEvent {
                    id,
                    pos: BlockPos::new(pos[0], pos[1], pos[2]),
                    data: 0,
                    source: None,
                });
            }
            // FallingBlockEntity.tick: spawnAtLocation, from where the entity
            // came to rest (on top of a turtle egg, not inside its cell).
            self.drop_falling_block(block.state, block.position);
        }
        None
    }
    /// The falling block `entity` at `position`, whose box is `bounds`, hits the acting player.
    pub fn falling_hurt_player(
        &mut self,
        entity: u32,
        position: Vec3,
        bounds: sunrise_core::geometry::Aabb,
        amount: f32,
        kind: &str,
    ) {
        if self.common.creative || self.common.spectator() {
            return;
        }
        if self
            .common
            .player
            .as_ref()
            .is_none_or(|p| p.health.dead_or_dying() || !bounds.intersects(p.bounds()))
        {
            return;
        }
        let Some(id) = self.common.health_resources.damage.id(kind) else {
            return;
        };
        let attack = sunrise_sim::health::Attack {
            attacker: entity as i32,
            direct: Some(entity as i32),
            position: Some([position.x, position.y, position.z]),
            ..Default::default()
        };
        let outcome = self
            .hurt_player_equipment(id, amount, None, false, entity as i32, false, &attack, None)
            .unwrap_or_default();
        if outcome.hurt {
            // LivingEntity.hurtServer: ENTITY_HURT_PLAYER, the falling block
            // (`DamageSources.fallingBlock(this)` and the like) being both the
            // causing and the direct entity.
            let block = sunrise_sim::advancements::EntityView {
                kind: "minecraft:falling_block".into(),
                position: [position.x, position.y, position.z],
                ..Default::default()
            };
            self.trigger_entity_hurt_player(&sunrise_sim::advancements::DamageView {
                kind: kind.to_owned(),
                direct: Some(block.clone()),
                is_direct: true,
                source: Some(block),
                original: amount,
                actual: outcome.dealt,
                blocked: false,
            });
        }
        self.finish_guest_damage();
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_save_additional::BlockEntitySaveAdditionalGameExt1 as _;
