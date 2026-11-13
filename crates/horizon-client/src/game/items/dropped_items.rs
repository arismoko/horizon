use super::*;

impl Game<'_> {
    pub(super) fn tick_dropped_items(&mut self, round: &mut native_ticks::TickRound) {
        let mut items = self.dropped.lift();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut removed = vec![false; items.len()];
        let mut cells = horizon_client::entities::ItemCells::new(&items);
        // A LAN or server guest only shows the host's items.
        let merging = self.remote.is_none();
        // `Entity.checkBelowWorld`: below `Level.getMinY() - 64`.
        let void = f64::from(self.level_min_y() - 64);
        let mut obstacles =
            horizon_client::entities::ObstacleGrid::new(self.item_entity_collisions());
        let mut nearby = Vec::new();
        let mut bubble_tops = Vec::new();
        let mut moves = ahead::Moves::work_out(self, &items, &obstacles, round, |item| {
            !self.joined_during_entity_pass(item.id)
                && ticking(
                    item.position.x.floor() as i32,
                    item.position.z.floor() as i32,
                )
        });
        // `ItemUtils.onContainerDestroyed`: what a destroyed item held, and
        // where it lay, for adding to the level once its own tick is done.
        let mut destroyed: Vec<(ItemStack, Vec3)> = Vec::new();
        let item_type = horizon_threads::work::entity_id("minecraft:item");
        for i in 0..items.len() {
            horizon_threads::work::set_entity(item_type);
            if removed[i] {
                // Merged into an earlier item before its own tick: it never ticked.
                round.not_ticked(items[i].id);
                continue;
            }
            // `addFreshEntity` during the entity pass: the item ticks from the
            // next tick on (`EntityTickList`).
            if merging && self.joined_during_entity_pass(items[i].id) {
                continue;
            }
            if !ticking(
                items[i].position.x.floor() as i32,
                items[i].position.z.floor() as i32,
            ) || round.canceled(items[i].id)
                || round.halted(items[i].id)
            {
                continue;
            }
            self.tick_saved_invulnerable_time(items[i].id);
            let item = &mut items[i];
            let mut at_turn = |item: &mut ItemEntity| {
                // `getEntityCollisions(entity, bounds.expandTowards(movement))`: the boxes around
                // where the item can reach this tick.
                obstacles.near(ahead::reach(item), &mut nearby);
                // ItemEntity.tick's noPhysics (servers only): out of a block, a solid entity
                // or the border, placed or closed on it, towards the closest open side (#1252).
                let no_physics = merging
                    && item.inside_block(
                        &self.world,
                        self.collision,
                        &nearby,
                        Some(self.border.geometry()),
                    );
                // `moveTowardsClosestSpace` draws the speed from the item's own random.
                let escape_speed = no_physics.then(|| item.random.next_float() * 0.2 + 0.1);
                let mut entered = Vec::new();
                let (alive, discarded) = self.tick_item_with_block_contacts(
                    item,
                    escape_speed,
                    &nearby,
                    false,
                    &mut |splash| entered.push(splash),
                );
                (no_physics, alive, discarded, entered)
            };
            let (no_physics, alive, discarded, entered) = match moves.take(i, item, &mut at_turn) {
                Some(moved) => moved,
                None => at_turn(item),
            };
            // Entity.doWaterSplashEffect: the server plays the sound (broadcast
            // to clients). Its addParticle is empty.
            for splash in entered {
                self.apply_item_splash(splash);
            }
            let from = item.previous;
            let to = item.position;
            if !discarded && round.sections() {
                self.record_dropped_item_step(items[i].id, from, to);
            }
            let item = &mut items[i];
            if discarded {
                removed[i] = true;
                cells.remove(i, item.previous);
                continue;
            }
            if merging && !no_physics {
                self.item_landing_event(item.id, item.previous, item.position, item.on_ground);
                self.entity_powder_snow_particles(
                    item.previous,
                    item.position,
                    item.bounds(),
                    item.velocity,
                    false,
                );
            }
            cells.moved(i, item.previous, item.position);
            // Entity.handleOnAboveBubbleColumn: the server sends the particles of
            // the surface of a bubble column the item is at the top of.
            if merging {
                bubble_tops.extend(horizon_client::motion::bubble_column_surfaces(
                    &self.world,
                    self.pack.assets.registry,
                    self.res.facts(),
                    self.collision,
                    item.bounds(),
                ));
            }
            // ItemEntity.tick: merge every 2 ticks while moving between
            // blocks, every 40 otherwise, before the lifetime check.
            let block = |v: Vec3| (v.x.floor(), v.y.floor(), v.z.floor());
            let rate = if block(item.previous) == block(item.position) {
                40
            } else {
                2
            };
            if merging
                && (alive || item.age == 6000 && item.position.y >= void)
                && item.ticks.is_multiple_of(rate)
            {
                let max_stack = |stack: &ItemStack| self.stack_max_size(stack).max(1);
                horizon_client::entities::merge_with_neighbours(
                    &mut items,
                    &mut removed,
                    &mut cells,
                    i,
                    true,
                    &max_stack,
                );
            }
            let item = &mut items[i];
            let mut alive = alive || (item.age < 6000 && item.position.y >= void);
            if merging && !removed[i] && item.age >= 6000 && item.position.y >= void {
                let mut extra_life = 0;
                self.native_item_expire(item.id, &item.stack, &mut extra_life);
                if extra_life > 0 {
                    item.age = 6000 - extra_life.min(32766);
                    alive = true;
                } else if self.serving.is_some() || self.options.dedicated.is_some() {
                    self.native_item_despawn(
                        item.id,
                        &item.stack.item,
                        item.stack.count as i32,
                        [item.position.x, item.position.y, item.position.z],
                    );
                }
            }
            if !alive || removed[i] {
                removed[i] = true;
                cells.remove(i, item.position);
                continue;
            }
            let item_id = item.id;
            let saved = native_components::ITEM_ENVIRONMENT_DAMAGE
                .map(|damage| self.saved_refuses_damage(item_id, damage, false));
            let tags = world_items!(self);
            let registry = self.pack.assets.registry;
            let block_registries = &self.pack.block_registries;
            let hooks = &self.damage_hooks;
            let environment = item.environment_tick_with_hook(
                &mut self.world,
                self.res.facts(),
                registry,
                &native_components::item_damage_resolver(
                    &self.pack.item_renderer.models,
                    &self.health_resources.damage,
                ),
                |world, damage| {
                    let vanilla = match damage {
                        "minecraft:on_fire" => saved[0],
                        "minecraft:lava" => saved[1],
                        "minecraft:in_fire" => saved[2],
                        "minecraft:cactus" => saved[3],
                        _ => false,
                    };
                    let source = horizon_client::health::HurtSource {
                        damage_type: damage,
                        causing: 0,
                        direct: 0,
                        position: None,
                    };
                    horizon_client::health::DamageLevel::of(
                        world,
                        registry,
                        Some(tags),
                        Some(block_registries),
                        hooks,
                    )
                    .invulnerability_check(
                        horizon_client::health::Hurting::Entity(
                            item_id as i32,
                            "minecraft:item",
                            source,
                        ),
                        vanilla,
                    )
                },
            );
            if environment.should_play_lava_hurt_sound(item.ticks) {
                let pitch = 2.0 + self.world.random.next_float() * 0.4;
                self.audio.play(
                    "minecraft:entity.generic.burn",
                    Some([item.position.x, item.position.y, item.position.z]),
                    0.4,
                    pitch,
                );
            }
            if !environment.alive {
                removed[i] = true;
                cells.remove(i, item.position);
                for stack in environment.spilled {
                    destroyed.push((stack, item.position));
                }
            }
        }
        for pos in bubble_tops {
            self.bubble_column_particles(pos);
        }
        let mut removed = removed.into_iter();
        items.retain(|_| !removed.next().unwrap_or(false));
        let Some(p) = self.player.as_ref() else {
            self.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            self.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [
                p.position.x - w - 1.0,
                p.position.y - 0.5,
                p.position.z - w - 1.0,
            ],
            [
                p.position.x + w + 1.0,
                p.position.y + f64::from(p.dimensions.height) + 0.5,
                p.position.z + w + 1.0,
            ],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let own = self.level.ensure_player_uuid();
        self.native_player_pickups(&mut items, (min, max), feet, own);
        self.put_back_dropped(items);
        self.spawn_destroyed_contents(destroyed);
    }
}
