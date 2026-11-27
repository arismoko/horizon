//! An entity's saved data, tags and name for native server mods, through an
//! `EntityRef` (docs/modding/src/concepts/parts/entity-parts-and-mob-goals.md, #7387): what
//! `/data` and `/tag` read and change, its custom name, its type's category
//! and whether it is in its level. They return `None` (or `false`) for a
//! stale handle or on a client, which the server sends no saved data or tags;
//! only the name reads there.
use super::commands::Target;
use mod_api::{EntityRef, EntityTags, MobCategory};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_game_data::{scoreboard::json_to_tag, text::Text};
use sunrise_sim::entity_nbt::flag;

/// The most tags an entity holds (`Entity.addTag`).
const MAX_TAGS: usize = 1024;

impl crate::game::ServerGame<'_> {
    /// The entity's data as `/data get entity` shows it
    /// (`Entity.saveWithoutId`): its compound without `id`. An entity with no
    /// UUID yet gets the one it will be saved with. API 386.
    pub fn entity_nbt(&mut self, entity: EntityRef) -> Option<Compound> {
        if !self.common.is_logical_server() {
            return None;
        }
        let t = self.native_entity_target(entity)?;
        match self.command_target_nbt(t)? {
            Tag::Compound(mut c) => {
                c.remove(&"id".into());
                Some(c)
            }
            _ => None,
        }
    }
    /// Merges `nbt` into the entity's data and loads the entity from it, as `/data merge
    /// entity` does (`EntityDataAccessor.setData`): true when it changed. False for a player
    /// (vanilla refuses them too), a kind Sunrise cannot reload yet (end crystals, the dragon
    /// and its parts, cushions, ominous item spawners, area effect clouds, fishing hooks, leash
    /// knots and general entity rows), data nested too deep, or data that changes nothing. An
    /// arrow, spectral arrow or trident reloads as `AbstractArrow.readAdditionalSaveData` reads
    /// it (`life`, `inGround`, `pickup`, ...); other projectiles take only their position,
    /// motion, rotation and flags. API 386.
    pub(in crate::game) fn entity_merge_nbt(&mut self, entity: EntityRef, nbt: Compound) -> bool {
        if !self.common.is_logical_server() {
            return false;
        }
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(old) = self.entity_nbt(entity).map(Tag::Compound) else {
            return false;
        };
        let nbt = Tag::Compound(nbt);
        if sunrise_game_data::nbt_path::too_deep(&nbt, 0) {
            return false;
        }
        let merged = sunrise_sim::data_cmd::merged(&old, &nbt);
        merged != old && self.apply_entity_nbt(t, merged).is_ok()
    }
    /// Borrow the entity's scoreboard tags (`Entity.entityTags()`) without the
    /// lossy conversion used by `/tag <entity> list`.
    pub(in crate::game) fn entity_tags(&self, entity: EntityRef) -> Option<EntityTags<'_>> {
        if !self.common.is_logical_server() {
            return None;
        }
        Some(self.saved_entity_tags(self.native_entity_target(entity)?))
    }
    /// Adds tag `tag` (`Entity.addTag`): false when it has it already, holds
    /// 1024, its kind can't store tags yet, or the handle is stale. API 386.
    pub(in crate::game) fn entity_add_tag(&mut self, entity: EntityRef, tag: &str) -> bool {
        if !self.common.is_logical_server() {
            return false;
        }
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        let mut tags = self.tags_of(t);
        if tags.len() >= MAX_TAGS || tags.iter().any(|x| x == tag) {
            return false;
        }
        tags.push(tag.to_owned());
        // False, not a silent no-op, for a kind with no saved compound to
        // write tags to (a natural leash knot).
        self.set_entity_tags(t, &tags)
    }
    /// Removes tag `tag` (`Entity.removeTag`): false when it didn't have it.
    /// API 386.
    pub(in crate::game) fn entity_remove_tag(&mut self, entity: EntityRef, tag: &str) -> bool {
        if !self.common.is_logical_server() {
            return false;
        }
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        let mut tags = self.tags_of(t);
        let before = tags.len();
        tags.retain(|x| x != tag);
        tags.len() != before && self.set_entity_tags(t, &tags)
    }
    /// Names the entity (`Entity.setCustomName`), or takes its name away
    /// with `None`: its name, its saved `CustomName` and, for a mob or an
    /// armor stand, what the players tracking it are sent. False for a
    /// player, a kind whose name Sunrise keeps nowhere yet (projectiles, the
    /// dragon and crystals, primed TNT, falling blocks, experience orbs,
    /// ominous item spawners), a stale handle or on a client.
    pub(in crate::game) fn entity_set_custom_name(
        &mut self,
        entity: EntityRef,
        name: Option<Text>,
    ) -> bool {
        let Some(id) = self.name_holder(entity) else {
            return false;
        };
        let saved = name.as_ref().map(|n| json_to_tag(&n.component()));
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            m.custom_name = name.as_ref().map(Text::plain);
            m.custom_name_text = name;
        } else if let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id) {
            a.custom_name = name;
        } else if let Some(f) = self.common.item_frames.iter_mut().find(|f| f.id == id) {
            f.custom_name = saved.clone();
        } else if let Some(i) = self.bolts.iter().position(|b| b.id == id) {
            self.bolts.update(i, |b| b.custom_name = saved.clone());
        } else if let Some(row) = self.resident_entities.general.row_mut(id) {
            // A general row's name is saved from the row, and synced as entry 2.
            row.custom_name = name;
            return true;
        }
        self.put_saved(id, "CustomName", saved);
        true
    }
    /// Whether the entity's name shows over it at any distance
    /// (`Entity.isCustomNameVisible`); false for the kinds that keep no name.
    pub(in crate::game) fn entity_custom_name_visible(&self, entity: EntityRef) -> Option<bool> {
        let t = self.native_entity_target(entity)?;
        let Some(id) = named_id(t) else {
            return Some(false);
        };
        Some(match t {
            Target::Mob(_) => self
                .common
                .mobs
                .iter()
                .any(|m| m.id == id && m.name_visible),
            Target::Stand(_) => self
                .common
                .armor_stands
                .iter()
                .any(|a| a.id == id && a.name_visible),
            Target::General(_) => self
                .common
                .resident_entities
                .general
                .row(id)
                .is_some_and(|row| row.custom_name_visible),
            _ => self
                .common
                .persist_raw(id)
                .is_some_and(|r| flag(r, "CustomNameVisible")),
        })
    }
    /// Shows the entity's name at any distance, or only when looked at
    /// (`Entity.setCustomNameVisible`): saved, and sent to the players
    /// tracking a mob or an armor stand. False where `set_custom_name` is.
    pub(in crate::game) fn entity_set_custom_name_visible(
        &mut self,
        entity: EntityRef,
        visible: bool,
    ) -> bool {
        let Some(id) = self.name_holder(entity) else {
            return false;
        };
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            m.name_visible = visible;
        } else if let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id) {
            a.name_visible = visible;
        } else if let Some(row) = self.resident_entities.general.row_mut(id) {
            row.custom_name_visible = visible;
            return true;
        }
        // `Entity.saveWithoutId` writes the flag only when set.
        let flag = visible.then_some(Tag::Byte(1));
        self.put_saved(id, "CustomNameVisible", flag);
        true
    }
    /// The category of the entity's type (`EntityType.getCategory`): a mob's
    /// from its type, `Monster` for the dragon, `Misc` for every other kind.
    pub(in crate::game) fn entity_classification(&self, entity: EntityRef) -> Option<MobCategory> {
        Some(match self.native_entity_target(entity)? {
            Target::Mob(id) => {
                mob_category(self.common.mobs.iter().find(|m| m.id == id)?.category())
            }
            Target::Dragon(_) => MobCategory::Monster,
            _ => MobCategory::Misc,
        })
    }
    /// `EntityType.getCategory()` of a registered type with no entity: a mob type's declared
    /// category (the mob table holds vanilla's and the mods' mob types), `Monster` for the
    /// dragon, `Misc` for every other registered type.
    pub(in crate::game) fn entity_type_category(
        &self,
        key: mod_api::EntityTypeKey,
    ) -> Option<MobCategory> {
        let name = mod_api::__private::entity_type_key_id(key)?;
        Some(match &*name {
            "minecraft:ender_dragon" => MobCategory::Monster,
            name => self
                .common
                .res
                .mob_types()
                .get(name)
                .map_or(MobCategory::Misc, |kind| mob_category(&kind.category)),
        })
    }
    /// Whether the entity is in its level: from when `entity_join_level` let
    /// it in (false while that hook runs for it and before) until it
    /// leaves or a mod removes it. Players, and the kinds that hook doesn't
    /// tell (displays, markers, interactions, evoker fangs, area effect
    /// clouds, ominous item spawners), are in it while the handle names them.
    /// False for a stale handle or on a client.
    pub(in crate::game) fn entity_added_to_level(&self, entity: EntityRef) -> bool {
        match self.native_entity_target(entity) {
            None => false,
            Some(
                Target::Player
                | Target::Guest(_)
                | Target::Other(_)
                | Target::Fangs(_)
                | Target::Cloud(_)
                | Target::Bolt(_)
                | Target::Spawner(_),
            ) => true,
            Some(_) => self
                .common
                .persist
                .live
                .binary_search_by_key(&entity.id(), |l| l.0)
                .is_ok(),
        }
    }
    /// The id of the entity a custom-name write changes, or None for a kind
    /// that keeps no name, a stale handle or on a client.
    fn name_holder(&self, entity: EntityRef) -> Option<u32> {
        if !self.common.is_logical_server() {
            return None;
        }
        named_id(self.native_entity_target(entity)?)
    }
    /// Sets (or with `None` removes) `key` in entity `id`'s saved compound.
    pub fn put_saved(&mut self, id: u32, key: &str, value: Option<Tag>) {
        match value {
            Some(v) => {
                let raw = (self.common.persist.raw)
                    .entry(id)
                    .or_insert_with(|| Tag::Compound(Compound::new()));
                if let Tag::Compound(c) = raw {
                    c.insert(key.into(), v);
                }
            }
            None => {
                if let Some(Tag::Compound(c)) = self.common.persist.raw.get_mut(&id) {
                    c.remove(&key.into());
                }
            }
        }
    }
}

/// A mob's `Entity.getCustomName`: its styled name, else the plain name a spawn gave it.
pub(in crate::game) fn mob_custom_name(mob: &sunrise_sim::mobs::Mob) -> Option<Text> {
    mob.custom_name_text
        .clone()
        .or_else(|| mob.custom_name.as_deref().map(Text::literal))
}

/// The id of an entity whose custom name the game keeps and saves
/// (`target_name` reads it): the struct's own for mobs, armor stands,
/// item frames and general rows, the saved compound's for the rest.
fn named_id(t: Target) -> Option<u32> {
    match t {
        Target::Mob(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Item(id)
        | Target::Painting(id)
        | Target::Boat(id)
        | Target::Cart(id)
        | Target::Other(id)
        | Target::Fangs(id)
        | Target::Cloud(id)
        | Target::Bolt(id)
        | Target::General(id) => Some(id),
        _ => None,
    }
}

/// A mob type's `MobCategory` from its table name (`monster`); an unknown name is `Misc`.
fn mob_category(name: &str) -> MobCategory {
    match name {
        "monster" => MobCategory::Monster,
        "creature" => MobCategory::Creature,
        "ambient" => MobCategory::Ambient,
        "axolotls" => MobCategory::Axolotls,
        "underground_water_creature" => MobCategory::UndergroundWaterCreature,
        "water_creature" => MobCategory::WaterCreature,
        "water_ambient" => MobCategory::WaterAmbient,
        _ => MobCategory::Misc,
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
