//! Explosions for native server mods (docs/modding/src/how-to/native-mods.md, "Explosions"): a mod
//! sets one off as `Level.explode(source, ...)` does, in the level in place, and the blast's
//! causing entity comes from its source as vanilla's `Explosion.getIndirectSourceEntity` has it.
use super::{
    Game,
    commands::Target,
    explosion::{BlockInteraction, ExplosionSource},
};
use horizon_client::{gamerules::vanilla, mobs::Who};
use mod_api::{EntityRef, ExplosionInteraction};

/// The largest radius `native_explode` takes. A blast's cost grows with its
/// radius (its 1352 rays walk `radius * 1.3` blocks at most); vanilla's
/// largest is the wither's charged spawn, 7.
pub(super) const NATIVE_EXPLOSION_MAX_RADIUS: f32 = 32.0;

/// Whether `native_explode` takes this centre and radius.
fn blast_ok(center: [f64; 3], radius: f32) -> bool {
    center
        .into_iter()
        .all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
        && radius.is_finite()
        && radius > 0.0
        && radius <= NATIVE_EXPLOSION_MAX_RADIUS
}

impl Game<'_> {
    /// What `entity` names as a blast's source: None for a stale or foreign
    /// handle, or a player in another level than the one in place.
    pub(super) fn native_explosion_source(&self, entity: EntityRef) -> Option<(u32, Target)> {
        let target = self.native_entity_target(entity)?;
        if let Target::Guest(conn) = target {
            let hub = &self.lan.as_ref()?.hub;
            hub.guest(conn).filter(|g| hub.here(g))?;
        }
        Some((entity.id(), target))
    }

    /// Explodes at `center` in the level in place as `Level.explode(source,
    /// x, y, z, radius, fire, interaction)` does: the blocks it reaches break
    /// or react as `interaction` says (`native_explosion_detonate` sees them
    /// first), entities are hurt and pushed, `fire` lights a third of the air
    /// above solid blocks, and everyone near hears and sees it. `source` is
    /// the exploding entity, already resolved. False, with nothing done, on a
    /// client's copy of the world, for a chunk that isn't loaded, or a radius
    /// that isn't in `0 < radius <= 32`. A blast inside
    /// `native_explosion_detonate` starts that hook again.
    pub(super) fn native_explode(
        &mut self,
        source: Option<(u32, Target)>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> bool {
        if (self.serving.is_none() && self.options.dedicated.is_none()) || !blast_ok(center, radius)
        {
            return false;
        }
        let chunk = (
            (center[0].floor() as i32) >> 4,
            (center[2].floor() as i32) >> 4,
        );
        if !self.world.chunks.contains_key(&chunk) {
            return false;
        }
        let id = source.map(|(id, _)| id);
        // `ServerLevel.explode`'s `ExplosionInteraction` to the blast's.
        let interaction = match interaction {
            ExplosionInteraction::None => BlockInteraction::Keep,
            ExplosionInteraction::Block => self.destroy_type(vanilla::BLOCK_EXPLOSION_DROP_DECAY),
            ExplosionInteraction::Mob if self.can_entity_grief(id) => {
                self.destroy_type(vanilla::MOB_EXPLOSION_DROP_DECAY)
            }
            ExplosionInteraction::Mob => BlockInteraction::Keep,
            ExplosionInteraction::Tnt => self.destroy_type(vanilla::TNT_EXPLOSION_DROP_DECAY),
            ExplosionInteraction::Trigger => BlockInteraction::Trigger,
            _ => unreachable!("ExplosionInteraction: unhandled variant"),
        };
        // Called from inside another blast (`native_explosion_detonate`), it
        // neither takes that blast's source, cause or flags nor leaves its own
        // behind: that blast gets them back after.
        let outer = self.take_blast_state();
        if let Some((id, target)) = source {
            self.attribute_native_blast(id, target);
        }
        // Only the source's `isSilent` keeps the blast quiet.
        let blast =
            ExplosionSource::default().with_silence(id.is_some_and(|id| self.entity_silent(id)));
        let done = self
            .server_explode_from(center, radius, interaction, fire, blast)
            .is_ok();
        self.restore_blast_state(outer);
        done
    }

    /// The blast's source `id` and its causing entity
    /// (`Explosion.getIndirectSourceEntity`): a primed TNT's owner; a living
    /// entity itself (a player; a mob, which `blast_attacker` falls back to;
    /// the dragon or an armour stand); a projectile's owner when that is
    /// living (one in the projectile list, a thrown trident, a firework
    /// rocket, a fishing bobber); anything else causes none. The damage
    /// source and kill credit follow the cause.
    fn attribute_native_blast(&mut self, id: u32, target: Target) {
        self.combat.explosion_source = Some(id);
        // `getEntities(this.source, ...)`: the source is no victim of its own blast.
        self.combat.explosion_source_named = true;
        let owner = match target {
            Target::Tnt(_) => return self.set_tnt_blast_owner(id),
            Target::Player => Some((Who::Player, self.lan_acting)),
            Target::Guest(conn) => Some((Who::Player, Some(conn))),
            Target::Dragon(_) | Target::Stand(_) => {
                let kind = match target {
                    Target::Dragon(_) => "minecraft:ender_dragon",
                    _ => "minecraft:armor_stand",
                };
                self.combat.blast_owner = Some((id, self.explosion_entity_name(id, kind)));
                return;
            }
            Target::Projectile(_) => self
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| Some((p.owner?, p.guest))),
            Target::Rocket(_) => self
                .rockets
                .iter()
                .find(|r| r.id == id)
                .and_then(|r| Some((r.owner?, r.guest))),
            // A thrown trident's owner is a player: a guest, or the player in place.
            Target::Trident(_) => self.tridents.iter().find(|t| t.id == id).and_then(|t| {
                let in_place = self.player.as_ref().map(|p| p.entity_id as u32);
                (t.guest.is_some() || in_place == Some(t.owner)).then_some((Who::Player, t.guest))
            }),
            Target::Hook(_) => self.fishing_hook_by_id(id).map(|(_, caster)| match caster {
                Target::Guest(conn) => (Who::Player, Some(conn)),
                _ => (Who::Player, self.lan_acting),
            }),
            _ => None,
        };
        match owner {
            Some((Who::Player, guest)) => self.set_blast_owner(Some(Who::Player), guest),
            Some((Who::Mob(mob), _)) => self.combat.explosion_owner = Some(mob),
            None => {}
        }
    }

    /// The blast going off now, cleared to a sourceless one.
    fn take_blast_state(&mut self) -> BlastState {
        let c = &mut self.combat;
        BlastState {
            explicit_cause: c.explicit_explosion_cause.take(),
            blast_owner: c.blast_owner.take(),
            blast_guest: c.blast_guest.take(),
            explicit_direct: c.explicit_explosion_direct.take(),
            source: c.explosion_source.take(),
            direct: c.explosion_direct.take(),
            cause: c.explosion_cause.take(),
            source_named: std::mem::take(&mut c.explosion_source_named),
            in_water: std::mem::take(&mut c.explosion_in_water),
            hive_emptying: std::mem::take(&mut c.hive_emptying_blast),
            rail_proof: std::mem::take(&mut c.rail_proof_blast),
            owner: c.explosion_owner.take(),
            tnt_owner_player: std::mem::take(&mut c.blast_owner_player),
        }
    }

    /// Puts back what `take_blast_state` cleared.
    fn restore_blast_state(&mut self, s: BlastState) {
        let c = &mut self.combat;
        c.explicit_explosion_cause = s.explicit_cause;
        c.blast_owner = s.blast_owner;
        c.blast_guest = s.blast_guest;
        c.explicit_explosion_direct = s.explicit_direct;
        c.explosion_source = s.source;
        c.explosion_direct = s.direct;
        c.explosion_cause = s.cause;
        c.explosion_source_named = s.source_named;
        c.explosion_in_water = s.in_water;
        c.hive_emptying_blast = s.hive_emptying;
        c.rail_proof_blast = s.rail_proof;
        c.explosion_owner = s.owner;
        c.blast_owner_player = s.tnt_owner_player;
    }
}

/// The fields of `CombatTracker` that describe the blast going off now.
struct BlastState {
    explicit_cause: Option<Option<super::explosion::ExplosionCause>>,
    blast_owner: Option<(u32, horizon_client::text::Text)>,
    blast_guest: Option<u32>,
    explicit_direct: Option<horizon_client::text::Text>,
    source: Option<u32>,
    direct: Option<u32>,
    cause: Option<horizon_client::advancements::EntityView>,
    source_named: bool,
    in_water: bool,
    hive_emptying: bool,
    rail_proof: bool,
    owner: Option<u32>,
    tnt_owner_player: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blast_needs_a_finite_centre_and_a_radius_in_range() {
        assert!(blast_ok([0.5, 64.0, 0.5], 4.0));
        assert!(blast_ok([0.5, 64.0, 0.5], NATIVE_EXPLOSION_MAX_RADIUS));
        assert!(!blast_ok([0.5, 64.0, 0.5], 0.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], -1.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], 33.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], f32::NAN));
        assert!(!blast_ok([f64::INFINITY, 64.0, 0.5], 4.0));
        assert!(!blast_ok([0.5, 64.0, 4.0e7], 4.0));
    }
}
