//! Common Ender Dragon helpers shared by client rendering/picking and server simulation.
use super::view_vector;
use crate::game::potions_common::{PotionCloud, Victim};
use mod_api::DragonPhase;
use sunrise_core::geometry::{Aabb, Vec3, trig};
use sunrise_core::{
    nbt::Tag,
    random::{LegacyRandom, RandomSource},
};
use sunrise_game_data::nbt_value;
use sunrise_level::random_source;

/// The part index (`0..8`) of `candidate_id` under `dragon_id`, if it is one of
/// the eight `EnderDragonPart` sub-entities (`dragon_id + 1..=dragon_id + 8`).
pub fn dragon_part_index(dragon_id: u32, candidate_id: u32) -> Option<usize> {
    let offset = candidate_id.wrapping_sub(dragon_id);
    (1..=8).contains(&offset).then(|| (offset - 1) as usize)
}

impl crate::game::CommonGame<'_> {
    /// The boxes of the living dragon's parts a projectile can hit, in `getSubEntities` order.
    pub fn dragon_part_boxes(&self) -> Option<[Aabb; 8]> {
        self.dragon
            .as_ref()
            .filter(|d| d.alive())
            .map(|d| std::array::from_fn(|i| d.part_box(i)))
    }

    /// Whether `id` is a live End crystal entity id.
    pub fn is_crystal_id(&self, id: u32) -> bool {
        self.crystals.iter().any(|c| c.id == id)
    }

    /// Targets for projectiles: crystals (by real entity id) and dragon parts.
    pub fn special_targets(&self) -> Vec<(u32, Aabb)> {
        let mut out = Vec::new();
        for c in self.crystals.iter() {
            out.push((c.id, c.bounds()));
        }
        if let Some(boxes) = self.dragon_part_boxes() {
            for (i, part) in boxes.into_iter().enumerate() {
                out.push((SPECIAL + i as u32, part));
            }
        }
        out
    }

    /// `getHeightmapPos(MOTION_BLOCKING_NO_LEAVES)`.
    pub fn motion_blocking_no_leaves(&self, x: i32, z: i32) -> i32 {
        (0..256)
            .rev()
            .find(|&y| {
                let f = self.res.facts().get(self.world.block(x, y, z));
                (f.full_collision
                    || f.legacy_solid
                    || f.fluid != sunrise_items::blocks::Fluid::None)
                    && !f.name.ends_with("_leaves")
            })
            .map_or(0, |y| y + 1)
    }

    pub fn podium_top(&self) -> Vec3 {
        Vec3::new(0.5, f64::from(self.motion_blocking_no_leaves(0, 0)), 0.5)
    }

    /// `getHeadLookVector`.
    pub fn dragon_look(&self) -> [f64; 3] {
        let Some(d) = self.dragon.as_ref() else {
            return [0.0; 3];
        };
        let pitch = match d.phase {
            Phase::Landing { .. } | Phase::Takeoff { .. } => {
                let p = self.podium_top();
                let dist = ((p.x - d.position.x).powi(2)
                    + (p.y - d.position.y).powi(2)
                    + (p.z - d.position.z).powi(2))
                .sqrt() as f32
                    / 4.0;
                -6.0 / dist.max(1.0) * 1.5 * 5.0
            }
            _ if d.phase.sitting() => -45.0,
            _ => 0.0,
        };
        view_vector(d.yaw, pitch)
    }

    /// `getHeadLookVector(1).normalize()`, which the phases' `doClientTick`s
    /// take (a look the dragon has never turned from stays the zero vector).
    pub fn dragon_look_normalised(&self) -> [f64; 3] {
        let look = self.dragon_look();
        let norm = (look[0] * look[0] + look[1] * look[1] + look[2] * look[2]).sqrt();
        if norm < 1.0e-5 {
            [0.0; 3]
        } else {
            [look[0] / norm, look[1] / norm, look[2] / norm]
        }
    }

    /// The part positions from the yaw and the flight history.
    pub fn place_dragon_parts(&mut self) {
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        // Mth.sin / Mth.cos on floats, as EnderDragon.aiStep places them.
        let msin = |a: f32| trig::sin(f64::from(a));
        let mcos = |a: f32| trig::cos(f64::from(a));
        let tilt = (d.sample(5).0 - d.sample(10).0) as f32 * 10.0 * trig::DEG_TO_RAD;
        let (ct, st) = (mcos(tilt), msin(tilt));
        let r1 = d.yaw * trig::DEG_TO_RAD;
        let (s1, c1) = (msin(r1), mcos(r1));
        let p = d.position;
        let set = |d: &mut Dragon, i: usize, x: f32, y: f64, z: f32| {
            d.parts[i].0 = Vec3::new(p.x + f64::from(x), p.y + y, p.z + f64::from(z));
        };
        set(d, 2, s1 * 0.5, 0.0, -c1 * 0.5);
        set(d, 6, c1 * 4.5, 2.0, s1 * 4.5);
        set(d, 7, c1 * -4.5, 2.0, s1 * -4.5);
        let r2 = d.yaw * trig::DEG_TO_RAD - d.yaw_accel * 0.01;
        let (s2, c2) = (msin(r2), mcos(r2));
        let y_off = if d.phase.sitting() {
            -1.0
        } else {
            d.sample(5).0 - d.sample(0).0
        } as f32;
        set(
            d,
            0,
            s2 * 6.5 * ct,
            f64::from(y_off + st * 6.5),
            -c2 * 6.5 * ct,
        );
        set(
            d,
            1,
            s2 * 5.5 * ct,
            f64::from(y_off + st * 5.5),
            -c2 * 5.5 * ct,
        );
        let p1 = d.sample(5);
        for i in 0..3 {
            let p0 = d.sample(12 + i * 2);
            let rot = d.yaw * trig::DEG_TO_RAD + wrap(p0.1 - p1.1) * trig::DEG_TO_RAD;
            let (ss, cc) = (msin(rot), mcos(rot));
            let dd = (i as f32 + 1.0) * 2.0;
            let x = -(s1 * 1.5 + ss * dd) * ct;
            let y = p0.0 - p1.0 - f64::from((dd + 1.5) * st) + 1.5;
            let z = (c1 * 1.5 + cc * dd) * ct;
            set(d, 3 + i, x, y, z);
        }
    }

    /// `EnderDragon.checkCrystals`' search: the nearest crystal whose box
    /// meets the dragon's box grown by 32.
    pub fn nearest_crystal_to(&self, p: Vec3) -> Option<usize> {
        // The dragon's 16 by 8 box.
        let reach = Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        )
        .inflate(Vec3::new(32.0, 32.0, 32.0));
        self.crystals
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                // An end crystal's 2 by 2 box.
                reach.intersects(Aabb::new(
                    Vec3::new(c.position.x - 1.0, c.position.y, c.position.z - 1.0),
                    Vec3::new(c.position.x + 1.0, c.position.y + 2.0, c.position.z + 1.0),
                ))
            })
            .min_by(|a, b| {
                let da = (a.1.position.x - p.x).powi(2)
                    + (a.1.position.y - p.y).powi(2)
                    + (a.1.position.z - p.z).powi(2);
                let db = (b.1.position.x - p.x).powi(2)
                    + (b.1.position.y - p.y).powi(2)
                    + (b.1.position.z - p.z).powi(2);
                da.total_cmp(&db)
            })
            .map(|(i, _)| i)
    }
}

pub trait DragonCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Resolves a crosshair target id (including synthetic dragon-part ids) to its wire entity id.
    fn wire_entity_id(&self, id: u32) -> Option<i32> {
        if (SPECIAL..SPECIAL + 8).contains(&id) {
            let dragon = self
                .dragon
                .as_ref()
                .filter(|d| d.id != 0 || self.side_has_remote())?;
            return Some(dragon_part_wire_id(dragon.id, (id - SPECIAL) as usize));
        }
        Some(id as i32)
    }

    /// The dragon the level holds and the index (`getSubEntities` order) of
    /// its part with that entity id, while the dragon is there, dying or
    /// not: a part is an entity for as long as its parent is.
    fn resident_dragon_part<'__loan>(&'__loan self, id: u32) -> Option<(&'__loan Dragon, usize)>
    where
        '__sg: '__loan,
    {
        let part = self.side_resident_dragon_part_index(id)?;
        Some((self.dragon.as_ref()?, part))
    }

    /// The box of the dragon part or end crystal guests know as entity `id`
    /// (`EnderDragonPart`s follow the dragon's id for its tracking lifetime).
    fn tracked_special_box(&self, id: i32) -> Option<Aabb> {
        let uid = id as u32;
        if let Some(d) = self
            .dragon
            .as_ref()
            .filter(|d| d.id != 0 || self.side_has_remote())
            && let Some(part) = dragon_part_index(d.id, uid)
        {
            return Some(d.part_box(part));
        }
        let id = u32::try_from(id).ok().filter(|&id| id != 0)?;
        self.crystals
            .iter()
            .find(|c| c.id == id)
            .map(Crystal::bounds)
    }

    /// `EndCrystalItem.useOn`'s checks: obsidian or bedrock at `pos`, air
    /// above it and no entity in the two blocks above (else FAIL).
    fn end_crystal_fits(&self, pos: [i32; 3]) -> bool {
        let name = &self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name;
        if !matches!(&**name, "minecraft:obsidian" | "minecraft:bedrock") {
            return false;
        }
        let above = [pos[0], pos[1] + 1, pos[2]];
        if !self
            .res
            .facts()
            .is_air(self.world.block(above[0], above[1], above[2]))
        {
            return false;
        }
        let (x, y, z) = (
            f64::from(above[0]),
            f64::from(above[1]),
            f64::from(above[2]),
        );
        !self.side_entity_in(Aabb::new(
            Vec3::new(x, y, z),
            Vec3::new(x + 1.0, y + 2.0, z + 1.0),
        ))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> DragonCommonGameExt1<'__sg> for G {}

/// `EnderDragon.recreateFromPacket`: part `part` (`0..8`) gets wire id `dragon_id + 1 + part`
/// using Java's wrapping 32-bit integer arithmetic.
pub fn dragon_part_id(dragon_id: u32, part: usize) -> u32 {
    dragon_id.wrapping_add(1).wrapping_add(part as u32)
}

/// `EnderDragon.recreateFromPacket`: signed wrapping `i32` id of part `part` (`i + dragonId + 1`).
pub fn dragon_part_wire_id(dragon_id: u32, part: usize) -> i32 {
    dragon_part_id(dragon_id, part) as i32
}
/// Mob ids from here on are dragon parts (`SPECIAL + part`). End crystals use their real entity ids.
pub const SPECIAL: u32 = u32::MAX - 4096;

pub const MAX_HEALTH: f32 = 200.0;
pub const PART_SIZES: [(f64, f64); 8] = [
    (1.0, 1.0),
    (3.0, 3.0),
    (5.0, 3.0),
    (2.0, 2.0),
    (2.0, 2.0),
    (2.0, 2.0),
    (4.0, 2.0),
    (4.0, 2.0),
];
/// `AreaEffectCloud.WAIT_TIME` default: the cloud is inert for its first 20
/// ticks (no radius growth, no effects).
pub const BREATH_WAIT: i32 = 20;

/// `AreaEffectCloud.serverTick`: discarded once `tickCount - waitTime >=
/// duration`, so the wait does not count against the duration.
pub fn breath_expired(age: i32, duration: i32) -> bool {
    age - BREATH_WAIT >= duration
}

/// `Mth.wrapDegrees(float)`: Java's `%` (truncating, as Rust's), then one step of 360.
pub fn wrap(d: f32) -> f32 {
    let mut r = d % 360.0;
    if r >= 180.0 {
        r -= 360.0;
    }
    if r < -180.0 {
        r += 360.0;
    }
    r
}
#[derive(Clone, Debug)]
pub struct Crystal {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub join: sunrise_sim::join_seq::JoinSeq,
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub position: Vec3,
    /// `Entity.xo`, `yo`, `zo`: where it was when its tick began (`setOldPosAndRot`). It never
    /// moves itself; a client's copy eases to where the server sends it.
    pub previous: Vec3,
    pub rotation: [f32; 2],
    pub fire: i32,
    pub show_bottom: bool,
    /// EndCrystal.time (the bob and spin), starting random.
    pub time: i32,
    pub beam: Option<[i32; 3]>,
    /// `setPermanentlyInvulnerable` (respawned spike crystals).
    pub invulnerable: bool,
    /// `onGround` and `fallDistance`: a crystal never moves itself, so they stay
    /// as loaded until `/tp` grounds it or a cobweb or powder snow resets the
    /// fall (`crystal.rs`).
    pub fall: (bool, f64),
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<sunrise_sim::pose::SizeAnswer>,
}
sunrise_sim::impl_joined!(Crystal);
sunrise_mod_runtime::impl_attachment_lifetime!(Crystal);
impl Crystal {
    /// `EntityType.END_CRYSTAL.getDimensions()`: width, height and eye height.
    pub const DIMENSIONS: [f32; 3] = [2.0, 2.0, 1.7];
    /// `EntityType.END_CRYSTAL`: 2 x 2.
    pub fn bounds(&self) -> Aabb {
        let p = self.position;
        let vanilla = Aabb::new(
            Vec3::new(p.x - 1.0, p.y, p.z - 1.0),
            Vec3::new(p.x + 1.0, p.y + 2.0, p.z + 1.0),
        );
        sunrise_sim::pose::SizeAnswer::sized_box(self.size_answer, Self::DIMENSIONS, p, vanilla)
    }
}
#[derive(Clone, Debug, Default)]
pub struct Path {
    pub nodes: Vec<[i32; 3]>,
    pub next: usize,
}
impl Path {
    pub fn done(&self) -> bool {
        self.next >= self.nodes.len()
    }
    pub fn advance(&mut self) {
        self.next += 1;
    }
    pub fn next_pos(&self) -> [i32; 3] {
        self.nodes[self.next]
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Holding {
        path: Option<PathBox>,
        target: Option<Vec3>,
        clockwise: bool,
    },
    Strafe {
        charge: i32,
        path: Option<PathBox>,
        target: Option<Vec3>,
        clockwise: bool,
        /// `attackTarget`: the player it strafes.
        player: Option<FightWho>,
    },
    LandingApproach {
        path: Option<PathBox>,
        target: Option<Vec3>,
    },
    Landing {
        target: Option<Vec3>,
    },
    Takeoff {
        first: bool,
        path: Option<PathBox>,
        target: Option<Vec3>,
    },
    Scanning {
        time: i32,
    },
    Attacking {
        ticks: i32,
    },
    Flaming {
        ticks: i32,
    },
    Charging {
        target: Option<Vec3>,
        since: i32,
    },
    Dying {
        target: Option<Vec3>,
        time: i32,
    },
    Hover {
        target: Option<Vec3>,
    },
}
/// A path compared by identity only (phases compare by kind).
#[derive(Clone, Debug)]
pub struct PathBox(pub Path);
impl PartialEq for PathBox {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Phase {
    /// `EnderDragonPhase` ids.
    pub fn id(&self) -> i32 {
        match self {
            Self::Holding { .. } => 0,
            Self::Strafe { .. } => 1,
            Self::LandingApproach { .. } => 2,
            Self::Landing { .. } => 3,
            Self::Takeoff { .. } => 4,
            Self::Flaming { .. } => 5,
            Self::Scanning { .. } => 6,
            Self::Attacking { .. } => 7,
            Self::Charging { .. } => 8,
            Self::Dying { .. } => 9,
            Self::Hover { .. } => 10,
        }
    }
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Strafe {
                charge: 0,
                path: None,
                target: None,
                clockwise: false,
                player: None,
            },
            2 => Self::LandingApproach {
                path: None,
                target: None,
            },
            3 => Self::Landing { target: None },
            4 => Self::Takeoff {
                first: true,
                path: None,
                target: None,
            },
            5 => Self::Flaming { ticks: 0 },
            6 => Self::Scanning { time: 0 },
            7 => Self::Attacking { ticks: 0 },
            8 => Self::Charging {
                target: None,
                since: 0,
            },
            9 => Self::Dying {
                target: None,
                time: 0,
            },
            10 => Self::Hover { target: None },
            _ => Self::Holding {
                path: None,
                target: None,
                clockwise: false,
            },
        }
    }
    pub fn sitting(&self) -> bool {
        matches!(
            self,
            Self::Scanning { .. }
                | Self::Attacking { .. }
                | Self::Flaming { .. }
                | Self::Hover { .. }
        )
    }
    pub fn target(&self) -> Option<Vec3> {
        match self {
            Self::Holding { target, .. }
            | Self::Strafe { target, .. }
            | Self::LandingApproach { target, .. }
            | Self::Landing { target }
            | Self::Takeoff { target, .. }
            | Self::Charging { target, .. }
            | Self::Dying { target, .. }
            | Self::Hover { target } => *target,
            _ => None,
        }
    }
    pub fn fly_speed(&self) -> f32 {
        match self {
            Self::Landing { .. } => 1.5,
            Self::Charging { .. } | Self::Dying { .. } => 3.0,
            // DragonHoverPhase.getFlySpeed.
            Self::Hover { .. } => 1.0,
            _ => 0.6,
        }
    }
}
pub struct Dragon {
    pub(in crate::game) residency: DragonResidency,
    pub join: sunrise_sim::join_seq::JoinSeq,
    /// `Entity.id` once tracked to guests (0 before); its parts take the
    /// eight ids after it (`EnderDragon`'s constructor reserves them).
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub previous_yaw: f32,
    pub body_yaw: f32,
    pub head_yaw: f32,
    pub body_rotation: sunrise_sim::mobs::control::BodyRotation,
    pub yaw_accel: f32,
    pub health: f32,
    pub hurt_time: i32,
    /// `LivingEntity.damageCooldownTime` and `lastHurt`: within the window
    /// only damage above the last hit lands, and only the excess.
    pub invulnerable: i32,
    pub last_hurt: f32,
    /// Transient `LivingEntity.lastHurtMob`; an owned arrow or wind charge can set it.
    pub last_hurt_mob: Option<(u32, i32)>,
    /// `lastHurtByPlayer` with its 100 tick memory: the player credited with
    /// the kill, and the dragon tick of their last hit.
    pub hurt_by_player: Option<(FightWho, i32)>,
    pub last_source: Option<(&'static str, Option<i32>, Option<i32>)>,
    pub death_time: i32,
    pub death_canceled: bool,
    pub flap: f32,
    pub old_flap: f32,
    pub in_wall: bool,
    pub phase: Phase,
    /// `Mob.isAggressive` (the synced mob flag), `persistenceRequired` and
    /// the target a mod set (`Mob.target`): kept for native mods, the
    /// dragon's own flight never reads them.
    pub aggressive: bool,
    pub persistent: bool,
    pub target: Option<u32>,
    /// The phase left was `Flaming` (`DragonSittingFlamingPhase.end`) and its
    /// cloud is still to be discarded.
    pub flame_ended: bool,
    pub flame_count: i32,
    pub sitting_damage: f32,
    /// `LivingEntity.equipment`: nothing in the fight reads it; it is kept
    /// for mods and saves.
    pub equipment: sunrise_items::equipment::Equipment,
    /// The live attribute map (`AttributeMap`), built when an item with
    /// attribute modifiers is worn or it is read.
    pub live: sunrise_sim::living_attributes::LivingAttributes,
    pub growl: i32,
    /// `Mob.ambientSoundTime`.
    pub ambient_time: i32,
    pub ticks: i32,
    /// DragonFlightHistory: (y, yRot) samples, newest at `head`.
    pub history: [(f64, f32); 64],
    pub(in crate::game) head: usize,
    /// head, neck, body, tail1-3, wing1-2.
    pub parts: [DragonPart; 8],
    pub nodes: Option<[[i32; 3]; 24]>,
    pub nearest_crystal: Option<usize>,
    /// The player a crystal destroyed while it held its pattern sends it
    /// after (`strafe_after_crystal`), taken on its next tick.
    pub strafe_request: Option<FightWho>,
    /// `Mob.isNoAi` (`NoAI`): it flaps in place, with no phase or flight.
    pub no_ai: bool,
    pub random: LegacyRandom,
}
/// A player in the fight: the window's own player, or a guest by
/// connection (a LAN or server player).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FightWho {
    Host,
    Guest(u32),
}
#[derive(Default)]
pub(in crate::game) struct DragonResidency {
    pub(in crate::game) lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
}

/// A real sub-entity: geometry, identity and attachment authority move and
/// retire together with this part, never with a shared parent attachment map.
pub struct DragonPart(
    pub Vec3,
    pub Vec3,
    pub sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub [i32; 4],
);
impl Default for DragonPart {
    fn default() -> Self {
        Self(
            Vec3::ZERO,
            Vec3::ZERO,
            Default::default(),
            sunrise_sim::save::insecure_uuid(&mut random_source::entity()),
        )
    }
}
impl sunrise_mod_runtime::attachments::HasAttachmentLifetime for DragonPart {
    fn attachment_lifetime(&mut self) -> &mut sunrise_mod_runtime::attachments::AttachmentLifetime {
        &mut self.2
    }
}
impl sunrise_mod_runtime::attachments::HasAttachmentLifetime for Dragon {
    fn attachment_lifetime(&mut self) -> &mut sunrise_mod_runtime::attachments::AttachmentLifetime {
        &mut self.residency.lifetime
    }
}

impl Crystal {
    /// `new EndCrystal(level, x, y, z)`: no beam, not invulnerable, on no
    /// ground with no fall (what `Entity`'s constructor leaves).
    pub fn new(id: u32, position: Vec3, show_bottom: bool, time: i32) -> Self {
        Self {
            size_answer: None,
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            position,
            previous: position,
            rotation: [0.0; 2],
            fire: 0,
            show_bottom,
            time,
            beam: None,
            invulnerable: false,
            fall: (false, 0.0),
        }
    }
    /// `EndCrystal.<init>`'s draws, all from the crystal's own random (`Entity.random`,
    /// `RandomSource.create()`): its UUID (`Mth.createInsecureUUID`), then
    /// `time = random.nextInt(100000)`. The level's random draws nothing. Returns the drawn time
    /// (for `start_time`) and the UUID.
    pub fn own_draws() -> (i32, [i32; 4]) {
        let mut random = sunrise_level::random_source::entity();
        let uuid = sunrise_sim::save::insecure_uuid(&mut random);
        (random.next_int_bound(100_000).unwrap_or(0), uuid)
    }
    /// The start of `time` (the spin and bob) for a crystal at `position`: the random draw
    /// vanilla makes, or under `SUNRISE_STILL` one seeded from the position alone. Which crystal
    /// gets which draw, and a server's entity ids, follow the order chunks load in, which differs
    /// between launches; the position is the same in every copy of the world. The draw is made
    /// either way, so the random stays where it would be.
    pub fn start_time(position: Vec3, drawn: i32, still: bool) -> i32 {
        if still {
            sunrise_sim::mobs::entity_random(0, position)
                .next_int_bound(100_000)
                .unwrap_or(0)
        } else {
            drawn
        }
    }
}

impl Dragon {
    /// `Entity.teleportSetPosition` with the command's relative motion flags.
    pub fn teleport_motion(&mut self, relative: [bool; 3]) {
        self.velocity = super::commands_common::teleport_velocity(self.velocity, relative, false);
    }
    /// `EnderDragon`'s constructor: full health, hovering (`EnderDragonPhase.HOVERING`). Only
    /// `EnderDragonFight.createNewDragon` sets the holding pattern; a dragon from `/summon`, a
    /// spawn egg or a save without `DragonPhase` keeps hovering where it was put.
    pub fn new(position: Vec3, yaw: f32, seed: i64) -> Self {
        Self {
            residency: Default::default(),
            join: Default::default(),
            id: 0,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            pitch: 0.0,
            previous_yaw: yaw,
            body_yaw: yaw,
            head_yaw: yaw,
            body_rotation: Default::default(),
            yaw_accel: 0.0,
            health: MAX_HEALTH,
            hurt_time: 0,
            invulnerable: 0,
            last_hurt: 0.0,
            last_hurt_mob: None,
            hurt_by_player: None,
            last_source: None,
            death_time: 0,
            death_canceled: false,
            flap: 0.0,
            old_flap: 0.0,
            in_wall: false,
            phase: Phase::Hover { target: None },
            aggressive: false,
            persistent: false,
            target: None,
            flame_ended: false,
            flame_count: 0,
            sitting_damage: 0.0,
            equipment: Default::default(),
            live: Default::default(),
            growl: 100,
            ambient_time: 0,
            ticks: 0,
            history: [(position.y, yaw); 64],
            head: 0,
            // The parts are entities of their own, at the origin until the first
            // `aiStep` places them (`tickPart`).
            parts: std::array::from_fn(|_| Default::default()),
            nodes: None,
            nearest_crystal: None,
            strafe_request: None,
            no_ai: false,
            random: LegacyRandom::new(seed),
        }
    }
    /// DragonFlightHistory.record.
    pub fn record(&mut self) {
        self.head = (self.head + 1) & 63;
        self.history[self.head] = (self.position.y, self.yaw);
    }
    /// DragonFlightHistory.get(delay).
    pub(in crate::game) fn sample(&self, delay: usize) -> (f64, f32) {
        self.history[(self.head + 64 - (delay & 63)) & 63]
    }
    /// `DragonFlightHistory.get(delay, partial)`, with
    /// `EnderDragonRenderer.extractRenderState`'s frozen history on death.
    pub fn sample_lerp(&self, delay: usize, partial: f32) -> (f64, f32) {
        let partial = if self.alive() { partial } else { 0.0 };
        let a = self.sample(delay);
        let b = self.sample(delay + 1);
        let yaw = b.1 + wrap(a.1 - b.1) * partial;
        (b.0 + (a.0 - b.0) * f64::from(partial), yaw)
    }
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
    /// The entity's own box, not any of the separate `EnderDragonPart` boxes.
    pub fn body_bounds(&self) -> Aabb {
        // EntityTypes.ENDER_DRAGON dimensions: 16 wide, 8 high.
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        )
    }
    /// Where part `i` stands and its box.
    pub fn part_geometry(&self, i: usize) -> (Vec3, Aabb) {
        (self.parts[i].0, self.part_box(i))
    }
    pub fn part_box(&self, i: usize) -> Aabb {
        let (w, h) = PART_SIZES[i];
        let p = self.parts[i].0;
        Aabb::new(
            Vec3::new(p.x - w / 2.0, p.y, p.z - w / 2.0),
            Vec3::new(p.x + w / 2.0, p.y + h, p.z + w / 2.0),
        )
    }
    /// Whether the phase, part scaling and source guards reach LivingEntity.
    pub fn reaches_base_check(&self, part: usize, damage: f32, counts: bool, arrow: bool) -> bool {
        let damage = if part != 0 && part != 1 {
            damage / 4.0 + damage.min(1.0)
        } else {
            damage
        };
        self.alive()
            && !matches!(self.phase, Phase::Dying { .. })
            && !(arrow && self.phase.sitting())
            && damage >= 0.01
            && counts
    }
    /// `ServerExplosion.hurtEntities` on the dragon: the dragon itself (its
    /// body takes it), then each part, in `Level.getEntities` order, each by
    /// its distance and how much of its box the blast sees (`exposure`).
    pub fn explosion_hits(
        &self,
        center: [f64; 3],
        radius: f32,
        exposure: impl Fn(Aabb) -> f32,
    ) -> Vec<(usize, f32)> {
        let diameter = f64::from(radius * 2.0);
        let dist = |p: Vec3| {
            ((p.x - center[0]).powi(2) + (p.y - center[1]).powi(2) + (p.z - center[2]).powi(2))
                .sqrt()
                / diameter
        };
        let p = self.position;
        let whole = Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        );
        std::iter::once((2, p, whole))
            .chain((0..8).map(|i| (i, self.parts[i].0, self.part_box(i))))
            .filter_map(|(part, at, b)| {
                let d = dist(at);
                (d <= 1.0).then(|| {
                    let pow = (1.0 - d) * f64::from(exposure(b));
                    (
                        part,
                        ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32,
                    )
                })
            })
            .collect()
    }
    /// `ServerExplosion.hurtEntities` knockback on the dragon itself:
    /// `pushFromExplosion` adds `(1 - dist) * exposure` along the blast to
    /// its eye to the motion (`Entity.push`); the parts are not pushed.
    pub fn whole_box(&self) -> Aabb {
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
            Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
        )
    }
    pub fn explosion_knockback(
        &self,
        center: [f64; 3],
        radius: f32,
        exposure: f32,
    ) -> Option<[f64; 3]> {
        let diameter = f64::from(radius * 2.0);
        let p = self.position;
        let at = |y: f64| [p.x - center[0], y - center[1], p.z - center[2]];
        let len = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let dist = len(at(p.y)) / diameter;
        if dist > 1.0 {
            return None;
        }
        // The dragon's eye is 0.85 of its 8 block height, in floats.
        let away = at(p.y + f64::from(8.0f32 * 0.85f32));
        let l = len(away);
        // `Vec3.normalize` gives zero below 1.0E-5F, then `scale(power)`.
        if l < 9.999_999_747_378_752e-6 {
            return Some([0.0; 3]);
        }
        let power = (1.0 - dist) * f64::from(exposure);
        Some(away.map(|d| d / l * power))
    }
    /// The phase as a native mod sees it.
    pub fn native_phase(&self) -> DragonPhase {
        DragonPhase::from_id(self.phase.id()).expect("the phases are the vanilla ones")
    }
    /// `EnderDragonPhaseManager.setPhase`: nothing when it is the phase it
    /// holds (a strafing dragon too), else the new phase begins fresh.
    pub fn set_native_phase(&mut self, phase: DragonPhase) {
        if phase != self.native_phase() {
            self.set_phase(Phase::from_id(phase.id()));
        }
    }
    pub fn set_phase(&mut self, phase: Phase) {
        if self.phase.id() == phase.id() && phase.id() != 1 {
            return;
        }
        // `EnderDragonPhaseManager.setPhase`: the outgoing `end()` first.
        if matches!(self.phase, Phase::Flaming { .. }) {
            self.flame_ended = true;
        }
        if let Phase::Flaming { .. } = phase {
            self.flame_count += 1;
        }
        self.phase = phase;
    }
}

/// Saving and loading the dragon and the crystals as chunk entities.
impl Dragon {
    /// `Entity.firstTick`: no tick has run yet.
    pub fn first_tick(&self) -> bool {
        self.ticks == 0
    }
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        raw: Option<&Tag>,
        items: &sunrise_items::items::Items,
    ) -> Tag {
        let mut tag = sunrise_sim::entity_nbt::vehicle_to_nbt(
            "minecraft:ender_dragon",
            self.position,
            self.velocity,
            [self.yaw, self.pitch],
            (false, 0.0),
            uuid,
            raw,
        );
        if let Tag::Compound(map) = &mut tag {
            map.insert("Health".into(), Tag::Float(self.health.to_bits()));
            map.insert("DragonPhase".into(), Tag::Int(self.phase.id()));
            map.insert("DragonDeathTime".into(), Tag::Int(self.death_time));
            if self.persistent {
                map.insert("PersistenceRequired".into(), Tag::Byte(1));
            }
            // Mob.addAdditionalSaveData writes NoAI only when set.
            if self.no_ai {
                map.insert("NoAI".into(), Tag::Byte(1));
            }
            map.insert(
                "sitting_damage_received".into(),
                Tag::Float(self.sitting_damage.to_bits()),
            );
            self.equipment.store(map, items);
            sunrise_sim::entity_nbt::put_live_attributes(map, &self.live);
        }
        tag
    }
    pub fn from_nbt(tag: &Tag, seed: i64) -> Option<Self> {
        let (position, velocity, _) = sunrise_sim::entity_nbt::boat_from_nbt(tag)?;
        let [yaw, pitch] = sunrise_sim::entity_nbt::loaded_rotation(tag);
        let mut d = Self::new(position, yaw, seed);
        d.pitch = pitch;
        d.velocity = velocity;
        d.apply_saved_fields(tag);
        Some(d)
    }

    fn apply_saved_fields(&mut self, tag: &Tag) {
        // Vanilla getFloatOr/getIntOr accept any numeric tag type.
        self.health = nbt_value::float(tag.get("Health")).map_or(self.health, |h| h as f32);
        self.sitting_damage =
            nbt_value::float(tag.get("sitting_damage_received")).unwrap_or(0.0) as f32;
        if let Some(phase) = saved_dragon_phase(tag.get("DragonPhase")) {
            self.phase = phase;
        }
        if let Some(t) = nbt_value::number(tag.get("DragonDeathTime")) {
            self.death_time = t as i32;
        }
        self.persistent = sunrise_sim::entity_nbt::flag(tag, "PersistenceRequired");
        if let Some(equipment) = tag.get("equipment") {
            self.equipment = sunrise_items::equipment::Equipment::read(equipment);
        }
        self.live.load(tag.get("attributes"), |_| true);
        self.no_ai = matches!(tag.get("NoAI"), Some(Tag::Byte(b)) if *b != 0);
    }
}

pub fn saved_dragon_phase(tag: Option<&Tag>) -> Option<Phase> {
    nbt_value::number(tag).map(|phase| Phase::from_id(phase as i32))
}

impl Crystal {
    pub fn to_nbt(&self, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
        let mut tag = sunrise_sim::entity_nbt::vehicle_to_nbt(
            "minecraft:end_crystal",
            self.position,
            Vec3::ZERO,
            self.rotation,
            self.fall,
            uuid,
            raw,
        );
        if let Tag::Compound(map) = &mut tag {
            map.insert("Fire".into(), Tag::Short(self.fire as i16));
            map.insert("ShowBottom".into(), Tag::Byte(i8::from(self.show_bottom)));
            if let Some(b) = self.beam {
                map.insert("beam_target".into(), Tag::IntArray(b.to_vec()));
            } else {
                map.remove(&"beam_target".into());
            }
            map.insert(
                "Invulnerable".into(),
                Tag::Byte(i8::from(self.invulnerable)),
            );
        }
        tag
    }
    pub fn from_nbt(tag: &Tag, time: i32) -> Option<Self> {
        let (position, _, _) = sunrise_sim::entity_nbt::boat_from_nbt(tag)?;
        // TagValueInput.getShortOr uses NumericTag.shortValue.
        let fire = match tag.get("Fire") {
            Some(Tag::Float(bits)) => sunrise_core::mth::floor_f32(f32::from_bits(*bits)) as i16,
            Some(Tag::Double(bits)) => sunrise_core::mth::floor(f64::from_bits(*bits)) as i16,
            value => nbt_value::number(value).unwrap_or_default() as i16,
        };
        Some(Self {
            size_answer: None,
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 0,
            position,
            previous: position,
            rotation: sunrise_sim::entity_nbt::loaded_rotation(tag),
            fire: i32::from(fire),
            show_bottom: sunrise_sim::entity_nbt::flag_or(tag, "ShowBottom", true),
            time,
            beam: match tag.get("beam_target") {
                Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            },
            invulnerable: sunrise_sim::entity_nbt::flag(tag, "Invulnerable"),
            fall: sunrise_sim::entity_nbt::ground_from_nbt(tag),
        })
    }
}
pub struct Breath {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub join: sunrise_sim::join_seq::JoinSeq,
    /// Its entity id once a server's players track it (0 until then).
    pub id: u32,
    pub at: [f64; 3],
    pub motion: [f64; 3],
    /// `Entity.fallDistance`: as loaded, until water resets or lava halves it.
    pub fall_distance: f64,
    /// `Entity.remainingFireTicks`: a cloud is fire immune, so its first tick
    /// clears a burning one to 0.
    pub fire_ticks: i32,
    /// `Entity.firstTick`: true until the first base tick finishes, gating `isInLava()`.
    pub first_tick: bool,
    pub radius: f32,
    pub age: i32,
    pub duration: i32,
    pub from_dragon: bool,
    /// Who it hit and until which age (`AreaEffectCloud.victims`).
    pub victims: Vec<(Victim, i32)>,
    /// A lingering potion's cloud (`ThrownLingeringPotion`): what it applies.
    pub potion: Option<PotionCloud>,
}

sunrise_sim::impl_joined!(Breath, Dragon);
sunrise_mod_runtime::impl_attachment_lifetime!(Breath);

impl Breath {
    /// `AreaEffectCloud.isWaiting`: before its wait time (20 ticks, 10 for a
    /// lingering potion's).
    pub fn waiting(&self) -> bool {
        self.age < self.potion.as_ref().map_or(20, |p| p.settings.wait_time)
    }
}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;

#[cfg(any(test, feature = "test-support"))]
impl Dragon {
    pub fn set_render_test_crystal(&mut self, crystal: Option<usize>) {
        self.nearest_crystal = crystal;
    }
}
