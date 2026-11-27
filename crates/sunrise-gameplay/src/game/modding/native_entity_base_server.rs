//! Server access to runtime entity base state and its family-specific saved representation.
use crate::game::ServerGame;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_gameplay_common::game::{
    commands_common::Target,
    general::general_type_catalog,
    native_components::NativeComponentsGameExt1 as _,
    native_entity_base::{BuiltinKind, EntityBase, owned},
};

/// An entity's base state with what the host reads beside it.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct BaseView {
    pub base: EntityBase,
    /// `Entity.fireImmune`: the type's immunity, or the live grant the body carries.
    pub fire_immune: bool,
    /// The entity's [`BuiltinKind`], when it is one.
    pub kind: Option<BuiltinKind>,
    /// The [`owned`] values the family keeps as fields of its body.
    owned: u8,
}

impl ServerGame<'_> {
    /// The [`BuiltinKind`] of the entity `t` names.
    fn builtin_kind(&self, t: Target) -> Option<BuiltinKind> {
        use sunrise_sim::projectiles::ProjectileKind;
        match t {
            Target::Tnt(_) => Some(BuiltinKind::Tnt),
            Target::Crystal(_) => Some(BuiltinKind::EndCrystal),
            Target::Dragon(_) | Target::DragonPart(_) => Some(BuiltinKind::EnderDragon),
            Target::Cloud(_) => Some(BuiltinKind::AreaEffectCloud),
            Target::Projectile(id) => {
                match self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)?
                    .kind
                {
                    ProjectileKind::WitherSkull { .. } => Some(BuiltinKind::WitherSkull),
                    ProjectileKind::ShulkerBullet { .. } => Some(BuiltinKind::ShulkerBullet),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The base state of the entity `t` names, or None when its body is gone.
    pub(in crate::game) fn read_entity_base(&self, t: Target) -> Option<BaseView> {
        let mut base = EntityBase::read(self.saved_entity_compound(t));
        let (owned, fire_immune) = self.load_owned_base(t, &mut base)?;
        let kind = self.builtin_kind(t);
        Some(BaseView {
            base,
            fire_immune,
            kind,
            owned,
        })
    }

    /// Stores edited base state in its owning body or compound. Returns false for missing bodies and dragon parts.
    pub(in crate::game) fn update_entity_base(
        &mut self,
        t: Target,
        edit: impl FnOnce(&mut EntityBase),
    ) -> bool {
        let Some(view) = self.read_entity_base(t) else {
            return false;
        };
        let mut new = view.base;
        edit(&mut new);
        let saved = new.differing(&view.base) & !view.owned;
        if saved != 0 && !self.with_saved_entity_mut(t, |tag| write_saved(&new, tag, saved)) {
            return false;
        }
        self.store_owned_base(t, &view.base, &new)
    }

    /// Edits or creates the saved compound for kinds that keep one, returning false for other kinds.
    fn with_saved_entity_mut(&mut self, t: Target, edit: impl FnOnce(&mut Tag)) -> bool {
        match t {
            Target::Falling(id) => {
                let uuid = self.uuid_for(id);
                let Some(i) = self.common.falling.iter().position(|f| f.id == id) else {
                    return false;
                };
                self.common.falling.update(i, |f| {
                    let saved = f
                        .saved
                        .get_or_insert_with(|| Box::new((uuid, Tag::Compound(Compound::new()))));
                    edit(&mut saved.1);
                });
                true
            }
            Target::Spawner(id) => {
                let Some(i) = self.common.ominous_spawners.iter().position(|s| s.id == id) else {
                    return false;
                };
                self.common.ominous_spawners.update(i, |spawner| {
                    let raw = spawner
                        .raw
                        .get_or_insert_with(|| Box::new(Tag::Compound(Compound::new())));
                    edit(raw);
                });
                true
            }
            Target::Hook(_) | Target::Knot(_) | Target::DragonPart(_) | Target::General(_) => false,
            t => match self.raw_mut(t) {
                Some(raw) => {
                    edit(raw);
                    true
                }
                None => false,
            },
        }
    }

    /// Overlays live fields over saved values and returns the [`owned`] mask and fire immunity.
    fn load_owned_base(&self, t: Target, base: &mut EntityBase) -> Option<(u8, bool)> {
        use owned::*;
        Some(match t {
            Target::Player | Target::Guest(_) => {
                let p = self.native_player_body(t)?;
                base.fire_ticks = p.fire_ticks;
                base.invulnerable = p.health.permanently_invulnerable;
                (FIRE | INVULNERABLE, p.mod_fire_immune)
            }
            Target::Mob(id) => {
                let m = self.common.mobs.iter().find(|m| m.id == id)?;
                *base = EntityBase {
                    fire_ticks: m.fire_ticks,
                    invulnerable: m.invulnerable_flag,
                    no_gravity: m.no_gravity,
                    silent: m.silent,
                    glowing: m.glowing,
                };
                (ALL, m.fire_immune())
            }
            Target::Stand(id) => {
                let s = self.common.armor_stands.iter().find(|s| s.id == id)?;
                base.fire_ticks = s.fire_ticks;
                base.no_gravity = s.no_gravity;
                (FIRE | NO_GRAVITY, s.mod_fire_immune)
            }
            Target::Item(id) => {
                let i = self.common.dropped.iter().find(|i| i.id == id)?;
                base.fire_ticks = i.fire_ticks;
                base.no_gravity = i.no_gravity;
                // `ItemEntity.fireImmune`: a stack that fire cannot hurt.
                let immune =
                    self.stack_can_be_hurt_by(&i.stack, "minecraft:in_fire") == Some(false);
                (FIRE | NO_GRAVITY, immune)
            }
            Target::Orb(id) => {
                let o = self.common.orbs.iter().find(|o| o.id == id)?;
                base.fire_ticks = o.fire_ticks;
                base.no_gravity = o.no_gravity;
                (FIRE | NO_GRAVITY, false)
            }
            Target::Boat(id) => {
                let b = self.common.boats.iter().find(|b| b.id == id)?;
                base.fire_ticks = b.fire_ticks;
                base.no_gravity = b.no_gravity;
                (FIRE | NO_GRAVITY, false)
            }
            Target::Cart(id) => {
                let c = self.common.carts.iter().find(|c| c.id == id)?;
                base.no_gravity = c.no_gravity;
                (NO_GRAVITY, false)
            }
            Target::Projectile(id) => {
                let p = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)?;
                base.no_gravity = p.no_gravity;
                match p.arrow.as_ref() {
                    Some(a) => {
                        base.fire_ticks = a.fire_ticks;
                        (FIRE | NO_GRAVITY, false)
                    }
                    None => {
                        base.fire_ticks = p.fire_ticks;
                        // No projectile type is built with `.fireImmune()`.
                        (FIRE | NO_GRAVITY, false)
                    }
                }
            }
            Target::Trident(id) => {
                let t = self.common.tridents.iter().find(|t| t.id == id)?;
                base.fire_ticks = t.fire_ticks;
                base.no_gravity = t.no_gravity;
                (FIRE | NO_GRAVITY, false)
            }
            Target::Tnt(id) => {
                let t = self.common.tnts.iter().find(|t| t.id == id)?;
                base.no_gravity = t.no_gravity;
                (NO_GRAVITY, BuiltinKind::Tnt.fire_immune())
            }
            Target::Falling(id) => {
                let f = self.common.falling.iter().find(|f| f.id == id)?;
                base.fire_ticks = f.fire_ticks;
                base.no_gravity = f.no_gravity;
                (FIRE | NO_GRAVITY, false)
            }
            Target::Crystal(id) => {
                let c = self.common.crystals.iter().find(|c| c.id == id)?;
                base.fire_ticks = c.fire;
                base.invulnerable = c.invulnerable;
                (FIRE | INVULNERABLE, BuiltinKind::EndCrystal.fire_immune())
            }
            Target::Bolt(id) => {
                let b = self.common.bolts.iter().find(|b| b.id == id)?;
                *base = EntityBase {
                    fire_ticks: b.fire_ticks,
                    invulnerable: b.invulnerable,
                    no_gravity: b.no_gravity,
                    silent: b.silent,
                    glowing: b.glowing,
                };
                (ALL, false)
            }
            Target::Rocket(id) => {
                let r = self.common.rockets.iter().find(|r| r.id == id)?;
                base.silent = r.silent;
                (SILENT, false)
            }
            Target::Fangs(id) => {
                let f = self.common.fangs.list.iter().find(|f| f.id == id)?;
                base.fire_ticks = f.fire_ticks;
                (FIRE, false)
            }
            Target::Dragon(_) => (0, BuiltinKind::EnderDragon.fire_immune()),
            Target::Cloud(_) => (0, BuiltinKind::AreaEffectCloud.fire_immune()),
            Target::General(id) => {
                let row = self.common.resident_entities.general.row(id)?;
                *base = row.base;
                (ALL, general_type_catalog().fire_immune(row.type_slot))
            }
            // The dragon's parts take its type.
            Target::DragonPart(_) => (ALL, BuiltinKind::EnderDragon.fire_immune()),
            // `EntityType.FISHING_BOBBER` is not fire immune; the bobber is never saved.
            Target::Hook(id) => {
                let (hook, _) = self.fishing_hook_by_id(id)?;
                *base = hook.base;
                (ALL, false)
            }
            Target::Knot(id) => {
                let k = self
                    .common
                    .resident_entities
                    .knots
                    .iter()
                    .find(|k| k.id == id)?;
                *base = EntityBase {
                    fire_ticks: k.fire_ticks,
                    invulnerable: k.invulnerable,
                    no_gravity: k.no_gravity,
                    silent: k.silent,
                    glowing: k.glowing,
                };
                (ALL, false)
            }
            Target::Frame(_)
            | Target::Painting(_)
            | Target::Cushion(_)
            | Target::Other(_)
            | Target::Spawner(_) => (0, false),
        })
    }

    /// Stores fields owned by the live body. Returns false for a gone body or a dragon part.
    fn store_owned_base(&mut self, t: Target, old: &EntityBase, new: &EntityBase) -> bool {
        let changed = new.differing(old);
        let sync = changed & (owned::FIRE | owned::NO_GRAVITY | owned::GLOWING) != 0;
        match t {
            Target::Player | Target::Guest(_) => {
                let Some(p) = self.native_player_body_mut(t) else {
                    return false;
                };
                p.fire_ticks = new.fire_ticks;
                p.health.permanently_invulnerable = new.invulnerable;
            }
            Target::Mob(id) => {
                let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
                    return false;
                };
                m.fire_ticks = new.fire_ticks;
                m.invulnerable_flag = new.invulnerable;
                m.no_gravity = new.no_gravity;
                m.silent = new.silent;
                m.glowing = new.glowing;
                m.needs_sync |= sync;
            }
            Target::Stand(id) => {
                let Some(s) = self.common.armor_stands.iter_mut().find(|s| s.id == id) else {
                    return false;
                };
                s.fire_ticks = new.fire_ticks;
                s.no_gravity = new.no_gravity;
            }
            Target::Item(id) => {
                let Some(i) = self.common.dropped.iter_mut().find(|i| i.id == id) else {
                    return false;
                };
                i.fire_ticks = new.fire_ticks;
                i.no_gravity = new.no_gravity;
                i.needs_sync |= changed & owned::NO_GRAVITY != 0;
            }
            Target::Orb(id) => {
                let Some(o) = self.common.orbs.iter_mut().find(|o| o.id == id) else {
                    return false;
                };
                o.fire_ticks = new.fire_ticks;
                o.no_gravity = new.no_gravity;
            }
            Target::Boat(id) => {
                let Some(i) = self.common.boats.iter().position(|b| b.id == id) else {
                    return false;
                };
                self.common.boats.update(i, |b| {
                    b.fire_ticks = new.fire_ticks;
                    b.no_gravity = new.no_gravity;
                });
            }
            Target::Cart(id) => {
                let Some(i) = self.common.carts.iter().position(|c| c.id == id) else {
                    return false;
                };
                self.common
                    .carts
                    .update(i, |c| c.no_gravity = new.no_gravity);
            }
            Target::Projectile(id) => {
                let Some(i) = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .position(|p| p.id == id)
                else {
                    return false;
                };
                self.common.creatures.projectiles.update(i, |p| {
                    p.no_gravity = new.no_gravity;
                    if let Some(a) = p.arrow.as_mut() {
                        a.fire_ticks = new.fire_ticks;
                    } else {
                        p.fire_ticks = new.fire_ticks;
                    }
                });
            }
            Target::Trident(id) => {
                let Some(i) = self.common.tridents.iter().position(|t| t.id == id) else {
                    return false;
                };
                self.common.tridents.update(i, |t| {
                    t.fire_ticks = new.fire_ticks;
                    t.no_gravity = new.no_gravity;
                });
            }
            Target::Tnt(id) => {
                let Some(i) = self.common.tnts.iter().position(|t| t.id == id) else {
                    return false;
                };
                self.common
                    .tnts
                    .update(i, |t| t.no_gravity = new.no_gravity);
            }
            Target::Falling(id) => {
                let Some(i) = self.common.falling.iter().position(|f| f.id == id) else {
                    return false;
                };
                self.common.falling.update(i, |f| {
                    f.fire_ticks = new.fire_ticks;
                    f.no_gravity = new.no_gravity;
                });
            }
            Target::Crystal(id) => {
                let Some(i) = self.common.crystals.iter().position(|c| c.id == id) else {
                    return false;
                };
                self.common.crystals.update(i, |c| {
                    c.fire = new.fire_ticks;
                    c.invulnerable = new.invulnerable;
                });
            }
            Target::Bolt(id) => {
                let Some(i) = self.common.bolts.iter().position(|b| b.id == id) else {
                    return false;
                };
                self.common.bolts.update(i, |b| {
                    b.fire_ticks = new.fire_ticks;
                    b.invulnerable = new.invulnerable;
                    b.no_gravity = new.no_gravity;
                    b.silent = new.silent;
                    b.glowing = new.glowing;
                });
            }
            Target::Rocket(id) => {
                let Some(i) = self.common.rockets.iter().position(|r| r.id == id) else {
                    return false;
                };
                self.common.rockets.update(i, |r| r.silent = new.silent);
            }
            Target::Fangs(id) => {
                let Some(i) = self.common.fangs.list.iter().position(|f| f.id == id) else {
                    return false;
                };
                self.common
                    .fangs
                    .list
                    .update(i, |f| f.fire_ticks = new.fire_ticks);
            }
            Target::General(id) => {
                let Some(row) = self.common.resident_entities.general.row_mut(id) else {
                    return false;
                };
                row.base = *new;
            }
            Target::Hook(id) => {
                let Some((_, caster)) = self.fishing_hook_by_id(id) else {
                    return false;
                };
                let Some(hook) = self.caster_hook_mut(caster) else {
                    return false;
                };
                hook.base = *new;
            }
            Target::Knot(id) => {
                let Some(i) = self
                    .common
                    .resident_entities
                    .knots
                    .iter()
                    .position(|k| k.id == id)
                else {
                    return false;
                };
                self.common.resident_entities.knots.update(i, |k| {
                    k.fire_ticks = new.fire_ticks;
                    k.invulnerable = new.invulnerable;
                    k.no_gravity = new.no_gravity;
                    k.silent = new.silent;
                    k.glowing = new.glowing;
                });
            }
            // Dragon part IDs are derived from the dragon and have no base-state record.
            Target::DragonPart(_) => return false,
            // Their values live in the saved compound, which `update_entity_base` wrote.
            Target::Dragon(_)
            | Target::Cloud(_)
            | Target::Frame(_)
            | Target::Painting(_)
            | Target::Cushion(_)
            | Target::Other(_)
            | Target::Spawner(_) => {}
        }
        true
    }
}

/// Writes the `mask` values of `base` into a saved entity compound.
fn write_saved(base: &EntityBase, tag: &mut Tag, mask: u8) {
    if let Tag::Compound(map) = tag {
        base.write(map, mask);
    }
}
