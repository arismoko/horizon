//! `ServerExplosion`: exploded positions (vanilla ray march over a Java
//! HashSet, so iteration and the following shuffle match vanilla's random
//! draws), entity damage with exposure and knockback, block interaction with
//! merged drops and TNT chain priming, and optional fire.
use super::entity::{Blast, Entity, EntityHandle, with_entity, with_entity_mut};
use mod_api::ExplosionStart;
use std::{cmp::Ordering, collections::HashSet};
use sunrise_core::{
    geometry::Vec3,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_game_data::{Result, text::Text};
use sunrise_gameplay_common::game::collider_rays::ColliderRays;
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;
use sunrise_items::{blocks::Fluid, items::ItemStack};
use sunrise_level::collisions::SceneCollisions;
use sunrise_sim::health::DamageLevel;
use sunrise_sim::mobs::Who;
use {
    sunrise_core::java_hash_map::{JavaHashMap, JavaKey},
    sunrise_world::{
        collision_context::{CollisionContext, CollisionEntity, CollisionItem},
        entity_obstruction::EntityIdentity,
        registry::BlockStateId,
        world_ray::{RayBlockMode, RayFluidMode, clip_world},
    },
};

/// The causing entity of an explicit explosion damage source. Minecart ignition
/// keeps this in memory only, just like `MinecartTNT.ignitionSource`.
#[derive(Clone, Debug)]
pub struct ExplosionCause {
    pub entity: (u32, Text),
    pub player: bool,
    pub guest: Option<u32>,
    pub species: Option<sunrise_sim::mobs::Species>,
}

/// `BlockPos` with Java `Vec3i.hashCode` / `compareTo`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key([i32; 3]);
impl JavaKey for Key {
    fn java_hash(&self) -> i32 {
        let [x, y, z] = self.0;
        y.wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x)
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        let ([x, y, z], [ox, oy, oz]) = (self.0, other.0);
        Some(y.cmp(&oy).then(z.cmp(&oz)).then(x.cmp(&ox)))
    }
    fn tie_break(&self, _other: &Self) -> Ordering {
        Ordering::Less
    }
}

/// `Explosion.BlockInteraction` for the explosions Sunrise produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockInteraction {
    Keep,
    Destroy,
    DestroyWithDecay,
    /// `TRIGGER_BLOCK`: nothing breaks; buttons, doors and the like react.
    Trigger,
}
impl BlockInteraction {
    /// `Explosion.BlockInteraction.shouldAffectBlocklikeEntities`.
    fn affects_blocklike(self) -> bool {
        matches!(self, Self::Destroy | Self::DestroyWithDecay)
    }
}

/// Properties of the direct entity passed to `ServerLevel.explode`.
#[derive(Clone, Copy, Default)]
pub struct ExplosionSource {
    silent: bool,
    pub(in crate::game) portal_proof: bool,
    /// The entity that exploded and its kind (`Explosion.getDirectSourceEntity`), when known.
    entity: Option<(u32, &'static str)>,
    /// The blast damage source's direct entity and its class (`DamageSource.getDirectEntity`),
    /// as its producer knows them: the exploding entity for a default source
    /// (`Explosion.getDefaultDamageSource`), none for a blast without one.
    damage_direct: Option<(u32, sunrise_sim::health::DirectEntity)>,
    /// The entity type of `damage_direct`, when known (`Damage.direct` on mob victims).
    damage_direct_kind: Option<&'static str>,
}

/// An explicit blast `DamageSource` (`ServerLevel.explode(source, damageSource, ...)`): its own
/// direct entity (id, type for its name, class) and causing entity, apart from the exploding one.
pub struct ExplicitBlastSource {
    pub direct: (u32, &'static str, sunrise_sim::health::DirectEntity),
    /// `None` is an explicitly ownerless source.
    pub cause: Option<ExplosionCause>,
}
impl ExplosionSource {
    pub fn with_silence(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }
    pub(in crate::game::server_side) fn with_portal_proof(mut self, portal_proof: bool) -> Self {
        self.portal_proof = portal_proof;
        self
    }
    /// The exploding entity, which a default damage source also names as its direct entity.
    pub fn with_entity(
        mut self,
        id: u32,
        kind: &'static str,
        class: sunrise_sim::health::DirectEntity,
    ) -> Self {
        self.entity = Some((id, kind));
        self.with_damage_direct(id, Some(kind), class)
    }
    pub(in crate::game::server_side) fn with_damage_direct(
        mut self,
        id: u32,
        kind: Option<&'static str>,
        class: sunrise_sim::health::DirectEntity,
    ) -> Self {
        self.damage_direct = Some((id, class));
        self.damage_direct_kind = kind;
        self
    }
    pub(super) fn direct(&self) -> Option<(u32, Option<&'static str>)> {
        self.damage_direct
            .map(|(id, _)| (id, self.damage_direct_kind))
    }
}

/// `ServerExplosion.hurtEntities`: an entity is hurt when its position is
/// within the blast's diameter of the centre.
pub(in crate::game::server_side) fn blast_reaches(
    center: [f64; 3],
    diameter: f64,
    at: [f64; 3],
) -> bool {
    let d =
        ((at[0] - center[0]).powi(2) + (at[1] - center[1]).powi(2) + (at[2] - center[2]).powi(2))
            .sqrt();
    d / diameter <= 1.0
}

/// `ItemEntity.ignoreExplosion` through
/// `ServerExplosion.shouldAffectBlocklikeEntities`: an explosion that does not
/// affect blocklike entities is ignored entirely by a dropped item. With
/// `mobGriefing` off (`canEntityGrief` for the exploding entity) that is every
/// blast that keeps blocks; with it on, none (no wind charge goes through
/// `server_explode`).
fn items_ignore_explosion(mob_griefing: bool, interaction: BlockInteraction) -> bool {
    !mob_griefing && !interaction.affects_blocklike()
}

/// Whether a blast leaves item frames, paintings, cushions and armour stands
/// alone. Without `mobGriefing` (`canEntityGrief` for the exploding entity), a
/// blast that keeps blocks is ignored (`ServerExplosion.shouldAffectBlocklikeEntities`
/// through their `ignoreExplosion`; no wind charge goes through `server_explode`).
/// `mob_refused`: a mob causes the blast and may not grief, so it does no harm
/// (`BlockAttachedEntity.hurtServer` and `ArmorStand.hurtServer`:
/// `source.getEntity() instanceof Mob`, with `canEntityGrief` for that mob).
pub(in crate::game::server_side) fn blast_spares_decorations(
    mob_griefing: bool,
    interaction: BlockInteraction,
    mob_refused: bool,
    source_in_water: bool,
) -> bool {
    // `BlockAttachedEntity.ignoreExplosion`: a source in water first.
    source_in_water || mob_refused || (!mob_griefing && !interaction.affects_blocklike())
}

/// How far the blast's `center` is from `p`.
fn blast_distance(center: [f64; 3], p: [f64; 3]) -> f64 {
    ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2)).sqrt()
}

/// The unit vector from `center` to `origin`, 0 when they coincide.
fn blast_direction(center: [f64; 3], origin: [f64; 3]) -> [f64; 3] {
    let d = [
        origin[0] - center[0],
        origin[1] - center[1],
        origin[2] - center[2],
    ];
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if l < 1.0e-4 {
        [0.0; 3]
    } else {
        [d[0] / l, d[1] / l, d[2] / l]
    }
}

/// The blast as `native_explosion_knockback` tells it: its centre and the
/// blocks it is about to break.
struct ExplosionHookContext<'a> {
    center: [f64; 3],
    blocks: &'a [[i32; 3]],
    explicit_cause: Option<Option<ExplosionCause>>,
    direct_name: Option<Text>,
    held_item: Option<Text>,
    credit: bool,
    /// The calculator's `shouldDamageEntity`, and its knockback multiplier for
    /// a player that isn't flying and for one that is.
    hurts: bool,
    knockback: f32,
    flying_knockback: f32,
    /// The player whose blast it is (`DamageSource.getEntity() instanceof Player`: a primed
    /// TNT's owner, the player who broke an end crystal), by entity id, with their team.
    player_cause: Option<(i32, Option<sunrise_game_data::scoreboard::Team>)>,
}

/// An entity in the blast's reach: how far (as a share of the blast's
/// diameter), how much of it the rays see, and the knockback.
struct Hit {
    dist: f64,
    exposure: f32,
    push: Vec3,
}

/// Apply blast damage when the mob's team and wither immunity allow it.
fn hurt_mob(
    level: &mut DamageLevel<'_>,
    m: &mut sunrise_sim::mobs::Mob,
    hurtable: bool,
    cause: Option<sunrise_sim::mobs::Species>,
    (amount, mut source): (f32, sunrise_sim::mobs::Damage),
) -> bool {
    source.attacker_species = cause;
    hurtable && m.damage(level, amount, source)
}

/// A blast's causing entity and its name (`Explosion.getIndirectSourceEntity`):
/// the mob that owns the exploding projectile while it is in the level
/// (`Projectile.getOwner` is null once it is removed), else the exploding
/// mob. `live` names a mob in the level, `exploded` the mobs whose blast
/// went off this tick and that already left it.
fn blast_cause(
    owner: Option<u32>,
    exploding: Option<u32>,
    live: impl Fn(u32) -> Option<Text>,
    exploded: &[(u32, Text)],
) -> Option<(u32, Text)> {
    if let Some(owner) = owner {
        return live(owner).map(|name| (owner, name));
    }
    let id = exploding?;
    let name = live(id).or_else(|| {
        exploded
            .iter()
            .rfind(|(m, _)| *m == id)
            .map(|(_, n)| n.clone())
    })?;
    Some((id, name))
}

thread_local! {
    /// Client-side randomness (explosion sound pitch), separate from the level random.
    static CLIENT_RANDOM: std::cell::RefCell<LegacyRandom> = std::cell::RefCell::new(LegacyRandom::new(0x5eed));
}
pub(in crate::game::server_side) fn client_float() -> f32 {
    CLIENT_RANDOM.with(|r| r.borrow_mut().next_float())
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn explosion_entity_name(&self, id: u32, kind: &str) -> Text {
        self.common
            .persist_raw(id)
            .and_then(|r| r.get("CustomName"))
            .map_or_else(
                || Text::translate(&format!("entity.{}", kind.replace(':', ".")), &[]),
                |name| {
                    sunrise_game_data::scoreboard::comp_text(
                        &sunrise_game_data::scoreboard::tag_to_json(name),
                    )
                },
            )
    }

    fn explosion_cause_resolves(&self, cause: &ExplosionCause) -> bool {
        if !cause.player {
            return self
                .common
                .mobs
                .iter()
                .any(|m| m.id == cause.entity.0 && !m.removed);
        }
        let acting = self
            .server
            .lan_acting_entity
            .or_else(|| self.common.player.as_ref().map(|p| p.entity_id as u32));
        acting == Some(cause.entity.0)
            || self.server.lan.as_ref().is_some_and(|host| {
                host.hub
                    .guests
                    .iter()
                    .any(|g| g.entity_id as u32 == cause.entity.0)
            })
    }

    /// An explicit DamageSource must not inherit an enclosing blast's cause.
    pub(in crate::game::server_side) fn with_explosion_cause<R>(
        &mut self,
        cause: Option<ExplosionCause>,
        direct_source: (u32, Text),
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        // The DamageSource retains the removed mob's display name. This is
        // deliberately not added to exploded_kinds: its EntityReference no
        // longer resolves for kill credit, unlike Sunrise's early exploders.
        if let Some(cause) = &cause
            && cause.species.is_some()
            && !self.common.mobs.iter().any(|m| m.id == cause.entity.0)
            && !self
                .server
                .combat
                .0
                .exploded
                .iter()
                .any(|(id, _)| *id == cause.entity.0)
        {
            self.server.combat.0.exploded.push(cause.entity.clone());
        }
        let source = self.server.combat.0.explosion_source.take();
        let owner = self.server.combat.0.explosion_owner.take();
        let player = self.server.combat.0.blast_owner.take();
        let guest = self.server.combat.0.blast_guest.take();
        let direct = self.server.combat.0.explosion_cause.take();
        self.server.combat.0.explosion_source = Some(direct_source.0);
        let outer_direct = self
            .server
            .combat
            .0
            .explicit_explosion_direct
            .replace(direct_source.1);
        if let Some(cause) = &cause {
            if cause.player {
                self.server.combat.0.blast_owner = Some(cause.entity.clone());
                self.server.combat.0.blast_guest = cause.guest;
            } else {
                self.server.combat.0.explosion_owner = Some(cause.entity.0);
            }
        }
        let outer = self.server.combat.0.explicit_explosion_cause.replace(cause);
        let result = run(self);
        self.server.combat.0.explicit_explosion_cause = outer;
        self.server.combat.0.explosion_source = source;
        self.server.combat.0.explosion_owner = owner;
        self.server.combat.0.blast_owner = player;
        self.server.combat.0.blast_guest = guest;
        self.server.combat.0.explosion_cause = direct;
        self.server.combat.0.explicit_explosion_direct = outer_direct;
        result
    }

    /// The player whose blast is going off (`DamageSource.getEntity() instanceof Player`), by
    /// entity id, with their team: an explicit cause that is a player (an end crystal they
    /// broke), else a primed TNT's owning player. Asked before any player is swapped in, so
    /// a cause that names no guest is the player in place.
    fn blast_player_cause(&self) -> Option<(i32, Option<sunrise_game_data::scoreboard::Team>)> {
        let (id, guest) = match &self.server.combat.0.explicit_explosion_cause {
            Some(cause) => cause
                .as_ref()
                .filter(|c| c.player)
                .map(|c| (c.entity.0, c.guest))?,
            None => self
                .server
                .combat
                .0
                .blast_owner
                .as_ref()
                .filter(|_| self.server.combat.0.blast_owner_player)
                .map(|(id, _)| (*id, self.server.combat.0.blast_guest))?,
        };
        let team = match guest {
            Some(conn) => self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .and_then(|g| self.common.score.board.players_team(&g.name)),
            None => self.player_team(),
        };
        Some((id as i32, team.cloned()))
    }

    /// The blast damage's cause. An explicit crystal/cart DamageSource may
    /// differ from the explosion's own indirect source.
    pub(in crate::game) fn blast_attacker(&self) -> Option<(u32, Text)> {
        if let Some(cause) = &self.server.combat.0.explicit_explosion_cause {
            return cause.as_ref().map(|c| {
                let name = (!c.player)
                    .then(|| self.mob_display_name(c.entity.0))
                    .flatten()
                    .unwrap_or_else(|| c.entity.1.clone());
                (c.entity.0, name)
            });
        }
        // `Explosion.getIndirectSourceEntity`: a primed TNT's owner names the
        // blast before any mob does, which makes the damage type
        // `minecraft:player_explosion` (`server_explode` asks this).
        self.server.combat.0.blast_owner.clone().or_else(|| {
            blast_cause(
                self.server.combat.0.explosion_owner,
                self.server.combat.0.explosion_source,
                |id| self.mob_display_name(id),
                &self.server.combat.0.exploded,
            )
        })
    }
    /// `Explosion.getIndirectSourceEntity` as the kind that caused it: a
    /// primed TNT's owner, the mob whose blast it is, or a projectile's owner.
    /// An explicit crystal or cart cause has no indirect source (`Explosion`'s
    /// switch).
    fn blast_indirect_source(&self) -> Option<Who> {
        if self.server.combat.0.explicit_explosion_cause.is_some() {
            return None;
        }
        if let Some((id, _)) = &self.server.combat.0.blast_owner {
            if self.server.combat.0.blast_owner_player {
                return Some(Who::Player);
            }
            // `PrimedTnt.getOwner` is an `EntityReference`: a removed owner
            // does not resolve, though the TNT still saves its UUID.
            return self
                .common
                .mobs
                .iter()
                .any(|m| m.id == *id && !m.removed)
                .then_some(Who::Mob(*id));
        }
        if let Some(id) = self.server.combat.0.explosion_owner {
            return self
                .common
                .mobs
                .iter()
                .any(|m| m.id == id && !m.removed)
                .then_some(Who::Mob(id));
        }
        // The exploding mob itself: `Explosion.getIndirectSourceEntity` names
        // it directly, even after its own tick marked it removed.
        self.server.combat.0.explosion_source.map(Who::Mob)
    }
    /// `ServerExplosion.getSeenPercent` for an entity box.
    pub(in crate::game::server_side) fn seen_percent(
        &self,
        center: [f64; 3],
        min: [f64; 3],
        max: [f64; 3],
    ) -> f32 {
        let xs = 1.0 / ((max[0] - min[0]) * 2.0 + 1.0);
        let ys = 1.0 / ((max[1] - min[1]) * 2.0 + 1.0);
        let zs = 1.0 / ((max[2] - min[2]) * 2.0 + 1.0);
        let x_offset = (1.0 - (1.0 / xs).floor() * xs) / 2.0;
        let z_offset = (1.0 - (1.0 / zs).floor() * zs) / 2.0;
        if xs < 0.0 || ys < 0.0 || zs < 0.0 {
            return 0.0;
        }
        // ClipContext(..., entity): the entity standing at the box's bottom.
        let mut rays = ColliderRays {
            clip_data: None,
            scene: SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            },
            context: CollisionContext::for_entity(
                CollisionEntity {
                    identity: EntityIdentity(0),
                    kind: sunrise_world::collision_context::EntityContextKind::None,
                    y: min[1],
                    descending: false,
                    main_hand: Some(CollisionItem::EMPTY),
                },
                false,
            ),
            report_unloaded: false,
        };
        let (mut hits, mut count) = (0, 0);
        let mut xx = 0.0;
        while xx <= 1.0 {
            let mut yy = 0.0;
            while yy <= 1.0 {
                let mut zz = 0.0;
                while zz <= 1.0 {
                    let x = min[0] + (max[0] - min[0]) * xx;
                    let y = min[1] + (max[1] - min[1]) * yy;
                    let z = min[2] + (max[2] - min[2]) * zz;
                    let from = Vec3::new(x + x_offset, y, z + z_offset);
                    let to = Vec3::new(center[0], center[1], center[2]);
                    let missed = clip_world(
                        &mut rays,
                        from,
                        to,
                        RayBlockMode::Collider,
                        RayFluidMode::None,
                        4096,
                    )
                    .is_ok_and(|r| r.miss);
                    if missed {
                        hits += 1;
                    }
                    count += 1;
                    zz += zs;
                }
                yy += ys;
            }
            xx += xs;
        }
        hits as f32 / count as f32
    }
    /// What the cell at `pos` resists the blast `explosion` with
    /// (`ExplosionDamageCalculator.getBlockExplosionResistance` and the
    /// calculator overrides `rules` holds): None for no
    /// resistance at all, which costs a ray only its step. Also whether the
    /// block is spared. With `answers`, a state whose parts declare their
    /// answer pure is asked once and answered from there after. The level is
    /// not changed.
    pub(in crate::game::server_side) fn cell_explosion_resistance(
        &mut self,
        pos: [i32; 3],
        rules: &BlastRules,
        explosion: &ExplosionStart<'_>,
        answers: Option<&mut PartAnswers>,
    ) -> (Option<f32>, bool) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let f = self.common.res.facts().get(state);
        if let Some(step) = self.blast_calculator_step(&f.name) {
            return step;
        }
        // USED_PORTAL_DAMAGE_CALCULATOR returns no resistance and never explodes a portal.
        if rules.portal_proof && &*f.name == "minecraft:nether_portal" {
            return (None, true);
        }
        let fluid = if f.fluid != Fluid::None {
            100.0f32
        } else {
            0.0
        };
        // A primed TNT minecart's rails and the blocks they
        // stand on: no resistance, and never blown up.
        let spared = rules.rail_proof
            && (self.rail(None, pos[0], pos[1], pos[2]).is_some()
                || self.rail(None, pos[0], pos[1] + 1, pos[2]).is_some());
        let resistance = match rules.resists {
            Some((at, resistance)) if at == pos => Some(resistance),
            _ if !self.common.res.facts().is_air(state) || f.fluid != Fluid::None => {
                let immune = rules
                    .wither_immune
                    .as_ref()
                    .is_some_and(|t| t.contains(&*f.name));
                let own = f.explosion_resistance;
                // WitherBoss.canDestroy for the skull's cap, or what the
                // block's parts answer for the skull.
                let kept = match (rules.cap, rules.direct) {
                    (Some(_), Some(skull)) => !self.entity_may_destroy(skull, pos, !immune),
                    _ => immune,
                };
                // Block.getExplosionResistance, or what the block's parts answer.
                let base = if spared {
                    0.0
                } else {
                    let asked = match answers {
                        Some(answers)
                            if sunrise_gameplay_common::game::api_values::block_state::explosion_resistance_pure(
                                state,
                            ) =>
                        {
                            *answers.entry(state).or_insert_with(|| {
                                self.part_explosion_resistance(pos, state, explosion)
                            })
                        }
                        _ => self.part_explosion_resistance(pos, state, explosion),
                    };
                    asked.unwrap_or(own).max(fluid)
                };
                Some(blast_resistance(base, rules.cap, kept))
            }
            _ => None,
        };
        (resistance, spared)
    }
    /// The rules of the blast going off now, `resists` overriding one block's
    /// resistance and `portal_proof` as its source has it.
    pub(in crate::game::server_side) fn blast_rules(
        &self,
        resists: Option<([i32; 3], f32)>,
        portal_proof: bool,
    ) -> BlastRules {
        let cap = self.server.combat.0.blast_resistance_cap;
        BlastRules {
            resists,
            cap,
            direct: self.server.combat.0.explosion_direct.map(|(id, _)| id),
            // WitherSkull.getBlockExplosionResistance: a dangerous skull's blast
            // caps a block the wither may destroy at 0.8.
            wither_immune: cap.and_then(|_| self.block_tag_blocks("minecraft:wither_immune")),
            rail_proof: self.server.combat.0.rail_proof_blast,
            portal_proof,
        }
    }
    /// The blocks of tag `tag`, if the game has it.
    fn block_tag_blocks(&self, tag: &str) -> Option<HashSet<String>> {
        self.common.res.items().block_tags.get(tag).cloned()
    }
    /// `ServerExplosion.calculateExplodedPositions`; `resists` overrides one
    /// block's resistance (`ExplosionDamageCalculator.getBlockExplosionResistance`).
    pub fn exploded_positions(
        &mut self,
        center: [f64; 3],
        radius: f32,
        resists: Option<([i32; 3], f32)>,
        portal_proof: bool,
        explosion: &ExplosionStart<'_>,
    ) -> Vec<[i32; 3]> {
        let (min_y, max_y) = self.with_level(|_, level, _| {
            use sunrise_world::level::LevelAccess;
            (level.min_y(), level.max_y())
        });
        let rules = self.blast_rules(resists, portal_proof);
        let mut set: JavaHashMap<Key, ()> = JavaHashMap::default();
        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let mut xd = f64::from(xx as f32 / 15.0 * 2.0 - 1.0);
                    let mut yd = f64::from(yy as f32 / 15.0 * 2.0 - 1.0);
                    let mut zd = f64::from(zz as f32 / 15.0 * 2.0 - 1.0);
                    let d = (xd * xd + yd * yd + zd * zd).sqrt();
                    xd /= d;
                    yd /= d;
                    zd /= d;
                    let mut power = radius * (0.7 + self.common.world.random.next_float() * 0.6);
                    let (mut xp, mut yp, mut zp) = (center[0], center[1], center[2]);
                    // A ray's steps are 0.3 long, about three to a block: a
                    // step in the block the last one was in takes what that
                    // one took (the level does not change while rays cast),
                    // and that block is in `set` already if it is to be.
                    let mut last: Option<([i32; 3], f32, bool)> = None;
                    while power > 0.0 {
                        let pos = [xp.floor() as i32, yp.floor() as i32, zp.floor() as i32];
                        if pos[1] < min_y
                            || pos[1] >= max_y
                            || !(-30_000_000..30_000_000).contains(&pos[0])
                            || !(-30_000_000..30_000_000).contains(&pos[2])
                        {
                            break;
                        }
                        let (taken, spared, again) = match last {
                            Some((at, taken, spared)) if at == pos => (taken, spared, true),
                            _ => {
                                let (resistance, spared) =
                                    self.cell_explosion_resistance(pos, &rules, explosion, None);
                                let taken = resistance.map_or(0.0, |r| (r + 0.3) * 0.3);
                                last = Some((pos, taken, spared));
                                (taken, spared, false)
                            }
                        };
                        power -= taken;
                        if power > 0.0 && !spared && !again {
                            set.put(Key(pos), ());
                        }
                        xp += xd * f64::from(0.3f32);
                        yp += yd * f64::from(0.3f32);
                        zp += zd * f64::from(0.3f32);
                        power -= 0.225_000_01;
                    }
                }
            }
        }
        set.entries().map(|(k, _)| k.0).collect()
    }
    /// `ServerExplosion.hurtEntities` for the player: damage scaled by
    /// distance and exposure, through a raised shield (`hurtServer` blocks
    /// explosions like any attack from where the blast is), then the push
    /// (less with explosion knockback resistance; none for spectators or a
    /// creative player flying).
    ///
    /// Returns the push their client adds from the explosion packet
    /// (`hitPlayers`), unless a hurt marked their velocity: that goes to them
    /// whole, this push in it, once their server copy has travelled
    /// (Entity.markHurt, then ServerEntity.sendChanges), and replaces what the
    /// packet would add.
    ///
    /// `cause` is the blast's source as predicates see it, read before the
    /// guests' turns swap their own combat state in.
    fn explosion_hurt_player(
        &mut self,
        context: &ExplosionHookContext<'_>,
        blast_attack: &sunrise_sim::health::Attack,
        radius: f32,
        damage_type: &str,
        exploding: Option<u32>,
        attacker: Option<&(u32, Text)>,
        cause: Option<&sunrise_sim::advancements::EntityView>,
    ) -> Option<[f64; 3]> {
        let ExplosionHookContext { center, blocks, .. } = *context;
        let flying = self.common.player.as_ref().is_some_and(|p| p.flying);
        let knockback = if flying {
            context.flying_knockback
        } else {
            context.knockback
        };
        let diameter = f64::from(radius * 2.0);
        // Blast protection: `EXPLOSION_KNOCKBACK_RESISTANCE` (0.15 per level and
        // piece, so two Blast Protection IV pieces cancel it).
        let resistance = self
            .common
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:explosion_knockback_resistance").ok())
            .unwrap_or(0.0);
        let p = self.common.player.as_ref()?;
        let feet = [p.position.x, p.position.y, p.position.z];
        let dist = ((feet[0] - center[0]).powi(2)
            + (feet[1] - center[1]).powi(2)
            + (feet[2] - center[2]).powi(2))
        .sqrt()
            / diameter;
        if dist > 1.0 {
            return None;
        }
        // `getEntities(this.source, ...)`: a player that is the blast's source is spared.
        let acting = self.server.lan_acting_entity.or(Some(p.entity_id as u32));
        if self.server.combat.0.explosion_source_named && exploding == acting {
            return None;
        }
        // ServerPlayer.onExplosionHit: the blast that hit the player is theirs.
        self.common.adv.marks.blast = Some((feet, cause.cloned()));
        let w = f64::from(p.dimensions.width) / 2.0;
        let h = f64::from(p.dimensions.height);
        let eye = [
            feet[0],
            feet[1] + f64::from(p.dimensions.eye_height),
            feet[2],
        ];
        // Neither hurt nor pushed: not checked for exposure.
        let exposure = if context.hurts || knockback != 0.0 {
            self.seen_percent(
                center,
                [feet[0] - w, feet[1], feet[2] - w],
                [feet[0] + w, feet[1] + h, feet[2] + w],
            )
        } else {
            0.0
        };
        let pow = (1.0 - dist) * f64::from(exposure);
        let amount = ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32;
        let d = [eye[0] - center[0], eye[1] - center[1], eye[2] - center[2]];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let dir = if l < 1.0e-4 {
            [0.0; 3]
        } else {
            [d[0] / l, d[1] / l, d[2] / l]
        };
        let power = pow * f64::from(knockback) * (1.0 - resistance);
        // `shouldDamageEntity`, then `source.doTeamsAllowDamage(entity)`, then
        // `ServerPlayer.hurtServer`'s `canHarmPlayer` of the player whose blast it is.
        let hurtable = context.hurts
            && exploding.is_none_or(|id| {
                sunrise_game_data::scoreboard::teams_allow_damage(
                    self.mob_team(id),
                    self.player_team(),
                )
            })
            && context.player_cause.as_ref().is_none_or(|(owner, team)| {
                self.player_may_harm(*owner, p.entity_id, team.as_ref(), self.player_team())
            });
        let blocking = self
            .player_blocking(Some(center), damage_type, false)
            .filter(|_| context.hurts);
        let raised = blocking.is_some();
        let (creative, spectator) = (self.common.creative, self.common.spectator());
        let mut outcome = sunrise_sim::health::BlockOutcome::default();
        let mut pushed = None;
        let explicit = context.explicit_cause.as_ref().and_then(|c| c.as_ref());
        let living_attacker = explicit.is_none_or(|c| c.player || c.species.is_some());
        let living_non_player = context
            .explicit_cause
            .as_ref()
            .map_or(attacker.is_some(), |c| {
                c.as_ref().is_some_and(|c| !c.player && c.species.is_some())
            });
        // The death message names the blast's causing entity. It goes to this
        // player's tracker, the host's or a guest's.
        if let Some((id, name)) = attacker.filter(|_| !creative && !spectator && hurtable) {
            if living_attacker && context.credit {
                self.note_player_attacker(
                    damage_type,
                    *id,
                    name.clone(),
                    context.held_item.clone(),
                );
            } else {
                self.note_uncredited_attacker(
                    damage_type,
                    *id,
                    name.clone(),
                    context.held_item.clone(),
                    living_attacker,
                );
            }
        } else if !creative
            && !spectator
            && hurtable
            && let Some(direct) = context.direct_name.clone()
        {
            self.note_direct_entity(damage_type, direct);
        }
        let rules = self.common.world.health_rules;
        // The blast's causing entity, lent to the hit's listeners: decided once, before the
        // player is borrowed for the hit.
        let causing = attacker.map_or(0, |(id, _)| *id as i32);
        let decision = crate::game::entity_server::causer_decision!(self, causing);
        if let Some(p) = self.common.player.as_mut()
            && let Some(id) = self.common.health_resources.damage.id(damage_type)
        {
            // Creative damage is attempted and refused by Player.hurtServer, not here.
            if !spectator
                && hurtable
                && let Ok(o) = p.hurt_caused_by(
                    crate::game::entity_server::damage_level_of!(
                        self,
                        blast_attack,
                        no_push,
                        crate::game::entity_server::player_victim_causer!(self, decision),
                        acting_equipment!(self.common)
                    ),
                    &self.common.health_resources,
                    rules,
                    id,
                    amount,
                    blocking,
                    living_non_player,
                    // The blast's causing entity.
                    attacker.map_or(0, |(id, _)| *id as i32),
                    false,
                )
            {
                outcome = o;
                // dealDefaultKnockback's `indicateDamage` points at the blast
                // (the source position), not the no-direction fallback.
                if o.knockback && o.blocked <= 0.0 {
                    p.health
                        .indicate_damage(center[0] - feet[0], center[2] - feet[2], p.yaw);
                }
            }
        }
        if !spectator {
            let mut push = [dir[0] * power, dir[1] * power, dir[2] * power];
            self.native_explosion_knockback(self.common.dim.id(), center, blocks, None, &mut push);
            if let Some(p) = self.common.player.as_mut() {
                p.velocity = Vec3::new(
                    p.velocity.x + push[0],
                    p.velocity.y + push[1],
                    p.velocity.z + push[2],
                );
                if !(creative && flying) {
                    // ServerExplosion.hitPlayers: the packet carries the push even when the hurt
                    // also marks the velocity (its motion packet follows).
                    pushed = Some(push);
                }
            }
        }
        self.server.combat.0.forget_noted(outcome.canceled);
        if raised {
            let source = mod_api::entity::health::HurtSource {
                damage_type,
                causing: attacker.map_or(0, |(id, _)| *id as i32),
                direct: blast_attack.direct.unwrap_or(0),
                position: Some(center),
                direct_type: None,
            };
            self.after_block(outcome, (None, false), source, blast_attack);
        }
        self.finish_guest_damage();
        pushed
    }

    /// The acting player, then every other player in the level (LAN guests), each swapped in
    /// as the player and hit when the blast's list visits them. The blast's source is the
    /// same for all of them (`onExplosionHit(this.source)`).
    fn explosion_hurt_players(
        &mut self,
        context: &ExplosionHookContext<'_>,
        blast_attack: &sunrise_sim::health::Attack,
        radius: f32,
        damage_type: &str,
        exploding: Option<u32>,
        attacker: Option<&(u32, Text)>,
        blast_source: Option<&sunrise_sim::advancements::EntityView>,
    ) -> Vec<(u32, [f64; 3])> {
        let own = if self
            .common
            .player
            .as_ref()
            .and_then(|p| u32::try_from(p.entity_id).ok())
            .is_some_and(|id| !self.server.combat.0.visits(id))
        {
            None
        } else {
            self.explosion_hurt_player(
                context,
                blast_attack,
                radius,
                damage_type,
                exploding,
                attacker,
                blast_source,
            )
        };
        let mut pushed: Vec<_> = self.server.lan_acting.zip(own).into_iter().collect();
        if let Some(host) = self.server.lan.as_ref() {
            let combat = &self.server.combat.0;
            let guests: Vec<u32> = host
                .hub
                .guests
                .iter()
                .filter(|g| host.hub.here(g))
                .filter(|g| u32::try_from(g.entity_id).map_or(true, |id| combat.visits(id)))
                .map(|g| g.conn)
                .collect();
            for conn in guests {
                let push = self.as_guest(conn, |g| {
                    g.explosion_hurt_player(
                        context,
                        blast_attack,
                        radius,
                        damage_type,
                        exploding,
                        attacker,
                        blast_source,
                    )
                });
                if let Some(Some(push)) = push {
                    pushed.push((conn, push));
                }
            }
        }
        pushed
    }

    /// Where a blast at `center` reaches `entity`, unless it ignores the blast
    /// or is out of range (`getEntities` and `ServerExplosion.hurtEntities`).
    /// Generic over the kind, so every walked list gets its own copy.
    fn blast_hit<E: Entity>(
        &self,
        entity: &E,
        center: [f64; 3],
        diameter: f64,
        blast: &Blast,
        seen: bool,
    ) -> Option<Hit> {
        if entity.ignores_explosion(blast) || !entity.pushed_by_explosion() {
            return None;
        }
        let feet = entity.position();
        let dist = blast_distance(center, [feet.x, feet.y, feet.z]) / diameter;
        if dist > 1.0 {
            return None;
        }
        let b = entity.explosion_box();
        let (lo, hi) = (b.min_position(), b.max_position());
        let exposure = if seen {
            self.seen_percent(center, [lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
        } else {
            0.0
        };
        let dir = blast_direction(
            center,
            [feet.x, feet.y + entity.explosion_eye_height(), feet.z],
        );
        let power = (1.0 - dist) * f64::from(exposure);
        Some(Hit {
            dist,
            exposure,
            push: Vec3::new(dir[0] * power, dir[1] * power, dir[2] * power),
        })
    }
    /// `ServerExplosion.hurtEntities` (player, mobs, items, primed TNT, minecarts);
    /// the player takes `damage_type`. Returns the players whose clients add a
    /// push from the explosion packet, and how much (`explosion_hurt_player`).
    fn explosion_hurt_entities(
        &mut self,
        center: [f64; 3],
        radius: f32,
        damage_type: &str,
        interaction: BlockInteraction,
        blocks: &[[i32; 3]],
        blast_source_spec: ExplosionSource,
    ) -> Vec<(u32, [f64; 3])> {
        if radius < 1.0e-5 {
            return Vec::new();
        }
        // The calculator's answers, read before the guests' turns swap their
        // own combat state in.
        let hurts = self.blast_damages_entities();
        let (knockback, flying_knockback) =
            (self.blast_knockback(false), self.blast_knockback(true));
        let kind = self.blast_damage_kind();
        // Without a listener's list the passes run kind by kind; with one, the walk below
        // reaches each entry's kind in the list's order.
        let listed = self.server.combat.0.blast_hit_list.is_some();
        if hurts && !listed {
            self.explosion_hits_special(
                center,
                radius,
                blocks,
                damage_type,
                blast_source_spec.direct(),
            );
        }
        let diameter = f64::from(radius * 2.0);
        let damage = |dist: f64, exposure: f32| {
            let pow = (1.0 - dist) * f64::from(exposure);
            ((pow * pow + pow) / 2.0 * 7.0 * diameter + 1.0) as f32
        };
        // `source.doTeamsAllowDamage(entity)`: the exploding mob spares its
        // team mates when its team has no friendly fire (they are still pushed).
        let exploding = self.server.combat.0.explosion_source;
        let teams_allow = |g: &Self, victim: Option<&sunrise_game_data::scoreboard::Team>| {
            exploding.is_none_or(|id| {
                sunrise_game_data::scoreboard::teams_allow_damage(g.mob_team(id), victim)
            })
        };
        // The player, then every other player in the level (LAN guests),
        // each swapped in as the player. In a guest's turn (their hit on an
        // end crystal) the player is that guest. The blast's source is the
        // same for all of them (`onExplosionHit(this.source)`).
        let attacker = self.blast_attacker();
        let blast_source = self.server.combat.0.explosion_cause.clone();
        let context = ExplosionHookContext {
            center,
            blocks,
            explicit_cause: self.server.combat.0.explicit_explosion_cause.clone(),
            direct_name: self.server.combat.0.explicit_explosion_direct.clone(),
            credit: self
                .server
                .combat
                .0
                .explicit_explosion_cause
                .as_ref()
                .is_none_or(|cause| {
                    cause
                        .as_ref()
                        .is_some_and(|cause| self.explosion_cause_resolves(cause))
                }),
            held_item: self
                .server
                .combat
                .0
                .explicit_explosion_cause
                .as_ref()
                .and_then(|c| c.as_ref())
                .and_then(|c| {
                    if c.player {
                        self.player_main_hand_custom_name(c.guest)
                    } else {
                        self.mob_main_hand_name(c.entity.0)
                    }
                }),
            hurts,
            knockback,
            flying_knockback,
            player_cause: self.blast_player_cause(),
        };
        // The blast's `DamageSource`, shared by every victim: the exploding or
        // explicitly supplied direct entity, its class and the blast's centre.
        let (blast_direct_id, blast_direct_kind) = blast_source_spec.damage_direct.map_or(
            (0, sunrise_sim::health::DirectEntity::Other),
            |(id, class)| (id as i32, class),
        );
        let mut blast_attack = self.server.combat.0.attack().clone();
        blast_attack.direct = Some(blast_direct_id);
        blast_attack.direct_kind = Some(blast_direct_kind);
        blast_attack.position = Some(center);
        if let Some(actor) = self.enchantment_actor(blast_direct_id) {
            blast_attack.enchantment_direct = Some(actor);
        }
        if blast_attack.attacker == 0
            && let Some(Some(cause)) = &self.server.combat.0.explicit_explosion_cause
            && cause.player
        {
            blast_attack.attacker = cause.entity.0 as i32;
        }
        if blast_attack.player_uuid.is_none() {
            if let Some(Some(cause)) = &self.server.combat.0.explicit_explosion_cause
                && cause.player
            {
                blast_attack.player_uuid = self.player_uuid_of(cause.guest);
            } else if self.server.combat.0.blast_owner_player
                && self.server.combat.0.explicit_explosion_cause.is_none()
            {
                blast_attack.player_uuid = self.player_uuid_of(self.server.combat.0.blast_guest);
            }
        }
        let mut pushed = if listed {
            // The list's own order reaches each player in the walk below.
            Vec::new()
        } else {
            self.explosion_hurt_players(
                &context,
                &blast_attack,
                radius,
                damage_type,
                exploding,
                attacker.as_ref(),
                blast_source.as_ref(),
            )
        };
        // Mobs. `WitherBoss.hurtServer`: a wither shrugs off an undead cause.
        let cause = self
            .server
            .combat
            .0
            .explicit_explosion_cause
            .as_ref()
            .and_then(|c| c.as_ref())
            .and_then(|c| c.species)
            .or_else(|| {
                self.server
                    .combat
                    .0
                    .explosion_owner
                    .or(exploding)
                    .and_then(|id| self.common.mobs.iter().find(|m| m.id == id))
                    .map(|m| m.species())
            });
        // `ServerExplosion.hurtEntities`: every entity in reach, on the walk. Each
        // answers `ignoreExplosion` itself (`Entity::ignores_explosion`); the rest
        // is the same for all, and only what a blast does to a kind differs. The
        // walk covers the entities there are now: what the blast drops or spawns
        // is not hit by it. Destroyed items and orbs are squeezed out afterwards
        // (as `retain` does), and a destroyed boat or cart shifts its list, so
        // `gone` counts how far.
        let blast = Blast {
            vehicles_ignore: matches!(self.blast_indirect_source(), Some(Who::Mob(_)))
                && !self
                    .common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING),
            items_ignore: items_ignore_explosion(self.blast_griefs(), interaction),
            spared: exploding.filter(|_| self.server.combat.0.explosion_source_named),
        };
        // `VehicleEntity.hurtServer` credits its ENTITY_DAMAGE to
        // `DamageSource.getEntity`, which for a blast is its indirect source.
        let blast_by = self.explosion_kill_credit();
        let mut destroyed_contents = Vec::new();
        macro_rules! blocked_damage {
            ($id:expr, $kind:expr) => {{
                let source = sunrise_sim::health::HurtSource {
                    damage_type,
                    causing: attacker.as_ref().map_or(0, |(id, _)| *id as i32),
                    direct: exploding.map_or(0, |id| id as i32),
                    position: Some(center),
                    direct_type: None,
                };
                let creative = self.creative_damage_source_id(source.causing);
                let vanilla = self.saved_refuses_damage($id as u32, damage_type, creative);
                crate::game::entity_server::damage_level_of!(
                    self,
                    self.server.combat.0.attack(),
                    no_push,
                    None
                )
                .invulnerability_check(
                    sunrise_sim::health::Hurting::Entity($id, $kind, source),
                    vanilla,
                )
            }};
        }
        // The walk visits the storages in storage order, unless a listening
        // `explosion_detonate` left the list, and then it visits exactly what
        // the list names, in its order (`explosion_detonate.rs`). The kinds
        // outside the storages (players, frames, paintings, cushions,
        // crystals, the dragon, hooks) answer no handle here: the player walk
        // and their own passes read the list for them.
        let mut cursor = self.entity_id_cursor();
        let mut listed_at = 0;
        loop {
            let next = if let Some(list) = self.server.combat.0.blast_hit_list.as_ref() {
                let Some(&id) = list.get(listed_at) else {
                    break;
                };
                listed_at += 1;
                // `ServerExplosion.hurtEntities` walks the edited list entry by entry, whatever
                // the kind and however often it is named.
                self.server.combat.0.blast_turn = Some(id);
                if self.blast_is_player(id) {
                    pushed.extend(self.explosion_hurt_players(
                        &context,
                        &blast_attack,
                        radius,
                        damage_type,
                        exploding,
                        attacker.as_ref(),
                        blast_source.as_ref(),
                    ));
                }
                if hurts {
                    self.explosion_hurt_listed_special(
                        id,
                        center,
                        radius,
                        blocks,
                        damage_type,
                        blast_source_spec.direct(),
                        diameter,
                        interaction,
                    );
                }
                self.blast_handle(id).map(|handle| (id, handle))
            } else {
                let Some(next) = self.next_entity_id(&mut cursor) else {
                    break;
                };
                Some(next)
            };
            let Some((entity, handle)) = next else {
                continue;
            };
            // A callback may discard a later original without sweeping its
            // storage yet. Removed entities no longer receive blast callbacks.
            // A discarded mob stays in its storage flagged removed, and its
            // selector target is gone, so the flag is read on its own.
            if with_entity!(self, handle, e => e.is_removed())
                || with_entity!(self, handle, e => e.selector_target())
                    .is_some_and(|target| self.target_removed(&target))
            {
                continue;
            }
            // An entity the blast's list does not name is neither hurt nor
            // pushed; without a listener the storages are the walk.
            let hit = if with_entity!(self, handle, e => self.server.combat.0.visits(e.id())) {
                let seen = hurts || knockback != 0.0;
                with_entity!(self, handle, e => self.blast_hit(e, center, diameter, &blast, seen))
            } else {
                None
            };
            let mut destroyed = false;
            if let Some(Hit {
                dist,
                exposure,
                push,
            }) = hit
            {
                let amount = damage(dist, exposure);
                match handle {
                    _ if !hurts => {}
                    EntityHandle::Mob(i) => {
                        let victim = self.common.mobs[i].id;
                        let hurtable = teams_allow(self, self.mob_team(victim));
                        // The exploding mob is the damage source's entity, so
                        // victims it kills drop its charged-creeper loot
                        // (`Creeper.killedEntity`).
                        let mut source = sunrise_sim::mobs::Damage {
                            kind,
                            // A projectile's owner (`getIndirectSourceEntity`),
                            // else the exploding mob itself.
                            attacker: self
                                .server
                                .combat
                                .0
                                .explicit_explosion_cause
                                .as_ref()
                                .and_then(|c| c.as_ref())
                                .filter(|c| c.player)
                                .map(|_| Who::Player)
                                .or_else(|| {
                                    let player = self.server.combat.0.blast_owner_player
                                        && self.server.combat.0.explicit_explosion_cause.is_none();
                                    player.then_some(Who::Player)
                                })
                                .or_else(|| {
                                    attacker
                                        .as_ref()
                                        .map(|&(id, _)| id)
                                        .filter(|&o| o != victim)
                                        .filter(|_| {
                                            self.server
                                                .combat
                                                .0
                                                .explicit_explosion_cause
                                                .as_ref()
                                                .is_none_or(|c| {
                                                    c.as_ref().is_some_and(|c| c.species.is_some())
                                                })
                                        })
                                        .map(sunrise_sim::mobs::Who::Mob)
                                }),
                            from: Some(center),
                            direct: None,
                            attacker_species: cause,
                        };
                        // The blast damage source's direct entity (`getDirectEntity`): the primed
                        // TNT, cart, fireball, skull or crystal that went off, or an explicit
                        // source's. An exploding mob that is also the attacker deals it itself.
                        let attacked_by_itself = blast_source_spec.damage_direct.as_ref().is_some_and(
                            |(id, _)| {
                                matches!(source.attacker, Some(sunrise_sim::mobs::Who::Mob(m)) if m == *id)
                            },
                        );
                        if !attacked_by_itself {
                            source.direct = blast_source_spec.damage_direct_kind;
                        }
                        // `EXPLOSION_KNOCKBACK_RESISTANCE`, not `KNOCKBACK_RESISTANCE`:
                        // its base is 0 for every mob type, so iron golems and
                        // ravagers are pushed.
                        let plan = self.prepare_knockback_credit(
                            victim as i32,
                            source.attacker,
                            &blast_attack,
                        );
                        // The blast's causing entity, lent beside its victim.
                        let causing = attacker.as_ref().map_or(0, |&(id, _)| id as i32);
                        let decision = crate::game::entity_server::causer_decision!(self, causing);
                        let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                        let (victim_mob, lend) = crate::game::entity_server::victim_and_causer!(
                            self, i, causing, decision
                        );
                        let hurt = hurt_mob(
                            crate::game::entity_server::damage_level_of!(
                                self,
                                &blast_attack,
                                &mut credit,
                                lend
                            ),
                            victim_mob,
                            hurtable,
                            cause,
                            (amount, source),
                        );
                        let hurt = self
                            .complete_mob_hurt(i, hurt)
                            .is_some_and(|(_, hurt)| hurt);
                        self.settle_mob_hurt(victim);
                        // The loan recorded the accepted hit's player credit
                        // (`resolvePlayerResponsibleForDamage`); a guest's also names its connection.
                        if hurt
                            && source.attacker == Some(Who::Player)
                            && let Some(host) = self.server.lan.as_mut()
                        {
                            match self.server.combat.0.blast_guest {
                                Some(conn) => {
                                    host.mob_players.insert(victim, conn);
                                }
                                None => {
                                    host.mob_players.remove(&victim);
                                }
                            }
                        }
                        self.finish_mob_damage_at(i);
                        // Enderman.hurtServer teleports inside the hurt, before
                        // the push (the mob order stays as it was).
                        if let Err(e) = self.settle_hurt_teleports() {
                            log::warn!("Sunrise: explosion: enderman teleport: {e}");
                        }
                    }
                    // `ItemEntity.hurtServer`: the damage comes off `Health`
                    // (5 to start) in f32 and the item is destroyed at zero, so
                    // a survivor keeps the health the blast left it. Stack damage
                    // resistance is resolved against this blast's damage type. A
                    // blast with a source entity under `mobGriefing` off never
                    // reaches here (`items_ignore`).
                    EntityHandle::Item(i) => {
                        let id = self.common.dropped[i].id as i32;
                        let blocked = blocked_damage!(id, "minecraft:item");
                        let Some(EntityHandle::Item(i)) =
                            self.refresh_entity_handle(entity, handle)
                        else {
                            continue;
                        };
                        let can_hurt = self
                            .stack_can_be_hurt_by(&self.common.dropped[i].stack, damage_type)
                            .unwrap_or(true);
                        let item = &mut self.common.dropped[i];
                        if !blocked && can_hurt {
                            item.health = (item.health as f32 - amount) as i32;
                            destroyed = item.health <= 0;
                            if destroyed {
                                let at = item.position;
                                // Health was already reduced above. Fire `ItemStack.onDestroyed`
                                // with the original intact stack before spilling and discarding.
                                let spilled = self.destroy_item_entity_drops(i, damage_type);
                                destroyed_contents
                                    .extend(spilled.into_iter().map(|stack| (stack, at)));
                            }
                        }
                    }
                    // `ExperienceOrb.hurtServer` takes the damage off their 5 health.
                    EntityHandle::Orb(i) => {
                        let id = self.common.orbs[i].id as i32;
                        if !blocked_damage!(id, "minecraft:experience_orb") {
                            let Some(EntityHandle::Orb(i)) =
                                self.refresh_entity_handle(entity, handle)
                            else {
                                continue;
                            };
                            destroyed = self.common.orbs[i].hurt(amount);
                        }
                    }
                    // `VehicleEntity.hurtServer` (explosions ignite TNT carts);
                    // a destroyed one is gone before the push.
                    EntityHandle::Cart(i) => {
                        let kind = self.common.carts[i].kind.id();
                        if !blocked_damage!(entity as i32, kind) {
                            let Some(EntityHandle::Cart(i)) =
                                self.refresh_entity_handle(entity, handle)
                            else {
                                continue;
                            };
                            self.hurt_cart(
                                i,
                                amount,
                                false,
                                true,
                                super::minecarts_server::CartHit::attributed(blast_by),
                            );
                        }
                    }
                    // `VehicleEntity.hurtServer`: damage * 10, broken past 40.
                    EntityHandle::Boat(i) => {
                        let kind = self.common.boats[i].entity_type();
                        if !blocked_damage!(entity as i32, &kind) {
                            let Some(EntityHandle::Boat(i)) =
                                self.refresh_entity_handle(entity, handle)
                            else {
                                continue;
                            };
                            self.hurt_boat(i, amount, false);
                        }
                    }
                    // `hurtServer` does nothing to the rest.
                    _ => {}
                }
                if destroyed {
                    match self.refresh_entity_handle(entity, handle) {
                        Some(EntityHandle::Item(i)) => {
                            self.common.dropped.remove(i);
                        }
                        Some(EntityHandle::Orb(i)) => {
                            self.common.orbs.remove(i);
                        }
                        _ => {}
                    }
                }
                let k = f64::from(knockback);
                let mut velocity = [push.x * k, push.y * k, push.z * k];
                self.native_explosion_knockback(
                    self.common.dim.id(),
                    center,
                    blocks,
                    Some(entity),
                    &mut velocity,
                );
                let push = Vec3::new(velocity[0], velocity[1], velocity[2]);
                let Some(handle) = self.refresh_entity_handle(entity, handle) else {
                    continue;
                };
                match handle {
                    EntityHandle::Item(_) | EntityHandle::Orb(_) if destroyed => {}
                    EntityHandle::Cart(i) => self
                        .common
                        .carts
                        .update(i, |cart| cart.push_from_explosion(push)),
                    EntityHandle::Boat(i) => self
                        .common
                        .boats
                        .update(i, |boat| boat.push_from_explosion(push)),
                    other => with_entity_mut!(self, other, e => e.push_from_explosion(push)),
                }
            }
        }
        // The original target walk is finished: released contents cannot be
        // hurt or pushed by the explosion that destroyed their container.
        self.spawn_destroyed_contents(destroyed_contents);
        self.native_leaves_flush();
        // Frames, paintings, cushions and armour stands last: vanilla lists
        // the entities in reach before hurting any, so the items they drop
        // here are neither destroyed nor pushed by the same blast.
        // BlockAttachedEntity.kill / ArmorStand.kill credit DamageSource.getEntity.
        // VehicleEntity.destroy above uses Entity.kill and keeps the vehicle as source.
        self.server.combat.0.blast_turn = None;
        if hurts && !listed {
            self.explosion_breaks_frames(center, diameter, interaction);
            self.explosion_breaks_decorations(center, diameter, interaction);
        }
        pushed
    }

    /// Whether `id` names the acting player or a guest here.
    fn blast_is_player(&self, id: u32) -> bool {
        self.common
            .player
            .as_ref()
            .is_some_and(|p| u32::try_from(p.entity_id) == Ok(id))
            || self.server.lan.as_ref().is_some_and(|host| {
                host.hub
                    .guests
                    .iter()
                    .any(|g| u32::try_from(g.entity_id) == Ok(id) && host.hub.here(g))
            })
    }

    /// The listed entry `id`'s turn for the kinds outside the storage walk: an end crystal or
    /// the dragon, an item frame, a painting, cushion or armour stand. The passes read the
    /// turn, so only that entity is reached.
    fn explosion_hurt_listed_special(
        &mut self,
        id: u32,
        center: [f64; 3],
        radius: f32,
        blocks: &[[i32; 3]],
        damage_type: &str,
        blast_direct: Option<(u32, Option<&'static str>)>,
        diameter: f64,
        interaction: BlockInteraction,
    ) {
        if self.common.crystals.iter().any(|c| c.id == id)
            || self.common.dragon.as_ref().is_some_and(|d| d.id == id)
        {
            self.explosion_hits_special(center, radius, blocks, damage_type, blast_direct);
        }
        if self.common.item_frames.iter().any(|f| f.id == id) {
            self.explosion_breaks_frames(center, diameter, interaction);
        }
        if self.common.paintings.iter().any(|p| p.id == id)
            || self.common.cushions.iter().any(|c| c.id == id)
            || self.common.armor_stands.iter().any(|a| a.id == id)
        {
            self.explosion_breaks_decorations(center, diameter, interaction);
        }
    }

    /// Decoration kills credit `DamageSource.getEntity`: the explicit blast
    /// cause, including a TNT igniter or the player who broke an end crystal.
    pub(in crate::game::server_side) fn explosion_kill_credit(
        &self,
    ) -> Option<super::commands::Target> {
        use super::commands::Target;
        // An explicit crystal or cart source names its own cause.
        if let Some(Some(cause)) = &self.server.combat.0.explicit_explosion_cause {
            return Some(if cause.player {
                cause.guest.map_or(Target::Player, Target::Guest)
            } else {
                Target::Mob(cause.entity.0)
            });
        }
        // A primed TNT's owner names the blast before any mob does; a mob's UUID
        // in its `owner` is that mob, not the player in place.
        if let Some((id, _)) = self.server.combat.0.blast_owner {
            return Some(if self.server.combat.0.blast_owner_player {
                self.server
                    .combat
                    .0
                    .blast_guest
                    .map_or(Target::Player, Target::Guest)
            } else {
                Target::Mob(id)
            });
        }
        if let Some((id, _)) = self.blast_attacker() {
            return Some(Target::Mob(id));
        }
        self.crystal_blast_player()
    }
    /// Decoration hurt checks use the damage source's cause (`instanceof Mob`),
    /// unlike vehicles, which use the explosion's indirect source.
    pub fn blast_spares_decorations(&self, interaction: BlockInteraction) -> bool {
        blast_spares_decorations(
            self.blast_griefs(),
            interaction,
            self.blast_mob_refused(),
            self.server.combat.0.explosion_in_water,
        )
    }
    /// `ServerExplosion.shouldAffectBlocklikeEntities`'s griefing read:
    /// `canEntityGrief` for the exploding entity (`this.source`), the rule without one.
    fn blast_griefs(&self) -> bool {
        self.can_entity_grief(self.server.combat.0.explosion_source)
    }
    /// A mob causes the blast (`source.getEntity() instanceof Mob`) and
    /// `canEntityGrief` refuses it: decorations take no harm.
    fn blast_mob_refused(&self) -> bool {
        // No listener and the rule on: no mob is refused, and no cause is looked up.
        if !crate::game::ServerGame::grief_hooked(self)
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
        {
            return false;
        }
        // The mob the damage names (an explicit cause only when it is a mob), found once.
        let mob = match &self.server.combat.0.explicit_explosion_cause {
            Some(cause) => cause
                .as_ref()
                .filter(|c| c.species.is_some())
                .map(|c| c.entity.0),
            None => self.blast_attacker().map(|(id, _)| id),
        };
        mob.is_some_and(|id| !self.can_entity_grief(Some(id)))
    }
    /// `Entity.isInWater` for an entity box: `updateFluidInteraction` over the box.
    pub(in crate::game::server_side) fn bounds_in_water(
        &self,
        bounds: sunrise_core::geometry::Aabb,
    ) -> bool {
        self.mob_world_base(&[])
            .fluids(bounds, bounds.max_position().y)
            .in_water
    }
    /// `ServerExplosion.hurtEntities` for paintings, cushions and armour
    /// stands (item frames: `explosion_breaks_frames`). A painting or cushion
    /// in reach is killed and dropped (`BlockAttachedEntity.hurtServer`,
    /// `Painting.dropItem`, `Cushion.dropItem`). An armour stand that is
    /// neither invisible nor a marker breaks (`ArmorStand.hurtServer` with
    /// `IS_EXPLOSION`: `brokenByAnything`, then `kill`): its equipment drops,
    /// the stand itself does not.
    fn explosion_breaks_decorations(
        &mut self,
        center: [f64; 3],
        diameter: f64,
        interaction: BlockInteraction,
    ) {
        // An armour stand is no `BlockAttachedEntity`: a source in water does not spare it.
        let spares_stands = blast_spares_decorations(
            self.blast_griefs(),
            interaction,
            self.blast_mob_refused(),
            false,
        );
        if !self.blast_spares_decorations(interaction) {
            self.explosion_breaks_hanging(center, diameter);
        }
        if spares_stands {
            return;
        }
        // `DamageSources.explosion`: a caused blast is `player_explosion`.
        let cause = self.blast_attacker().map(|(id, _)| id);
        let damage_type = self.blast_damage_type();
        let causing = cause.unwrap_or(0);
        let mut i = 0;
        while i < self.common.armor_stands.len() {
            let a = &self.common.armor_stands[i];
            let p = a.position;
            if !super::armor_stands::spared_by_blast(a)
                && !self.target_removed(&super::commands::Target::Stand(a.id))
                && self.server.combat.0.visits(a.id)
                && blast_reaches(center, diameter, [p.x, p.y, p.z])
            {
                let id = a.id;
                let source = sunrise_sim::health::HurtSource {
                    damage_type,
                    causing: causing as i32,
                    direct: causing as i32,
                    position: Some(center),
                    direct_type: None,
                };
                let creative = self.creative_damage_source_id(source.causing);
                let vanilla = self.saved_refuses_damage(id, damage_type, creative);
                if crate::game::entity_server::damage_level_of!(
                    self,
                    self.server.combat.0.attack(),
                    no_push,
                    None
                )
                .invulnerability_check(
                    sunrise_sim::health::Hurting::ArmorStand(id as i32, source),
                    vanilla,
                ) {
                    i += 1;
                    continue;
                }
                // The invulnerability callback can move or remove the stand.
                let index = if self
                    .common
                    .armor_stands
                    .get(i)
                    .is_some_and(|stand| stand.id == id)
                {
                    Some(i)
                } else {
                    self.common
                        .armor_stands
                        .iter()
                        .position(|stand| stand.id == id)
                };
                if let Some(index) = index {
                    self.break_armor_stand_by_anything(index, damage_type, causing);
                }
                // An accepted removal advances the list under us. If the row
                // stayed, advance the cursor ourselves, even after a refusal.
                if self
                    .common
                    .armor_stands
                    .get(i)
                    .is_some_and(|stand| stand.id == id)
                {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }
    /// `ExplosionEvent.Start` for the explosion `start`: true when a mod
    /// cancels it, and nothing of it happens.
    pub(in crate::game::server_side) fn explosion_start_cancelled(
        &mut self,
        start: &ExplosionStart<'_>,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ExplosionStart) {
            return false;
        }
        let mut cancel = false;
        self.native_explosion_start(start, &mut cancel);
        cancel
    }
    /// Paintings and cushions in reach (`BlockAttachedEntity.hurtServer`).
    fn explosion_breaks_hanging(&mut self, center: [f64; 3], diameter: f64) {
        let mut i = 0;
        while i < self.common.paintings.len() {
            // HangingEntity.recalculateBoundingBox: a painting stands at the
            // centre of its box.
            let (lo, hi) = self.common.paintings[i].bounds();
            let at = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
            // A painting a mod keeps stays at `i`.
            if !(self.server.combat.0.visits(self.common.paintings[i].id)
                && blast_reaches(center, diameter, at)
                && self.break_painting_by(i, false, self.explosion_kill_credit()))
            {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.common.cushions.len() {
            let p = self.common.cushions[i].position;
            if !(self.server.combat.0.visits(self.common.cushions[i].id)
                && blast_reaches(center, diameter, [p.x, p.y, p.z])
                && self.break_cushion_by(i, self.explosion_kill_credit()))
            {
                i += 1;
            }
        }
    }
    /// `ServerLevel.getDestroyType`: a drop decay rule picks between decaying
    /// and plain destruction.
    pub(in crate::game) fn destroy_type(
        &self,
        rule: sunrise_game_data::gamerules::RuleIndex,
    ) -> BlockInteraction {
        if self.common.ui.rules.bool_at(rule) {
            BlockInteraction::DestroyWithDecay
        } else {
            BlockInteraction::Destroy
        }
    }
    /// Runs an explosion whose source empties the hives it destroys
    /// (`BeehiveBlock.getDrops`: primed TNT, creeper, wither skull, wither,
    /// TNT minecart).
    pub(in crate::game::server_side) fn with_hive_emptying_blast<R>(
        &mut self,
        blast: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.server.combat.0.hive_emptying_blast = true;
        let result = blast(self);
        self.server.combat.0.hive_emptying_blast = false;
        result
    }
    /// `Level.explode` on the server: positions, entities, blocks, fire; then
    /// the client's explosion sound.
    pub fn server_explode(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
    ) -> Result<()> {
        self.server_explode_from(
            center,
            radius,
            interaction,
            fire,
            ExplosionSource::default(),
        )
    }
    /// `ServerLevel.explode`: only the direct source's `isSilent` suppresses
    /// the sound; terrain, entity damage and the visual packet are unchanged.
    pub fn server_explode_from(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        source: ExplosionSource,
    ) -> Result<()> {
        self.server_explode_from_cause(center, radius, interaction, fire, source, None)
    }
    /// An explicit `DamageSource` is independent of the blast's indirect owner.
    /// `Some(None)` is explicitly ownerless; `None` keeps the ordinary source.
    pub fn server_explode_from_cause(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        source: ExplosionSource,
        explicit: Option<ExplicitBlastSource>,
    ) -> Result<()> {
        let source = match &explicit {
            Some(e) => source.with_damage_direct(e.direct.0, Some(e.direct.1), e.direct.2),
            None => source,
        };
        let blast = |g: &mut Self| {
            let damage_type = g.blast_damage_type();
            g.server_explode_as_source(center, radius, interaction, fire, damage_type, None, source)
        };
        match explicit {
            // A player causes it (a lit TNT's igniter, a mod blast's source):
            // their hits land as that player's.
            None if self.server.combat.0.blast_owner_player => {
                let attack = self.blast_owner_attack();
                self.with_attack(attack, blast)
            }
            None => blast(self),
            Some(ExplicitBlastSource {
                direct: (id, kind, _),
                cause,
            }) => {
                let direct = (id, self.explosion_entity_name(id, kind));
                self.with_explosion_cause(cause, direct, blast)
            }
        }
    }

    /// The blast going off now as the hooks hear it, with the source and
    /// causing entity the combat state holds.
    pub(in crate::game::server_side) fn explosion_start<'a>(
        &self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        damage_type: &'a str,
    ) -> ExplosionStart<'a> {
        let entity = |id| {
            Some(mod_api::EntityRef::new(
                self.common.native_lifecycle.epoch()?,
                id,
            ))
        };
        ExplosionStart::from(mod_api::__private::built::ExplosionStart {
            dimension: self.common.dim.key(),
            center,
            radius,
            fire,
            interaction: match interaction {
                BlockInteraction::Keep => mod_api::BlockInteraction::Keep,
                BlockInteraction::Destroy => mod_api::BlockInteraction::Destroy,
                BlockInteraction::DestroyWithDecay => mod_api::BlockInteraction::DestroyWithDecay,
                BlockInteraction::Trigger => mod_api::BlockInteraction::TriggerBlock,
            },
            damage_type,
            source: self.server.combat.0.explosion_source.and_then(entity),
            // `getIndirectSourceEntity`, as the blast's attacker is.
            owner: self.blast_attacker().and_then(|(id, _)| entity(id)),
        })
    }
    /// `server_explode` with the damage type the player takes and a block
    /// whose explosion resistance the damage calculator overrides.
    pub fn server_explode_as(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        damage_type: &str,
        resists: Option<([i32; 3], f32)>,
    ) -> Result<()> {
        self.server_explode_as_source(
            center,
            radius,
            interaction,
            fire,
            damage_type,
            resists,
            ExplosionSource::default(),
        )
    }
    fn server_explode_as_source(
        &mut self,
        center: [f64; 3],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        damage_type: &str,
        resists: Option<([i32; 3], f32)>,
        source: ExplosionSource,
    ) -> Result<()> {
        let start = self.explosion_start(center, radius, interaction, fire, damage_type);
        if self.common.is_logical_server() {
            if self.explosion_start_cancelled(&start) {
                return Ok(());
            }
            self.native_explosion_prime(center, radius);
        }
        // Explosion.explode: the blast is heard.
        let ctx = sunrise_level::sculk::Context::default();
        self.post_game_event("minecraft:explode", center, ctx);
        let mut targets =
            self.exploded_positions(center, radius, resists, source.portal_proof, &start);
        // `shouldBlockExplode`: a calculator that explodes no blocks keeps none.
        if !self.blast_explodes_blocks() {
            targets.clear();
        }
        // The walks that hurt entities follow the list a listening
        // `explosion_detonate` leaves: a blast set off inside one of them
        // (its own detonate, a nested `level_explode`) keeps its own list.
        let outer = std::mem::take(&mut self.server.combat.0.blast_hit_list);
        let outer_ids = std::mem::take(&mut self.server.combat.0.blast_hit_ids);
        let outer_turn = self.server.combat.0.blast_turn.take();
        self.explosion_detonate(center, radius, &mut targets);
        let pushed = self.explosion_hurt_entities(
            center,
            radius,
            damage_type,
            interaction,
            &targets,
            source,
        );
        self.server.combat.0.blast_hit_list = outer;
        self.server.combat.0.blast_hit_ids = outer_ids;
        self.server.combat.0.blast_turn = outer_turn;
        self.explosion_block_step(&mut targets, radius, interaction, fire, &start)?;
        let effects = self.blast_effects();
        if !source.silent {
            let pitch = (1.0 + (client_float() - client_float()) * 0.2) * 0.7;
            self.server_play_sound(&effects.sound, center, 4.0, pitch);
        }
        // ServerLevel.explode sends toBlow.size() whatever the interaction, and
        // `ServerExplosion.isSmall` also covers an explosion that keeps blocks.
        let block_count = targets.len();
        let small = radius < 2.0 || interaction == BlockInteraction::Keep;
        self.lan_explode(center, small, radius, block_count, pushed, effects);
        Ok(())
    }
    /// `InfestedBlock.spawnAfterBreak` without a tool that prevents it: while
    /// blocks drop, its silverfish (`spawnInfestation`) comes out at the
    /// bottom centre of the block, with its spawn animation.
    pub(in crate::game::server_side) fn spawn_infestation(&mut self, pos: [i32; 3]) {
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
        {
            return;
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        if let Some(id) = self.spawn_mob(
            "minecraft:silverfish",
            at,
            false,
            sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
        ) {
            self.mob_poof(id);
        }
    }
    /// `BlockBehaviour.onExplosionHit` (TNT: `wasExploded` primes a short fuse).
    /// The blast's block step on `targets` (`ServerExplosion.explode` from
    /// `interactWithBlocks` to `createFire`): a trigger blast works the
    /// blocks it may; one that breaks blocks shuffles them with the level's
    /// random and hits each, its drops merged across all of them and popped
    /// after the last; then, with `fire`, a third of the cells left air above
    /// a solid block catch fire, whatever the interaction. The number of
    /// blocks hit: the ones that were not air when their turn came.
    pub(in crate::game) fn explosion_block_step(
        &mut self,
        targets: &mut [[i32; 3]],
        radius: f32,
        interaction: BlockInteraction,
        fire: bool,
        start: &ExplosionStart<'_>,
    ) -> Result<usize> {
        let mut hit = 0;
        if interaction == BlockInteraction::Trigger {
            self.explosion_trigger_blocks(targets)?;
        } else if interaction != BlockInteraction::Keep {
            // Util.shuffle(list, level.random)
            for i in (2..=targets.len()).rev() {
                let swap = self
                    .common
                    .world
                    .random
                    .next_int_bound(i as i32)
                    .unwrap_or(0) as usize;
                targets.swap(i - 1, swap);
            }
            let mut drops = Vec::new();
            for &pos in targets.iter() {
                if self.explosion_hit_block(pos, radius, interaction, start, &mut drops)? {
                    hit += 1;
                }
            }
            for (pos, stack) in drops {
                self.pop_resource(pos, stack);
            }
        }
        if fire {
            for &pos in targets.iter() {
                if self.common.world.random.next_int_bound(3).unwrap_or(1) == 0
                    && self
                        .common
                        .res
                        .facts()
                        .is_air(self.common.world.block(pos[0], pos[1], pos[2]))
                    && pos[1].checked_sub(1).is_some_and(|below| {
                        self.common
                            .res
                            .facts()
                            .get(self.common.world.block(pos[0], below, pos[2]))
                            .solid_render
                    })
                {
                    // BaseFireBlock.getState: soul fire on its base blocks,
                    // fire shaped by its neighbours elsewhere.
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    let fire =
                        self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, at));
                    self.level_set(pos, fire)?;
                }
            }
        }
        Ok(hit)
    }
    pub fn explosion_hit_block(
        &mut self,
        pos: [i32; 3],
        radius: f32,
        interaction: BlockInteraction,
        explosion: &ExplosionStart<'_>,
        dropped: &mut Vec<([i32; 3], ItemStack)>,
    ) -> Result<bool> {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if self.common.res.facts().is_air(state) {
            return Ok(false);
        }
        let name = self.common.res.facts().get(state).name.clone();
        let is_tnt = &*name == "minecraft:tnt";
        // CreakingHeartBlock.onExplosionHit: its creaking dies, and a blast
        // whose indirect source is a player (a primed TNT's owner) awards
        // the heart's experience (`tryAwardExperience`).
        if &*name == "minecraft:creaking_heart" {
            // tryAwardExperience checks the TNT owner's own abilities.
            let award = match self.server.combat.0.blast_guest {
                Some(conn) => self
                    .command_guest(conn)
                    .is_some_and(|g| !g.creative() && !g.spectator()),
                None => {
                    self.server.combat.0.blast_owner_player
                        && !self.common.creative
                        && !self.common.spectator()
                }
            };
            self.creaking_heart_broken_by(pos, award);
        }
        // Block.onExplosionHit: the drops copy from the block entity, which
        // then spills its contents as the block turns to air.
        let is_hive = matches!(&*name, "minecraft:beehive" | "minecraft:bee_nest");
        // BeehiveBlock.getDrops: the bees of a hive blown up by TNT, a
        // creeper, a wither skull, a wither or a TNT minecart come out.
        if is_hive && self.server.combat.0.hive_emptying_blast {
            let facing = self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("facing").cloned());
            self.empty_hive(
                pos,
                sunrise_sim::beehive::facing_step(facing.as_deref()),
                sunrise_sim::beehive::Release::Emergency,
            );
        }
        // Block.dropFromExplosion (only TntBlock refuses), or what the
        // block's parts answer: it gates spawnAfterBreak and the loot.
        let drops = self
            .part_drops_from_explosion(pos, state, explosion)
            .unwrap_or(!is_tnt);
        // Block.onExplosionHit runs spawnAfterBreak before the drops.
        if drops && name.starts_with("minecraft:infested_") {
            self.spawn_infestation(pos);
        }
        // Its dropExperience is whether the blast's indirect source is a
        // player (a primed TNT's owner, not a mob's): ores, sculk and spawners
        // sample their experience then, and pop it while blocks drop.
        if drops && self.server.combat.0.blast_owner_player {
            let xp = sunrise_world::level::removal::break_experience(
                &mut self.common.world.random,
                &name,
            )
            .unwrap_or(0);
            if xp > 0
                && self
                    .common
                    .ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
            {
                self.spawn_orbs(pos.map(|v| f64::from(v) + 0.5), xp);
            }
        }
        if drops {
            // Capture the entity the admitted roll actually sees. Spill, on removal below,
            // uses the stored slots rather than the serializer's output.
            if let Some(saved) = self.loot_block_entity(pos, &name) {
                self.common
                    .pending
                    .record_from(&self.common.world, pos, saved);
            }
            let dynamic = self.common.pending.dynamic(pos);
            // LootContextParams.EXPLOSION_RADIUS under DESTROY_WITH_DECAY: the
            // tables' survives_explosion and explosion_decay draw from their
            // random sequence.
            let decay = (interaction == BlockInteraction::DestroyWithDecay).then_some(radius);
            let place = self
                .common
                .loot_place_at(pos.map(|v| f64::from(v) + 0.5), None);
            // `Explosion.getDirectSourceEntity`.
            let entity = self
                .server
                .combat
                .0
                .explosion_direct
                .map(|(id, _)| id)
                .or(self.server.combat.0.explosion_source);
            let mut drops = sunrise_world::loot::with_native_loot_place(place, || {
                let roll = sunrise_items::items::BlockLootRoll {
                    dynamic: &dynamic,
                    explosion: decay,
                    ..Default::default()
                };
                self.roll_block_loot(pos, state, entity, roll)
            });
            self.native_block_loot(pos, state, None, None, Some(radius), &mut drops);
            let items = self.common.res.items();
            for stack in drops {
                if &*stack.item == "minecraft:air" || stack.count == 0 {
                    continue;
                }
                add_or_append_stack(dropped, stack, pos, &|s| {
                    items.get(&s.item).map_or(64, |i| i.max_stack)
                });
            }
        }
        self.common.pending.clear(pos);
        self.notify_block_exploded(pos, state, explosion);
        self.level_set(pos, self.common.world.air)?;
        // BeehiveBlock.onExplosionHit: the bees about turn on the player.
        if is_hive {
            self.anger_nearby_bees(pos);
        }
        // TntBlock.wasExploded: new PrimedTnt (its push drawn first), then
        // getRandomShortFuse(80); no game event.
        if is_tnt
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
        {
            let r = &mut self.common.world.random;
            let rot = r.next_double() * f64::from(std::f32::consts::TAU);
            let fuse = r.next_int_bound(20).unwrap_or(0) + 10;
            // TntBlock.wasExploded hands the new TNT the blast's indirect
            // source, so a chain reaction keeps a primed TNT's owner or the
            // mob whose blast primed it.
            let (owner, guest) = match self.blast_indirect_source() {
                Some(Who::Player) => (Some(Who::Player), self.server.combat.0.blast_guest),
                Some(Who::Mob(id)) => (Some(Who::Mob(id)), None),
                None => (None, None),
            };
            self.add_primed_tnt(
                BlockPos::new(pos[0], pos[1], pos[2]),
                rot,
                fuse,
                owner,
                guest,
            );
        }
        Ok(true)
    }
    /// ServerExplosion: rays from a 16^3 grid lose (resistance + 0.3) * 0.3 per step.
    /// A mob explosion (`ServerLevel.explode` with `ExplosionInteraction.MOB`:
    /// blocks are kept without mobGriefing, else the mob_explosion_drop_decay
    /// rule picks decay).
    pub fn explode(
        &mut self,
        source: Option<u32>,
        center: [f64; 3],
        radius: f32,
        blast: ExplosionSource,
    ) -> Result<()> {
        self.explode_with(source, center, radius, false, blast)
    }
    /// `ServerLevel.explode` with `ExplosionInteraction.MOB` and the caller's fire flag.
    pub(in crate::game::server_side) fn explode_mob(
        &mut self,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        blast: ExplosionSource,
    ) -> Result<()> {
        self.explode_with(None, center, radius, fire, blast)
    }
    /// `SulfurCube.tickFuse`: `ExplosionInteraction.TNT` (the
    /// tnt_explosion_drop_decay rule) with mobGriefing, `NONE` without.
    pub(in crate::game::server_side) fn explode_sulfur_cube(
        &mut self,
        source: Option<u32>,
        center: [f64; 3],
        radius: f32,
        blast: ExplosionSource,
    ) -> Result<()> {
        let interaction = if !self.can_entity_grief(source) {
            BlockInteraction::Keep
        } else {
            self.destroy_type(sunrise_game_data::gamerules::vanilla::TNT_EXPLOSION_DROP_DECAY)
        };
        self.server_explode_from(center, radius, interaction, false, blast)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl crate::game::ServerGame<'_> {
    /// Exercise the real explosion entity walk on an empty, headless world.
    pub fn run_explosion_items_fixture(&mut self, directory: &std::path::Path) -> Result<()> {
        #[derive(serde::Deserialize)]
        struct Case {
            name: String,
            stack: String,
            health: i32,
            distances: Vec<f64>,
            #[serde(default)]
            sibling: bool,
        }
        let cases: Vec<Case> =
            serde_json::from_str(&std::fs::read_to_string(directory.join("inputs.json"))?)?;
        let mut output = Vec::new();
        for case in cases {
            self.common.dropped.clear();
            let stack =
                sunrise_items::item_data::parse_stack(&case.stack)?.ok_or("empty fixture stack")?;
            self.spawn_item(stack, [0.0, 100.0, 0.0], [0.0; 3], 0);
            self.common.dropped[0].health = case.health;
            let original = self.common.dropped[0].id;
            if case.sibling {
                self.spawn_item(
                    ItemStack::new("minecraft:dirt", 1),
                    [0.0, 100.0, 0.0],
                    [0.0; 3],
                    0,
                );
            }
            let mut steps = Vec::new();
            for distance in case.distances {
                for item in &mut self.common.dropped {
                    item.position = Vec3::new(distance, 100.0, 0.0);
                }
                self.explosion_hurt_entities(
                    [0.0, 100.0, 0.0],
                    1.0,
                    "minecraft:explosion",
                    BlockInteraction::Destroy,
                    &[],
                    ExplosionSource::default(),
                );
                let mut remaining: Vec<_> = self
                    .common
                    .dropped
                    .iter()
                    .map(|item| (item.stack.item.to_string(), item.stack.count, item.health))
                    .collect();
                remaining.sort();
                steps.push(serde_json::json!(remaining));
                if !self.common.dropped.iter().any(|item| item.id == original) {
                    break;
                }
            }
            output.push(serde_json::json!({"name": case.name, "steps": steps}));
        }
        std::fs::write(directory.join("rust.json"), serde_json::to_vec(&output)?)?;
        Ok(())
    }
}

/// `ServerExplosion.addOrAppendStack`: `ItemEntity.areMergable` (same item and
/// components, combined count within the item's max stack size), then
/// `ItemEntity.merge(.., 16)`.
fn add_or_append_stack(
    stacks: &mut Vec<([i32; 3], ItemStack)>,
    mut stack: ItemStack,
    pos: [i32; 3],
    max_stack: &dyn Fn(&ItemStack) -> u32,
) {
    let max = i64::from(max_stack(&stack));
    for (_, existing) in stacks.iter_mut() {
        if existing.same_item_same_components(&stack)
            && i64::from(existing.count) + i64::from(stack.count) <= max
        {
            let moved = (max.min(16) - i64::from(existing.count))
                .min(i64::from(stack.count))
                .max(0) as u32;
            existing.count += moved;
            stack.count -= moved;
            if stack.count == 0 {
                return;
            }
        }
    }
    stacks.push((pos, stack));
}

/// The answers of the parts that declare their explosion resistance pure, by
/// state, for one read.
pub(in crate::game) type PartAnswers = std::collections::HashMap<BlockStateId, Option<f32>>;

/// What decides a blast's resistance in a cell besides the cell itself: the
/// damage calculator's overrides (`ExplosionDamageCalculator` and the ones
/// that replace it).
pub(in crate::game) struct BlastRules {
    /// One block that resists this much (a respawn anchor's water cell).
    resists: Option<([i32; 3], f32)>,
    /// A dangerous wither skull's cap on what the wither may destroy.
    cap: Option<f32>,
    /// The blast's direct source, which `cap` asks about the block.
    direct: Option<u32>,
    /// `#wither_immune`, read while `cap` is set.
    wither_immune: Option<HashSet<String>>,
    /// A primed TNT minecart's: its rails and the blocks under them neither
    /// resist nor break.
    rail_proof: bool,
    /// `USED_PORTAL_DAMAGE_CALCULATOR`: a nether portal neither resists nor breaks.
    portal_proof: bool,
}

/// `WitherSkull.getBlockExplosionResistance`: `isDangerous() &&
/// WitherBoss.canDestroy(state) ? min(0.8, resistance) : resistance`. Air has
/// no resistance to begin with, and a block the skull may not destroy
/// (`kept`: `#wither_immune`, or as its parts answer) keeps its own.
fn blast_resistance(base: f32, cap: Option<f32>, kept: bool) -> f32 {
    match cap {
        Some(cap) if !kept => base.min(cap),
        _ => base,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[cfg(test)]
    use sunrise_gameplay_common::game::server_side::explosion::tests::zombie_type;

    use super::*;

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn explosion_loot_merges_and_joins_without_block_drops() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_level::random_source::{record_seeds, replay_seeds, unique_seed};

        struct Join {
            id: u32,
            pos: [f64; 3],
            level_draw: i32,
            seed: i64,
        }

        #[derive(Clone, Default)]
        struct Watch {
            block_drops_calls: Arc<AtomicUsize>,
            joins: Arc<Mutex<Vec<Join>>>,
        }

        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Watch {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                _: &mut mod_api::BlockDropsEvent<'_>,
            ) -> mod_api::Flow {
                self.block_drops_calls.fetch_add(1, Ordering::Relaxed);
                mod_api::Flow::Continue
            }

            fn loot_drops(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::LootDropsEvent<'_, '_>,
            ) {
                if event.context.explosion.is_some()
                    && event.context.block.is_some_and(|(pos, _)| pos[0] == 3)
                {
                    let diamond = mod_api::__private::item_key("minecraft:diamond").unwrap();
                    event.drops.clear();
                    event
                        .drops
                        .push(mod_api::ItemStack::new(diamond, 1).unwrap())
                        .unwrap();
                }
            }

            fn entity_join_level(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::EntityJoinLevelEvent<'_>,
            ) -> mod_api::Flow {
                if event.kind == "minecraft:item" {
                    let level = host.level("minecraft:overworld").unwrap();
                    let level_draw = level.with_random(host, |random| random.next_int()).unwrap();
                    self.joins.lock().unwrap().push(Join {
                        id: event.subject.id(),
                        pos: event.pos,
                        level_draw,
                        seed: unique_seed(),
                    });
                }
                mod_api::Flow::Continue
            }
        }

        let watch = Watch::default();
        let mods = vec![mod_api::ModEntry::new(
            "watch-blast",
            Box::new(watch.clone()),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.load_server_loot();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                let mut targets = [[1, 64, 1], [2, 64, 1], [3, 64, 1]];
                for [x, y, z] in targets {
                    game.world.set(x, y, z, dirt).unwrap();
                }
                let start = ExplosionStart::from(mod_api::__private::built::ExplosionStart {
                    dimension: mod_api::DimensionKey::OVERWORLD,
                    center: [0.0; 3],
                    radius: 4.0,
                    fire: false,
                    interaction: mod_api::BlockInteraction::Destroy,
                    damage_type: "minecraft:explosion",
                    source: None,
                    owner: None,
                });
                game.world.random.set_seed(7);
                let (seeds, replay) =
                    record_seeds(|| [unique_seed(), unique_seed(), unique_seed(), unique_seed()]);
                let hit = replay_seeds(replay.clone(), || {
                    game.explosion_block_step(
                        &mut targets,
                        4.0,
                        BlockInteraction::Destroy,
                        false,
                        &start,
                    )
                })
                .unwrap();

                assert_eq!(hit, 3);
                assert_eq!(watch.block_drops_calls.load(Ordering::Relaxed), 0);
                assert_eq!(game.dropped.len(), 2);
                assert!(
                    replay.complete(),
                    "one constructor and one join probe seed per pile"
                );
                let joins = watch.joins.lock().unwrap();
                assert_eq!(joins.len(), 2);
                // Java Random(7) shuffles the cells to x=1, 3, 2. Each pile then
                // draws three offset doubles immediately before the join's nextInt().
                let expected = [
                    (
                        "minecraft:dirt",
                        2,
                        [1.6245848015668165, 64.29915485151562, 1.6986385713710523],
                        -1_253_369_595,
                    ),
                    (
                        "minecraft:diamond",
                        1,
                        [3.534574816762766, 64.60132812053513, 1.3763313282274685],
                        -1_303_187_972,
                    ),
                ];
                for (i, (name, count, pos, draw)) in expected.into_iter().enumerate() {
                    let item = &game.dropped[i];
                    let join = &joins[i];
                    assert_eq!((&*item.stack.item, item.stack.count), (name, count));
                    assert_eq!(join.pos, pos);
                    assert_eq!(join.level_draw, draw);
                    assert_eq!(join.seed, seeds[2 * i + 1]);
                    assert_eq!(join.id, item.id);
                    let uuid =
                        sunrise_sim::save::insecure_uuid(&mut LegacyRandom::new(seeds[2 * i]));
                    assert_eq!(game.persist.uuids.get(&item.id), Some(&uuid));
                }
                assert_eq!(game.world.random.next_int(), 357_425_922);
            });
        });
    }

    #[cfg(test)]
    /// Loot modifiers run before the normal prepared drop event on a mob break;
    /// canceling the event also suppresses an infested block's silverfish.
    #[test]
    #[ignore = "daily"]
    fn mob_breaks_apply_block_drops() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::{Arc, Mutex};
        use sunrise_sim::mobs::MobEvent;

        struct ReplaceDrops(Arc<Mutex<Vec<[i32; 3]>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ReplaceDrops {
            fn loot_drops(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::LootDropsEvent<'_, '_>,
            ) {
                let diamond = mod_api::__private::item_key("minecraft:diamond").unwrap();
                event.drops.clear();
                event
                    .drops
                    .push(mod_api::ItemStack::new(diamond, 1).unwrap())
                    .unwrap();
            }

            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut mod_api::BlockDropsEvent<'_>,
            ) -> mod_api::Flow {
                assert!(event.breaker.is_none() && event.tool.is_none());
                assert!(
                    event.breaker_entity.is_none()
                        && event.block_entity.is_none()
                        && event.block_entity_data.is_none()
                );
                assert_eq!(event.drops.len(), 1);
                let stack = event.drops.get(0).unwrap().stack().unwrap();
                assert_eq!(
                    mod_api::__private::item_key_id(stack.item()),
                    Some("minecraft:diamond")
                );
                self.0
                    .lock()
                    .unwrap()
                    .push([event.pos.x, event.pos.y, event.pos.z]);
                if event.pos.x == 4 {
                    return mod_api::Flow::Cancel;
                }
                let emerald = mod_api::__private::item_key("minecraft:emerald").unwrap();
                event.drops.clear();
                event
                    .drops
                    .push(mod_api::ItemDrop::new(
                        mod_api::ItemStack::new(emerald, 3).unwrap(),
                        [7.0, 80.0, 7.0],
                        [0.1, 0.2, 0.3],
                        42,
                    ))
                    .unwrap();
                event.experience = 5;
                mod_api::Flow::Continue
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "mob-break-drops",
            Box::new(ReplaceDrops(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let dirt = registry.default_state("minecraft:dirt").unwrap();
                let infested = registry.default_state("minecraft:infested_stone").unwrap();
                for (x, state) in [(3, dirt), (4, infested), (6, infested)] {
                    game.dropped.clear();
                    game.orbs.clear();
                    game.world.set(x, 64, 1, state).unwrap();
                    game.handle_mob_events(vec![MobEvent::BreakBlock {
                        pos: [x, 64, 1],
                        drop: true,
                        by: None,
                        breaker: None,
                    }])
                    .unwrap();
                    assert_eq!(game.world.block(x, 64, 1), game.world.air);
                    match x {
                        4 => {
                            assert!(game.dropped.is_empty() && game.orbs.is_empty());
                            assert!(game.mobs.is_empty(), "cancel suppresses spawnAfterBreak");
                        }
                        _ => {
                            assert_eq!(game.dropped.len(), 1);
                            let item = &game.dropped[0];
                            assert_eq!(
                                (&*item.stack.item, item.stack.count),
                                ("minecraft:emerald", 3)
                            );
                            assert_eq!(item.position, Vec3::new(7.0, 80.0, 7.0));
                            assert_eq!(item.velocity, Vec3::new(0.1, 0.2, 0.3));
                            assert_eq!(item.pickup_delay, 42);
                            assert_eq!(
                                game.orbs
                                    .iter()
                                    .map(|orb| orb.points() * orb.count)
                                    .sum::<u32>(),
                                5
                            );
                        }
                    }
                }
                assert_eq!(
                    game.mobs.len(),
                    1,
                    "allowed spawnAfterBreak runs exactly once"
                );
                assert_eq!(
                    game.mobs[0].species(),
                    sunrise_sim::mobs::Species::Silverfish
                );
            });
        });
        assert_eq!(*seen.lock().unwrap(), [[3, 64, 1], [4, 64, 1], [6, 64, 1]]);
    }

    /// `exploded_positions` before rays reused their last block's lookup:
    /// every step looked its block up.
    pub fn exploded_positions_stepwise(
        game: &mut crate::game::ServerGame<'_>,
        center: [f64; 3],
        radius: f32,
        resists: Option<([i32; 3], f32)>,
    ) -> Vec<[i32; 3]> {
        let (min_y, max_y) = game.with_level(|_, level, _| {
            use sunrise_world::level::LevelAccess;
            (level.min_y(), level.max_y())
        });
        let mut set: JavaHashMap<Key, ()> = JavaHashMap::default();
        for xx in 0..16 {
            for yy in 0..16 {
                for zz in 0..16 {
                    if !(xx == 0 || xx == 15 || yy == 0 || yy == 15 || zz == 0 || zz == 15) {
                        continue;
                    }
                    let mut xd = f64::from(xx as f32 / 15.0 * 2.0 - 1.0);
                    let mut yd = f64::from(yy as f32 / 15.0 * 2.0 - 1.0);
                    let mut zd = f64::from(zz as f32 / 15.0 * 2.0 - 1.0);
                    let d = (xd * xd + yd * yd + zd * zd).sqrt();
                    (xd, yd, zd) = (xd / d, yd / d, zd / d);
                    let mut power = radius * (0.7 + game.world.random.next_float() * 0.6);
                    let (mut xp, mut yp, mut zp) = (center[0], center[1], center[2]);
                    while power > 0.0 {
                        let pos = [xp.floor() as i32, yp.floor() as i32, zp.floor() as i32];
                        if pos[1] < min_y || pos[1] >= max_y {
                            break;
                        }
                        let state = game.world.block(pos[0], pos[1], pos[2]);
                        let f = game.res.facts().get(state);
                        let fluid = if f.fluid != Fluid::None {
                            100.0f32
                        } else {
                            0.0
                        };
                        match resists {
                            Some((at, resistance)) if at == pos => {
                                power -= (resistance + 0.3) * 0.3;
                            }
                            _ if !game.res.facts().is_air(state) || f.fluid != Fluid::None => {
                                let base = f.explosion_resistance.max(fluid);
                                power -= (blast_resistance(base, None, false) + 0.3) * 0.3;
                            }
                            _ => {}
                        }
                        if power > 0.0 {
                            set.put(Key(pos), ());
                        }
                        xp += xd * f64::from(0.3f32);
                        yp += yd * f64::from(0.3f32);
                        zp += zd * f64::from(0.3f32);
                        power -= 0.225_000_01;
                    }
                }
            }
        }
        set.entries().map(|(k, _)| k.0).collect()
    }

    #[cfg(test)]
    /// A dangerous wither skull's blast treats a block as at most 0.8 harder to
    /// break, but a `#wither_immune` block keeps its own resistance and any
    /// other blast is unchanged.
    #[test]
    fn a_dangerous_wither_skull_caps_block_resistance() {
        assert_eq!(blast_resistance(4.0, Some(0.8), false), 0.8);
        assert_eq!(blast_resistance(0.3, Some(0.8), false), 0.3, "already low");
        assert_eq!(blast_resistance(4.0, Some(0.8), true), 4.0, "wither immune");
        assert_eq!(blast_resistance(4.0, None, false), 4.0, "another blast");
    }

    #[cfg(test)]
    /// A dropped item survives a mob's blast untouched while `mobGriefing` is
    /// off, and is thrown and hurt as usual by one that destroys blocks or by
    /// any blast with the rule back on.
    #[test]
    fn items_ignore_a_blast_that_keeps_blocks_with_griefing_off() {
        assert!(items_ignore_explosion(false, BlockInteraction::Keep));
        assert!(!items_ignore_explosion(false, BlockInteraction::Destroy));
        assert!(!items_ignore_explosion(
            false,
            BlockInteraction::DestroyWithDecay
        ));
        assert!(!items_ignore_explosion(true, BlockInteraction::Keep));
    }

    #[cfg(test)]
    #[test]
    fn blasts_reach_entities_within_their_diameter() {
        // A TNT blast: radius 4, so the entity pass reaches 8 blocks out.
        let center = [0.5, 64.0, 0.5];
        assert!(blast_reaches(center, 8.0, [8.5, 64.0, 0.5]));
        assert!(blast_reaches(center, 8.0, [-3.5, 68.0, 4.5]));
        assert!(!blast_reaches(center, 8.0, [8.6, 64.0, 0.5]));
    }

    #[cfg(test)]
    #[test]
    fn a_mob_that_refuses_the_hit_is_still_pushed() {
        let mut test_level = sunrise_sim::health::TestLevel::new(0);
        let level = &mut test_level.level();
        use sunrise_sim::mobs::{Damage, DamageKind, Mob, Species};
        let hit = (10.0, Damage::of(DamageKind::Explosion));
        let push = Vec3::new(0.5, 0.25, -0.5);
        let mob = |id, species| {
            let mut kind = (*zombie_type()).clone();
            kind.species = species;
            Mob::new(id, std::sync::Arc::new(kind), Vec3::ZERO, 0.0)
        };
        let pushed = |m: &Mob| {
            assert_eq!(m.velocity, Vec3::new(0.5, 0.25, -0.5));
            assert!(m.needs_sync);
        };
        // An ordinary mob is hurt and pushed.
        let mut m = mob(1, Species::Zombie);
        let health = m.health;
        hurt_mob(level, &mut m, true, None, hit);
        m.push_from_explosion(push);
        assert!(m.health < health);
        pushed(&m);
        // An invulnerable one refuses the hit, and is pushed all the same.
        let mut m = mob(2, Species::Zombie);
        m.invulnerable_flag = true;
        let health = m.health;
        hurt_mob(level, &mut m, true, None, hit);
        m.push_from_explosion(push);
        assert_eq!(m.health, health);
        pushed(&m);
        // A team mate of the exploding mob is not hurt, but pushed.
        let mut m = mob(3, Species::Zombie);
        let health = m.health;
        hurt_mob(level, &mut m, false, None, hit);
        m.push_from_explosion(push);
        assert_eq!(m.health, health);
        pushed(&m);
        // A wither shrugs off a blast an undead caused, and is pushed.
        let mut m = mob(4, Species::Wither);
        let health = m.health;
        hurt_mob(level, &mut m, true, Some(Species::Skeleton), hit);
        m.push_from_explosion(push);
        assert_eq!(m.health, health);
        pushed(&m);
    }

    #[cfg(test)]
    #[test]
    fn mob_griefing_off_spares_decorations_from_kept_and_mob_blasts() {
        use BlockInteraction::{Destroy, DestroyWithDecay, Keep};
        // mobGriefing on: every blast breaks them, a creeper's included.
        assert!(!blast_spares_decorations(
            true,
            DestroyWithDecay,
            false,
            false
        ));
        assert!(!blast_spares_decorations(true, Keep, false, false));
        // Off: TNT still breaks them (ExplosionInteraction.TNT destroys blocks).
        assert!(!blast_spares_decorations(false, Destroy, false, false));
        assert!(!blast_spares_decorations(
            false,
            DestroyWithDecay,
            false,
            false
        ));
        // Off: a blast that keeps blocks, or one a refused mob causes, does not.
        assert!(blast_spares_decorations(false, Keep, false, false));
        assert!(blast_spares_decorations(false, Destroy, true, false));
        // A source in water spares them whatever the rules (`BlockAttachedEntity.ignoreExplosion`).
        assert!(blast_spares_decorations(
            true,
            DestroyWithDecay,
            false,
            true
        ));
        assert!(blast_spares_decorations(true, Destroy, true, true));
    }

    #[cfg(test)]
    /// The type a blast hurts with and the attacker its death message names
    /// come from one cause (`DamageSources.explosion`): `player_explosion`
    /// "blown up by" that mob, or `explosion` with no one when it has none.
    fn blast_hurt(
        owner: Option<u32>,
        exploding: Option<u32>,
        live: &[(u32, Text)],
        exploded: &[(u32, Text)],
    ) -> (&'static str, Option<(u32, Text)>) {
        let find = |id| live.iter().find(|(m, _)| *m == id).map(|(_, n)| n.clone());
        let cause = blast_cause(owner, exploding, find, exploded);
        let kind = sunrise_sim::mobs::DamageKind::Explosion;
        (
            sunrise_gameplay_common::game::creatures::damage_type(kind, cause.is_some()),
            cause,
        )
    }

    #[cfg(test)]
    #[test]
    fn a_caused_blast_names_its_cause_under_player_explosion() {
        let creeper = Text::translate("entity.minecraft.creeper", &[]);
        let ghast = Text::translate("entity.minecraft.ghast", &[]);
        // A creeper's blast: it left the mob list, its event named it.
        assert_eq!(
            blast_hurt(None, Some(7), &[], &[(7, creeper.clone())]),
            ("minecraft:player_explosion", Some((7, creeper)))
        );
        // A ghast's fireball: its owner is still in the level.
        assert_eq!(
            blast_hurt(Some(3), None, &[(3, ghast.clone())], &[]),
            ("minecraft:player_explosion", Some((3, ghast.clone())))
        );
        // The ghast was removed before its fireball landed: no owner.
        assert_eq!(
            blast_hurt(Some(3), None, &[], &[]),
            ("minecraft:explosion", None)
        );
        // Nothing caused it.
        assert_eq!(
            blast_hurt(None, None, &[], &[]),
            ("minecraft:explosion", None)
        );
        // A projectile's owner wins over the exploding mob, and a removed
        // owner gives no cause even when a mob is exploding.
        let zombie = Text::translate("entity.minecraft.zombie", &[]);
        let live = [(3, ghast.clone()), (5, zombie.clone())];
        assert_eq!(
            blast_hurt(Some(3), Some(5), &live, &[]),
            ("minecraft:player_explosion", Some((3, ghast)))
        );
        assert_eq!(
            blast_hurt(Some(4), Some(5), &live, &[]),
            ("minecraft:explosion", None)
        );
        // A mob still in the level is named as it is now, before its name
        // in the exploded list.
        let named = Text::literal("Bob".to_owned());
        assert_eq!(
            blast_hurt(None, Some(5), &live, &[(5, named)]),
            ("minecraft:player_explosion", Some((5, zombie)))
        );
    }

    #[cfg(test)]
    #[test]
    fn block_pos_hash_matches_vec3i() {
        // Vec3i.hashCode: (y + z * 31) * 31 + x
        assert_eq!(Key([1, 2, 3]).java_hash(), (2 + 3 * 31) * 31 + 1);
        assert_eq!(Key([-5, 64, -7]).java_hash(), (64 - 7 * 31) * 31 - 5);
        assert_eq!(
            Key([0, 1, 0]).comparable_compare(&Key([5, 0, 9])),
            Some(Ordering::Greater)
        );
        assert_eq!(
            Key([1, 0, 2]).comparable_compare(&Key([0, 0, 3])),
            Some(Ordering::Less)
        );
    }

    #[cfg(test)]
    #[test]
    fn drops_merge_up_to_sixteen() {
        let stack = |item: &str| ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let max = |s: &ItemStack| {
            if &*s.item == "minecraft:diamond_sword" {
                1
            } else {
                64
            }
        };
        let mut stacks = Vec::new();
        for _ in 0..20 {
            add_or_append_stack(&mut stacks, stack("minecraft:dirt"), [0, 0, 0], &max);
        }
        add_or_append_stack(&mut stacks, stack("minecraft:stone"), [1, 0, 0], &max);
        let counts: Vec<u32> = stacks.iter().map(|(_, s)| s.count).collect();
        assert_eq!(counts, vec![16, 4, 1]);
    }

    #[cfg(test)]
    #[test]
    fn unstackable_drops_stay_separate() {
        let sword = ItemStack {
            item: "minecraft:diamond_sword".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let max = |_: &ItemStack| 1;
        let mut stacks = Vec::new();
        add_or_append_stack(&mut stacks, sword.clone(), [0, 0, 0], &max);
        add_or_append_stack(&mut stacks, sword, [0, 0, 0], &max);
        let counts: Vec<u32> = stacks.iter().map(|(_, s)| s.count).collect();
        assert_eq!(counts, vec![1, 1]);
    }

    #[cfg(test)]
    #[test]
    fn drops_with_different_damage_stay_separate() {
        let pick = |damage| ItemStack {
            item: "minecraft:iron_pickaxe".into(),
            count: 1,
            damage,
            extra: None,
        };
        let max = |_: &ItemStack| 64;
        let mut stacks = Vec::new();
        add_or_append_stack(&mut stacks, pick(0), [0, 0, 0], &max);
        add_or_append_stack(&mut stacks, pick(5), [0, 0, 0], &max);
        assert_eq!(stacks.len(), 2);
    }

    #[cfg(test)]
    /// A blast and a burst of wind cast their rays at, and push from the eyes of,
    /// the mob as it stands: a baby's and a size-4 slime's own box and eye
    /// height, not the adult kind's (`Entity.getBoundingBox`, `getEyeHeight`).
    #[test]
    fn a_scaled_mob_is_blasted_by_its_own_box_and_eye_height() {
        use sunrise_gameplay_common::game::entity::Entity;
        use sunrise_sim::mobs::{Mob, Species};
        let mob = |species| {
            let mut kind = (*zombie_type()).clone();
            kind.species = species;
            Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0)
        };
        let adult = mob(Species::Zombie);
        assert_eq!(adult.explosion_box(), adult.bounds());
        let mut baby = mob(Species::Zombie);
        baby.extra.age = -24000;
        let mut slime = mob(Species::Slime);
        slime.extra.size = 4;
        for m in [&baby, &slime] {
            let b = Entity::explosion_box(m);
            assert_eq!(b, m.bounds());
            assert_eq!(Entity::explosion_eye_height(m), m.eye_height());
            assert_ne!(
                f64::from(m.height()),
                f64::from(adult.height()),
                "the case must scale the box"
            );
            assert_eq!(
                b.max_position().y - b.min_position().y,
                f64::from(m.height())
            );
        }
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::cart_rails::CartRailsGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::can_drop_from_explosion::CanDropFromExplosionGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::explosion_resistance::ExplosionResistanceGameExt1 as _;
