//! Portable server-side block-entity tick state (bells, conduits, end gateways, shulker boxes).
use crate::fast_hash::FxHashMap;
use crate::shulker_box::{Shulker, direction_index, from_3d_data_value};
use sunrise_core::random::RandomSource;

/// What the bell's tick asks about an entity it gathered when rung.
#[derive(Clone, Copy, Debug)]
pub struct BellEntity {
    pub position: [f64; 3],
    /// `isAlive() && !isRemoved()`.
    pub alive: bool,
    /// In `EntityTypeTags.RAIDERS`.
    pub raider: bool,
}

/// `BellBlockEntity`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bell {
    pub ticks: i32,
    pub shaking: bool,
    /// `clickDirection` as a 3D data value.
    pub click_direction: Option<u8>,
    pub last_ring_timestamp: i64,
    /// `nearbyEntities`: the living entities around the bell when it was last
    /// gathered, as caller-defined handles.
    pub nearby: Option<Vec<u64>>,
    pub resonating: bool,
    pub resonation_ticks: i32,
}

/// Side effects of one [`Bell::tick`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BellTick {
    /// Resonation started: `BELL_RESONATE` (played by the server).
    pub resonate: bool,
    /// Resonation ended (`ResonationEndAction`): the client's
    /// `showBellParticles` entity-effect particles (position and colour) and
    /// the server's `makeRaidersGlow` targets.
    pub ended: Option<(Vec<([f64; 3], i32)>, Vec<u64>)>,
}

/// `Vec3i.closerToCenterThan`: `p` is within `distance` of the block's centre.
pub fn closer_to_center_than(pos: [i32; 3], p: [f64; 3], distance: f64) -> bool {
    let dx = f64::from(pos[0]) + 0.5 - p[0];
    let dy = f64::from(pos[1]) + 0.5 - p[1];
    let dz = f64::from(pos[2]) + 0.5 - p[2];
    dx * dx + dy * dy + dz * dz < distance * distance
}

impl Bell {
    /// `triggerEvent(1, direction)`. `gather` lists the living entities whose
    /// position is inside the bell's block box inflated by 48
    /// (`updateEntities`, refreshed at most every 60 ticks).
    pub fn trigger_event(
        &mut self,
        b0: i32,
        b1: i32,
        game_time: i64,
        gather: impl FnOnce() -> Vec<u64>,
    ) -> bool {
        if b0 != 1 {
            return false;
        }
        if game_time > self.last_ring_timestamp + 60 || self.nearby.is_none() {
            self.last_ring_timestamp = game_time;
            self.nearby = Some(gather());
        }
        self.resonation_ticks = 0;
        self.click_direction = Some(from_3d_data_value(b1));
        self.ticks = 0;
        self.shaking = true;
        true
    }
    /// `BellBlockEntity.tick` with the client's resonation end action.
    pub fn tick(&mut self, pos: [i32; 3], entity: impl Fn(u64) -> Option<BellEntity>) -> BellTick {
        let mut out = BellTick::default();
        if self.shaking {
            self.ticks += 1;
        }
        if self.ticks >= 50 {
            self.shaking = false;
            self.ticks = 0;
        }
        let nearby = self.nearby.clone().unwrap_or_default();
        let raider_in = |e: &BellEntity, distance: f64| {
            e.alive && closer_to_center_than(pos, e.position, distance) && e.raider
        };
        if self.ticks >= 5
            && self.resonation_ticks == 0
            && nearby
                .iter()
                .filter_map(|&h| entity(h))
                .any(|e| raider_in(&e, 32.0))
        {
            self.resonating = true;
            out.resonate = true;
        }
        if self.resonating {
            if self.resonation_ticks < 40 {
                self.resonation_ticks += 1;
            } else {
                // showBellParticles / makeRaidersGlow.
                let mut color = 16_700_985;
                let views: Vec<(u64, BellEntity)> = nearby
                    .iter()
                    .filter_map(|&h| Some((h, entity(h)?)))
                    .collect();
                let count = views
                    .iter()
                    .filter(|(_, e)| closer_to_center_than(pos, e.position, 48.0))
                    .count() as i32;
                let per = ((count - 21) / -2).clamp(3, 15);
                let mut particles = Vec::new();
                let mut glow = Vec::new();
                for (h, e) in &views {
                    if !raider_in(e, 48.0) {
                        continue;
                    }
                    glow.push(*h);
                    let (bx, bz) = (f64::from(pos[0]), f64::from(pos[2]));
                    let dist = ((e.position[0] - bx) * (e.position[0] - bx)
                        + (e.position[2] - bz) * (e.position[2] - bz))
                        .sqrt();
                    let x3 = bx + f64::from(0.5f32) + 1.0 / dist * (e.position[0] - bx);
                    let z3 = bz + f64::from(0.5f32) + 1.0 / dist * (e.position[2] - bz);
                    for _ in 0..per {
                        color += 5;
                        particles.push(([x3, f64::from(pos[1]) + 0.5, z3], color));
                    }
                }
                out.ended = Some((particles, glow));
                self.resonating = false;
            }
        }
        out
    }
}

/// `ConduitBlockEntity.updateShape`: water all around, then the frame blocks
/// (`BlockTags.CONDUIT_EFFECT_BLOCK`) in the three 5x5 rings.
pub fn update_conduit_shape(
    effect_blocks: &mut Vec<[i32; 3]>,
    pos: [i32; 3],
    is_water: &impl Fn([i32; 3]) -> bool,
    is_frame: &impl Fn([i32; 3]) -> bool,
) -> bool {
    effect_blocks.clear();
    for ox in -1..=1 {
        for oy in -1..=1 {
            for oz in -1..=1 {
                if !is_water([pos[0] + ox, pos[1] + oy, pos[2] + oz]) {
                    return false;
                }
            }
        }
    }
    for ox in -2i32..=2 {
        for oy in -2i32..=2 {
            for oz in -2i32..=2 {
                let (ax, ay, az) = (ox.abs(), oy.abs(), oz.abs());
                if (ax > 1 || ay > 1 || az > 1)
                    && (ox == 0 && (ay == 2 || az == 2)
                        || oy == 0 && (ax == 2 || az == 2)
                        || oz == 0 && (ax == 2 || ay == 2))
                {
                    let p = [pos[0] + ox, pos[1] + oy, pos[2] + oz];
                    if is_frame(p) {
                        effect_blocks.push(p);
                    }
                }
            }
        }
    }
    effect_blocks.len() >= 16
}

/// What `ConduitBlockEntity.serverTick`'s 40-tick check decided.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConduitCheck {
    /// The shape changed: `CONDUIT_ACTIVATE` (true) or `CONDUIT_DEACTIVATE`.
    pub toggled: Option<bool>,
    /// The frame blocks counted, while the conduit is active: `applyEffects`
    /// and `updateAndAttackTarget` follow (the attack needs 42 or more).
    pub frames: Option<usize>,
}

/// `ConduitBlockEntity` (server side): what `serverTick` keeps between ticks.
#[derive(Clone, Debug, Default)]
pub struct ConduitServer {
    is_active: bool,
    effect_blocks: Vec<[i32; 3]>,
    /// `destroyTarget`: the mob id of the enemy being struck.
    pub target: Option<u32>,
    next_ambient_sound: i64,
}

impl ConduitServer {
    /// The `gameTime % 40` part of `serverTick`: the shape is checked again.
    pub fn check(
        &mut self,
        pos: [i32; 3],
        game_time: i64,
        is_water: impl Fn([i32; 3]) -> bool,
        is_frame: impl Fn([i32; 3]) -> bool,
    ) -> Option<ConduitCheck> {
        if game_time % 40 != 0 {
            return None;
        }
        let active = update_conduit_shape(&mut self.effect_blocks, pos, &is_water, &is_frame);
        let toggled = (active != self.is_active).then_some(active);
        self.is_active = active;
        Some(ConduitCheck {
            toggled,
            frames: active.then_some(self.effect_blocks.len()),
        })
    }
    /// The sounds of an active conduit at the end of `serverTick`: whether
    /// `CONDUIT_AMBIENT` (every 80 ticks) and `CONDUIT_AMBIENT_SHORT` (every
    /// 60 to 99 ticks) play.
    pub fn ambient(&mut self, game_time: i64, random: &mut impl RandomSource) -> (bool, bool) {
        if !self.is_active {
            return (false, false);
        }
        let short = game_time > self.next_ambient_sound;
        if short {
            self.next_ambient_sound =
                game_time + 60 + i64::from(random.next_int_bound(40).unwrap_or(0));
        }
        (game_time % 80 == 0, short)
    }
}

/// `TheEndGatewayBlockEntity` fields the beam reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EndGateway {
    pub age: i64,
    pub teleport_cooldown: i32,
}
impl EndGateway {
    /// `beamAnimationTick` (the client ticker).
    pub fn tick(&mut self) {
        self.age = self.age.wrapping_add(1);
        if self.teleport_cooldown > 0 {
            self.teleport_cooldown -= 1;
        }
    }
    /// `portalTick` on the server copy: whether it called `triggerCooldown`,
    /// whose block event 1 reaches the client copy.
    pub fn portal_tick(&mut self) -> bool {
        let cooling = self.teleport_cooldown > 0;
        self.age = self.age.wrapping_add(1);
        if cooling {
            self.teleport_cooldown -= 1;
            false
        } else if self.age % 2400 == 0 {
            self.teleport_cooldown = 40;
            true
        } else {
            false
        }
    }
    /// `EndGatewayBlock.entityInside` on the server copy: an entity that can
    /// use the portal starts the cooldown unless it is already running.
    pub fn enter(&mut self) -> bool {
        if self.teleport_cooldown > 0 {
            return false;
        }
        self.teleport_cooldown = 40;
        true
    }
    /// `triggerEvent`: event 1 starts the 40 tick cooldown.
    pub fn trigger_event(&mut self, event: i32, _data: i32) -> bool {
        if event == 1 {
            self.teleport_cooldown = 40;
            true
        } else {
            false
        }
    }
}

/// The client ticker a block entity runs (`getTicker` on a client level),
/// or [`Ticker::Pot`] for decorated pots, which keep wobble state unticked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ticker {
    /// `ChestBlockEntity` / `EnderChestBlockEntity.lidAnimateTick`.
    Lid,
    /// `EnchantingTableBlockEntity.bookAnimationTick`.
    Book,
    /// `ShulkerBoxBlockEntity.tick`, with the box's facing.
    Shulker {
        facing: u8,
    },
    /// `BellBlockEntity.clientTick`.
    Bell,
    /// `ConduitBlockEntity.clientTick`.
    Conduit,
    /// `SkullBlockEntity.animation` (dragon and piglin heads only).
    Skull {
        powered: bool,
    },
    Pot,
    /// `TheEndGatewayBlockEntity.beamAnimationTick`.
    Gateway,
}

/// The [`Ticker`] of a block state, by block name and property lookup.
pub fn ticker(name: &str, prop: impl Fn(&str) -> Option<String>) -> Option<Ticker> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let unwaxed = name.strip_prefix("waxed_").unwrap_or(name);
    Some(match unwaxed {
        "chest"
        | "trapped_chest"
        | "ender_chest"
        | "copper_chest"
        | "exposed_copper_chest"
        | "weathered_copper_chest"
        | "oxidized_copper_chest" => Ticker::Lid,
        "enchanting_table" => Ticker::Book,
        "bell" => Ticker::Bell,
        "conduit" => Ticker::Conduit,
        "decorated_pot" => Ticker::Pot,
        "end_gateway" => Ticker::Gateway,
        "dragon_head" | "dragon_wall_head" | "piglin_head" | "piglin_wall_head" => Ticker::Skull {
            powered: prop("powered").as_deref() == Some("true"),
        },
        n if n.ends_with("shulker_box") => Ticker::Shulker {
            facing: prop("facing")
                .as_deref()
                .and_then(direction_index)
                .unwrap_or(1),
        },
        _ => return None,
    })
}

/// Classifies every state in `registry` into its [`Ticker`].
pub fn classify_tickers(registry: &sunrise_world::registry::BlockRegistry) -> Vec<Option<Ticker>> {
    (0..registry.len() as u32)
        .map(|id| {
            registry
                .state(sunrise_world::registry::BlockStateId(id))
                .and_then(|s| ticker(&s.name, |k| s.properties.get(k).cloned()))
        })
        .collect()
}

/// Server-ticked block-entity state for a level, by position.
#[derive(Clone, Debug, Default)]
pub struct BlockEntityTicks {
    pub shulkers: FxHashMap<[i32; 3], Shulker>,
    pub bells: FxHashMap<[i32; 3], Bell>,
    pub gateway_servers: FxHashMap<[i32; 3], EndGateway>,
    pub conduit_servers: FxHashMap<[i32; 3], ConduitServer>,
    pub kinds: Vec<Option<Ticker>>,
}

impl BlockEntityTicks {
    /// Fills [`Self::kinds`] for every state of the registry.
    pub fn classify(&mut self, registry: &sunrise_world::registry::BlockRegistry) {
        self.kinds = classify_tickers(registry);
    }
    /// The ticker of a block state.
    pub fn ticker(&self, state: u32) -> Option<Ticker> {
        self.kinds.get(state as usize).copied().flatten()
    }
    /// Drops the state of positions whose block entity is gone.
    pub fn remove_position(&mut self, pos: [i32; 3]) {
        self.shulkers.remove(&pos);
        self.bells.remove(&pos);
        self.gateway_servers.remove(&pos);
        self.conduit_servers.remove(&pos);
    }
    pub fn retain(&mut self, keep: impl Fn([i32; 3]) -> bool) {
        self.shulkers.retain(|p, _| keep(*p));
        self.bells.retain(|p, _| keep(*p));
        self.gateway_servers.retain(|p, _| keep(*p));
        self.conduit_servers.retain(|p, _| keep(*p));
    }
    /// Swaps only the per-level position maps, keeping the registry-wide `kinds` table intact.
    pub fn swap_level(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.shulkers, &mut other.shulkers);
        std::mem::swap(&mut self.bells, &mut other.bells);
        std::mem::swap(&mut self.gateway_servers, &mut other.gateway_servers);
        std::mem::swap(&mut self.conduit_servers, &mut other.conduit_servers);
    }
}
