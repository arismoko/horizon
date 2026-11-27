use sunrise_core::random::LegacyRandom;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::save::PendingContainerLoot;

#[derive(Clone, Copy)]
pub enum LootTarget {
    Block((i32, i32, i32)),
    Cart(u32),
    Boat(u32),
}

/// Consume the sole block-owned marker before any callback or inventory mutation.
pub fn take_block_pending(
    world: &mut sunrise_level::world::World,
    key: (i32, i32, i32),
) -> Option<PendingContainerLoot> {
    if !world.block_entities.contains_key(&key)
        || !world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|c| c.saved.as_ref())
            .is_some_and(|s| s.pending_loot.contains_key(&key))
    {
        return None;
    }
    let mut pending = None;
    world.with_chunk_extras(key, |extras| pending = extras.pending_loot.remove(&key));
    world.modified.insert((key.0 >> 4, key.2 >> 4));
    pending
}

pub fn mixed_state_dropped(entity: &BlockEntity) -> bool {
    !matches!(entity, BlockEntity::Mod(_)) && entity.slots().iter().any(Option::is_some)
}

/// The random a container's loot drew from (`LootContext.Builder.create`: the explicit seed,
/// the table's named random sequence, then the level random), kept for the slot shuffle.
pub enum FillRandom {
    Seeded(LegacyRandom),
    Sequence(String),
    Level,
}

/// `LootTable.fill`'s roll of the pending table (`getRandomItems` with the stack splitter),
/// with `context` read by its conditions. None for a missing table, which draws nothing.
pub fn roll_fill(
    pending: &PendingContainerLoot,
    loot: &sunrise_world::loot::LootData,
    level_random: &mut LegacyRandom,
    sequences: &mut sunrise_items::random_sequences::RandomSequences,
    world_seed: i64,
    context: sunrise_world::loot::ContainerLootContext<'_>,
) -> Option<(Vec<sunrise_world::loot::LootStack>, FillRandom)> {
    let table = pending.table_id();
    if let Some(seed) = pending.seed_value().filter(|seed| *seed != 0) {
        let mut random = LegacyRandom::new(seed);
        let items = loot.roll_container_with(&table, &mut random, context)?;
        Some((items, FillRandom::Seeded(random)))
    } else if let Some(id) = loot.random_sequence(&table) {
        let items = loot.roll_container_with(&table, sequences.get(id, world_seed), context)?;
        Some((items, FillRandom::Sequence(id.to_owned())))
    } else {
        let items = loot.roll_container_with(&table, level_random, context)?;
        Some((items, FillRandom::Level))
    }
}

/// `LootTable.fill`'s placement into the free slots, drawing on from the random the roll
/// used. Entry points discard a pre-existing mixed state; items added by the player's
/// advancement rewards keep their slots while this fills the free ones.
pub fn place_fill(
    items: Vec<sunrise_world::loot::LootStack>,
    slots: &mut [Option<sunrise_items::items::ItemStack>],
    loot: &sunrise_world::loot::LootData,
    random: FillRandom,
    level_random: &mut LegacyRandom,
    sequences: &mut sunrise_items::random_sequences::RandomSequences,
    world_seed: i64,
) {
    let mut placed = vec![None; slots.len()];
    let free = slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.is_none().then_some(i))
        .collect();
    match random {
        FillRandom::Seeded(mut random) => {
            loot.place_container_loot(items, &mut placed, free, &mut random)
        }
        FillRandom::Sequence(id) => {
            loot.place_container_loot(items, &mut placed, free, sequences.get(&id, world_seed))
        }
        FillRandom::Level => loot.place_container_loot(items, &mut placed, free, level_random),
    }
    for (slot, stack) in slots.iter_mut().zip(placed) {
        if let Some(stack) = stack {
            *slot = Some(sunrise_sim::structure_blocks::loot_item_with_components(
                &stack,
            ));
        }
    }
}

impl super::GameLevel<'_> {
    /// Null-player accesses inside world behavior (comparator, destruction).
    pub(in crate::game) fn unpack_loot(
        &mut self,
        target: LootTarget,
        player: Option<mod_api::PlayerRef>,
    ) {
        debug_assert!(player.is_none());
        if let LootTarget::Cart(id) = target {
            let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                return;
            };
            let Some(pending) = self.carts.update(i, |c| c.loot.take()) else {
                return;
            };
            let Some(loot) = self.loot.as_ref() else {
                return;
            };
            let p = self.carts[i].position;
            let pos = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
            let biome = self.world.chunks.get(&(pos.0 >> 4, pos.2 >> 4)).map(|c| {
                self.biomes
                    .get(c.biome(pos.0 & 15, pos.1, pos.2 & 15))
                    .name
                    .clone()
            });
            let key = (
                i32::MAX - 15,
                sunrise_sim::lan::menu::VEHICLE_KEY_Y,
                id as i32,
            );
            let mut slots = self.carts[i].items.clone();
            if let Some(BlockEntity::Chest(open)) = self.world.block_entities.get(&key) {
                slots.clone_from(open);
            }
            if slots.iter().any(Option::is_some) {
                self.carts.update(i, |cart| cart.items = slots);
                return;
            }
            let loot = loot.clone();
            let mut random = self.world.random.clone();
            let mut sequences = std::mem::take(self.rand_sequences);
            let origin = [p.x, p.y, p.z];
            let rolled = {
                let level = self.location_view();
                let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                    sunrise_sim::advancements::location(predicate, at, &level)
                };
                let context = sunrise_world::loot::ContainerLootContext {
                    biome: biome.as_deref(),
                    location: Some(sunrise_sim::advancements::loot_location(origin, &read)),
                    ..Default::default()
                };
                roll_fill(
                    &pending,
                    &loot,
                    &mut random,
                    &mut sequences,
                    self.seed,
                    context,
                )
            };
            if let Some((mut items, source)) = rolled {
                self.container_fill_hook(&pending.table_id(), target, origin, player, &mut items);
                place_fill(
                    items,
                    &mut slots,
                    &loot,
                    source,
                    &mut random,
                    &mut sequences,
                    self.seed,
                );
            }
            self.world.random = random;
            *self.rand_sequences = sequences;
            self.carts.update(i, |cart| cart.items = slots);
            if let Some(BlockEntity::Chest(open)) = self.world.block_entities.get_mut(&key) {
                open.clone_from(&self.carts[i].items);
            }
            return;
        }
        let LootTarget::Block(key) = target else {
            unreachable!("level inventory access")
        };
        let Some(pending) = take_block_pending(self.world, key) else {
            return;
        };
        let Some(loot) = self.loot.as_ref() else {
            return;
        };
        let biome = self.world.chunks.get(&(key.0 >> 4, key.2 >> 4)).map(|c| {
            self.biomes
                .get(c.biome(key.0 & 15, key.1, key.2 & 15))
                .name
                .clone()
        });
        let Some(entity) = self.world.block_entities.get(&key) else {
            return;
        };
        if mixed_state_dropped(entity) {
            return;
        }
        let mut slots = entity.slots().to_vec();
        let loot = loot.clone();
        let mut random = self.world.random.clone();
        let mut sequences = std::mem::take(self.rand_sequences);
        let origin = [key.0, key.1, key.2].map(|c| f64::from(c) + 0.5);
        let rolled = {
            let level = self.location_view();
            let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                sunrise_sim::advancements::location(predicate, at, &level)
            };
            let context = sunrise_world::loot::ContainerLootContext {
                biome: biome.as_deref(),
                location: Some(sunrise_sim::advancements::loot_location(origin, &read)),
                ..Default::default()
            };
            roll_fill(
                &pending,
                &loot,
                &mut random,
                &mut sequences,
                self.seed,
                context,
            )
        };
        if let Some((mut items, source)) = rolled {
            self.container_fill_hook(&pending.table_id(), target, origin, player, &mut items);
            place_fill(
                items,
                &mut slots,
                &loot,
                source,
                &mut random,
                &mut sequences,
                self.seed,
            );
        }
        self.world.random = random;
        *self.rand_sequences = sequences;
        if let Some(entity) = self.world.block_entities.get_mut(&key) {
            entity.replace_slots(&slots);
        }
    }
}

impl super::GameLevel<'_> {
    /// The other lists stay shared while carts can be mutated by inventory reads.
    pub fn entities(&self) -> super::entity::EntityLists<'_> {
        super::entity::EntityLists {
            carts: &*self.carts,
            ..self.other_entities
        }
    }
    /// Detector rails prioritize command carts, then read the first container.
    pub(in crate::game) fn detector_rail_signal(
        &mut self,
        pos: sunrise_core::position::BlockPos,
        powered: bool,
    ) -> i32 {
        use sunrise_core::geometry::{Aabb, Vec3};
        if !powered {
            return 0;
        }
        let (x, y, z) = (pos.x as f64, pos.y as f64, pos.z as f64);
        let area = Aabb::new(
            Vec3::new(x + 0.2, y, z + 0.2),
            Vec3::new(x + 0.8, y + 0.8, z + 0.8),
        );
        let mut container = None;
        for (i, ((lo, hi), cart)) in self.cart_boxes.iter().zip(self.carts.iter()).enumerate() {
            if !area.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )) {
                continue;
            }
            match cart.kind {
                super::minecarts::CartKind::CommandBlock => return cart.command.success_count,
                super::minecarts::CartKind::Chest | super::minecarts::CartKind::Hopper
                    if container.is_none() =>
                {
                    container = Some(i)
                }
                _ => {}
            }
        }
        let Some(i) = container else { return 0 };
        self.unpack_loot(LootTarget::Cart(self.carts[i].id), None);
        let cart = &self.carts[i];
        let key = (
            i32::MAX - 15,
            sunrise_sim::lan::menu::VEHICLE_KEY_Y,
            cart.id as i32,
        );
        let slots = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Chest(open)) => open,
            _ => &cart.items,
        };
        super::minecarts::Minecart::container_signal(slots, self.items)
    }
}
