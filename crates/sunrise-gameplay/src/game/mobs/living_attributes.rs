//! The item modifiers of the live attribute maps of mobs, armor stands and the
//! dragon (`LivingEntity.collectEquipmentChanges`). The map and the NBT are in
//! `sunrise_client::living_attributes`.
use super::commands::Target;
use crate::game::ServerGame;
use std::convert::Infallible;
use std::sync::Arc;
use sunrise_core::equipment_slot::EquipmentSlot;
use sunrise_items::items::Items;
use sunrise_sim::living_attributes::{ModifierChanges, ModifierSnapshot};
use sunrise_sim::mobs::{
    Mob,
    finalize::{self, Attr, Op, equipment_attribute_seed as equipment_seed},
};
use sunrise_world::attributes::{
    AttributeError, Instance, Modifier, Observer, Operation,
    entity::{AttributeId, Catalog},
};

/// Refresh the live equipment modifiers outside the body tick, e.g. after a
/// mod writes gear or an attribute write rebuilds the map. This does not
/// consume the next `LivingEntity.detectEquipmentUpdates` observation.
pub(in crate::game::server_side) fn collect_equipment(
    catalog: &Arc<Catalog>,
    items: &Items,
    m: &mut Mob,
) {
    sunrise_sim::mobs::armor::prepare(m, catalog);
    let mut live = std::mem::take(&mut m.extra.live);
    let result = live.refresh_equipment(
        catalog,
        &m.kind.id,
        items,
        |slot| finalize::equipment_slot(m, slot),
        || equipment_seed(m),
    );
    warn_failed(m.id, &m.kind.id, result);
    m.extra.live = live;
}

pub(in crate::game::server_side) fn modifier_invalidation_undo(
    catalog: &Arc<Catalog>,
    mob: &Mob,
) -> Option<ModifierChanges> {
    match mob
        .extra
        .live
        .modifier_invalidation_undo(catalog, &mob.kind.id, || equipment_seed(mob))
    {
        Ok(undo) => undo,
        Err(error) => {
            warn_failed(mob.id, &mob.kind.id, Err(error));
            None
        }
    }
}

pub(in crate::game::server_side) fn undo_mob_modifiers(
    catalog: &Arc<Catalog>,
    m: &mut Mob,
    changes: ModifierChanges,
) {
    let mut live = std::mem::take(&mut m.extra.live);
    let result = live.undo_modifier_changes(catalog, &m.kind.id, changes, || equipment_seed(m));
    warn_failed(m.id, &m.kind.id, result);
    m.extra.live = live;
}

/// The attribute `name` when it is one a mob keeps itself: `finalize`'s store,
/// which its gameplay reads and its save writes, for the attributes its type has.
fn own_attribute(m: &Mob, name: &str) -> Option<Attr> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    finalize::command_attr(short).filter(|a| finalize::default_value(m, *a).is_some())
}

/// `write` on one of the mob's own attributes; a lower max health cuts its
/// health (`LivingEntity.onAttributeUpdated`).
fn write_own_attribute(m: &mut Mob, a: Attr, write: AttributeWrite) -> bool {
    let done = write.apply_to_mob(m, a);
    if a == Attr::MaxHealth && m.health > m.attrs.max_health {
        m.health = m.attrs.max_health;
        m.needs_sync = true;
    }
    done
}

/// An attribute write on a mob that is in no level yet (`finalize_spawn`'s
/// lent mob), then its equipment's modifiers collected again, as
/// `Game::write_attribute` does for a joined one. Only the attributes the mob
/// keeps itself take it: the rest live in maps the level keeps per joined
/// entity. None for another attribute, `Some(false)` when an add finds the id
/// taken or a remove finds none.
pub(in crate::game::server_side) fn write_unspawned_attribute(
    catalog: &Arc<Catalog>,
    items: &Items,
    m: &mut Mob,
    name: &str,
    write: AttributeWrite,
) -> Option<bool> {
    let done = write_own_attribute(m, own_attribute(m, name)?, write);
    collect_equipment(catalog, items, m);
    Some(done)
}

impl ServerGame<'_> {
    /// `LivingEntity.detectEquipmentUpdates`, paused after base tick and item use.
    /// With the owning level restored, observe each remaining slot in order so
    /// an earlier listener's writes to a later slot are visible this tick.
    pub fn detect_mob_equipment_updates(
        &mut self,
        id: u32,
        join: sunrise_sim::join_seq::JoinSeq,
        first: EquipmentSlot,
    ) {
        let mut deltas = sunrise_sim::living_attributes::EquipmentDeltas::default();
        for &slot in &EquipmentSlot::ALL[first as usize..] {
            let Some(i) = self
                .common
                .mob_index
                .find(&self.common.mobs, id)
                .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
            else {
                return;
            };
            let Some(change) = finalize::changed_equipment_slot(&self.common.mobs[i], slot) else {
                continue;
            };
            let current = change.current.map(|read| read.project().into_owned());
            if let Some(epoch) = self.living_equipment_notify_epoch() {
                let previous = change.previous.cloned();
                let living = mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, id));
                self.dispatch_living_equipment_change(
                    living,
                    slot,
                    previous.as_ref(),
                    current.as_ref(),
                );
            }
            deltas.set(slot, current);
        }
        if let Some(i) = self
            .common
            .mob_index
            .find(&self.common.mobs, id)
            .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
        {
            let catalog = self.common.motion.attributes.clone();
            let items = server_world_items!(self);
            finalize::finish_equipment_tick(&mut self.common.mobs[i], &catalog, items, deltas);
        }
    }

    /// Attribute upkeep for the separately simulated stands and dragon.
    pub fn refresh_stand_and_dragon_equipment_attributes(&mut self) {
        if !self.common.is_logical_server() {
            return;
        }
        let catalog = self.common.motion.attributes.clone();
        let items = server_world_items!(self);
        for a in self.common.armor_stands.iter_mut() {
            if a.equipment.is_empty() && !a.live.has_worn() {
                continue;
            }
            let changed = a.live.collect_equipment_changes(
                &catalog,
                "minecraft:armor_stand",
                &a.equipment,
                items,
                Vec::new,
            );
            warn_failed(a.id, "minecraft:armor_stand", changed);
        }
        if let Some(d) = self.common.dragon.as_mut()
            && !(d.equipment.is_empty() && !d.live.has_worn())
        {
            let changed = d.live.collect_equipment_changes(
                &catalog,
                "minecraft:ender_dragon",
                &d.equipment,
                items,
                Vec::new,
            );
            warn_failed(d.id, "minecraft:ender_dragon", changed);
        }
    }

    /// Snapshot before any write that can refresh or invalidate the live map.
    /// The non-transactional paths never read or allocate a modifier snapshot.
    pub(in crate::game::server_side) fn snapshot_mob_modifiers(
        &self,
        id: u32,
    ) -> Option<ModifierSnapshot> {
        if self.common.transactions.is_empty() {
            return None;
        }
        let mob = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        snapshot(&self.common.motion.attributes, mob)
    }

    /// Refresh equipment immediately, keeping the live diff if the caller took
    /// a transaction snapshot. The ordinary tick remains the other writers' safety net.
    pub(in crate::game::server_side) fn collect_mob_equipment_attributes(
        &mut self,
        id: u32,
        before: Option<ModifierSnapshot>,
    ) {
        if !self.common.is_logical_server() {
            return;
        }
        let catalog = self.common.motion.attributes.clone();
        let items = server_world_items!(self);
        let Some(m) = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
        else {
            return;
        };
        collect_equipment(&catalog, items, m);
        if let Some(before) = before
            && let Some(after) = snapshot(&catalog, m)
            && let Some(changes) = before.diff(&after)
        {
            let dim = self.common.dim;
            self.common
                .transactions
                .journal_modifier_changes(dim, id, changes);
        }
    }
}

fn snapshot(catalog: &Arc<Catalog>, mob: &Mob) -> Option<ModifierSnapshot> {
    match mob
        .extra
        .live
        .modifier_snapshot(catalog, &mob.kind.id, || equipment_seed(mob))
    {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            warn_failed(mob.id, &mob.kind.id, Err(error));
            None
        }
    }
}

/// An item modifier the map refused (a bad modifier id) is logged, naming the
/// entity, instead of being lost.
fn warn_failed(id: u32, kind: &str, result: sunrise_game_data::Result<()>) {
    if let Err(error) = result {
        log::warn!("equipment attributes of {kind} {id}: {error}");
    }
}

/// One attribute write: the one path of `/attribute` (`AttributeCommand`'s
/// `base set`, `base reset`, `modifier add` and `modifier remove`) and the mod
/// API's `LivingRef` writes, through `Game::write_attribute`.
#[derive(Clone)]
pub enum AttributeWrite {
    /// `addTransientModifier` or `addPermanentModifier`, refused when the id
    /// is taken; with `replace`, the modifier with that id goes first
    /// (`addOrUpdateTransientModifier`, `addOrReplacePermanentModifier`).
    Add {
        modifier: Modifier,
        permanent: bool,
        replace: bool,
    },
    /// `removeModifier`.
    Remove(String),
    /// `setBaseValue`.
    Base(f64),
    /// `setBaseValue` of the entity type's default.
    ResetBase,
}

impl AttributeWrite {
    /// An add of the modifier `id`, or None for an id `Modifier` refuses.
    pub(in crate::game) fn add(
        id: &str,
        amount: f64,
        operation: Operation,
        permanent: bool,
        replace: bool,
    ) -> Option<Self> {
        Some(Self::Add {
            modifier: Modifier::new(id, amount, operation).ok()?,
            permanent,
            replace,
        })
    }

    /// The write on an instance, whose type's default base is `default`:
    /// false when an add finds the id taken or a remove finds nothing.
    fn apply<O: Observer<Error = Infallible>>(
        self,
        value: &mut Instance,
        observer: &mut O,
        default: f64,
    ) -> Result<bool, AttributeError<Infallible>> {
        match self {
            Self::Add {
                modifier,
                permanent,
                replace,
            } => {
                if value.modifier(modifier.id()).is_some() {
                    if !replace {
                        return Ok(false);
                    }
                    value.remove(modifier.id(), observer)?;
                }
                if permanent {
                    value.add_permanent(modifier, observer)?;
                } else {
                    value.add_transient(modifier, observer)?;
                }
                Ok(true)
            }
            Self::Remove(id) => value.remove(&id, observer),
            Self::Base(base) => {
                value.set_base_value(base, observer)?;
                Ok(true)
            }
            Self::ResetBase => {
                value.set_base_value(default, observer)?;
                Ok(true)
            }
        }
    }

    /// The write on one of the attributes a mob's gameplay reads from
    /// `finalize`, which saves its bases and permanent modifiers.
    fn apply_to_mob(self, m: &mut Mob, a: Attr) -> bool {
        let taken = |m: &Mob, id: &str| {
            finalize::has_modifier(m, a, id)
                || finalize::has_transient_modifier(m, a, id)
                || (id == "minecraft:baby" && finalize::transient(m, a).is_some())
        };
        let remove = |m: &mut Mob, id: &str| {
            finalize::remove_baby_modifier(m, a, id);
            finalize::remove_modifier(m, a, id);
        };
        match self {
            Self::Add {
                modifier,
                permanent,
                replace,
            } => {
                let id = modifier.id();
                if taken(m, id) {
                    if !replace {
                        return false;
                    }
                    remove(m, id);
                }
                let op = Op::from(modifier.operation());
                if permanent {
                    finalize::add_modifier(m, a, id.to_owned(), modifier.amount(), op);
                } else {
                    finalize::add_transient_modifier(m, a, id, modifier.amount(), op);
                }
                true
            }
            Self::Remove(id) => {
                let had = taken(m, &id);
                if had {
                    remove(m, &id);
                }
                had
            }
            Self::Base(base) => {
                finalize::set_base(m, a, base);
                true
            }
            Self::ResetBase => {
                let default = finalize::default_value(m, a).unwrap_or_default();
                finalize::set_base(m, a, default);
                true
            }
        }
    }

    /// What a live map keeps of the transient modifiers written to it after
    /// this write: the added one, or None to forget the id.
    fn transient_record(&self) -> Option<(&str, Option<Modifier>)> {
        match self {
            Self::Add {
                modifier,
                permanent,
                ..
            } => Some((modifier.id(), (!permanent).then(|| modifier.clone()))),
            Self::Remove(id) => Some((id, None)),
            Self::Base(_) | Self::ResetBase => None,
        }
    }
}

impl ServerGame<'_> {
    /// Applies `write` to the attribute `name` of `t` on the record that the
    /// entity's gameplay and its clients' attribute sync read:
    /// - a player's (or a guest's, in its turn) own attribute map;
    /// - for a mob, `finalize`'s store for the attributes its behavior reads
    ///   there (`finalize::Attr`, so it moves, hits and falls by them at once);
    ///   for its armor and toughness its armor map (`mobs/armor.rs`); for the
    ///   rest, as for an armor stand, its generic map (`cmds.attributes`),
    ///   which the sync sends.
    ///
    /// A mob's and a stand's live map (`living_attributes`, which the mod
    /// API's reads and the save use) takes the same write and keeps it
    /// through a rebuild. A lower max health cuts a mob's health
    /// (`LivingEntity.onAttributeUpdated`).
    ///
    /// None when the entity has no such attribute or takes no attribute writes
    /// (the dragon, as `/attribute` refuses it); `Some(false)` when an add
    /// finds the id taken or a remove finds none.
    pub fn write_attribute(
        &mut self,
        t: Target,
        name: &str,
        write: AttributeWrite,
    ) -> Option<bool> {
        let before = match t {
            Target::Mob(id) => self.snapshot_mob_modifiers(id),
            _ => None,
        };
        let explicit = if self.common.transactions.is_empty() || !matches!(t, Target::Mob(_)) {
            None
        } else {
            write.transient_record().map(|(id, _)| id.to_owned())
        };
        let done = self.apply_attribute_write(t, name, write)?;
        // A finalize write drops the mob's live map (its seed changed), which
        // takes the equipment's modifiers with it: collect them again at
        // once, so the next read — a hit's armor, the attribute sync — sees
        // them (#941).
        if let Target::Mob(id) = t {
            // Invalidation is part of the explicit-write step. Keep its
            // collateral changes separate from what equipment collection does
            // after it, even when both steps write the same modifier key.
            let after_write = before
                .as_ref()
                .and_then(|_| self.snapshot_mob_modifiers(id));
            if let (Some(before), Some(after)) = (before.as_ref(), after_write.as_ref())
                && let Some(changes) = before.diff(after)
            {
                let dim = self.common.dim;
                self.common
                    .transactions
                    .journal_modifier_changes(dim, id, changes);
            }
            if done && let Some(modifier) = explicit {
                // The explicit write persists through abort. Earlier entries
                // for that row must not undo it; the subsequent refresh,
                // however, is journaled against the newly written value.
                let dim = self.common.dim;
                self.common
                    .transactions
                    .keep_explicit_modifier_write(dim, id, name, &modifier);
            }
            self.collect_mob_equipment_attributes(id, after_write);
        }
        Some(done)
    }

    fn apply_attribute_write(
        &mut self,
        t: Target,
        name: &str,
        write: AttributeWrite,
    ) -> Option<bool> {
        let catalog = self.common.motion.attributes.clone();
        let id = catalog.id(name)?;
        let default = match write {
            AttributeWrite::ResetBase => catalog.default_base(&self.target_type(t), id)?,
            _ => 0.0,
        };
        match t {
            Target::Player | Target::Guest(_) => self
                .for_player(t, |g| {
                    let live = &mut g.player.as_mut()?.attributes;
                    let handle = live.get_instance(Some(live.key(name)?)).ok()??;
                    live.edit(&handle, |value, observer| {
                        write.apply(value, observer, default)
                    })
                    .ok()
                })
                .flatten(),
            Target::Mob(mob) => {
                if self.sulfur_archetype_attribute(t, name) {
                    let done = self.write_generic_attribute(t, name, id, write.clone(), default)?;
                    // The body's map owns the command value, while bodiless physics
                    // reads the base and permanent modifiers from finalize's cache.
                    if done && name == "minecraft:knockback_resistance" {
                        let m = self
                            .common
                            .mobs
                            .iter_mut()
                            .find(|m| m.id == mob && !m.removed)?;
                        // Admission belongs to the body's map. Its archetype changes can
                        // remove an ID without touching finalize's mirror, so an accepted
                        // add replaces any stale copy there rather than being refused twice.
                        let mirror = match write {
                            AttributeWrite::Add {
                                modifier,
                                permanent,
                                ..
                            } => AttributeWrite::Add {
                                modifier,
                                permanent,
                                replace: true,
                            },
                            other => other,
                        };
                        write_own_attribute(m, Attr::KnockbackResistance, mirror);
                    }
                    return Some(done);
                }
                let m = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == mob && !m.removed)?;
                if let Some(a) = own_attribute(m, name) {
                    return Some(write_own_attribute(m, a, write));
                }
                self.write_generic_attribute(t, name, id, write, default)
            }
            Target::Stand(_) => self.write_generic_attribute(t, name, id, write, default),
            _ => None,
        }
    }

    fn write_generic_attribute(
        &mut self,
        t: Target,
        name: &str,
        id: AttributeId,
        write: AttributeWrite,
        default: f64,
    ) -> Option<bool> {
        let map = self.generic_attributes(t)?;
        if !map.has(id) {
            return None;
        }
        let done = map
            .edit(id, |value, observer| {
                write.clone().apply(value, observer, default)
            })?
            .ok()?;
        if done {
            self.write_live_attributes(t, name, write, default);
            // `onAttributeUpdated(SCALE)`: the mob's dimensions follow.
            if name == "minecraft:scale"
                && let Target::Mob(id) = t
                && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed)
            {
                sunrise_sim::mobs::finalize::scale_changed(m);
            }
        }
        Some(done)
    }

    /// The same write on a mob's or a stand's live map: its base and
    /// permanent modifiers go to the map's saved entries, a transient modifier
    /// to its own list, so a rebuilt map has them again.
    fn write_live_attributes(
        &mut self,
        t: Target,
        name: &str,
        write: AttributeWrite,
        default: f64,
    ) {
        let catalog = self.common.motion.attributes.clone();
        let (live, entity, seed) = match t {
            Target::Mob(id) => {
                let Some(m) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                else {
                    return;
                };
                // The seed is only read when the map is not built yet.
                let seed = if m.extra.live.built().is_none() {
                    sunrise_sim::entity_nbt::attribute_entries(m)
                        .into_iter()
                        .map(|(_, entry)| entry)
                        .collect()
                } else {
                    Vec::new()
                };
                (&mut m.extra.live, m.kind.id.clone(), seed)
            }
            Target::Stand(id) => {
                let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id) else {
                    return;
                };
                (&mut a.live, "minecraft:armor_stand".to_owned(), Vec::new())
            }
            _ => return,
        };
        let record = write.transient_record().map(|(id, m)| (id.to_owned(), m));
        let done = live
            .edit(
                &catalog,
                &entity,
                name,
                true,
                || seed,
                |value, observer| write.apply(value, observer, default),
            )
            .unwrap_or(false);
        if done && let Some((id, modifier)) = record {
            live.keep_transient(name, &id, modifier);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_sim::{entities::ArmorStand, mobs::Mob};

    /// A zombie, an armor stand and the dragon, ids 12, 13 and 14.
    pub fn spawn(game: &mut crate::game::ServerGame<'_>) {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
        game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
        let mut dragon =
            sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 14;
        game.dragon.replace(dragon);
    }

    pub fn read(
        game: &crate::game::ServerGame<'_>,
        id: u32,
        name: &str,
    ) -> (Option<f64>, Option<f64>) {
        let living = game.living(game.entity(id).unwrap()).unwrap();
        (
            living.attribute_value(game, name),
            living.attribute_base(game, name),
        )
    }

    pub fn living(game: &crate::game::ServerGame<'_>, id: u32) -> mod_api::LivingRef {
        game.living(game.entity(id).unwrap()).unwrap()
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
