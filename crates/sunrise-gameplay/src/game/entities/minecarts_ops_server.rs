//! Server-side minecart operations split from common `minecarts.rs`, `minecarts/hit.rs`, and `minecarts/save_lending.rs`.
use crate::game::ServerGame;
use crate::game::minecarts_server::CartHit;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_game_data::sound_source::BLOCKS;
use sunrise_gameplay_common::game::minecarts::{
    CART_KEY_Y, CartKind, Minecart, TNT_PRIME_EVENT, cart_is_destroyed,
};
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;

impl ServerGame<'_> {
    /// `getDismountLocationForPassenger`: beside the cart's motion direction,
    /// else on top.
    pub(in crate::game) fn dismount_cart(&mut self, i: usize) {
        let id = self.common.carts[i].id;
        if self.common.riding == Some(id) && !self.native_may_leave(id) {
            return;
        }
        self.common.riding = None;
        if self.dismount_in_portal(self.common.carts[i].position) {
            return;
        }
        let target = self.cart_player_dismount_target(i);
        let from = self.common.player.as_ref().map(|player| player.position);
        if let Some(p) = self.common.player.as_mut() {
            p.position = target;
            p.velocity = Vec3::ZERO;
        }
        super::minecarts_server::section_steps::tell_player(self, from.map(|from| (from, target)));
    }

    /// The slots of the cart `id`, for a command: `getChestVehicleItem` and
    /// `setChestVehicleItem` first unpack its loot table with no player, so
    /// no `player_generates_container_loot` fires later. While its menu is
    /// open the live slots are the placeholder copy (`open_cart_container`).
    pub fn edit_cart_slots<R>(
        &mut self,
        id: u32,
        edit: impl FnOnce(&mut Vec<Option<ItemStack>>) -> R,
    ) -> Option<R> {
        let i = self.common.carts.iter().position(|c| c.id == id)?;
        self.unpack_loot(
            sunrise_gameplay_common::game::container_loot_level::LootTarget::Cart(id),
            None,
        );
        match self
            .common
            .world
            .block_entities
            .get_mut(&(i32::MAX - 15, CART_KEY_Y, id as i32))
        {
            Some(BlockEntity::Chest(open)) => Some(edit(open)),
            _ => Some(self.common.carts.update(i, |c| edit(&mut c.items))),
        }
    }

    /// `Containers.dropContents`: spill a vehicle's inventory from the entity
    /// position `at`: each slot (an empty one too) from a spot in its block,
    /// in pieces of 10 to 30 with the level random's scatter, no pickup
    /// delay. `AbstractChestBoat.remove` and `AbstractMinecartContainer.remove`
    /// spill this way for a destroying removal (kill or discard), and
    /// `chestVehicleDestroyed` again for the emptied container.
    pub(in crate::game) fn drop_vehicle_contents(
        &mut self,
        at: [f64; 3],
        items: &[Option<ItemStack>],
    ) {
        for stack in items {
            let count = stack.as_ref().map_or(0, |s| s.count);
            let pieces = sunrise_world::level::removal::drop_item_stack_at(
                &mut self.common.world.random,
                at,
                count,
            );
            if let Some(stack) = stack {
                for piece in pieces {
                    let piece_stack = ItemStack {
                        count: piece.count,
                        ..stack.clone()
                    };
                    self.spawn_item(piece_stack, piece.pos, piece.motion, 0);
                }
            }
        }
    }

    /// `MinecartTNT.primeFuse`: fuse 80, then `Level.broadcastEntityEvent`
    /// (`EntityEvent.TNT_PRIME`, 70) and, unless the cart is silent,
    /// `SoundEvents.TNT_PRIMED` on `SoundSource.BLOCKS`. A client that
    /// receives 70 only sets the fuse (`handleEntityEvent`).
    pub(in crate::game) fn prime_cart(&mut self, c: &mut Minecart) {
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
        {
            return;
        }
        c.fuse = 80;
        let at = [c.position.x, c.position.y, c.position.z];
        let id = c.id as i32;
        self.lan_entity_event(id, TNT_PRIME_EVENT, at);
        if !self.entity_silent(c.id) {
            self.server_play_sound_on(BLOCKS, "minecraft:entity.tnt.primed", at, 1.0, 1.0);
        }
    }

    /// `VehicleEntity.hurtServer`: shake, add the damage, break past 40 (at
    /// once for creative players); TNT minecarts prime instead when moving
    /// or hurt by fire or an explosion (`damageSourceIgnitesTnt`).
    pub fn hurt_cart(&mut self, i: usize, damage: f32, creative: bool, fire: bool, hit: CartHit) {
        self.hurt_cart_result(i, damage, creative, fire, hit);
    }

    /// `VehicleEntity.destroy(level, dropItem)`: the cart's item, carrying the
    /// vehicle's `CustomName` as `DataComponents.CUSTOM_NAME`.
    pub(in crate::game) fn cart_drop(&self, kind: CartKind, id: u32) -> ItemStack {
        let mut stack = ItemStack::new(kind.item(), 1);
        if let Some(name) = self
            .common
            .persist_raw(id)
            .and_then(|raw| raw.get("CustomName"))
        {
            let text = sunrise_sim::entity_nbt::name_text(name);
            stack.data_mut().custom_name = Some(sunrise_game_data::scoreboard::component_tag(
                &text.component(),
            ));
        }
        stack
    }

    /// `chestVehicleDestroyed`'s `angerNearbyPiglins`, for a container cart
    /// (`AbstractMinecartContainer`) that the hit's direct entity, a player,
    /// broke.
    pub(in crate::game) fn cart_destroy_angers(&mut self, hit: CartHit, kind: CartKind) {
        if hit.by_player && kind.slots() > 0 {
            self.anger_piglins(true);
        }
    }

    /// Accepted damage is distinct from survival, destruction, or TNT priming.
    pub fn hurt_cart_result(
        &mut self,
        i: usize,
        mut damage: f32,
        creative: bool,
        fire: bool,
        hit: CartHit,
    ) -> bool {
        let id = self.common.carts[i].id;
        let join = self.common.carts[i].join;
        let target = crate::game::commands::Target::Cart(id);
        if self.common.persist.destroying_vehicles.contains(&target)
            || self
                .common
                .persist
                .preparing_vehicle_removals
                .contains(&target)
        {
            return false;
        }
        let kind = self.common.carts[i].kind.id();
        let mut cancel = false;
        self.native_vehicle_damage(kind, &mut damage, &mut cancel);
        if cancel {
            return false;
        }
        let Some(i) = self.common.carts.iter().position(|c| {
            c.id == id
                && c.join == join
                && !self
                    .common
                    .persist
                    .removals
                    .contains(&crate::game::commands::Target::Cart(id))
        }) else {
            return true;
        };
        let destroyed = self.common.carts.update(i, |c| {
            c.hurt_dir = -c.hurt_dir;
            c.hurt_time = 10;
            c.damage += damage * 10.0;
            // MinecartTNT.shouldSourceDestroy: fire and explosions destroy
            // (so ignite) a TNT cart whatever damage it has taken.
            cart_is_destroyed(creative, c.damage, fire && c.kind == CartKind::Tnt)
        });
        // VehicleEntity.hurtServer: `gameEvent(ENTITY_DAMAGE, source.getEntity())`.
        let hurt_at = self.common.carts[i].position.components();
        self.entity_event_by("minecraft:entity_damage", hurt_at, hit.by);
        if destroyed {
            let speed = self.common.carts[i].velocity.x * self.common.carts[i].velocity.x
                + self.common.carts[i].velocity.z * self.common.carts[i].velocity.z;
            let tnt_prime =
                self.common.carts[i].kind == CartKind::Tnt && (fire || speed >= f64::from(0.01f32));
            if tnt_prime {
                let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
                let mut c = self
                    .common
                    .carts
                    .extract(self.common.carts[i].join)
                    .unwrap();
                // MinecartTNT.destroy: prime instead of breaking.
                if c.fuse < 0 {
                    self.prime_cart(&mut c);
                    c.fuse = c.random.next_int_bound(20).unwrap_or(0)
                        + c.random.next_int_bound(20).unwrap_or(0);
                }
                self.common.carts.put_back(i, c);
            } else {
                let join = self.common.carts[i].join;
                let at = self.common.carts[i].position.components();
                self.native_vehicle_destroy(self.common.carts[i].kind.id(), at);
                if !self.common.carts.iter().any(|c| {
                    c.id == id && c.join == join && !self.common.persist.removals.contains(&target)
                }) {
                    return true;
                }
                let Some(cart) = self
                    .common
                    .carts
                    .iter()
                    .find(|c| c.id == id && c.join == join)
                else {
                    return true;
                };
                let (kind, slots) = (cart.kind, cart.items.len());
                let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
                if self.kill_vehicle_object(target) && !creative {
                    self.entity_killed_event(at, false);
                    if self
                        .common
                        .ui
                        .rules
                        .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
                    {
                        self.spawn_item_own(self.cart_drop(kind, id), at, 10);
                        self.drop_vehicle_contents(at, &vec![None; slots]);
                        // ...and angers the piglins that see the player who broke it.
                        self.cart_destroy_angers(hit, kind);
                    }
                }
            }
        }
        true
    }
}

use sunrise_gameplay_common::game::minecarts::CartDismountGameExt as _;
