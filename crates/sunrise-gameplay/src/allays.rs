//! Allays on the logical side: the allay's tick, its liked player and its delivery, on `MobLevel`.
use crate::mob_level::MobLevel;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    entities::ItemEntity,
    join_seq::JoinList,
    mobs::{Mob, Species, allay},
};

/// A player an allay can like: where they stand and where their eyes are.
pub struct AllayPlayer {
    pub position: Vec3,
    pub eye: [f64; 3],
    pub spectator: bool,
}

/// One read-only view of the level for a whole sensor scan: mobs, dropped items and `LivingEntity.hasLineOfSight`.
pub struct SightView<'a> {
    pub mobs: &'a JoinList<Mob>,
    pub dropped: &'a JoinList<ItemEntity>,
    pub line_of_sight: &'a dyn Fn(Vec3, Vec3) -> bool,
}

/// What the allay tick calls beyond `MobLevel`.
pub trait AllayLevel: MobLevel {
    /// The UUID of the host's own player, if the level has one.
    fn level_player_uuid(&self) -> Option<[i32; 4]>;
    /// The host's own player (`ServerLevel.getPlayers` first), none for a server without one.
    fn host_player(&self) -> Option<AllayPlayer>;
    /// The guest in the level with this UUID who is not a spectator (`lan_present`).
    fn guest_player(&self, uuid: [i32; 4]) -> Option<AllayPlayer>;
    /// The level's dimension id.
    fn dimension_id(&self) -> &'static str;
    /// `LevelSound` bound to mob `index`, as `Level.playSound(null, mob, ...)`.
    fn play_mob_bound_sound(
        &mut self,
        index: usize,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        except_actor: bool,
    );
    /// Runs `scan` on one `SightView` of the level.
    fn with_sight_view<R>(&self, scan: impl FnOnce(SightView<'_>) -> R) -> R;
    /// The mod and advancement trigger for each player in the level with this UUID who is not a spectator.
    fn trigger_block(&mut self, uuid: [i32; 4], name: &str, pos: [i32; 3], stack: &ItemStack);
    /// `new ItemEntity(level, x, y, z, stack, dx, dy, dz)`.
    fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: i32,
    );
}

pub fn tick_allays(level: &mut impl AllayLevel) {
    let griefing = level.mob_griefing();
    let count = level.parts().mobs.len();
    for i in 0..count {
        let p = level.parts();
        if p.mobs[i].species() != Species::Allay
            || !p.mobs[i].alive()
            || !p.ticked.contains(&p.mobs[i].id)
        {
            continue;
        }
        let pos = p.mobs[i].position;
        // The dance follows the jukebox events the allay hears (`mob_listen`), `Allay.tick` and `aiStep`.
        let Some(held) = p.mobs[i].extra.main_hand_stack().cloned() else {
            p.mobs[i].extra.point = None;
            continue;
        };
        let id = p.mobs[i].id;
        let loots = p.mobs[i].can_pick_up_loot();
        let no_ai = p.mobs[i].no_ai;
        // `Mob.aiStep`'s looting loop asks for no AI: `canPickUpLoot` and mob_griefing gate it.
        let wants = loots && level.entity_grief(id, griefing);
        let p = level.parts();
        let reach = allay::pickup_reach(p.mobs[i].bounds());
        let eye = Vec3::new(pos.x, p.mobs[i].eye_y(), pos.z);
        let b = p.mobs[i].bounds();
        // Without AI only the looting loop runs: it takes an equal drop already in `getPickupReach`.
        let wanted = if !wants {
            None
        } else if no_ai {
            p.dropped.iter().position(|e| {
                e.pickup_delay == 0
                    && allay::considers_item_equal(&held, &e.stack)
                    && room(&p.mobs[i], &e.stack) > 0
                    && reach.contains(e.position)
            })
        } else {
            // The sensor looks in the allay's box grown by (32, 16, 32).
            let (low, high) = (b.min_position().y - 16.0, b.max_position().y + 16.0);
            level.with_sight_view(|v| {
                v.dropped
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        e.pickup_delay == 0
                            && allay::considers_item_equal(&held, &e.stack)
                            && room(&v.mobs[i], &e.stack) > 0
                    })
                    .map(|(j, e)| (j, e.position.distance_squared(pos), e.position))
                    .filter(|(_, d, _)| *d < 32.0 * 32.0)
                    .filter(|(_, _, at)| at.y <= high && at.y + 0.25 >= low)
                    // ItemEntity's eye height is 0.2125.
                    .filter(|(_, _, at)| {
                        (v.line_of_sight)(eye, Vec3::new(at.x, at.y + 0.2125, at.z))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(j, _, _)| j)
            })
        };
        let Some(j) = wanted else {
            level.parts().mobs[i].extra.point = None;
            // `GiveInventoryToLookTargetPlayer` is the brain's too.
            if !no_ai {
                allay_deliver(level, i);
            }
            continue;
        };
        let p = level.parts();
        let e = &p.dropped[j];
        if reach.contains(e.position) {
            let item = e.clone();
            let max = p.mobs[i].kind.inventory_stack_limit(&e.stack);
            let take = p.mobs[i]
                .extra
                .allay
                .as_mut()
                .unwrap()
                .add(e.stack.clone(), max);
            level.take(id, item);
            let e = &mut level.parts().dropped[j];
            e.stack.count -= take;
            if e.stack.count == 0 {
                level.discard_item(j);
            }
            let at = [pos.x, pos.y, pos.z];
            let random = &mut level.parts().world.random;
            let pitch = (random.next_float() - random.next_float()) * 1.4 + 2.0;
            level.play_sound("minecraft:entity.item.pickup", at, 0.2, pitch);
            level.parts().mobs[i].extra.point = None;
        } else {
            p.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
        }
    }
}

/// The UUID of the player an allay likes (`LIKED_PLAYER`).
fn allay_liked_uuid(level: &mut impl AllayLevel, i: usize) -> [i32; 4] {
    let owner = level.parts().mobs[i].extra.owner;
    // An allay saved before its player was kept belongs to the host.
    owner
        .filter(|u| *u != [0, 0, 0, 1])
        .or(level.level_player_uuid())
        .unwrap_or([0, 0, 0, 1])
}

/// `AllayAi.getLikedPlayer`: the host or a guest of the level by UUID, not a spectator, within 64 blocks.
fn allay_liked_player(level: &mut impl AllayLevel, i: usize) -> Option<Vec3> {
    let (tame, mob_position) = {
        let m = &level.parts().mobs[i];
        (m.extra.tame, m.position)
    };
    if !tame {
        return None;
    }
    let uuid = allay_liked_uuid(level, i);
    let host = level
        .host_player()
        .filter(|p| !p.spectator && level.level_player_uuid().unwrap_or([0, 0, 0, 1]) == uuid)
        .map(|p| (p.position, p.eye));
    let found = host.or_else(|| level.guest_player(uuid).map(|p| (p.position, p.eye)));
    // The 64-block search uses the feet; the deposit point is the eyes (`new EntityTracker(player, true)`).
    found
        .filter(|(feet, _)| feet.distance_squared(mob_position) < 64.0 * 64.0)
        .map(|(_, eye)| Vec3::new(eye[0], eye[1], eye[2]))
}

/// How many of `stack` the allay's inventory still takes.
fn room(m: &Mob, stack: &ItemStack) -> u32 {
    m.extra
        .allay
        .as_ref()
        .map_or(0, |st| st.room(stack, m.kind.inventory_stack_limit(stack)))
}

/// `GoAndGiveItemsToTarget` / `AllayAi.throwItem`: within 3 blocks of the deposit, one item a tick,
/// each restarting the three-second pause before it picks anything up again.
fn allay_deliver(level: &mut impl AllayLevel, i: usize) {
    if allay::inventory(&level.parts().mobs[i]).is_none() {
        return;
    }
    // AllayAi.getItemDepositPosition: the liked note block, else the player.
    let dimension = level.dimension_id();
    let p = level.parts();
    let noteblock = allay::noteblock_deposit(&mut p.mobs[i], dimension, |at| {
        &*p.facts.get(p.world.block(at[0], at[1], at[2])).name == "minecraft:note_block"
    });
    let player = allay_liked_player(level, i);
    let m = &level.parts().mobs[i];
    let target = match noteblock {
        Some(p) => Vec3::new(p[0], p[1], p[2]),
        None => match player.filter(|_| m.extra.tame) {
            Some(p) => p,
            None => return,
        },
    };
    let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
    if target.distance_squared(eye) >= 9.0 {
        return;
    }
    let Some(item) = allay::inventory(m).cloned() else {
        return;
    };
    let from = [
        m.position.x,
        m.position.y + m.eye_height() - 0.2,
        m.position.z,
    ];
    // BehaviorUtils.throwItem: the vector runs from the allay's feet.
    let d = [
        target.x - m.position.x,
        target.y + 1.0 - m.position.y,
        target.z - m.position.z,
    ];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let v = [d[0] / len * 0.2, d[1] / len * 0.3, d[2] / len * 0.2];
    // AllayAi.throwItem: the liked player hears of the block below the deposit.
    let below = [
        target.x.floor() as i32,
        (target.y - 1.0).floor() as i32,
        target.z.floor() as i32,
    ];
    let uuid = allay_liked_uuid(level, i);
    // AllayAi.throwItem: one item of the pocket stack, its components and all.
    let stack = ItemStack { count: 1, ..item };
    level.trigger_block(uuid, "minecraft:allay_drop_item_on_block", below, &stack);
    let m = &mut level.parts().mobs[i];
    m.extra.allay.as_mut().unwrap().take(1);
    allay::set_pickup_cooldown(m, Some(60));
    // ItemEntity.setDefaultPickUpDelay.
    level.spawn_item(stack, from, v, 10);
    // BehaviorUtils.throwItem: ItemEntity.setThrower(the allay).
    let id = level.parts().mobs[i].id;
    let allay = level.mob_uuid(id);
    if let Some(thrown) = level.just_spawned_item() {
        thrown.thrower = Some(allay);
    }
    let p = level.parts();
    if p.world.game_time % 7 == 0 && p.world.random.next_double() < 0.9 {
        const PITCHES: [f32; 16] = [
            0.5625, 0.625, 0.75, 0.9375, 1.0, 1.0, 1.125, 1.25, 1.5, 1.875, 2.0, 2.25, 2.5, 3.0,
            3.75, 4.0,
        ];
        let pitch = PITCHES[p.world.random.next_int_bound(16).unwrap_or(4) as usize];
        level.play_mob_bound_sound(
            i,
            "minecraft:entity.allay.item_thrown",
            6,
            1.0,
            pitch,
            false,
        );
    }
}
