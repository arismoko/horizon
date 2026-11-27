//! A mob lent during finalization or the position check, or just after a natural spawn attempt:
//! the scoped read-only view ([`BorrowedUnspawnedMob`]) and mutable editor
//! ([`BorrowedUnspawnedMobMut`]), the shared read/write helpers used by pending children, and
//! the spawn group data at the API boundary (`SpawnGroup`).

use super::living_attributes::{AttributeWrite, collect_equipment, write_unspawned_attribute};
use crate::game::ServerGame;
use mod_api::{
    AgeableGroup, AttributeModifier, AttributeOperation, EntityRef, EquipmentSlot, Identifier,
    ItemStack, SpawnGroup, TropicalVariant, UnspawnedMobError, UnspawnedMobRead, UnspawnedMobWrite,
    WorldEpoch,
};
use std::sync::Arc;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
use sunrise_items::items::Items;
use sunrise_sim::mobs::finalize::{Ageable, Group};
use sunrise_sim::mobs::{Mob, equine, llama};
use sunrise_world::attribute_map::ValueKind;
use sunrise_world::attributes::{Operation, entity::Catalog};

/// The most tags an entity holds (`Entity.addTag`).
const MAX_TAGS: usize = 1024;

/// Read-only scoped borrow of an unspawned mob (or metadata-only when `mob` is `None`).
pub struct BorrowedUnspawnedMob<'a> {
    kind: &'a str,
    position: [f64; 3],
    mob: Option<&'a Mob>,
    attributes: &'a Arc<Catalog>,
}

impl UnspawnedMobRead for BorrowedUnspawnedMob<'_> {
    fn kind(&self) -> Result<Identifier, UnspawnedMobError> {
        match self.mob {
            Some(mob) => read_mob_kind(mob),
            None => Identifier::parse(self.kind).map_err(|_| UnspawnedMobError::Unsupported),
        }
    }

    fn position(&self) -> Result<[f64; 3], UnspawnedMobError> {
        match self.mob {
            Some(mob) => read_mob_position(mob),
            None => Ok(self.position),
        }
    }

    fn equipment(&self, slot: EquipmentSlot) -> Result<Option<ItemStack>, UnspawnedMobError> {
        read_mob_equipment(self.mob.ok_or(UnspawnedMobError::Unsupported)?, slot)
    }

    fn attribute_base(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        read_mob_attribute_base(
            self.mob.ok_or(UnspawnedMobError::Unsupported)?,
            self.attributes,
            attribute,
        )
    }

    fn attribute_value(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        read_mob_attribute_value(
            self.mob.ok_or(UnspawnedMobError::Unsupported)?,
            self.attributes,
            attribute,
        )
    }

    fn tags(&self) -> Result<Vec<String>, UnspawnedMobError> {
        read_mob_tags(self.mob.ok_or(UnspawnedMobError::Unsupported)?)
    }

    fn custom_name(&self) -> Result<Option<mod_api::Text>, UnspawnedMobError> {
        read_mob_custom_name(self.mob.ok_or(UnspawnedMobError::Unsupported)?)
    }
}

/// Mutable scoped loan of an unspawned mob (or metadata-only when `mob` is `None`).
pub struct BorrowedUnspawnedMobMut<'a> {
    kind: &'a str,
    position: [f64; 3],
    mob: Option<&'a mut Mob>,
    items: &'a Items,
    attributes: &'a Arc<Catalog>,
}

impl UnspawnedMobRead for BorrowedUnspawnedMobMut<'_> {
    fn kind(&self) -> Result<Identifier, UnspawnedMobError> {
        match self.mob.as_deref() {
            Some(mob) => read_mob_kind(mob),
            None => Identifier::parse(self.kind).map_err(|_| UnspawnedMobError::Unsupported),
        }
    }

    fn position(&self) -> Result<[f64; 3], UnspawnedMobError> {
        match self.mob.as_deref() {
            Some(mob) => read_mob_position(mob),
            None => Ok(self.position),
        }
    }

    fn equipment(&self, slot: EquipmentSlot) -> Result<Option<ItemStack>, UnspawnedMobError> {
        read_mob_equipment(
            self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?,
            slot,
        )
    }

    fn attribute_base(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        read_mob_attribute_base(
            self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?,
            self.attributes,
            attribute,
        )
    }

    fn attribute_value(&self, attribute: &Identifier) -> Result<f64, UnspawnedMobError> {
        read_mob_attribute_value(
            self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?,
            self.attributes,
            attribute,
        )
    }

    fn tags(&self) -> Result<Vec<String>, UnspawnedMobError> {
        read_mob_tags(self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?)
    }

    fn custom_name(&self) -> Result<Option<mod_api::Text>, UnspawnedMobError> {
        read_mob_custom_name(self.mob.as_deref().ok_or(UnspawnedMobError::Unsupported)?)
    }
}

impl UnspawnedMobWrite for BorrowedUnspawnedMobMut<'_> {
    fn set_equipment(
        &mut self,
        slot: EquipmentSlot,
        item: Option<ItemStack>,
    ) -> Result<(), UnspawnedMobError> {
        write_mob_equipment(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            self.items,
            self.attributes,
            slot,
            item,
        )
    }

    fn set_attribute_base(
        &mut self,
        attribute: &Identifier,
        base: f64,
    ) -> Result<(), UnspawnedMobError> {
        write_mob_attribute_base(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            self.items,
            self.attributes,
            attribute,
            base,
        )
    }

    fn add_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        modifier: AttributeModifier,
    ) -> Result<(), UnspawnedMobError> {
        write_mob_add_attribute_modifier(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            self.items,
            self.attributes,
            attribute,
            modifier,
        )
    }

    fn remove_attribute_modifier(
        &mut self,
        attribute: &Identifier,
        id: &Identifier,
    ) -> Result<bool, UnspawnedMobError> {
        write_mob_remove_attribute_modifier(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            self.items,
            self.attributes,
            attribute,
            id,
        )
    }

    fn add_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError> {
        write_mob_add_tag(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            tag,
        )
    }

    fn remove_tag(&mut self, tag: &str) -> Result<bool, UnspawnedMobError> {
        write_mob_remove_tag(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            tag,
        )
    }

    fn set_custom_name(&mut self, name: Option<mod_api::Text>) -> Result<(), UnspawnedMobError> {
        write_mob_set_custom_name(
            self.mob
                .as_deref_mut()
                .ok_or(UnspawnedMobError::Unsupported)?,
            name,
        )
    }
}

pub(super) fn read_mob_kind(mob: &Mob) -> Result<Identifier, UnspawnedMobError> {
    Identifier::parse(&mob.kind.id).map_err(|_| UnspawnedMobError::Unsupported)
}

pub(super) fn read_mob_position(mob: &Mob) -> Result<[f64; 3], UnspawnedMobError> {
    Ok([mob.position.x, mob.position.y, mob.position.z])
}

pub(super) fn read_mob_equipment(
    mob: &Mob,
    slot: EquipmentSlot,
) -> Result<Option<ItemStack>, UnspawnedMobError> {
    mob.equipment_stacks()[slot as usize]
        .try_into_api()
        .map_err(|error| error.into_write_error().into())
}

pub(super) fn read_mob_attribute_base(
    mob: &Mob,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
) -> Result<f64, UnspawnedMobError> {
    read_mob_attribute(mob, attributes, attribute, ValueKind::Base)
}

pub(super) fn read_mob_attribute_value(
    mob: &Mob,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
) -> Result<f64, UnspawnedMobError> {
    read_mob_attribute(mob, attributes, attribute, ValueKind::Computed)
}

fn read_mob_attribute(
    mob: &Mob,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    kind: ValueKind,
) -> Result<f64, UnspawnedMobError> {
    mob.extra
        .live
        .read(
            attributes,
            &mob.kind.id,
            &attribute.to_string(),
            kind,
            || {
                sunrise_sim::entity_nbt::attribute_entries(mob)
                    .into_iter()
                    .map(|(_, entry)| entry)
                    .collect()
            },
        )
        .ok_or(UnspawnedMobError::Unsupported)
}

pub(super) fn read_mob_tags(mob: &Mob) -> Result<Vec<String>, UnspawnedMobError> {
    Ok(mob.extra.spawn_tags.clone())
}

pub(super) fn read_mob_custom_name(mob: &Mob) -> Result<Option<mod_api::Text>, UnspawnedMobError> {
    mob.custom_name_text.try_into_api().map_err(Into::into)
}

pub(super) fn write_mob_equipment(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    slot: EquipmentSlot,
    item: Option<ItemStack>,
) -> Result<(), UnspawnedMobError> {
    let stack = item
        .try_into_engine(items)
        .map_err(|_| UnspawnedMobError::Invalid)?;
    if !mob.set_equipment_stack(slot as usize, stack) {
        return Err(UnspawnedMobError::Unsupported);
    }
    collect_equipment(attributes, items, mob);
    Ok(())
}

pub(super) fn write_mob_attribute_base(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    base: f64,
) -> Result<(), UnspawnedMobError> {
    if !base.is_finite() {
        return Err(UnspawnedMobError::Invalid);
    }
    write_mob_attribute(
        mob,
        items,
        attributes,
        attribute,
        AttributeWrite::Base(base),
    )
    .map(|_| ())
}

pub(super) fn write_mob_add_attribute_modifier(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    modifier: AttributeModifier,
) -> Result<(), UnspawnedMobError> {
    let operation = match modifier.operation {
        AttributeOperation::AddValue => Operation::AddValue,
        AttributeOperation::AddMultipliedBase => Operation::AddMultipliedBase,
        AttributeOperation::AddMultipliedTotal => Operation::AddMultipliedTotal,
        _ => return Err(UnspawnedMobError::Invalid),
    };
    if !modifier.amount.is_finite() {
        return Err(UnspawnedMobError::Invalid);
    }
    let write = AttributeWrite::add(
        &modifier.id.to_string(),
        modifier.amount,
        operation,
        true,
        false,
    )
    .ok_or(UnspawnedMobError::Invalid)?;
    match write_mob_attribute(mob, items, attributes, attribute, write)? {
        true => Ok(()),
        false => Err(UnspawnedMobError::Invalid),
    }
}

pub(super) fn write_mob_remove_attribute_modifier(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    id: &Identifier,
) -> Result<bool, UnspawnedMobError> {
    write_mob_attribute(
        mob,
        items,
        attributes,
        attribute,
        AttributeWrite::Remove(id.to_string()),
    )
}

fn write_mob_attribute(
    mob: &mut Mob,
    items: &Items,
    attributes: &Arc<Catalog>,
    attribute: &Identifier,
    write: AttributeWrite,
) -> Result<bool, UnspawnedMobError> {
    let name = attribute.to_string();
    // Every write here is a mod's, so a base stays in the attribute's range.
    let write = match write {
        AttributeWrite::Base(base) => AttributeWrite::Base(
            attributes
                .definition(&name)
                .map_or(base, |d| d.sanitize(base)),
        ),
        other => other,
    };
    write_unspawned_attribute(attributes, items, mob, &name, write)
        .ok_or(UnspawnedMobError::Unsupported)
}

pub(super) fn write_mob_add_tag(mob: &mut Mob, tag: &str) -> Result<bool, UnspawnedMobError> {
    let tags = &mut mob.extra.spawn_tags;
    if tags.iter().any(|t| t == tag) {
        return Ok(false);
    }
    if tag.is_empty() || tags.len() >= MAX_TAGS {
        return Err(UnspawnedMobError::Invalid);
    }
    tags.push(tag.to_owned());
    Ok(true)
}

pub(super) fn write_mob_remove_tag(mob: &mut Mob, tag: &str) -> Result<bool, UnspawnedMobError> {
    let tags = &mut mob.extra.spawn_tags;
    let before = tags.len();
    tags.retain(|t| t != tag);
    Ok(tags.len() != before)
}

pub(super) fn write_mob_set_custom_name(
    mob: &mut Mob,
    name: Option<mod_api::Text>,
) -> Result<(), UnspawnedMobError> {
    let name = name.try_into_engine(&())?;
    mob.custom_name = name.as_ref().map(Text::plain);
    mob.custom_name_text = name;
    Ok(())
}

impl ServerGame<'_> {
    /// Builds a read-only scoped borrow of an unspawned mob (or metadata-only when `mob` is `None`).
    pub fn borrowed_unspawned_mob<'a>(
        &'a self,
        kind: &'a str,
        position: [f64; 3],
        mob: Option<&'a Mob>,
    ) -> BorrowedUnspawnedMob<'a> {
        BorrowedUnspawnedMob {
            kind,
            position,
            mob,
            attributes: &self.common.motion.attributes,
        }
    }

    /// Builds a mutable scoped loan of an unspawned mob (or metadata-only when `mob` is `None`).
    pub fn borrowed_unspawned_mob_mut<'a>(
        &'a self,
        kind: &'a str,
        position: [f64; 3],
        mob: Option<&'a mut Mob>,
    ) -> BorrowedUnspawnedMobMut<'a> {
        BorrowedUnspawnedMobMut {
            kind,
            position,
            mob,
            items: server_world_items!(self),
            attributes: &self.common.motion.attributes,
        }
    }

    /// The world a lend's handles belong to; a game with none open lends
    /// under epoch 0, which no open world has.
    pub fn lend_epoch(&self) -> WorldEpoch {
        self.common
            .native_lifecycle
            .epoch()
            .unwrap_or(WorldEpoch::new(0))
    }

    /// The tags a mod gave the mobs from index `from` on before they joined
    /// move to where the level keeps a joined entity's tags.
    pub(in crate::game) fn join_spawn_tags(&mut self, from: usize) {
        for i in from..self.common.mobs.len() {
            if self.common.mobs[i].extra.spawn_tags.is_empty() {
                continue;
            }
            let given = std::mem::take(&mut self.common.mobs[i].extra.spawn_tags);
            let target = super::commands::Target::Mob(self.common.mobs[i].id);
            let mut tags = self.tags_of(target);
            for tag in given {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
            self.set_entity_tags(target, &tags);
        }
    }
}

/// The pack's group data as the API's, a school's leader a handle of `epoch`;
/// None where a value has no API name.
pub(in crate::game) fn native_spawn_group(
    group: &Group,
    epoch: WorldEpoch,
    variants: &sunrise_sim::mobs::variants::VariantTables,
) -> Option<SpawnGroup> {
    let named = |names: &[&'static str], i: u8| names.get(usize::from(i)).map(|n| (*n).to_owned());
    Some(match group {
        Group::Ageable(a) => SpawnGroup::Ageable(native_ageable(a)),
        Group::Zombie { baby, jockey, husk } => SpawnGroup::Zombie {
            baby: *baby,
            jockey: *jockey,
            tried_camel: *husk,
        },
        Group::Spider(effect) => SpawnGroup::Spider {
            effect: match effect {
                Some(e) => Some(Identifier::parse(e).ok()?),
                None => None,
            },
        },
        Group::Horse(a, v) => SpawnGroup::Horse {
            ageable: native_ageable(a),
            variant: named(&equine::VARIANT_NAMES, *v)?,
        },
        Group::Llama(a, v) => SpawnGroup::Llama {
            ageable: native_ageable(a),
            variant: named(&llama::VARIANTS, *v)?,
        },
        Group::Fox(a, v) => SpawnGroup::Fox {
            ageable: native_ageable(a),
            variant: named(&FOX_VARIANTS, *v)?,
        },
        Group::Rabbit(a, v) => SpawnGroup::Rabbit {
            ageable: native_ageable(a),
            variant: RABBIT_VARIANTS
                .iter()
                .find(|(id, _)| id == v)
                .map(|(_, n)| (*n).to_owned())?,
        },
        Group::Axolotl(a, [first, second]) => SpawnGroup::Axolotl {
            ageable: native_ageable(a),
            variants: [
                named(&sunrise_sim::entity_nbt::AXOLOTL_VARIANTS, *first)?,
                named(&sunrise_sim::entity_nbt::AXOLOTL_VARIANTS, *second)?,
            ],
        },
        Group::Wolf(a, v) => SpawnGroup::Wolf {
            ageable: native_ageable(a),
            variant: Identifier::parse(variants.wolf.id(*v)?).ok()?,
        },
        Group::School(leader, look) => SpawnGroup::School {
            leader: EntityRef::new(epoch, *leader),
            tropical: match look {
                Some((pattern, base, color)) => Some(TropicalVariant::new(
                    named(&sunrise_sim::entity_nbt::TROPICAL_PATTERNS, *pattern)?,
                    named(&sunrise_level::dyes::DYE_NAMES, *base)?,
                    named(&sunrise_level::dyes::DYE_NAMES, *color)?,
                )),
                None => None,
            },
        },
    })
}

/// The API's group data as the game's; None for a name the game does not know,
/// which drops the group.
pub(in crate::game) fn spawn_group(
    group: &SpawnGroup,
    variants: &sunrise_sim::mobs::variants::VariantTables,
) -> Option<Group> {
    let index = |names: &[&str], name: &str| names.iter().position(|n| *n == name).map(|i| i as u8);
    Some(match group {
        SpawnGroup::Ageable(a) => Group::Ageable(ageable(a)),
        SpawnGroup::Zombie {
            baby,
            jockey,
            tried_camel,
        } => Group::Zombie {
            baby: *baby,
            jockey: *jockey,
            husk: *tried_camel,
        },
        SpawnGroup::Spider { effect } => Group::Spider(match effect {
            Some(id) => {
                let def = sunrise_game_data::potions::effect(&id.to_string())?;
                Some(sunrise_game_data::potions::namespaced(def.id))
            }
            None => None,
        }),
        SpawnGroup::Horse {
            ageable: a,
            variant,
        } => Group::Horse(ageable(a), index(&equine::VARIANT_NAMES, variant)?),
        SpawnGroup::Llama {
            ageable: a,
            variant,
        } => Group::Llama(ageable(a), index(&llama::VARIANTS, variant)?),
        SpawnGroup::Fox {
            ageable: a,
            variant,
        } => Group::Fox(ageable(a), index(&FOX_VARIANTS, variant)?),
        SpawnGroup::Rabbit {
            ageable: a,
            variant,
        } => Group::Rabbit(
            ageable(a),
            RABBIT_VARIANTS.iter().find(|(_, n)| n == variant)?.0,
        ),
        SpawnGroup::Axolotl {
            ageable: a,
            variants: [first, second],
        } => Group::Axolotl(
            ageable(a),
            [
                index(&sunrise_sim::entity_nbt::AXOLOTL_VARIANTS, first)?,
                index(&sunrise_sim::entity_nbt::AXOLOTL_VARIANTS, second)?,
            ],
        ),
        SpawnGroup::Wolf {
            ageable: a,
            variant,
        } => {
            let r = variants.wolf.lookup(variant.as_str())?;
            Group::Wolf(ageable(a), r)
        }
        SpawnGroup::School { leader, tropical } => Group::School(
            leader.id(),
            match tropical {
                Some(look) => Some((
                    index(&sunrise_sim::entity_nbt::TROPICAL_PATTERNS, &look.pattern)?,
                    index(&sunrise_level::dyes::DYE_NAMES, &look.base_color)?,
                    index(&sunrise_level::dyes::DYE_NAMES, &look.pattern_color)?,
                )),
                None => None,
            },
        ),
        _ => return None,
    })
}

/// `Fox.Variant` names, by the game's index.
const FOX_VARIANTS: [&str; 2] = ["red", "snow"];
/// `Rabbit.Variant`: the game's id and its saved name.
const RABBIT_VARIANTS: [(u8, &str); 7] = [
    (0, "brown"),
    (1, "white"),
    (2, "black"),
    (3, "white_splotched"),
    (4, "gold"),
    (5, "salt"),
    (99, "evil"),
];

fn native_ageable(a: &Ageable) -> AgeableGroup {
    let mut out = AgeableGroup::new(a.should_spawn_baby, a.baby_chance);
    out.size = a.size;
    out
}

fn ageable(a: &AgeableGroup) -> Ageable {
    Ageable {
        size: a.size,
        should_spawn_baby: a.should_spawn_baby,
        baby_chance: a.baby_chance,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use sunrise_gameplay_common::game::server_side::native_unspawned_mob::tests::AT;

    #[cfg(test)]
    use sunrise_gameplay_common::game::server_side::native_unspawned_mob::tests::id;

    #[cfg(test)]
    use super::*;
    use sunrise_gameplay_common::game::creatures::SpawnSite;
    use sunrise_sim::mobs::spawn_rules::Reason;

    pub fn spawn_pig(game: &mut crate::game::ServerGame<'_>) -> Option<u32> {
        game.spawn_mob(
            "minecraft:pig",
            AT,
            false,
            SpawnSite::Finalizing(Reason::Natural),
        )
    }

    #[cfg(test)]
    /// Group data crosses the boundary by the names vanilla saves its
    /// variants under, both ways, and a name the game does not know drops
    /// the group.
    #[test]
    fn group_data_crosses_by_vanilla_names() {
        let epoch = WorldEpoch::new(3);
        let ageable = Ageable::babies(true);
        let api = AgeableGroup::new(true, 0.05);
        let variants = sunrise_sim::mobs::variants::VariantTables {
            wolf: sunrise_sim::mobs::variants::VariantTable::from_entries(vec![
                (
                    ("minecraft".into(), "spotted".into()),
                    sunrise_sim::mobs::variants::WolfVariant {
                        assets: sunrise_sim::mobs::variants::WolfAssetInfo {
                            wild: "minecraft:entity/wolf/wolf_spotted".into(),
                            tame: "minecraft:entity/wolf/wolf_spotted_tame".into(),
                            angry: "minecraft:entity/wolf/wolf_spotted_angry".into(),
                        },
                        baby_assets: sunrise_sim::mobs::variants::WolfAssetInfo {
                            wild: "minecraft:entity/wolf/wolf_spotted_baby".into(),
                            tame: "minecraft:entity/wolf/wolf_spotted_tame_baby".into(),
                            angry: "minecraft:entity/wolf/wolf_spotted_angry_baby".into(),
                        },
                        spawn_conditions: Vec::new(),
                    },
                ),
                (
                    ("test".into(), "mint".into()),
                    sunrise_sim::mobs::variants::WolfVariant {
                        assets: sunrise_sim::mobs::variants::WolfAssetInfo {
                            wild: "test:entity/wolf/mint".into(),
                            tame: "test:entity/wolf/mint_tame".into(),
                            angry: "test:entity/wolf/mint_angry".into(),
                        },
                        baby_assets: sunrise_sim::mobs::variants::WolfAssetInfo {
                            wild: "test:entity/wolf/mint_baby".into(),
                            tame: "test:entity/wolf/mint_tame_baby".into(),
                            angry: "test:entity/wolf/mint_angry_baby".into(),
                        },
                        spawn_conditions: Vec::new(),
                    },
                ),
            ]),
            ..Default::default()
        };
        let spotted = variants.wolf.lookup("minecraft:spotted").unwrap();
        let mint = variants.wolf.lookup("test:mint").unwrap();
        let cases = [
            (
                Group::Horse(ageable, 1),
                SpawnGroup::Horse {
                    ageable: api,
                    variant: "creamy".into(),
                },
            ),
            (
                Group::Rabbit(ageable, 99),
                SpawnGroup::Rabbit {
                    ageable: api,
                    variant: "evil".into(),
                },
            ),
            (
                Group::Fox(ageable, 1),
                SpawnGroup::Fox {
                    ageable: api,
                    variant: "snow".into(),
                },
            ),
            (
                Group::Axolotl(ageable, [0, 4]),
                SpawnGroup::Axolotl {
                    ageable: api,
                    variants: ["lucy".into(), "blue".into()],
                },
            ),
            (
                Group::Wolf(ageable, spotted),
                SpawnGroup::Wolf {
                    ageable: api,
                    variant: id("minecraft:spotted"),
                },
            ),
            (
                Group::Wolf(ageable, mint),
                SpawnGroup::Wolf {
                    ageable: api,
                    variant: id("test:mint"),
                },
            ),
            (
                Group::Spider(Some("minecraft:speed")),
                SpawnGroup::Spider {
                    effect: Some(id("minecraft:speed")),
                },
            ),
            (
                Group::School(7, Some((0, 1, 0))),
                SpawnGroup::School {
                    leader: EntityRef::new(epoch, 7),
                    tropical: Some(TropicalVariant::new(
                        "kob".into(),
                        "orange".into(),
                        "white".into(),
                    )),
                },
            ),
            (
                Group::Zombie {
                    baby: true,
                    jockey: false,
                    husk: Some(true),
                },
                SpawnGroup::Zombie {
                    baby: true,
                    jockey: false,
                    tried_camel: Some(true),
                },
            ),
        ];
        for (game, api) in cases {
            assert_eq!(
                native_spawn_group(&game, epoch, &variants).as_ref(),
                Some(&api)
            );
            assert_eq!(spawn_group(&api, &variants), Some(game));
        }
        let unknown = SpawnGroup::Horse {
            ageable: api,
            variant: "plaid".into(),
        };
        assert_eq!(spawn_group(&unknown, &variants), None);
        let unknown_wolf = SpawnGroup::Wolf {
            ageable: api,
            variant: id("test:missing"),
        };
        assert_eq!(spawn_group(&unknown_wolf, &variants), None);
    }
}
