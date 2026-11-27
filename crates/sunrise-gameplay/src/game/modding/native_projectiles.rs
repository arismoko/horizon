//! Projectile impacts for native mods (`native_projectile_impact`, NeoForge's
//! `ProjectileImpactEvent`). `tick_projectiles` flies every projectile first and
//! collects the impacts; once its worlds are let go, the mods hear each one, and a
//! projectile whose impact a mod canceled flies the tick again from where it was,
//! through whatever it hit.
use super::{commands::Target, dragon_common::SPECIAL, lan_pvp::meets_players};
use crate::game::native_hooks::mob_kind;
use std::ops::Range;
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_gameplay_common::game::creatures::TargetQuery;
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;
use sunrise_level::fast_hash::{FxHashMap, FxHashSet};
use sunrise_sim::{
    entities::arrows::{ArrowEvent, ArrowSound},
    join_seq::{JoinList, JoinSeq},
    mobs::Who,
    projectiles::{Hit, Projectile},
};

/// The hit target state an independent projectile decided against before a hook.
#[derive(PartialEq)]
pub enum ImpactTarget {
    Block(sunrise_world::registry::BlockStateId),
    Entity(mod_api::EntityRef, [f64; 3]),
}

/// A projectile's impact in one tick of `tick_projectiles`.
pub struct Impact {
    /// Where the projectile was in the tick's list.
    pub index: Option<usize>,
    /// The original flight's keep decision, once section movement has been accounted for.
    pub keep: bool,
    /// The projectile before the tick.
    pub before: Projectile,
    pub hit: Hit,
    pub target: Option<ImpactTarget>,
    /// An earlier impact can break a wind charge's vehicle. Tell its real hit at apply.
    pub wind_vehicle: bool,
    /// Where its hit is in the tick's `hits`.
    pub hit_slot: usize,
    /// Where its arrow events are in the tick's `arrow_events`.
    pub events: Range<usize>,
}

pub struct ProjectileHit {
    pub projectile: Projectile,
    pub hit: Hit,
    /// An ordinary (not deferred) block arrow finished its first tick already;
    /// dispatch that shot after the landing, just as for a deferred arrow.
    pub shot_after_hit: bool,
}

pub type ProjectileShot = (Option<Who>, Option<u32>, [f64; 3]);

/// A notified hit still waiting to be applied after the intervening callbacks.
pub struct PendingImpact {
    keep: bool,
    canceled: bool,
    target: Option<ImpactTarget>,
}

/// The type the mods know a projectile by: a spectral arrow is its own.
fn projectile_type(p: &Projectile) -> &'static str {
    p.entity_type()
}

/// Events produced by applying the contact, not the arrow's earlier flight.
pub(in crate::game) fn projectile_impact_event(event: &ArrowEvent) -> bool {
    matches!(
        event,
        ArrowEvent::Block { .. }
            | ArrowEvent::Sound {
                sound: ArrowSound::HitGround
                    | ArrowSound::CrossbowHit
                    | ArrowSound::TridentHitGround
                    | ArrowSound::TridentHit,
                ..
            }
            | ArrowEvent::Drop
    )
}

/// An owned flight pass restored only if its dispatch unwinds.
struct ProjectileDispatch<'game, 'resources> {
    game: &'game mut crate::game::ServerGame<'resources>,
    pass: Option<JoinList<Projectile, false>>,
}
impl Drop for ProjectileDispatch<'_, '_> {
    fn drop(&mut self) {
        if let Some(pass) = self.pass.take() {
            self.game
                .creatures
                .projectiles
                .restore(pass, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Snapshot the hit precondition so listener writes can invalidate its
    /// application without rerunning the collision decision.
    pub(in crate::game) fn native_impact_target(&self, hit: Hit) -> Option<ImpactTarget> {
        let (entity, block, _, _) = self.pearl_hit(hit);
        if let Some(id) = entity {
            let entity = self.entity(id)?;
            Some(ImpactTarget::Entity(entity, self.entity_position(entity)?))
        } else if matches!(hit, Hit::Block { .. }) {
            Some(ImpactTarget::Block(
                self.common.world.block(block[0], block[1], block[2]),
            ))
        } else {
            None
        }
    }

    /// Dispatch a hit for a projectile ticked outside the shared projectile list.
    /// The caller checks the server opt-in gate and exposes the live projectile
    /// before calling; misses never dispatch.
    pub(in crate::game::server_side) fn native_separate_projectile_impact(
        &mut self,
        projectile: u32,
        kind: &str,
        owner: Option<Who>,
        hit: Hit,
    ) -> bool {
        if matches!(hit, Hit::None) {
            return false;
        }
        let owner = match owner {
            Some(Who::Mob(id)) => self.native_impact_entity_id(id),
            Some(Who::Player) => self.own_entity_id() as u32,
            None => 0,
        };
        let (entity, block, face, at) = self.pearl_hit(hit);
        let entity_kind = entity
            .and_then(|id| self.entity(id))
            .and_then(|entity| self.entity_kind(entity)?.id(self));
        let mut canceled = false;
        self.native_projectile_impact(mod_api::__private::ProjectileImpactCall {
            dimension: self.common.dim.id(),
            projectile,
            kind,
            owner,
            entity,
            entity_kind: entity_kind.as_deref().unwrap_or_default(),
            block,
            face,
            at,
            canceled: &mut canceled,
        });
        canceled
    }

    /// Guard just the dispatch: normal completion returns the still-owned pass
    /// to its caller, which restores explicitly at the established tick boundary.
    pub fn with_projectile_dispatch(
        &mut self,
        pass: &mut JoinList<Projectile, false>,
        fire: impl FnOnce(&mut Self),
    ) {
        let mut guard = ProjectileDispatch {
            game: self,
            pass: Some(pass.take_owned_pass()),
        };
        fire(&mut *guard.game);
        *pass = guard.pass.take().unwrap();
    }

    /// Whether this game is a server that flies projectiles for the mods (the
    /// server thread or a dedicated server, never a client) and a mod asked to hear their impacts
    /// (`native_hears_projectile_impacts`). Without one, the tick keeps no copy
    /// of its projectiles: it costs nothing.
    pub(in crate::game) fn native_hears_impacts(&self) -> bool {
        (self.server.serving.is_some() || self.common.options.dedicated.is_some())
            && mod_api::__private::enabled_listened(mod_api::HookId::ProjectileImpact)
    }

    /// The entity id of the one who shot `p`: a mob's, a player's, else 0.
    fn native_impact_owner(&self, p: &Projectile) -> u32 {
        match p.owner {
            Some(Who::Mob(id)) => id,
            Some(Who::Player) => self.native_thrower(p.guest),
            None => 0,
        }
    }

    /// The entity id of the player who threw something: the guest `guest`, or
    /// the host.
    fn native_thrower(&self, guest: Option<u32>) -> u32 {
        match guest {
            Some(conn) => self
                .lan_guest_entities()
                .into_iter()
                .find(|&(c, _)| c == conn)
                .map_or(0, |(_, id)| id),
            None => 0,
        }
    }

    /// `ProjectileImpactEvent` for a player's trident, once its tick has exposed the owner.
    pub(in crate::game::server_side) fn native_trident_impact(
        &mut self,
        trident: u32,
        guest: Option<u32>,
        hit: Hit,
    ) -> bool {
        let owner = self.native_thrower(guest);
        let (entity, entity_kind, block, face, at) = match hit {
            Hit::Entity { who, at } => {
                let (entity, kind) = match who {
                    Who::Mob(id) => self.native_impact_entity(id),
                    Who::Player => (self.own_entity_id() as u32, "minecraft:player".into()),
                };
                (Some(entity), kind, [0; 3], [0; 3], at)
            }
            Hit::Block { pos, face, at } => (None, String::new(), pos, face, at),
            Hit::None => return false,
        };
        let dimension = self.common.dim.id();
        let mut canceled = false;
        self.native_projectile_impact(mod_api::__private::ProjectileImpactCall {
            dimension,
            projectile: trident,
            kind: "minecraft:trident",
            owner,
            entity,
            entity_kind: &entity_kind,
            block,
            face,
            at: [at.x, at.y, at.z],
            canceled: &mut canceled,
        });
        canceled
    }

    /// The entity id a projectile's hit on the reserved or ordinary `id` stands for: a dragon
    /// part is the dragon, every other entity (including End crystals) uses its own id.
    fn native_impact_entity_id(&self, id: u32) -> u32 {
        if id < SPECIAL {
            id
        } else {
            self.common.dragon.as_ref().map_or(0, |d| d.id)
        }
    }

    /// The entity id and type a projectile's hit on the reserved or ordinary
    /// `id` stands for: a dragon part is the dragon, every other entity uses its own id.
    fn native_impact_entity(&self, id: u32) -> (u32, String) {
        let entity = self.native_impact_entity_id(id);
        if id >= SPECIAL {
            return (entity, "minecraft:ender_dragon".into());
        }
        if let Some(kind) = self.entity_kind_for_who(Who::Mob(id)) {
            return (id, kind);
        }
        if let Some(mob) = self.common.mobs.iter().find(|m| m.id == id) {
            return (id, mob_kind(mob));
        }
        (id, String::new())
    }

    /// `EntityTeleportEvent.EnderPearl` for `owner`'s pearl that hit `hit`:
    /// the listeners' `(target, damage, cancel)`, starting from `to` and `damage`.
    pub fn native_pearl_teleport(
        &mut self,
        owner: mod_api::EntityRef,
        pearl: u32,
        from: Vec3,
        to: Vec3,
        mut damage: f32,
        hit: Hit,
    ) -> ([f64; 3], f32, bool) {
        let mut target = [to.x, to.y, to.z];
        let mut cancel = false;
        if !mod_api::__private::listened(mod_api::HookId::EnderPearlTeleport) {
            return (target, damage, cancel);
        }
        let (hit_entity, hit_block, hit_face, hit_at) = self.pearl_hit(hit);
        self.native_ender_pearl_teleport(mod_api::__private::EnderPearlTeleportCall {
            owner,
            pearl,
            from: [from.x, from.y, from.z],
            target: &mut target,
            damage: &mut damage,
            cancel: &mut cancel,
            hit_entity,
            hit_block,
            hit_face,
            hit_at,
        });
        (target, damage, cancel)
    }

    /// What a pearl hit (`getHitResult`), as `ender_pearl_teleport` tells it:
    /// `(hit_entity, hit_block, hit_face, hit_at)`. An entity's block is the
    /// cell of the hit point and its face `[0, 0, 0]`; a dragon part is the dragon.
    pub fn pearl_hit(&self, hit: Hit) -> (Option<u32>, [i32; 3], [i32; 3], [f64; 3]) {
        match hit {
            Hit::Block { pos, at, face } => (None, pos, face, [at.x, at.y, at.z]),
            Hit::Entity { who, at } => {
                let entity = match who {
                    Who::Mob(id) => self.native_impact_entity_id(id),
                    Who::Player => self.own_entity_id() as u32,
                };
                let cell = [
                    at.x.floor() as i32,
                    at.y.floor() as i32,
                    at.z.floor() as i32,
                ];
                (Some(entity), cell, [0; 3], [at.x, at.y, at.z])
            }
            Hit::None => (None, [0; 3], [0; 3], [0.0; 3]),
        }
    }

    /// An explicitly removed owner is already gone to simulation, even while its row
    /// waits for the root transaction and the level's ordinary removal sweep.
    pub(in crate::game) fn projectile_removal_pending(&self, id: u32) -> bool {
        let target = Target::Projectile(id);
        self.removal_pending(&target) || self.common.persist.removals.contains(&target)
    }

    pub(in crate::game::server_side) fn native_projectile_impact_canceled(
        &mut self,
        p: &Projectile,
        hit: Hit,
    ) -> bool {
        if !self.native_hears_impacts() {
            return false;
        }
        let (entity, entity_kind, block, face, at) = match hit {
            Hit::Block { pos, at, face } => (None, String::new(), pos, face, at),
            Hit::Entity { who, at } => {
                let (entity, kind) = match who {
                    Who::Mob(id) => self.native_impact_entity(id),
                    Who::Player => (self.own_entity_id() as u32, "minecraft:player".into()),
                };
                (Some(entity), kind, [0; 3], [0; 3], at)
            }
            Hit::None => return false,
        };
        let mut canceled = false;
        self.native_projectile_impact(mod_api::__private::ProjectileImpactCall {
            dimension: self.common.dim.id(),
            projectile: p.id,
            kind: projectile_type(p),
            owner: self.native_impact_owner(p),
            entity,
            entity_kind: &entity_kind,
            block,
            face,
            at: [at.x, at.y, at.z],
            canceled: &mut canceled,
        });
        canceled
    }

    /// Hand back the owning rows (including terminal impacts) while mods run. A veto
    /// alone replays the flight; all other captured contacts keep their row until the
    /// apply boundary (or, for an explicit discard, the removal sweep).
    pub fn native_settle_impacts(
        &mut self,
        projectiles: &mut JoinList<Projectile, false>,
        impacts: Vec<Impact>,
        hits: &mut Vec<ProjectileHit>,
        arrow_events: &mut Vec<(u32, Vec3, ArrowEvent)>,
        shoots: &mut Vec<ProjectileShot>,
    ) -> FxHashMap<JoinSeq, PendingImpact> {
        // Close the already-lifted pass, not a new one over the spawn buffer.
        self.common.creatures.projectiles.restore(
            projectiles.take_owned_pass(),
            sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst,
        );
        let mut canceled = Vec::with_capacity(impacts.len());
        for impact in &impacts {
            canceled.push(
                !impact.wind_vehicle
                    && impact.index.is_some()
                    && self.entity(impact.before.id).is_some()
                    && self.native_projectile_impact_canceled(&impact.before, impact.hit),
            );
        }
        *projectiles = self.common.creatures.projectiles.lift();
        let places: FxHashMap<_, _> = projectiles
            .iter()
            .enumerate()
            .map(|(index, p)| (p.join, index))
            .collect();
        let mut drop_hits = vec![false; hits.len()];
        let mut drop_events = vec![false; arrow_events.len()];
        let mut flown = Vec::new();
        let mut retired = FxHashSet::default();
        let mut append = FxHashSet::default();
        let mut pending = FxHashMap::default();
        for (impact, canceled) in impacts.into_iter().zip(canceled) {
            let mut before = impact.before;
            let snapshot = &hits[impact.hit_slot].projectile;
            if canceled
                && !self.projectile_removal_pending(before.id)
                && let Some(&index) = places.get(&before.join)
                && &projectiles[index] == snapshot
                && impact.target.is_some()
                && impact.target == self.native_impact_target(impact.hit)
            {
                drop_hits[impact.hit_slot] = true;
                for i in impact.events {
                    drop_events[i] = true;
                }
                // Preserve the first shot already queued by the original flight.
                before.has_been_shot = snapshot.has_been_shot;
                if let (Some(arrow), Some(snapshot)) = (&mut before.arrow, &snapshot.arrow) {
                    arrow.has_been_shot = snapshot.has_been_shot;
                }
                let (p, keep, events) = self.native_projectile_through(before);
                for (at, event) in events {
                    if event == ArrowEvent::ShootEvent && matches!(impact.hit, Hit::Block { .. }) {
                        shoots.push((p.owner, p.guest, at.components()));
                    } else {
                        flown.push((p.id, at, event));
                    }
                }
                let join = p.join;
                if keep {
                    projectiles.store_back(index, p);
                    if !impact.keep {
                        append.insert(join);
                    }
                } else {
                    retired.insert(join);
                }
            } else {
                pending.insert(
                    before.join,
                    PendingImpact {
                        keep: impact.keep,
                        canceled,
                        target: impact.target,
                    },
                );
            }
        }
        projectiles.retain(|p| !retired.contains(&p.join));
        if !append.is_empty() {
            // A terminal projectile which continues keeps its join, but goes at the tail,
            // as in the existing canceled-hit replay. Never retire/recreate its attachment.
            let replayed = projectiles.extract_if(|p| append.contains(&p.join));
            projectiles.extend(replayed);
        }
        let mut n = 0;
        hits.retain(|_| {
            n += 1;
            !drop_hits[n - 1]
        });
        let mut n = 0;
        arrow_events.retain(|_| {
            n += 1;
            !drop_events[n - 1]
        });
        arrow_events.extend(flown);
        pending
    }

    /// The contact still belongs to the same projectile with the same motion.
    fn captured_projectile_index(&self, p: &Projectile) -> Option<usize> {
        let i = self.common.creatures.projectiles.locate(p.join)?;
        let live = &self.common.creatures.projectiles[i];
        (live.position == p.position
            && live.velocity == p.velocity
            && live.owner == p.owner
            && live.guest == p.guest
            && live.owner_uuid == p.owner_uuid
            && live.kind == p.kind
            && live.stuck == p.stuck
            && live.arrow.as_ref().map(|a| a.no_physics) == p.arrow.as_ref().map(|a| a.no_physics))
        .then_some(i)
    }

    /// Tell the actual contact in tick order, once earlier hits have had a chance to break
    /// the vehicle. A listener-invalidated contact never asks the world for another hit.
    pub(in crate::game) fn apply_vehicle_wind_impact(
        &mut self,
        mut p: Projectile,
        mut hit: Hit,
        mut pending: PendingImpact,
        mut before: Projectile,
    ) -> Option<(Projectile, Hit)> {
        let index = self.captured_projectile_index(&p)?;
        if pending.target.is_some() && self.wind_vehicle_broken(hit) {
            let from = p.position;
            (p, pending.keep, hit) = self.broken_vehicle_retick(before.clone());
            pending.target = self.native_impact_target(hit);
            self.common
                .creatures
                .projectiles
                .store_back(index, p.clone());
            self.entity_section_step(p.id, from.components(), p.position.components());
            if self.projectile_removal_pending(p.id) {
                return None;
            }
            let index = self.captured_projectile_index(&p)?;
            if hit == Hit::None {
                let ceiling = if pending.keep {
                    self.wind_ceiling_contact(&p)
                } else {
                    None
                };
                if !pending.keep || ceiling.is_some() {
                    self.common.creatures.projectiles.remove(index);
                }
                return ceiling.map(|hit| (p, hit));
            }
        }
        if pending.target.is_none() || pending.target != self.native_impact_target(hit) {
            return None;
        }
        let canceled = self.native_projectile_impact_canceled(&p, hit);
        if self.projectile_removal_pending(p.id) {
            return None;
        }
        if canceled {
            let index = self.captured_projectile_index(&p)?;
            if pending.target != self.native_impact_target(hit) {
                return None;
            }
            // A plain veto keeps the projectile's original owner, as other cancel-only replays do.
            before.has_been_shot = p.has_been_shot;
            let (through, keep, _) = self.native_projectile_through(before);
            if keep {
                let to = through.position;
                self.common.creatures.projectiles.store_back(index, through);
                if !pending.keep
                    && let Some(owner) = self.common.creatures.projectiles.extract(p.join)
                {
                    let end = self.common.creatures.projectiles.len();
                    self.common.creatures.projectiles.put_back(end, owner);
                }
                self.entity_section_step(p.id, p.position.components(), to.components());
            } else {
                self.common.creatures.projectiles.remove(index);
            }
            return None;
        }
        self.apply_captured_impact(p, hit, pending)
            .map(|p| (p, hit))
    }

    /// Recheck and apply the captured response after the earlier callbacks.
    pub(in crate::game) fn apply_captured_impact(
        &mut self,
        snapshot: Projectile,
        hit: Hit,
        pending: PendingImpact,
    ) -> Option<Projectile> {
        let i = self.captured_projectile_index(&snapshot)?;
        if pending.canceled
            || pending.target.is_none()
            || pending.target != self.native_impact_target(hit)
        {
            return None;
        }
        let mut p = self.common.creatures.projectiles[i].clone();
        if let (Some(_), Hit::Block { pos, .. }) = (&p.arrow, hit) {
            let from = p.position;
            {
                let rain = self.arrow_rain();
                sunrise_sim::entities::arrows::apply_block_contact(
                    &mut p,
                    &self.arrow_scene(&rain),
                    BlockPos::new(pos[0], pos[1], pos[2]),
                );
            }
            let events = p
                .arrow
                .as_mut()
                .map(|a| std::mem::take(&mut a.events))
                .unwrap_or_default();
            if pending.keep {
                self.common.creatures.projectiles.store_back(i, p.clone());
                self.entity_section_step(p.id, from.components(), p.position.components());
            } else {
                self.common.creatures.projectiles.remove(i);
            }
            for event in events {
                self.server_arrow_event(p.position, event);
            }
        } else if !pending.keep {
            self.common.creatures.projectiles.remove(i);
        }
        Some(p)
    }

    /// `AbstractArrow.tick` resumes after a refused contact or after publishing
    /// the accepted hit and `PROJECTILE_LAND`. A discarded arrow is already done.
    pub(in crate::game) fn finish_deferred_arrow_tick(&mut self, id: u32, join: JoinSeq) {
        if self.projectile_removal_pending(id) {
            return;
        }
        let Some(i) = self.common.creatures.projectiles.locate(join) else {
            return;
        };
        let mut p = self.common.creatures.projectiles[i].clone();
        let keep = {
            let rain = self.arrow_rain();
            sunrise_sim::entities::arrows::finish_block_contact(&mut p, &self.arrow_scene(&rain))
        };
        let events = p
            .arrow
            .as_mut()
            .map(|a| std::mem::take(&mut a.events))
            .unwrap_or_default();
        let (owner, guest, at) = (p.owner, p.guest, p.position);
        if keep {
            self.common.creatures.projectiles.store_back(i, p);
        } else {
            self.common.creatures.projectiles.remove(i);
        }
        for event in events {
            if event == ArrowEvent::ShootEvent {
                self.projectile_shoot_event(owner, guest, at.components());
            } else {
                self.server_arrow_event(at, event);
            }
        }
    }

    /// `p`, as it was before its tick, flown that tick through what it hit
    /// (`Projectile::tick_through`, `arrows::tick_through`): the projectile after
    /// it, whether it stays, and the arrow events it made.
    pub(super) fn native_projectile_through(
        &self,
        mut p: Projectile,
    ) -> (Projectile, bool, Vec<(Vec3, ArrowEvent)>) {
        let mob_shot = !matches!(p.owner, Some(Who::Player));
        let pvp = !mob_shot && meets_players(&p.kind);
        // A guest's shot flies from that guest, as in `tick_projectiles`.
        let shooter = p
            .guest
            .filter(|_| pvp)
            .and_then(|c| self.lan_guest_entities().into_iter().find(|&(g, _)| g == c))
            .map(|(_, id)| Who::Mob(id));
        if shooter.is_some() {
            p.owner = shooter;
        }
        let include_guests = mob_shot || pvp;
        let query = TargetQuery::for_projectile(&p, include_guests);
        let live = self.live_proj_world(query, &[]);
        let rain = self.arrow_rain();
        let scene = self.arrow_scene(&rain);
        let (keep, _) = if p.arrow.is_some() {
            let mut scratch = sunrise_sim::entities::arrows::ArrowHitScratch::new();
            sunrise_sim::entities::arrows::tick_through_live(&mut p, &scene, &live, &mut scratch)
        } else {
            p.tick_through(&live)
        };
        if shooter.is_some() {
            p.owner = Some(Who::Player);
        }
        let at = p.position;
        let events = p
            .arrow
            .as_mut()
            .map(|a| a.events.drain(..).map(|e| (at, e)).collect())
            .unwrap_or_default();
        (p, keep, events)
    }
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
