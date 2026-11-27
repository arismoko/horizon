//! First-access loot shared by blocks and chest vehicles.
use crate::game::ServerGame;
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::container_loot_level::{
    LootTarget, mixed_state_dropped, place_fill, roll_fill, take_block_pending,
};
use sunrise_level::containers::BlockEntity;
use sunrise_sim::save::PendingContainerLoot;

impl ServerGame<'_> {
    pub fn block_loot_pending(&self, key: (i32, i32, i32)) -> bool {
        self.common
            .world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|chunk| chunk.saved.as_ref())
            .is_some_and(|saved| saved.pending_loot.contains_key(&key))
    }

    pub fn loot_inventory_item(
        &mut self,
        key: (i32, i32, i32),
        index: usize,
    ) -> Option<sunrise_items::items::ItemStack> {
        self.unpack_loot(LootTarget::Block(key), None);
        self.common
            .world
            .block_entities
            .get(&key)?
            .slots()
            .get(index)?
            .clone()
    }
    /// A hopper cart's tick owns its metadata while its resident row is lent out.
    /// Publish only a pending inventory for this access, then recover the result.
    pub(in crate::game::server_side) fn unpack_ticked_cart_loot(
        &mut self,
        cart: &mut super::minecarts::Minecart,
    ) -> bool {
        if cart.loot.is_none() {
            return true;
        }
        let Some(i) = self
            .common
            .carts
            .iter()
            .position(|c| c.id == cart.id && c.join == cart.join)
        else {
            return false;
        };
        self.common.carts.update(i, |owner| {
            owner.loot = cart.loot.take();
            owner.items.clone_from(&cart.items);
            owner.position = cart.position;
        });
        self.unpack_loot(LootTarget::Cart(cart.id), None);
        let Some(i) = self
            .common
            .carts
            .iter()
            .position(|c| c.id == cart.id && c.join == cart.join)
        else {
            return false;
        };
        self.common.carts.update(i, |owner| {
            cart.loot = owner.loot.take();
            cart.items.clone_from(&owner.items);
        });
        true
    }
    /// Consumes metadata before callbacks, so reentrant access cannot roll twice.
    /// A player context is captured from the admitted actor, also during as_guest.
    pub fn unpack_loot(&mut self, target: LootTarget, player: Option<mod_api::PlayerRef>) {
        let (pending, pos) = match target {
            LootTarget::Block(key) => {
                let Some(pending) = take_block_pending(&mut self.common.world, key) else {
                    return;
                };
                if mixed_state_dropped(&self.common.world.block_entities[&key]) {
                    return;
                }
                (pending, key)
            }
            LootTarget::Cart(id) => {
                let Some(i) = self.common.carts.iter().position(|c| c.id == id) else {
                    return;
                };
                let Some(pending) = self.common.carts.update(i, |c| c.loot.take()) else {
                    return;
                };
                self.common.store_cart_items((
                    i32::MAX - 15,
                    super::minecarts::CART_KEY_Y,
                    id as i32,
                ));
                let c = &self.common.carts[i];
                (
                    pending,
                    (
                        c.position.x.floor() as i32,
                        c.position.y.floor() as i32,
                        c.position.z.floor() as i32,
                    ),
                )
            }
            LootTarget::Boat(id) => {
                let Some(i) = self.common.boats.iter().position(|b| b.id == id) else {
                    return;
                };
                let Some(pending) = self
                    .common
                    .persist
                    .raw
                    .get(&id)
                    .and_then(PendingContainerLoot::from_tag)
                else {
                    return;
                };
                if let Some(Tag::Compound(map)) = self.common.persist.raw.get_mut(&id) {
                    map.remove(&"LootTable".into());
                    map.remove(&"LootTableSeed".into());
                }
                self.common.store_cart_items((
                    i32::MAX - 15,
                    super::minecarts::CART_KEY_Y,
                    id as i32,
                ));
                let b = &self.common.boats[i];
                (
                    pending,
                    (
                        b.position.x.floor() as i32,
                        b.position.y.floor() as i32,
                        b.position.z.floor() as i32,
                    ),
                )
            }
        };
        // ContainerEntity unpacks the table even when some vehicle slots are taken.
        // LootTable.fill chooses only the free slots.
        let table = pending.table_id();
        if player.is_some() {
            self.trigger_loot_table(&table);
        }
        let Some(loot) = self.server.server_loot.0.get() else {
            return;
        };
        let biome = self
            .biome_name_at(super::Vec3::new(pos.0 as f64, pos.1 as f64, pos.2 as f64))
            .unwrap_or_default();
        let mut entity = player.map(|_| self.killer_loot_entity(None));
        if let Some(entity) = &mut entity {
            entity.on_fire = self
                .common
                .player
                .as_ref()
                .is_some_and(sunrise_sim::motion::PlayerMotion::is_on_fire);
        }
        let luck = if player.is_some() {
            self.common
                .player
                .as_mut()
                .and_then(|p| p.attribute("minecraft:luck").ok())
                .unwrap_or(0.0) as f32
        } else {
            0.0
        };
        let Some(mut slots) = (match target {
            LootTarget::Block(key) => {
                let world = &self.common.world;
                world.block_entities.get(&key).map(|entity| {
                    let slots = entity.slots();
                    if !matches!(entity, BlockEntity::Mod(_)) {
                        return slots.to_vec();
                    }
                    let state = world.block(key.0, key.1, key.2);
                    let block = &self.common.res.facts().get(state).name;
                    let count = sunrise_sim::save::block_mod_item_slots(block);
                    let mut items = slots[..slots.len().min(count)].to_vec();
                    items.resize(count, None);
                    items
                })
            }
            LootTarget::Cart(id) => self
                .common
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.items.clone()),
            LootTarget::Boat(id) => self
                .common
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.items.clone()),
        }) else {
            return;
        };
        let origin = match target {
            LootTarget::Block(_) => [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5],
            LootTarget::Cart(id) => {
                let p = self
                    .common
                    .carts
                    .iter()
                    .find(|c| c.id == id)
                    .unwrap()
                    .position;
                [p.x, p.y, p.z]
            }
            LootTarget::Boat(id) => {
                let p = self
                    .common
                    .boats
                    .iter()
                    .find(|b| b.id == id)
                    .unwrap()
                    .position;
                [p.x, p.y, p.z]
            }
        };
        let mut random = self.common.world.random.clone();
        let mut sequences = std::mem::take(&mut self.server.rand_sequences);
        let rolled = {
            let level = crate::game::advancements::View(&*self);
            let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                sunrise_sim::advancements::location(predicate, at, &level)
            };
            let context = sunrise_world::loot::ContainerLootContext {
                biome: Some(&biome),
                location: Some(sunrise_sim::advancements::loot_location(origin, &read)),
                luck,
                entity: entity.as_ref(),
            };
            roll_fill(
                &pending,
                &loot,
                &mut random,
                &mut sequences,
                self.common.level.seed,
                context,
            )
        };
        if let Some((mut items, source)) = rolled {
            self.container_fill_hook(&table, target, origin, player, &mut items);
            place_fill(
                items,
                &mut slots,
                &loot,
                source,
                &mut random,
                &mut sequences,
                self.common.level.seed,
            );
        }
        self.common.world.random = random;
        self.server.rand_sequences = sequences;
        match target {
            LootTarget::Block(key) => {
                if let Some(e) = self.common.world.block_entities.get_mut(&key) {
                    e.replace_slots(&slots);
                }
            }
            LootTarget::Cart(id) => {
                if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
                    self.common.carts.update(i, |c| c.items = slots);
                }
            }
            LootTarget::Boat(id) => {
                if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
                    self.common.boats.update(i, |b| b.items = slots);
                }
            }
        }
        let id = match target {
            LootTarget::Cart(id) | LootTarget::Boat(id) => Some(id),
            _ => None,
        };
        if let Some(id) = id {
            let items = match target {
                LootTarget::Cart(_) => self
                    .common
                    .carts
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.items.clone()),
                _ => self
                    .common
                    .boats
                    .iter()
                    .find(|b| b.id == id)
                    .map(|b| b.items.clone()),
            };
            if let (Some(items), Some(BlockEntity::Chest(open))) = (
                items,
                self.common.world.block_entities.get_mut(&(
                    i32::MAX - 15,
                    super::minecarts::CART_KEY_Y,
                    id as i32,
                )),
            ) {
                *open = items;
            }
        }
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::native_hooks::container_loot_filled::ContainerLootFilledGameExt1 as _;
