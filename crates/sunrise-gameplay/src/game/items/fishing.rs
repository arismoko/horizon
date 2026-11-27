//! Fishing: `FishingRodItem.use` (cast / reel in), the `FishingHook` (flight,
//! bobbing, lure and bite timers, open water, hooking mobs, retrieving the
//! `gameplay/fishing` loot) and `FishingHookRenderer` (bobber and line).
use super::lan_mobs::guest_bounds;
use std::sync::Arc;
use sunrise_core::{
    geometry::{Aabb, Vec3, trig},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::game::creatures::ProjectileHitter;
#[cfg(test)]
use sunrise_gameplay_common::game::fishing_common::inside_medium;
use sunrise_gameplay_common::game::fishing_common::{
    Hook, HookLerp, HookMedium, SIZE, State, bite_impulse, bounds, rebound, synchronized_random,
};
use sunrise_gameplay_common::game::item_abilities::ItemAbilitiesGameExt1 as _;
use sunrise_protocol::entity_data::{Value, serializer};
use sunrise_sim::advancements::EntityView;
use sunrise_sim::lan::entity::States;
use {
    sunrise_items::blocks::Fluid, sunrise_items::items::ItemStack,
    sunrise_level::collisions::SceneCollisions, sunrise_level::particle_options::ParticleType,
    sunrise_level::random_source,
};

#[path = "fishing/impact.rs"]
pub(in crate::game) mod impact;

/// The table a catch rolls where no mod's fishing medium names one.
pub const FISHING_TABLE: &str = "minecraft:gameplay/fishing";

/// `LootContextParamSets.FISHING`'s entities: the hook is THIS_ENTITY and its
/// owner the attacking entity, with the luck the hook and the owner add.
pub fn hook_params(
    luck: f32,
    owner: Option<sunrise_world::loot::LootEntity>,
) -> sunrise_world::loot::EntityLootParams {
    sunrise_world::loot::EntityLootParams {
        this: sunrise_world::loot::LootEntity {
            living: false,
            ..sunrise_world::loot::LootEntity::new("minecraft:fishing_bobber")
        },
        luck,
        attacker: owner,
        ..Default::default()
    }
}

/// `FishingHook.retrieve`: the table's named random sequence, or the level
/// random when a datapack table does not name one.
#[allow(clippy::too_many_arguments)]
pub fn fishing_loot(
    loot: &sunrise_world::loot::LootData,
    table: &str,
    sequences: &mut sunrise_items::random_sequences::RandomSequences,
    level_random: &mut LegacyRandom,
    world_seed: i64,
    biome: Option<&str>,
    location: Option<sunrise_world::loot::LootLocation<'_>>,
    open_water: bool,
    params: &sunrise_world::loot::EntityLootParams,
) -> Vec<sunrise_world::loot::LootStack> {
    match loot.random_sequence(table) {
        Some(sequence) => loot.roll_as(
            table,
            sequences.get(sequence, world_seed),
            biome,
            location,
            Some(open_water),
            params,
        ),
        None => loot.roll_as(
            table,
            level_random,
            biome,
            location,
            Some(open_water),
            params,
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::game) enum EntityCollision {
    Hooked,
    Deflected { first: bool, rot: f32 },
}
/// `catchingFish`'s wait for a fish: vanilla's 100 to 600 ticks, or a medium's
/// own `[min, max]` with `min` at least 1 and `max` at least `min`, less the
/// rod's Lure. Not clamped: a wait of 0 or less goes on to the hook phase.
pub(in crate::game::server_side) fn lure_wait(
    random: &mut LegacyRandom,
    range: Option<[i32; 2]>,
    lure: i32,
) -> i32 {
    let (min, span) = match range {
        None => (100, 501),
        Some([min, max]) => {
            let min = min.max(1);
            // `max - min + 1` fits: `min` is at least 1.
            (min, max.max(min) - min + 1)
        }
    };
    // `min + draw` is at most `max`.
    (min + random.next_int_bound(span).unwrap_or(0)).saturating_sub(lure)
}
/// Mods' table ids already warned about, so each malformed or missing one warns once.
pub static WARNED_TABLES: std::sync::Mutex<Vec<(&'static str, String)>> =
    std::sync::Mutex::new(Vec::new());
/// A player a hook can catch (`canHitEntity`): their entity id, which
/// guest they are (none: the host) and their box.
pub(in crate::game) struct HookTarget {
    pub(in crate::game) id: u32,
    pub(in crate::game) bounds: Aabb,
}
#[cfg(test)]
pub(in crate::game::server_side) fn segment_entry(
    start: Vec3,
    end: Vec3,
    bounds: Aabb,
) -> Option<f64> {
    let min = bounds.min_position();
    let max = bounds.max_position();
    let start = [start.x, start.y, start.z];
    let min = [min.x, min.y, min.z];
    let max = [max.x, max.y, max.z];
    if (0..3).all(|axis| start[axis] >= min[axis] && start[axis] < max[axis]) {
        return None;
    }
    let delta = [end.x - start[0], end.y - start[1], end.z - start[2]];
    let mut entry: f64 = 0.0;
    let mut exit: f64 = 1.0;
    for axis in 0..3 {
        if delta[axis] == 0.0 {
            if start[axis] < min[axis] || start[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let a = (min[axis] - start[axis]) / delta[axis];
        let b = (max[axis] - start[axis]) / delta[axis];
        entry = entry.max(a.min(b));
        exit = exit.min(a.max(b));
        if entry > exit {
            return None;
        }
    }
    (entry <= 1.0 && exit >= 0.0).then_some(entry)
}
/// `ProjectileUtil.computeMargin`: the fishing hook's target-box margin grows
/// to 0.3 blocks over its first eight ticks.
#[cfg(test)]
pub(in crate::game::server_side) fn entity_hit_margin(tick_count: i32) -> f64 {
    f64::from((tick_count - 2).clamp(0, 6)) / 20.0
}
#[cfg(test)]
pub(in crate::game::server_side) fn first_entity_hit(
    start: Vec3,
    end: Vec3,
    margin: f64,
    targets: impl IntoIterator<Item = (u32, Aabb)>,
) -> Option<(u32, Vec3)> {
    let mut hit = None;
    let mut hit_at = f64::INFINITY;
    for (id, target_bounds) in targets {
        if let Some(at) = segment_entry(
            start,
            end,
            target_bounds.inflate(Vec3::new(margin, margin, margin)),
        ) && at < hit_at
        {
            hit = Some((
                id,
                Vec3::new(
                    start.x + (end.x - start.x) * at,
                    start.y + (end.y - start.y) * at,
                    start.z + (end.z - start.z) * at,
                ),
            ));
            hit_at = at;
        }
    }
    hit
}
pub(in crate::game::server_side) fn pull_item(
    item: &mut sunrise_sim::entities::ItemEntity,
    delta: Vec3,
) {
    item.velocity = item.velocity.add_vector(delta);
    // The change is made outside ItemEntity::tick, so its velocity comparison
    // cannot request the immediate motion update for tracking clients.
    item.needs_sync = true;
}
/// `FishingHook.pullEntity`: add a tenth of the vector from the hook to its owner.
pub(in crate::game::server_side) fn pull_hooked(velocity: Vec3, hook: Vec3, owner: Vec3) -> Vec3 {
    Vec3::new(
        velocity.x + (owner.x - hook.x) * 0.1,
        velocity.y + (owner.y - hook.y) * 0.1,
        velocity.z + (owner.z - hook.z) * 0.1,
    )
}
/// Minecarts a hook can catch. `AbstractMinecart.isPickable` is true while the
/// cart exists, and `Projectile.canHitEntity` skips the vehicle the owner rides.
#[cfg(test)]
pub(in crate::game::server_side) fn minecart_hook_targets<'a>(
    carts: impl Iterator<Item = &'a super::minecarts::Minecart> + 'a,
    own_vehicle: Option<u32>,
) -> impl Iterator<Item = (u32, Aabb)> + 'a {
    carts
        .filter(move |c| Some(c.id) != own_vehicle)
        .map(|c| (c.id, c.bounds()))
}

/// `FishingHook`'s constructor and `Projectile.updateRotation` read the current velocity.
pub(in crate::game::server_side) fn hook_rotation(v: Vec3) -> [f32; 2] {
    let atan2 = sunrise_core::geometry::trig::atan2_degrees;
    [atan2(v.x, v.z), atan2(v.y, v.horizontal_distance())]
}

/// `FishingHook` constructor's three independent triangle draws.
pub(in crate::game::server_side) fn cast_velocity(
    y_sin: f32,
    y_cos: f32,
    dir_y: f32,
    random: &mut LegacyRandom,
) -> Vec3 {
    let v = Vec3::new(f64::from(-y_sin), f64::from(dir_y), f64::from(-y_cos));
    let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    Vec3::new(
        v.x * (0.6 / len + random.triangle(0.5, 0.0103365)),
        v.y * (0.6 / len + random.triangle(0.5, 0.0103365)),
        v.z * (0.6 / len + random.triangle(0.5, 0.0103365)),
    )
}
impl crate::game::ServerGame<'_> {
    /// Run real `catchingFish` states for the vanilla differential harness.
    pub(in crate::game::server_side) fn fishing_tick_reference(
        &mut self,
    ) -> Vec<serde_json::Value> {
        self.common.world.weather.raining = false;
        let mut rows = Vec::new();
        for seed in [0_i64, 1, 42, 123456789, -1] {
            for phase in 0..5 {
                let mut hook = Hook::remote(1, Vec3::new(0.5, 400.0, 0.5), [0, 0, 0, 1]);
                hook.random = LegacyRandom::new(seed);
                hook.nibble = if phase == 4 { 2 } else { 0 };
                hook.time_until_lured = if phase == 1 { 2 } else { 0 };
                hook.time_until_hooked = match phase {
                    2 => 3,
                    3 => 1,
                    _ => 0,
                };
                hook.fish_angle = 12.5;
                self.catching_fish(&mut hook, [0, 400, 0]);
                rows.push(serde_json::json!({
                    "seed": seed, "phase": phase, "nibble": hook.nibble,
                    "lured": hook.time_until_lured, "hooked": hook.time_until_hooked,
                    "angle_bits": hook.fish_angle.to_bits() as i32, "next": hook.random.next_long(),
                }));
            }
        }
        rows
    }
    /// Exercise the actual rod-use path with a table without a named sequence.
    pub(in crate::game::server_side) fn fishing_fallback_reference(
        &mut self,
    ) -> Vec<serde_json::Value> {
        let rod = ItemStack {
            item: Arc::from("minecraft:fishing_rod"),
            count: 1,
            damage: 0,
            extra: None,
        };
        let mut rows = Vec::new();
        for seed in [0_i64, 1, 42, 123456789, -1] {
            let position = self
                .common
                .player
                .as_ref()
                .expect("harness player")
                .position;
            let Some(id) = self.server.next_entity.checked_add(1) else {
                break;
            };
            self.server.next_entity = id;
            let mut hook = Hook::remote(id, position, [0, 0, 0, 1]);
            hook.nibble = 10;
            hook.random = LegacyRandom::new(seed);
            self.install_fishing_hook(hook);
            let item_start = self.common.dropped.len();
            let orb_start = self.common.orbs.len();
            self.common.world.random.set_seed(seed);
            self.use_fishing_rod(&rod);
            let items: Vec<_> = self
                .common
                .dropped
                .iter()
                .skip(item_start)
                .map(|item| item.stack.item.to_string())
                .collect();
            let xp: Vec<_> = self
                .common
                .orbs
                .iter()
                .skip(orb_start)
                .map(|orb| orb.value)
                .collect();
            rows.push(serde_json::json!({
                "seed": seed, "level_next": self.common.world.random.next_long(), "items": items, "xp": xp,
            }));
        }
        rows
    }
    /// Exercise `FishingHook.tick` in a still-water block on the server.
    pub(in crate::game::server_side) fn fishing_bobbing_reference(
        &mut self,
    ) -> Vec<serde_json::Value> {
        let old_position = self.common.player.as_ref().unwrap().position;
        let block = [
            old_position.x.floor() as i32,
            300,
            old_position.z.floor() as i32,
        ];
        let old_block = self.common.world.block(block[0], block[1], block[2]);
        let water = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        self.level_set(block, water).unwrap();
        assert_eq!(
            self.common.world.block(block[0], block[1], block[2]),
            water,
            "bobbing fixture water not placed"
        );
        let x = f64::from(block[0]) + 0.5;
        let z = f64::from(block[2]) + 0.5;
        self.common.player.as_mut().unwrap().position = Vec3::new(x, 300.0, z - 2.0);
        let old_offhand = self.common.offhand.replace(ItemStack {
            item: Arc::from("minecraft:fishing_rod"),
            count: 1,
            damage: 0,
            extra: None,
        });
        let old_hook = self.common.hook.extract();
        self.common.world.weather.raining = false;
        let mut rows = Vec::new();
        for seed in [0_i64, 1, 42, 123456789, -1] {
            let Some(id) = self.server.next_entity.checked_add(1) else {
                break;
            };
            self.server.next_entity = id;
            let mut hook = Hook::remote(id, Vec3::new(x, 300.5, z), [0, 0, 0, 1]);
            hook.state = State::Bobbing;
            hook.velocity = Vec3::new(0.1, 0.03, -0.2);
            hook.nibble = 0;
            hook.random = LegacyRandom::new(seed);
            self.install_fishing_hook(hook);
            self.tick_fishing();
            let mut hook = self.common.hook.extract().expect("bobbing hook removed");
            rows.push(serde_json::json!({
                "seed": seed,
                "vx": hook.velocity.x.to_bits() as i64, "vy": hook.velocity.y.to_bits() as i64,
                "vz": hook.velocity.z.to_bits() as i64, "nibble": hook.nibble, "next": hook.random.next_long(),
            }));
            self.common.hook.retire_extracted(hook.id, hook.join);
        }
        // The client must take the same block hit before moving as the server;
        // otherwise it lands farther away and slides back on the next update.
        let stone = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        self.level_set(block, stone).unwrap();
        for (case, motion) in [
            Vec3::new(0.2, -0.4, 0.1),
            Vec3::new(-0.2, -0.4, 0.1),
            Vec3::new(0.0, -0.6, -0.2),
        ]
        .into_iter()
        .enumerate()
        {
            let mut state = (
                Vec3::new(x, 301.1, z),
                motion,
                State::Flying,
                Default::default(),
            );
            let mut random = LegacyRandom::new(0);
            for tick in 0..5 {
                state = self.client_hook_step(1, None, false, state, &mut random);
                rows.push(serde_json::json!({
                    "case": case, "tick": tick,
                    "position": [state.0.x - f64::from(block[0]), state.0.y, state.0.z - f64::from(block[2])],
                    "motion": [state.1.x, state.1.y, state.1.z],
                }));
            }
        }
        self.common.hook.discard();
        if let Some(hook) = old_hook {
            self.common.hook.put_back(hook);
        }
        self.common.offhand = old_offhand;
        self.common.player.as_mut().unwrap().position = old_position;
        self.level_set(block, old_block).unwrap();
        rows
    }
    /// Whether `stack` holds a cast line as a rod does (`fishing_rod_cast`).
    fn casts(&self, stack: &ItemStack) -> bool {
        self.stack_can_perform(
            stack,
            sunrise_gameplay_common::game::item_abilities::ability::FISHING_ROD_CAST,
            self.is_rod(stack),
        )
    }
    /// `FishingHook.shouldStopFishing`: a hand still holds a casting stack.
    pub(in crate::game::server_side) fn holds_rod(&self) -> bool {
        self.common.held().is_some_and(|s| self.casts(s))
            || self.common.offhand.as_ref().is_some_and(|s| self.casts(s))
    }
    /// `FishingRodItem.use`.
    pub fn use_fishing_rod(&mut self, stack: &ItemStack) -> bool {
        if !self.is_rod(stack) {
            return false;
        }
        // The mods hear a server player's cast or reel-in first.
        if self.common.player.is_some()
            && let (Some(uuid), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            )
        {
            let state = match self.common.hook.as_ref() {
                None => 0,
                Some(h) if matches!(h.state, State::Hooked(_)) => 4,
                Some(h) if h.nibble > 0 => 2,
                Some(_) => 3,
            };
            let mut cancel = false;
            self.native_player_fish(uuid, &name, state, &mut cancel);
            if cancel {
                return true;
            }
        }
        let Some((pos, eye_height)) = self
            .common
            .player
            .as_ref()
            .map(|p| (p.position, p.dimensions.eye_height))
        else {
            return false;
        };
        let at = [pos.x, pos.y, pos.z];
        if self.common.hook.is_some() {
            let damage = self.retrieve_hook(stack);
            self.damage_held(damage);
            let pitch = 0.4 / (self.common.world.random.next_float() * 0.4 + 0.8);
            self.server_play_sound("minecraft:entity.fishing_bobber.retrieve", at, 1.0, pitch);
            // A rod that broke during retrieval is now an empty ItemStack,
            // whose use-effects component is absent.
            if let Some(rod) = self.common.held().cloned() {
                self.use_vibration(&rod, "minecraft:item_interact_finish");
            }
        } else {
            let pitch = 0.4 / (self.common.world.random.next_float() * 0.4 + 0.8);
            self.server_play_sound("minecraft:entity.fishing_bobber.throw", at, 0.5, pitch);
            let (yaw, pitch) = (self.common.yaw, self.common.pitch);
            let (y_sin, y_cos, dir_y) = cast_direction(yaw, pitch);
            let eye_y = pos.y + f64::from(eye_height);
            // snapTo(x, getEyeY(), z): the hook's feet start at the owner's eye.
            let start = Vec3::new(
                pos.x - f64::from(y_sin) * 0.3,
                eye_y,
                pos.z - f64::from(y_cos) * 0.3,
            );
            // FishingRodItem samples lure time, then luck, from the fisher's stream.
            let shooter = self.own_entity_id() as u32;
            let lure = self.entity_enchantment_value(
                stack,
                shooter,
                "minecraft:player",
                "minecraft:fishing_time_reduction",
                0.0,
            );
            let lure = ((if lure < 0.0 { 0.0 } else { lure }) * 20.0) as i32;
            let luck = (self.entity_enchantment_value(
                stack,
                shooter,
                "minecraft:player",
                "minecraft:fishing_luck_bonus",
                0.0,
            ) as i32)
                .max(0);
            // The hook's own random (`Entity.random`).
            let mut random = random_source::entity();
            let uuid = sunrise_sim::save::insecure_uuid(&mut random);
            // FishingHook's second random is reseeded from UUID/game time on
            // use, but its construction still takes its own unique seed.
            let _synchronized = random_source::create();
            let v = cast_velocity(y_sin, y_cos, dir_y, &mut random);
            let [hook_yaw, hook_pitch] = hook_rotation(v);
            self.server.next_entity += 1;
            self.install_fishing_hook(Hook {
                uuid,
                attachment_lifetime: Default::default(),
                join: sunrise_sim::join_seq::JoinSeq::UNJOINED,
                caster: None,
                caster_generation: sunrise_sim::join_seq::JoinSeq::UNJOINED,
                id: self.server.next_entity,
                position: start,
                previous: start,
                velocity: v,
                state: State::Flying,
                hooked_in: None,
                left_owner: false,
                yaw: hook_yaw,
                pitch: hook_pitch,
                // Projectile.shoot ends with yRotO = getYRot(), xRotO = getXRot().
                previous_yaw: hook_yaw,
                previous_pitch: hook_pitch,
                last_deflected_by: None,
                needs_sync: false,
                tick_count: 0,
                life: 0,
                ground: Default::default(),
                nibble: 0,
                time_until_lured: 0,
                time_until_hooked: 0,
                fish_angle: 0.0,
                out_of_water: 0,
                open_water: true,
                luck,
                lure,
                medium: None,
                lerp: HookLerp::default(),
                random,
                base: Default::default(),
            });
            // FishingRodItem.use counts a cast, not a reel-in.
            self.award_stat("used", &stack.item, 1);
            self.use_vibration(stack, "minecraft:item_interact_start");
        }
        self.swing_arm();
        true
    }
    /// `FishingHook.retrieve`: the rod damage it costs.
    pub fn retrieve_hook(&mut self, rod: &ItemStack) -> u32 {
        let Some(mut hook) = self.common.hook.extract() else {
            return 0;
        };
        let (id, join) = (hook.id, hook.join);
        let damage = self.retrieve_hook_extracted(rod, &mut hook);
        self.common.hook.retire_extracted(id, join);
        damage
    }
    pub fn retrieve_hook_extracted(&mut self, rod: &ItemStack, hook: &mut Hook) -> u32 {
        let Some(p) = self.common.player.as_ref() else {
            return 0;
        };
        let owner = p.position;
        let mut damage = 0;
        if let Some(id) = hook.hooked_in() {
            // pullEntity: 0.1 of the way to the owner.
            let d = Vec3::new(
                owner.x - hook.position.x,
                owner.y - hook.position.y,
                owner.z - hook.position.z,
            );
            let pulled = |v: Vec3| pull_hooked(v, hook.position, owner);
            let mut hooked_item = false;
            if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                m.velocity = pulled(m.velocity);
            } else if let Some(item) = self.common.dropped.iter_mut().find(|i| i.id == id) {
                pull_item(item, Vec3::new(d.x * 0.1, d.y * 0.1, d.z * 0.1));
                hooked_item = true;
            } else if let Some(t) = self
                .common
                .tnts
                .iter_mut()
                .find(|t| t.id != 0 && t.id == id)
            {
                t.velocity = pulled(t.velocity);
            } else if let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id) {
                a.velocity = pulled(a.velocity);
            } else if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
                self.common.boats.update(i, |b| {
                    b.velocity = pulled(b.velocity);
                });
            } else if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
                // `AbstractMinecart` is pickable, so `pullEntity` moves it too.
                self.common
                    .carts
                    .update(i, |c| c.velocity = pulled(c.velocity));
            } else if let Some(i) = self.common.falling.iter().position(|f| f.id == id) {
                self.common
                    .falling
                    .update(i, |f| f.velocity = pulled(f.velocity));
            } else if self
                .common
                .player
                .as_ref()
                .is_some_and(|pl| pl.entity_id as u32 == id)
            {
                if let Some(pl) = self.common.player.as_mut() {
                    pl.velocity = pulled(pl.velocity);
                }
            } else if self.server.hook_targets.iter().any(|t| t.id == id) {
                // A player: the hub moves them when the tick ends.
                self.server
                    .hook_pulls
                    .push((id, Vec3::new(d.x * 0.1, d.y * 0.1, d.z * 0.1)));
            }
            // CriteriaTriggers.FISHING_ROD_HOOKED for any hooked entity type.
            let hooked = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| self.mob_entity(m))
                .or_else(|| {
                    let kind = if self
                        .common
                        .player
                        .as_ref()
                        .is_some_and(|pl| pl.entity_id as u32 == id)
                    {
                        Some("minecraft:player".to_owned())
                    } else {
                        self.entity_kind_for_who(sunrise_sim::mobs::Who::Mob(id))
                    }?;
                    Some(sunrise_sim::advancements::EntityView {
                        kind,
                        position: [hook.position.x, hook.position.y, hook.position.z],
                        ..Default::default()
                    })
                });
            if let Some(hooked) = hooked {
                self.trigger_fishing_rod_hooked(rod, &hooked, &[]);
            }
            // `hookedIn instanceof ItemEntity ? 3 : 5`.
            damage = if hooked_item { 3 } else { 5 };
        } else if hook.nibble > 0 {
            let biome = self.biome_name_at(hook.position);
            let Some(loot) = self.server.server_loot.0.get() else {
                return damage;
            };
            // `withLuck(this.luck + owner.getLuck())`: the rod's Luck of the Sea
            // plus the owner's luck attribute (the Luck and Bad Luck effects).
            let owner_luck = self
                .common
                .player
                .as_mut()
                .and_then(|p| p.attribute("minecraft:luck").ok())
                .unwrap_or(0.0) as f32;
            // FishingHook.retrieve calls LootTable.getRandomItems without an
            // explicit random source. LootContext uses the table's sequence,
            // or the level random if the table has no sequence.
            let luck = hook.luck as f32 + owner_luck;
            let table = hook
                .medium
                .as_ref()
                .and_then(|m| m.loot_table.as_deref())
                .unwrap_or(FISHING_TABLE);
            // `LootContextParamSets.FISHING`: the hook is THIS_ENTITY, the rod
            // the TOOL, and the hook's position the ORIGIN; its owner is the
            // attacking entity.
            let origin = [hook.position.x, hook.position.y, hook.position.z];
            let params = hook_params(luck, Some(self.killer_loot_entity(None)));
            let caller = self.caller_loot(
                Some(origin),
                Some(sunrise_gameplay_common::game::creatures::to_loot(rod)),
            );
            let mut sequences = std::mem::take(&mut self.server.rand_sequences);
            let mut random = self.common.world.random.clone();
            let items = caller.with(|| {
                let level = crate::game::advancements::View(&*self);
                let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                    sunrise_sim::advancements::location(predicate, at, &level)
                };
                fishing_loot(
                    &loot,
                    table,
                    &mut sequences,
                    &mut random,
                    self.common.level.seed,
                    biome.as_deref(),
                    Some(sunrise_sim::advancements::loot_location(origin, &read)),
                    hook.open_water,
                    &params,
                )
            });
            self.server.rand_sequences = sequences;
            self.common.world.random = random;
            let mut caught: Vec<ItemStack> = items
                .into_iter()
                .map(|stack| {
                    let mut item = ItemStack {
                        item: Arc::from(stack.item.as_str()),
                        count: stack.count.max(1) as u32,
                        damage: stack.damage.max(0) as u32,
                        extra: None,
                    };
                    for (id, level) in &stack.enchantments {
                        if &*item.item == "minecraft:enchanted_book" {
                            sunrise_items::item_data::set_level(
                                &mut item.data_mut().stored_enchantments,
                                id,
                                (*level).max(0) as u32,
                            );
                        } else {
                            item.enchant(id, (*level).max(0) as u32);
                        }
                    }
                    if let Some(potion) = &stack.potion {
                        item = item.with_potion(potion);
                    }
                    item
                })
                .collect();
            // ItemFishedEvent may change the catch, rod damage, or cancel rewards.
            let mut rod_damage = if hook.ground.on_ground { 2 } else { 1 };
            if let Some(player) = self.server.lan_acting_uuid
                && mod_api::__private::listened(mod_api::HookId::ItemFished)
            {
                let fished = mod_api::ItemFished::from(mod_api::__private::built::ItemFished {
                    hook: hook.id,
                    position: [hook.position.x, hook.position.y, hook.position.z],
                });
                let mut cancel = false;
                let entity_id = self.own_entity_id();
                self.native_item_fished(
                    player,
                    entity_id,
                    &fished,
                    &mut caught,
                    &mut rod_damage,
                    &mut cancel,
                );
                if cancel {
                    return rod_damage.max(0) as u32;
                }
            }
            // CriteriaTriggers.FISHING_ROD_HOOKED: the hook itself is the entity.
            let bobber = EntityView {
                kind: "minecraft:fishing_bobber".into(),
                position: [hook.position.x, hook.position.y, hook.position.z],
                on_ground: hook.ground.on_ground,
                fishing_open_water: Some(hook.open_water),
                ..Default::default()
            };
            self.trigger_fishing_rod_hooked(rod, &bobber, &caught);
            for item in caught.iter().cloned() {
                let fish = self
                    .common
                    .res
                    .items()
                    .item_tags
                    .get("minecraft:fishes")
                    .is_some_and(|tags| tags.contains(&*item.item));
                let (dx, dy, dz) = (
                    owner.x - hook.position.x,
                    owner.y - hook.position.y,
                    owner.z - hook.position.z,
                );
                let v = [
                    dx * 0.1,
                    dy * 0.1 + (dx * dx + dy * dy + dz * dz).sqrt().sqrt() * 0.08,
                    dz * 0.1,
                ];
                self.spawn_item(
                    item,
                    [hook.position.x, hook.position.y, hook.position.z],
                    v,
                    0,
                );
                let xp = hook.random.next_int_bound(6).unwrap_or(0) as u32 + 1;
                // Vanilla constructs a single orb directly; ExperienceOrb.award
                // would split it and draw a merge id from the level random.
                self.add_orb([owner.x, owner.y + 0.5, owner.z + 0.5], xp);
                if fish {
                    self.award_custom_stat("fish_caught", 1);
                }
            }
            return rod_damage.max(0) as u32;
        }
        if hook.ground.on_ground {
            damage = 2;
        }
        damage
    }
    /// getOpenWaterTypeForBlock: 0 above water, 1 inside water, 2 invalid.
    pub fn open_water_type(&self, x: i32, y: i32, z: i32) -> u8 {
        let state = self.common.world.block(x, y, z);
        let f = self.common.res.facts().get(state);
        // `!state.isAir()`: cave_air and void_air are air too.
        if self.common.res.facts().is_air(state) || &*f.name == "minecraft:lily_pad" {
            return 0;
        }
        if f.fluid == Fluid::Water
            && f.fluid_level == 8
            && !f.fluid_falling
            && !f.full_collision
            && !f.legacy_solid
        {
            1
        } else {
            2
        }
    }
    /// calculateOpenWater over the 5x4x5 area around the hook.
    pub fn calculate_open_water(&self, b: [i32; 3]) -> bool {
        let mut previous = 2u8;
        for dy in -1..=2 {
            let mut layer = None;
            for dx in -2..=2 {
                for dz in -2..=2 {
                    let t = self.open_water_type(b[0] + dx, b[1] + dy, b[2] + dz);
                    layer = Some(match layer {
                        None => t,
                        Some(l) if l == t => l,
                        Some(_) => 2,
                    });
                }
            }
            let layer = layer.unwrap_or(2);
            match layer {
                0 if previous == 2 => return false,
                1 if previous == 0 => return false,
                2 => return false,
                _ => {}
            }
            previous = layer;
        }
        true
    }
    /// `FishingHook.catchingFish`.
    pub(in crate::game::server_side) fn catching_fish(&mut self, h: &mut Hook, b: [i32; 3]) {
        let mut speed = 1;
        let above = [b[0], b[1] + 1, b[2]];
        let sky = self.common.world.light(above[0], above[1], above[2]) >> 4 == 15;
        // Level.isRainingAt: rain (not snow) reaches the position.
        if h.random.next_float() < 0.25
            && self.with_level(|b, level, _| {
                b.is_raining_at(level, BlockPos::new(above[0], above[1], above[2]))
            })
        {
            speed += 1;
        }
        if h.random.next_float() < 0.5 && !sky {
            speed -= 1;
        }
        let at = [h.position.x, h.position.y, h.position.z];
        if h.nibble > 0 {
            h.nibble -= 1;
            if h.nibble <= 0 {
                h.time_until_lured = 0;
                h.time_until_hooked = 0;
            }
        } else if h.time_until_hooked > 0 {
            h.time_until_hooked -= speed;
            if h.time_until_hooked > 0 {
                // The fish swimming up: a bubble now and then, and a fishing
                // particle to each side of its path.
                h.fish_angle += h.random.triangle(0.0, 9.188) as f32;
                let angle = h.fish_angle * (std::f64::consts::PI / 180.0) as f32;
                let (sin, cos) = (trig::sin(f64::from(angle)), trig::cos(f64::from(angle)));
                let distance = h.time_until_hooked as f32 * 0.1;
                let fish = [
                    h.position.x + f64::from(sin * distance),
                    f64::from(h.position.y.floor() as f32 + 1.0),
                    h.position.z + f64::from(cos * distance),
                ];
                if self.water_block(fish) {
                    if h.random.next_float() < 0.15 {
                        let spread = [f64::from(sin), 0.1, f64::from(cos)];
                        let at = [fish[0], fish[1] - f64::from(0.1f32), fish[2]];
                        self.send_particles(ParticleType::Bubble, at, spread, 0.0, 1);
                    }
                    let (x, z) = (sin * 0.04, cos * 0.04);
                    for side in [1.0f32, -1.0] {
                        let spread = [f64::from(z * side), 0.01, f64::from(-x * side)];
                        self.send_particles(ParticleType::Fishing, fish, spread, 1.0, 0);
                    }
                }
            } else {
                // The mods hear a server player's fish bite, and may call it
                // off: the wait starts over.
                if let (Some(uuid), Some(name)) = (
                    self.server.lan_acting_uuid,
                    self.server.lan_acting_name.clone(),
                ) {
                    let mut cancel = false;
                    self.native_player_fish(uuid, &name, 1, &mut cancel);
                    if cancel {
                        h.time_until_lured = 0;
                        h.time_until_hooked = 0;
                        return;
                    }
                }
                let pitch = 1.0 + (h.random.next_float() - h.random.next_float()) * 0.4;
                self.server_play_sound("minecraft:entity.fishing_bobber.splash", at, 0.25, pitch);
                // The bite: bubbles and fishing particles around the bobber.
                let at = [h.position.x, h.position.y + 0.5, h.position.z];
                let count = (1.0 + SIZE as f32 * 20.0) as u32;
                let spread = [SIZE, 0.0, SIZE];
                for kind in [ParticleType::Bubble, ParticleType::Fishing] {
                    self.send_particles(kind, at, spread, f64::from(0.2f32), count);
                }
                trigger_bite(h, self.common.world.game_time);
            }
        } else if h.time_until_lured > 0 {
            h.time_until_lured -= speed;
            let mut tease = 0.15f32;
            if h.time_until_lured < 20 {
                tease += (20 - h.time_until_lured) as f32 * 0.05;
            } else if h.time_until_lured < 40 {
                tease += (40 - h.time_until_lured) as f32 * 0.02;
            } else if h.time_until_lured < 60 {
                tease += (60 - h.time_until_lured) as f32 * 0.01;
            }
            if h.random.next_float() < tease {
                // A splash where a fish is teasing the bait.
                let angle = (h.random.next_float() * 360.0) * (std::f64::consts::PI / 180.0) as f32;
                let distance = h.random.next_float() * 35.0 + 25.0;
                let fish = [
                    h.position.x + f64::from(trig::sin(f64::from(angle)) * distance) * 0.1,
                    f64::from(h.position.y.floor() as f32 + 1.0),
                    h.position.z + f64::from(trig::cos(f64::from(angle)) * distance) * 0.1,
                ];
                if self.water_block(fish) {
                    let count = 2 + h.random.next_int_bound(2).unwrap_or(0) as u32;
                    let spread = [f64::from(0.1f32), 0.0, f64::from(0.1f32)];
                    self.send_particles(ParticleType::Splash, fish, spread, 0.0, count);
                }
            }
            if h.time_until_lured <= 0 {
                h.fish_angle = h.random.next_float() * 360.0;
                h.time_until_hooked = 20 + h.random.next_int_bound(61).unwrap_or(0);
            }
        } else {
            let range = h.medium.as_ref().and_then(|m| m.lure_ticks);
            h.time_until_lured = lure_wait(&mut h.random, range, h.lure);
        }
    }
    /// `catchingFish`'s check below a fish: the block under it is water.
    pub(in crate::game::server_side) fn water_block(&self, fish: [f64; 3]) -> bool {
        let state = self.common.world.block(
            fish[0].floor() as i32,
            (fish[1] - 1.0).floor() as i32,
            fish[2].floor() as i32,
        );
        &*self.common.res.facts().get(state).name == "minecraft:water"
    }
    /// `FishingHook.tick`.
    pub fn tick_fishing(&mut self) {
        let Some(hook) = self.common.hook.extract() else {
            return;
        };
        let (id, join) = (hook.id, hook.join);
        let from = hook.position;
        match self.tick_fishing_extracted(hook) {
            Some(hook) => {
                let to = hook.position;
                self.common.hook.put_back(hook);
                self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
            }
            None => self.common.hook.retire_extracted(id, join),
        }
    }
    pub(in crate::game::server_side) fn tick_fishing_extracted(
        &mut self,
        mut h: Hook,
    ) -> Option<Hook> {
        h.needs_sync = false;
        let owner = self.common.player.as_ref().map(|p| p.position)?;
        // shouldStopFishing: a rod in either hand, within 32 blocks.
        let d2 = (h.position.x - owner.x).powi(2)
            + (h.position.y - owner.y).powi(2)
            + (h.position.z - owner.z).powi(2);
        if !self.holds_rod() || d2 > 1024.0 {
            return None;
        }
        h.tick_count += 1;
        h.previous_yaw = h.yaw;
        h.previous_pitch = h.pitch;
        // Projectile.tick: update leftOwner using the caster's full root-vehicle tree.
        if !h.left_owner {
            let swept = h
                .bounds()
                .expand_towards(h.velocity)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
            h.left_owner =
                self.outside_owner_collision_range(sunrise_sim::mobs::Who::Player, swept);
        }
        // The players it can catch, as they stand now (the host's own hook;
        // guests' hooks use the ones taken as their turns start).
        if self.server.lan_acting.is_none() && matches!(h.state, State::Flying | State::Hooked(_)) {
            self.refresh_hook_targets();
        }
        h.previous = h.position;
        if h.ground.on_ground {
            h.life += 1;
            if h.life >= 1200 {
                return None;
            }
        } else {
            h.life = 0;
        }
        let b = [
            h.position.x.floor() as i32,
            h.position.y.floor() as i32,
            h.position.z.floor() as i32,
        ];
        let liquid = self
            .common
            .water_height(b[0], b[1], b[2])
            .map_or(0.0, |w| w.0);
        // Out of water, a mod may say the bobber floats here all the same.
        let hooked = matches!(h.state, State::Hooked(_));
        let medium = if liquid > 0.0 || hooked {
            None
        } else {
            self.hook_medium(h.id, h.position, Some(owner), true)
        };
        let surface = medium.as_ref().map(|m| m.answer.surface);
        match medium {
            Some(m) => {
                let loot_table = match h.medium.take() {
                    Some(kept)
                        if kept.mod_id == m.mod_id && kept.answered == m.answer.loot_table =>
                    {
                        kept.loot_table
                    }
                    _ => m
                        .answer
                        .loot_table
                        .as_deref()
                        .and_then(|id| self.medium_table(m.mod_id, id)),
                };
                h.medium = Some(HookMedium {
                    mod_id: m.mod_id,
                    answered: m.answer.loot_table,
                    loot_table,
                    lure_ticks: m.answer.lure_ticks,
                });
            }
            // Out of the medium (in water, or answered None) it rolls vanilla's table again.
            None if !hooked => h.medium = None,
            None => {}
        }
        let in_water = liquid > 0.0 || surface.is_some();
        match h.state {
            State::Flying => {
                // FishingHook.tick: if hookedIn was set on a prior tick, freeze and transition.
                if let Some(id) = h.hooked_in {
                    h.velocity = Vec3::ZERO;
                    h.state = State::Hooked(id);
                    return Some(h);
                }
                if in_water {
                    h.velocity =
                        Vec3::new(h.velocity.x * 0.3, h.velocity.y * 0.2, h.velocity.z * 0.3);
                    h.state = State::Bobbing;
                    return Some(h);
                }
                // checkCollision: find the first block and entity on the movement segment.
                let to = h.position.add_vector(h.velocity);
                let block_hit = self.common.hook_block_hit(h.position, to);
                let entity_end = block_hit.map_or(to, |hit| hit.location);
                let margin = sunrise_sim::projectiles::compute_margin(h.tick_count);
                let search_area = h
                    .bounds()
                    .expand_towards(h.velocity)
                    .inflate(Vec3::new(1.0, 1.0, 1.0));
                let mut best = sunrise_sim::projectiles::NearestHit::new();
                let query = sunrise_gameplay_common::game::creatures::TargetQuery {
                    include_guests: true,
                    include_items: true,
                    ..Default::default()
                };
                self.visit_projectile_targets(query, &[], |c| {
                    if !h.left_owner
                        && self.in_same_vehicle_tree(sunrise_sim::mobs::Who::Player, c.who)
                    {
                        return;
                    }
                    best.consider(c, search_area, h.position, entity_end, margin);
                });
                let hit = best.finish().map(|(who, at)| {
                    let id = match who {
                        sunrise_sim::mobs::Who::Player => self.own_entity_id() as u32,
                        sunrise_sim::mobs::Who::Mob(id) => id,
                    };
                    (id, at)
                });
                let impact = match (hit, block_hit) {
                    (Some((id, at)), _) => sunrise_sim::projectiles::Hit::Entity {
                        who: sunrise_sim::mobs::Who::Mob(id),
                        at,
                    },
                    (None, Some(hit)) => sunrise_sim::projectiles::Hit::Block {
                        pos: [hit.block.x, hit.block.y, hit.block.z],
                        face: hit.direction.normal(),
                        at: hit.location,
                    },
                    (None, None) => sunrise_sim::projectiles::Hit::None,
                };
                let (hook, canceled) = self.fishing_impact(h, impact)?;
                h = hook;
                // A veto skips only hit handling; Entity.move below still
                // stops a flying bobber against solid collision shapes.
                if let Some((id, at)) = hit.filter(|_| !canceled) {
                    if id >= sunrise_gameplay_common::game::dragon_common::SPECIAL {
                        // Vanilla onSyncedDataUpdated resolves level.getEntity(part.id) == null, clearing hookedIn.
                        self.projectile_landed_event(
                            h.id,
                            Some(sunrise_sim::mobs::Who::Player),
                            [at.x, at.y, at.z],
                        );
                    } else if id != 0 {
                        let breeze = self
                            .common
                            .mobs
                            .iter()
                            .find(|m| {
                                m.id == id
                                    && m.alive()
                                    && m.species() == sunrise_sim::mobs::Species::Breeze
                            })
                            .map(|m| [m.position.x, m.position.y, m.position.z]);
                        if breeze.is_none() {
                            self.redirect_projectile(
                                id,
                                Some(sunrise_sim::mobs::Who::Player),
                                self.common.level.player_uuid,
                                self.server.lan_acting,
                                None,
                            );
                        }
                        match hit_entity(&mut h, id, breeze.is_some()) {
                            EntityCollision::Deflected { first, rot } => {
                                if first {
                                    h.previous_yaw += rot;
                                    if let Some(at) = breeze {
                                        self.server_play_sound(
                                            "minecraft:entity.breeze.deflect",
                                            at,
                                            1.0,
                                            1.0,
                                        );
                                    }
                                }
                            }
                            EntityCollision::Hooked => {
                                self.projectile_landed_event(
                                    h.id,
                                    Some(sunrise_sim::mobs::Who::Player),
                                    [at.x, at.y, at.z],
                                );
                            }
                        }
                    }
                } else if let Some(hit) = block_hit.filter(|_| !canceled && hit.is_none()) {
                    let projectile = ProjectileHitter {
                        owner: Some(sunrise_sim::mobs::Who::Player),
                        guest: self.server.lan_acting,
                        arrow: false,
                        kind: "minecraft:fishing_bobber",
                        position: h.position,
                    };
                    let pos = [hit.block.x, hit.block.y, hit.block.z];
                    let face = hit.direction.normal();
                    if let Err(error) =
                        self.projectile_block_impact(&projectile, pos, hit.location, face)
                    {
                        log::error!("fishing block impact: {error}");
                    }
                    self.target_block_hit(&projectile, pos, hit.location, face);
                    // Projectile.onHit on a block: PROJECTILE_LAND at block center with post-callback state.
                    self.projectile_block_landed_event(
                        h.id,
                        Some(sunrise_sim::mobs::Who::Player),
                        pos,
                    );
                    h.velocity = rebound(h.velocity, hit.location, h.position);
                }
            }
            State::Hooked(id) => {
                // updateHookedEntity: ride the hooked mob, player or item at 0.8 of its height.
                let hooked = self
                    .common
                    .mobs
                    .iter()
                    .find(|m| m.id == id && m.alive())
                    .map(|m| (m.position, f64::from(m.height())))
                    .or_else(|| {
                        self.server
                            .hook_targets
                            .iter()
                            .find(|t| t.id == id)
                            .map(|t| {
                                let (lo, hi) = (t.bounds.min_position(), t.bounds.max_position());
                                (
                                    Vec3::new((lo.x + hi.x) / 2.0, lo.y, (lo.z + hi.z) / 2.0),
                                    hi.y - lo.y,
                                )
                            })
                    })
                    .or_else(|| {
                        self.common
                            .armor_stands
                            .iter()
                            .find(|a| a.id == id)
                            .map(|a| {
                                let b = a.bounds();
                                (a.position, b.max_position().y - b.min_position().y)
                            })
                    })
                    .or_else(|| {
                        self.common.boats.iter().find(|b| b.id == id).map(|b| {
                            (
                                b.position,
                                sunrise_gameplay_common::game::boats_common::HEIGHT,
                            )
                        })
                    })
                    .or_else(|| {
                        self.common
                            .carts
                            .iter()
                            .find(|c| c.id == id)
                            .map(|c| (c.position, super::minecarts::HEIGHT))
                    })
                    .or_else(|| {
                        self.common
                            .dropped
                            .iter()
                            .find(|i| i.id == id)
                            .map(|i| (i.position, sunrise_sim::entities::ItemEntity::WIDTH))
                    })
                    .or_else(|| {
                        self.common
                            .tnts
                            .iter()
                            .find(|t| t.id != 0 && t.id == id)
                            .map(|t| {
                                (
                                    t.position,
                                    sunrise_gameplay_common::game::tnt_common::PrimedTnt::SIZE,
                                )
                            })
                    })
                    .or_else(|| {
                        self.common
                            .item_frames
                            .iter()
                            .find(|f| f.id == id)
                            .map(|f| {
                                let (lo, hi) = f.bounds();
                                (
                                    Vec3::new(
                                        (lo[0] + hi[0]) * 0.5,
                                        (lo[1] + hi[1]) * 0.5,
                                        (lo[2] + hi[2]) * 0.5,
                                    ),
                                    f64::from(0.5_f32),
                                )
                            })
                    })
                    .or_else(|| {
                        self.common.paintings.iter().find(|p| p.id == id).map(|p| {
                            let (lo, hi) = p.bounds();
                            (
                                Vec3::new(
                                    (lo[0] + hi[0]) * 0.5,
                                    (lo[1] + hi[1]) * 0.5,
                                    (lo[2] + hi[2]) * 0.5,
                                ),
                                f64::from(0.5_f32),
                            )
                        })
                    })
                    .or_else(|| {
                        self.common
                            .falling
                            .iter()
                            .find(|f| f.id == id)
                            .map(|f| (f.position, f64::from(0.98_f32)))
                    })
                    .or_else(|| {
                        self.common
                            .crystals
                            .iter()
                            .find(|c| c.id != 0 && c.id == id)
                            .map(|c| (c.position, 2.0))
                    })
                    .or_else(|| {
                        self.common
                            .cushions
                            .iter()
                            .find(|c| c.id == id)
                            .map(|c| (c.position, f64::from(0.25_f32)))
                    })
                    .or_else(|| {
                        self.common
                            .resident_entities
                            .knots
                            .iter()
                            .find(|k| k.id == id)
                            .map(|k| {
                                (
                                    sunrise_sim::mobs::leash::knot_state(k.fence).position,
                                    f64::from(0.5_f32),
                                )
                            })
                    })
                    .or_else(|| {
                        self.common
                            .player
                            .as_ref()
                            .filter(|pl| pl.entity_id as u32 == id)
                            .map(|pl| (pl.position, f64::from(pl.dimensions.height)))
                    })
                    .or_else(|| {
                        self.server
                            .others
                            .0
                            .iter()
                            .find(|o| o.id == id && o.kind == "minecraft:leash_knot")
                            .map(|o| (o.position, f64::from(0.5_f32)))
                    });
                match hooked {
                    Some((at, height)) => h.position = Vec3::new(at.x, at.y + height * 0.8, at.z),
                    None => {
                        h.hooked_in = None;
                        h.state = State::Flying;
                    }
                }
                return Some(h);
            }
            State::Bobbing => {
                let v = h.velocity;
                let mut force = match surface {
                    Some(surface) => h.position.y + v.y - surface,
                    None => h.position.y + v.y - f64::from(b[1]) - f64::from(liquid),
                };
                if force.abs() < 0.01 {
                    force += force.signum() * 0.1;
                }
                let r = f64::from(h.random.next_float());
                h.velocity = Vec3::new(v.x * 0.9, v.y - force * r * 0.2, v.z * 0.9);
                if h.nibble <= 0 && h.time_until_hooked <= 0 {
                    h.open_water = true;
                } else {
                    // A medium has no water around it to be open: it counts as open.
                    h.open_water = h.open_water
                        && h.out_of_water < 10
                        && (surface.is_some() || self.calculate_open_water(b));
                }
                if in_water {
                    h.out_of_water = (h.out_of_water - 1).max(0);
                    if h.nibble > 0 {
                        let mut sync_random =
                            synchronized_random(h.uuid, self.common.world.game_time);
                        let dip = -0.1
                            * f64::from(sync_random.next_float())
                            * f64::from(sync_random.next_float());
                        h.velocity.y += dip;
                    }
                    self.catching_fish(&mut h, b);
                } else {
                    h.out_of_water = (h.out_of_water + 1).min(10);
                }
            }
        }
        if !in_water && !h.ground.on_ground && h.hooked_in.is_none() {
            // FishingHook.tick adds all three components, including the horizontal zeroes.
            h.velocity = h
                .velocity
                .add_vector(Vec3::new(0.0, -f64::from(0.03_f32), 0.0));
        }
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        // The bobber's Entity.move goes through the move gate (#2414); PR 1a
        // records the movement unchanged.
        let mut gate = sunrise_sim::move_gate::armed(
            &self.common.loop_context,
            h.id,
            "minecraft:fishing_bobber",
            sunrise_world::entity_move::MoverType::SelfMovement,
            h.velocity,
        );
        let wanted = gate.delta();
        if let Ok(moved) = sunrise_sim::move_gate::scene_resolve(
            &mut gate,
            &mut scene,
            bounds(h.position),
            wanted,
            0.0,
            h.ground.on_ground,
        ) {
            h.position = Vec3::new(
                h.position.x + moved.x,
                h.position.y + moved.y,
                h.position.z + moved.z,
            );
            self.server_move_bobber_response(
                h.id,
                h.position,
                &mut h.ground,
                wanted,
                moved,
                &mut h.velocity,
            );
            // FishingHook.tick turns before grounding or drag zeros/scales motion.
            let [yaw, pitch] = hook_rotation(h.velocity);
            let lerp = sunrise_sim::entities::arrows::lerp_rotation;
            h.yaw = lerp(h.previous_yaw, yaw);
            h.pitch = lerp(h.previous_pitch, pitch);
            if h.state == State::Flying && (h.ground.on_ground || h.ground.horizontal_collision) {
                h.velocity = Vec3::ZERO;
            }
        }
        self.entity_powder_snow_particles(h.previous, h.position, h.bounds(), h.velocity, false);
        h.velocity = Vec3::new(
            h.velocity.x * 0.92,
            h.velocity.y * 0.92,
            h.velocity.z * 0.92,
        );
        Some(h)
    }
    pub(in crate::game) fn server_move_bobber_response(
        &mut self,
        id: u32,
        position: Vec3,
        ground: &mut sunrise_world::ground_support::GroundState,
        wanted: Vec3,
        moved: Vec3,
        velocity: &mut Vec3,
    ) {
        let side = game_side!(self);
        let motion = self.common.body_motion(&side);
        let authoritative =
            motion.context.side() == sunrise_world::native_load_context::LoopSide::Server;
        motion.record_move(
            sunrise_sim::motion::BodyCollision {
                footprint: (position, bounds(position)),
                movement: (wanted, moved),
                authoritative,
                context: sunrise_sim::motion::BodyMotion::entity_context(id, position),
                subject: None,
            },
            ground,
        );
        let effect = motion.legacy_contact(position, ground);
        let entity = (id, "minecraft:fishing_bobber");
        let response = motion.restitute(
            entity,
            sunrise_sim::motion::BodyMove {
                wanted,
                moved,
                effect,
            },
            f64::from(0.03_f32),
            0.98_f32,
            *velocity,
            0.0,
        );
        *velocity = self.finish_body_restitution(entity, position, response);
        let side = game_side!(self);
        let motion = self.common.body_motion(&side);
        motion.apply_block_speed(position, velocity);
        let writes = self.body_step_on(entity, (position, bounds(position)), ground, velocity);
        super::entity::apply_contact_writes(writes, velocity, None, None, None);
    }
    /// The players a hook can catch: everyone but spectators and the dead
    /// (`Projectile.canHitEntity`), the host and the guests in the level.
    pub(in crate::game::server_side) fn refresh_hook_targets(&mut self) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let mut targets = Vec::new();
        for g in &host.hub.guests {
            if host.hub.here(g) && g.loaded && !g.dead && !g.spectator() {
                targets.push(HookTarget {
                    id: g.entity_id as u32,
                    bounds: guest_bounds(g),
                });
            }
        }
        self.server.hook_targets = targets;
    }
    /// `FishingHook.pullEntity` on the players the hooks reeled in this tick
    /// pulled: their velocity moves, and a guest's reaches their client.
    pub(in crate::game::server_side) fn flush_hook_pulls(&mut self) {
        let pulls = std::mem::take(&mut self.server.hook_pulls);
        for (id, delta) in pulls {
            let push = |g: &mut Self| {
                if let Some(p) = g.player.as_mut() {
                    p.velocity = p.velocity.add_vector(delta);
                }
            };
            let conn = self.server.lan.as_ref().and_then(|host| {
                host.hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == id)
                    .map(|g| g.conn)
            });
            if let Some(conn) = conn {
                self.guest_hit_by(conn, push);
            }
        }
    }
}
impl crate::game::ServerGame<'_> {
    /// The cast hooks of the level in place and who cast each: the player in
    /// place, then the guests here. A guest displaced by a nested swap holds
    /// someone else's hook for the moment, so theirs is left out.
    pub(in crate::game::server_side) fn fishing_hooks_here(
        &self,
    ) -> impl Iterator<
        Item = (
            &Hook,
            sunrise_gameplay_common::game::commands_common::Target,
        ),
    > {
        use sunrise_gameplay_common::game::commands_common::Target;
        let own = self.common.hook.iter().map(|hook| (hook, Target::Player));
        let guests = self.server.lan.iter().flat_map(|host| {
            host.hub
                .guests
                .iter()
                .filter(|g| {
                    host.hub.here(g)
                        && self.server.lan_acting != Some(g.conn)
                        && !self.server.lan_guest_swaps.contains(&g.conn)
                })
                .filter_map(|g| Some((host.play.get(&g.conn)?.hook.as_ref()?, g.conn)))
                .map(|(hook, conn)| (hook, Target::Guest(conn)))
        });
        own.chain(guests)
    }
    /// The cast hook of the acting player or a guest.
    pub(in crate::game) fn caster_hook_mut(
        &mut self,
        caster: sunrise_gameplay_common::game::commands_common::Target,
    ) -> Option<&mut Hook> {
        use sunrise_gameplay_common::game::commands_common::Target;
        match caster {
            Target::Guest(conn) if Some(conn) != self.server.lan_acting => self
                .server
                .lan
                .as_mut()
                .and_then(|host| host.play.get_mut(&conn))
                .and_then(|play| play.hook.as_mut()),
            Target::Player | Target::Guest(_) => self.common.hook.as_mut(),
            _ => None,
        }
    }

    /// Update a cast bobber's support in the level that owns the hook.
    pub(in crate::game::server_side) fn set_fishing_hook_ground(
        &mut self,
        id: u32,
        on_ground: bool,
    ) -> Result<bool, mod_api::LevelWriteError> {
        let Some((hook, caster)) = self.fishing_hook_by_id(id) else {
            return Ok(false);
        };
        let (at, bounds, mut ground) = (hook.position, hook.bounds(), hook.ground);
        ground.on_ground = on_ground;
        let mut scene = sunrise_level::collisions::SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        let scan = scene.update_ground_support(
            &mut ground,
            at,
            bounds,
            None,
            sunrise_sim::motion::BodyMotion::entity_context(id, at),
        );
        let Some(hook) = self.caster_hook_mut(caster) else {
            return Ok(false);
        };
        hook.ground = ground;
        scan.map(|()| true)
            .map_err(|_| mod_api::LevelWriteError::Unavailable)
    }

    /// Takes a cast hook out of the level (`Entity.discard`).
    pub fn remove_fishing_hook(&mut self, id: u32) {
        use sunrise_sim::join_seq::residency::{Kind, Source, table::EntityLookup};
        let found = self
            .common
            .resident_entities
            .table
            .borrow_mut()
            .server_entity_lookup(id, |at| self.locate_server_resident_owner(at));
        let EntityLookup::Found { location, .. } = found else {
            return;
        };
        if location.owner.kind != Kind::Hook {
            return;
        }
        let Source::Hook { caster } = location.source else {
            return;
        };
        if caster == self.server.lan_acting {
            self.common.hook.discard();
        } else if let Some(play) =
            caster.and_then(|conn| self.server.lan.as_mut()?.play.get_mut(&conn))
        {
            play.hook.discard();
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// The cast hook with an entity id, and who cast it.
    pub fn fishing_hook_by_id(
        &self,
        id: u32,
    ) -> Option<(
        &Hook,
        sunrise_gameplay_common::game::commands_common::Target,
    )> {
        use sunrise_sim::join_seq::residency::{
            Kind, Source,
            table::{EntityLookup, SourceEntity},
        };
        let found = self
            .common
            .resident_entities
            .table
            .borrow_mut()
            .server_entity_lookup(id, |at| {
                if at.owner.kind != Kind::Hook {
                    return None;
                }
                let source = self.specialized_resident_source(at)?;
                let super::player_residency::SpecializedSource::Hook(hook) = source.value.0 else {
                    return None;
                };
                let Source::Hook { caster } = at.source else {
                    return None;
                };
                Some(SourceEntity {
                    id: hook.id(),
                    value: (
                        hook,
                        caster.map_or(
                            sunrise_gameplay_common::game::commands_common::Target::Player,
                            sunrise_gameplay_common::game::commands_common::Target::Guest,
                        ),
                    ),
                })
            });
        match found {
            EntityLookup::Found { value, .. } => Some(value),
            EntityLookup::Taken | EntityLookup::Missing => None,
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// The hooks guests track: each player's in the level.
    pub(in crate::game::server_side) fn lan_hook_states(&self, out: &mut States) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
            if let Some(h) = host.play.get(&g.conn).and_then(|p| p.hook.as_ref()) {
                track(h, g.entity_id, out);
            }
        }
    }

    /// The table `id` a mod's medium names, as a catch rolls it: None, so vanilla's
    /// table rolls, for an id that is not an identifier or names no table the server
    /// has, with one warning naming the mod and the id.
    pub(in crate::game::server_side) fn medium_table(
        &self,
        mod_id: &'static str,
        id: &str,
    ) -> Option<String> {
        let table = sunrise_core::identifier::Identifier::parse(id)
            .ok()
            .map(|table| table.to_string());
        let Some(loot) = self.server.server_loot.0.get() else {
            // No tables loaded: nothing rolls, so there is nothing to check against.
            return table;
        };
        let malformed = table.is_none();
        if let Some(table) = table.filter(|table| loot.has_table(table)) {
            return Some(table);
        }
        let mut warned = WARNED_TABLES.lock().unwrap_or_else(|e| e.into_inner());
        if !warned.iter().any(|(m, t)| *m == mod_id && t == id) {
            let why = if malformed {
                "not an identifier"
            } else {
                "not loaded"
            };
            log::warn!(
                "mod {mod_id}: fishing_medium names the loot table {id:?}, which is {why}; \
                 the catch rolls {FISHING_TABLE}"
            );
            warned.push((mod_id, id.to_owned()));
        }
        None
    }
}

/// `FishingHook(Player, Level, int, int)`: `(ySin, yCos, y)` of the cast direction, using
/// `Mth.sin`/`Mth.cos` (the 65536-entry table), so straight down gives `xCos == -0.0`, y `-5`.
pub(in crate::game::server_side) fn cast_direction(yaw: f32, pitch: f32) -> (f32, f32, f32) {
    let ya = f64::from(-yaw * trig::DEG_TO_RAD - std::f32::consts::PI);
    let pa = f64::from(-pitch * trig::DEG_TO_RAD);
    let (y_sin, y_cos) = (trig::sin(ya), trig::cos(ya));
    let (x_cos, x_sin) = (-trig::cos(pa), trig::sin(pa));
    (y_sin, y_cos, (-(x_sin / x_cos)).clamp(-5.0, 5.0))
}

fn hit_entity(hook: &mut Hook, id: u32, deflects: bool) -> EntityCollision {
    if deflects {
        let first = hook.last_deflected_by != Some(id);
        let mut rot = 0.0;
        if first {
            rot = 170.0 + hook.random.next_float() * 20.0;
            hook.yaw += rot;
            hook.velocity = hook.velocity.scale(-0.5);
            hook.last_deflected_by = Some(id);
            hook.needs_sync = true;
        }
        return EntityCollision::Deflected { first, rot };
    }
    // FishingHook.onHitEntity sets hookedIn only; state flips to HOOKED_IN_ENTITY next tick.
    hook.hooked_in = Some(id);
    EntityCollision::Hooked
}

/// `FishingHook.catchingFish`: the nibble timer, and DATA_BITING's plunge
/// (`onSyncedDataUpdated` runs on the server too).
fn trigger_bite(hook: &mut Hook, game_time: i64) {
    hook.nibble = 20 + hook.random.next_int_bound(21).unwrap_or(0);
    hook.velocity.y = bite_impulse(&mut synchronized_random(hook.uuid, game_time));
}

/// `FishingHook`'s entity for the players tracking it: the owner's id in
/// its add packet, DATA_HOOKED_ENTITY (the hooked id plus one) and
/// DATA_BITING when they differ from their defaults.
fn track(hook: &Hook, owner: i32, out: &mut States) {
    let p = hook.position;
    let e = out.add(
        hook.id as i32,
        sunrise_sim::lan::playerdata::ints_uuid(hook.uuid),
        "minecraft:fishing_bobber",
    );
    e.position = [p.x, p.y, p.z];
    e.velocity = [hook.velocity.x, hook.velocity.y, hook.velocity.z];
    e.on_ground = hook.ground.on_ground;
    e.needs_sync = hook.needs_sync;
    e.data = owner;
    if let State::Hooked(id) = hook.state {
        e.meta.push((8, serializer::INT, Value::Int(id as i32 + 1)));
    }
    if hook.nibble > 0 {
        e.meta.push((9, serializer::BOOLEAN, Value::Bool(true)));
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{Hook, cast_direction, entity_hit_margin, first_entity_hit, pull_item, rebound};

    /// `fishing_medium`'s surface: a bobber is in the medium below `floor(surface) + 1`, and a
    /// surface that is not finite, or under the bobber, is no medium.
    #[test]
    fn a_medium_holds_the_bobber_up_to_the_top_of_its_surface_block() {
        use sunrise_core::geometry::Vec3;
        let medium = |surface| mod_api::FishingMedium {
            surface,
            ..Default::default()
        };
        let at = |y| Vec3::new(0.5, y, 0.5);
        assert!(super::inside_medium(at(64.9), &medium(64.0)));
        assert!(!super::inside_medium(at(65.0), &medium(64.0)));
        assert!(super::inside_medium(at(-63.5), &medium(-64.0)));
        assert!(!super::inside_medium(at(-62.5), &medium(-64.0)));
        for surface in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -5.0] {
            assert!(
                !super::inside_medium(at(64.0), &medium(surface)),
                "{surface}"
            );
        }
    }

    /// `catchingFish`'s wait in a medium: one `nextInt(max - min + 1)` from `min`, less Lure,
    /// unclamped, and no range panics or overflows.
    #[test]
    fn a_mediums_lure_range_draws_once_and_never_overflows() {
        use sunrise_core::random::{LegacyRandom, RandomSource};
        // java.util.Random(42): nextInt(41) is 14, and the next nextInt(1000) is 763.
        let mut random = LegacyRandom::new(42);
        assert_eq!(super::lure_wait(&mut random, Some([20, 60]), 0), 34);
        assert_eq!(random.next_int_bound(1000).unwrap(), 763, "one draw");
        // Lure III (300 ticks) takes any wait of 20 to 60 below zero, not clamped.
        let mut random = LegacyRandom::new(42);
        assert_eq!(super::lure_wait(&mut random, Some([20, 60]), 300), -266);
        // [0, -5] is [1, 1] and [60, 20] is [60, 60]: the one value, after one draw.
        let mut random = LegacyRandom::new(42);
        assert_eq!(super::lure_wait(&mut random, Some([0, -5]), 0), 1);
        assert_eq!(super::lure_wait(&mut random, Some([60, 20]), 100), -40);
        // java.util.Random(7): nextInt(2147483647) is 1569164236.
        let mut random = LegacyRandom::new(7);
        assert_eq!(
            super::lure_wait(&mut random, Some([1, i32::MAX]), 0),
            1_569_164_237
        );
        let mut random = LegacyRandom::new(7);
        assert_eq!(
            super::lure_wait(&mut random, Some([i32::MAX, i32::MAX]), 300),
            i32::MAX - 300
        );
        assert_eq!(
            super::lure_wait(&mut random, Some([i32::MIN, i32::MIN]), i32::MAX),
            1 - i32::MAX
        );
    }

    /// Compare the production loot selector to a real FishingHook.retrieve oracle.
    #[test]
    #[ignore = "requires prepared resources and the vanilla fishing reference"]
    fn fishing_retrieve_reference() {
        run_fishing_retrieve_reference();
    }

    pub(in crate::game::server_side) fn run_fishing_retrieve_reference() {
        use sunrise_core::random::{LegacyRandom, RandomSource};
        use sunrise_world::loot::{LootData, PackOverlay};
        let resources = std::env::var_os("SUNRISE_FISHING_RESOURCES").unwrap();
        let path = std::env::var_os("SUNRISE_FISHING_REFERENCE").unwrap();
        let rows: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(rows.len(), 5, "missing vanilla retrieval cases");
        let root = std::path::Path::new(&resources);
        let defaults = sunrise_world::item_defaults::read(root).unwrap();
        let loot = LootData::load(root, &PackOverlay::NONE, &defaults).unwrap();
        for row in rows {
            let seed = row["sequence_seed"].as_i64().unwrap();
            let mut sequences = sunrise_items::random_sequences::RandomSequences::default();
            let mut level_random = LegacyRandom::new(1);
            let items = super::fishing_loot(
                &loot,
                super::FISHING_TABLE,
                &mut sequences,
                &mut level_random,
                seed,
                None,
                None,
                true,
                &super::hook_params(0.0, None),
            );
            let drops = row["drops"].as_array().unwrap();
            let expected_items: Vec<_> = drops.iter().filter(|d| d.get("item").is_some()).collect();
            assert_eq!(row["damage"].as_i64(), Some(1), "seed {seed}");
            assert!(
                !expected_items.is_empty(),
                "missing vanilla catch for seed {seed}"
            );
            assert_eq!(items.len(), expected_items.len(), "seed {seed}");
            let mut hook_random = LegacyRandom::new(row["hook_seed"].as_i64().unwrap());
            for (item, expected) in items.iter().zip(expected_items) {
                assert_eq!(item.item, expected["item"].as_str().unwrap(), "seed {seed}");
                assert_eq!(i64::from(item.count), expected["count"].as_i64().unwrap());
                let xp = hook_random.next_int_bound(6).unwrap() + 1;
                assert!(
                    drops
                        .iter()
                        .any(|d| d["xp"].as_i64() == Some(i64::from(xp)))
                );
            }
            assert_eq!(
                hook_random.next_long(),
                row["hook_next"].as_i64().unwrap(),
                "seed {seed}"
            );
            assert_eq!(
                sequences
                    .get("minecraft:gameplay/fishing", seed)
                    .next_long(),
                row["sequence_next"].as_i64().unwrap(),
                "seed {seed}"
            );
        }
        let casts: Vec<serde_json::Value> = serde_json::from_slice(
            &std::fs::read(std::env::var_os("SUNRISE_FISHING_CAST_REFERENCE").unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(casts.len(), 5, "missing vanilla cast cases");
        for row in casts {
            let mut random = LegacyRandom::new(row["seed"].as_i64().unwrap());
            let (sin, cos, vertical) = super::cast_direction(
                row["yaw"].as_f64().unwrap() as f32,
                row["pitch"].as_f64().unwrap() as f32,
            );
            let velocity = super::cast_velocity(sin, cos, vertical, &mut random);
            for (name, value) in [
                ("x_bits", velocity.x),
                ("y_bits", velocity.y),
                ("z_bits", velocity.z),
            ] {
                assert_eq!(value.to_bits() as i64, row[name].as_i64().unwrap(), "{row}");
            }
            assert_eq!(random.next_long(), row["next"].as_i64().unwrap(), "{row}");
        }
    }
    #[test]
    fn a_breeze_deflects_the_hook_without_attaching_it() {
        use super::{EntityCollision, Hook, State};
        let mut hook = Hook::remote(1, Vec3::ZERO, [0, 0, 0, 1]);
        hook.velocity = Vec3::new(2.0, -0.4, 1.0);
        assert!(matches!(
            super::hit_entity(&mut hook, 10, true),
            EntityCollision::Deflected { first: true, rot } if (170.0..190.0).contains(&rot)
        ));
        assert_eq!(hook.state, State::Flying);
        assert!(hook.needs_sync);
        assert_eq!(hook.velocity, Vec3::new(-1.0, 0.2, -0.5));
        assert_eq!(
            super::hit_entity(&mut hook, 10, true),
            EntityCollision::Deflected {
                first: false,
                rot: 0.0,
            }
        );
        assert_eq!(hook.velocity, Vec3::new(-1.0, 0.2, -0.5));
        assert!(matches!(
            super::hit_entity(&mut hook, 11, true),
            EntityCollision::Deflected { first: true, rot } if (170.0..190.0).contains(&rot)
        ));
        assert_eq!(hook.velocity, Vec3::new(0.5, -0.1, 0.25));
    }

    #[test]
    fn ordinary_targets_still_hook_and_a_deflection_is_tracked_immediately() {
        use super::{EntityCollision, Hook, State};
        let mut hook = Hook::remote(1, Vec3::ZERO, [0, 0, 0, 1]);
        hook.velocity = Vec3::new(2.0, 0.0, 0.0);
        super::hit_entity(&mut hook, 10, true);
        let mut states = sunrise_sim::lan::entity::States::default();
        super::track(&hook, 99, &mut states);
        let state = &states.as_slice()[0];
        assert!(state.needs_sync);
        assert_eq!(state.velocity, [-1.0, -0.0, -0.0]);
        assert!(!state.meta.iter().any(|entry| entry.0 == 8));
        assert_eq!(
            super::hit_entity(&mut hook, 20, false),
            EntityCollision::Hooked
        );
        assert_eq!(hook.hooked_in, Some(20));
        assert_eq!(hook.state, State::Flying);
        assert_eq!(hook.velocity, Vec3::new(-1.0, -0.0, -0.0));
    }

    #[test]
    #[ignore = "run by tools/test_breeze_hook_parity.py"]
    fn breeze_hook_parity_fixture() {
        run_breeze_hook_parity_fixture();
    }

    pub(in crate::game::server_side) fn run_breeze_hook_parity_fixture() {
        use super::{EntityCollision, Hook};
        use std::io::Write;
        let input =
            std::fs::read_to_string(std::env::var("SUNRISE_BREEZE_HOOK_INPUT").unwrap()).unwrap();
        let mut out =
            std::fs::File::create(std::env::var("SUNRISE_BREEZE_HOOK_OUTPUT").unwrap()).unwrap();
        for line in input.lines() {
            let parts: Vec<_> = line.split_whitespace().collect();
            let mut hook = Hook::remote(1, Vec3::ZERO, [0, 0, 0, 1]);
            hook.velocity = Vec3::new(
                parts[1].parse().unwrap(),
                parts[2].parse().unwrap(),
                parts[3].parse().unwrap(),
            );
            let mut trace = Vec::new();
            for target in &parts[4..] {
                hook.needs_sync = false;
                let response =
                    super::hit_entity(&mut hook, target.parse::<u32>().unwrap() + 10, true);
                let sounds = i32::from(matches!(
                    response,
                    EntityCollision::Deflected { first: true, .. }
                ));
                let landed = i32::from(matches!(response, EntityCollision::Hooked));
                trace.push(format!(
                    "{:016x},{:016x},{:016x}:{sounds}:{landed}:{}:{}",
                    hook.velocity.x.to_bits(),
                    hook.velocity.y.to_bits(),
                    hook.velocity.z.to_bits(),
                    i32::from(hook.needs_sync),
                    i32::from(hook.hooked_in().is_some())
                ));
            }
            writeln!(out, "{}", trace.join(" ")).unwrap();
        }
    }

    use sunrise_core::geometry::{Aabb, Vec3};
    use sunrise_items::items::ItemStack;
    use sunrise_sim::entities::ItemEntity;

    #[test]
    fn casting_straight_down_aims_down_and_straight_up_aims_up() {
        assert_eq!(cast_direction(0.0, 90.0).2, -5.0);
        assert_eq!(cast_direction(123.0, 89.9).2, -5.0);
        assert_eq!(cast_direction(0.0, -90.0).2, 5.0);
        assert_eq!(cast_direction(0.0, 0.0).2, 0.0);
    }

    #[test]
    fn cast_direction_uses_the_vanilla_sine_table() {
        // pitch 45: y = -(sin(-a) / -cos(-a)) with table entries built independently.
        let a = f64::from(-45.0_f32 * std::f32::consts::PI / 180.0);
        let table = |i: i64| ((i & 65535) as f64 / 10430.378350470453).sin() as f32;
        let (s, c) = (
            table((a * 10430.378350470453) as i64),
            -table((a * 10430.378350470453 + 16384.0) as i64),
        );
        let (_, _, y) = cast_direction(0.0, 45.0);
        assert!((y - (-(s / c))).abs() < 1e-3, "{y}");
    }

    /// `HitResult.distanceTo` is squared: a hit 0.5 away rebounds at 0.25.
    #[test]
    fn a_block_hit_rebounds_by_the_squared_distance() {
        let v = Vec3::new(0.0, 0.0, -2.0);
        let hit = Vec3::new(0.0, 0.0, -0.5);
        assert_eq!(rebound(v, hit, Vec3::ZERO), Vec3::new(0.0, 0.0, -0.25));
    }

    #[test]
    fn reeling_an_item_requests_an_immediate_motion_update() {
        let mut item = ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            health: 5,
            fire_ticks: 0,
            id: 1,
            stack: ItemStack::new("minecraft:stone", 1),
            thrower: None,
            target: None,
            position: Vec3::ZERO,
            previous: Vec3::ZERO,
            velocity: Vec3::new(0.2, 0.0, -0.1),
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            vertical_collision: false,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        };

        pull_item(&mut item, Vec3::new(0.5, 0.1, -0.2));

        assert!((item.velocity.x - 0.7).abs() < 1.0e-12);
        assert!((item.velocity.y - 0.1).abs() < 1.0e-12);
        assert!((item.velocity.z + 0.3).abs() < 1.0e-12);
        assert!(item.needs_sync);
    }

    #[test]
    fn a_flying_hook_hits_a_small_target_between_ticks() {
        let target = Aabb::new(Vec3::new(0.45, -0.1, -0.1), Vec3::new(0.7, 0.15, 0.1));

        assert!(super::segment_entry(Vec3::ZERO, Vec3::new(0.8, 0.0, 0.0), target).is_some());
        assert!(super::segment_entry(Vec3::ZERO, Vec3::new(0.4, 0.0, 0.0), target).is_none());
    }

    #[test]
    fn a_flying_hook_uses_vanillas_growing_entity_hit_margin() {
        let start = Vec3::ZERO;
        let end = Vec3::new(1.0, 0.0, 0.0);
        let target = Aabb::new(Vec3::new(0.5, 0.31, -0.1), Vec3::new(0.7, 0.5, 0.1));

        assert_eq!(entity_hit_margin(2), 0.0);
        assert_eq!(entity_hit_margin(8), 0.3);
        assert_eq!(
            first_entity_hit(start, end, entity_hit_margin(8), [(1, target)]).map(|(id, _)| id),
            None
        );
        let within_margin = Aabb::new(Vec3::new(0.5, 0.29, -0.1), Vec3::new(0.7, 0.5, 0.1));
        assert_eq!(
            first_entity_hit(start, end, entity_hit_margin(8), [(1, within_margin)])
                .map(|(id, _)| id),
            Some(1)
        );
    }

    #[test]
    fn a_flying_hook_does_not_hit_a_target_it_starts_inside() {
        let target = Aabb::new(Vec3::new(-0.1, -0.1, -0.1), Vec3::new(0.7, 0.15, 0.1));

        assert_eq!(
            first_entity_hit(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 0.0, [(1, target)])
                .map(|(id, _)| id),
            None
        );
    }

    #[test]
    fn a_flying_hook_uses_the_first_target_along_its_path() {
        let start = Vec3::ZERO;
        let end = Vec3::new(1.0, 0.0, 0.0);
        let near = Aabb::new(Vec3::new(0.45, -0.1, -0.1), Vec3::new(0.7, 0.15, 0.1));
        let far = Aabb::new(Vec3::new(0.8, -0.1, -0.1), Vec3::new(0.95, 0.15, 0.1));

        assert_eq!(
            first_entity_hit(start, end, 0.0, [(2, far), (1, near)]).map(|(id, _)| id),
            Some(1)
        );
    }

    #[test]
    fn bite_triggers_initial_downward_plunge_impulse() {
        for _ in 0..100 {
            let mut hook = Hook::remote(1, Vec3::ZERO, [0, 0, 0, 1]);
            assert_eq!(hook.velocity.y, 0.0);
            super::trigger_bite(&mut hook, 0);
            assert!(
                hook.velocity.y < 0.0,
                "vertical velocity should be negative on bite, got {}",
                hook.velocity.y
            );
            assert!(
                hook.velocity.y <= -0.24 && hook.velocity.y >= -0.40,
                "vertical velocity {} not between -0.24 and -0.40",
                hook.velocity.y
            );
            assert!(
                hook.nibble >= 20 && hook.nibble <= 40,
                "nibble {} not in 20..=40",
                hook.nibble
            );
        }
    }

    /// A minecart is pickable (`AbstractMinecart.isPickable`), so the hook
    /// meets it. The cart the owner rides is the same vehicle and is skipped.
    #[test]
    fn a_fishing_hook_meets_a_minecart_but_not_the_one_it_rides() {
        use super::super::minecarts::{CartKind, Minecart};
        let cart = Minecart::new(7, CartKind::Rideable, Vec3::new(0.6, 0.0, 0.0));
        let ridden = Minecart::new(3, CartKind::Chest, Vec3::new(0.2, 0.0, 0.0));
        let boxes = super::minecart_hook_targets([&cart, &ridden].into_iter(), Some(3));
        assert_eq!(
            first_entity_hit(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 0.0, boxes).map(|(id, _)| id),
            Some(7)
        );
    }

    /// `FishingHook.pullEntity` scales the hook-to-owner vector by 0.1.
    #[test]
    fn reeling_pulls_a_hooked_minecart_a_tenth_of_the_way() {
        // Still cart at the origin, owner 10 east and 2 up: +1 x and +0.2 y,
        // on top of the cart's own 0.5.
        let pulled = super::pull_hooked(
            Vec3::new(0.5, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::new(10.0, 2.0, 0.0),
        );
        assert_eq!(pulled, Vec3::new(1.5, 0.2, 0.0));
    }
}
