//! A server-side roll of `LivingEntity.dropFromLootTable`, collected instead of spawned.

#[cfg(test)]
#[path = "death_loot_tests.rs"]
mod tests;

use crate::game::{ServerGame, server_side::loot_rolls::ListenedRolls};
use mod_api::{DeathLootError, EntityRef, Identifier, LivingRef, LootDamageSource, PlayerRef};
use sunrise_gameplay_common::game::{
    commands_common::{CommandsCommonGameExt2 as _, Target},
    creatures::from_loot,
    native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _,
    native_living_body::NativeLivingBodyGameExt6 as _,
    native_server::PlayerRead,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::drops::{self, PlainRolls};
use sunrise_world::{
    attribute_map::ValueKind,
    loot::{EntityLootParams, LootDamage},
};

impl ServerGame<'_> {
    pub fn entity_roll_death_loot(
        &mut self,
        entity: EntityRef,
        table: Option<&Identifier>,
        source: LootDamageSource<'_>,
        last_damage_player: Option<PlayerRef>,
    ) -> Result<Vec<ItemStack>, DeathLootError> {
        if !self.is_logical_server() {
            return Err(DeathLootError::Unavailable);
        }
        let target = self
            .native_entity_target(entity)
            .ok_or(DeathLootError::StaleEntity)?;
        if !target.is_living() {
            return Err(DeathLootError::NotLiving);
        }
        if self.target_level(target) != self.dim {
            return Err(DeathLootError::StaleEntity);
        }
        let origin = self
            .target_position(target)
            .ok_or(DeathLootError::StaleEntity)?;
        let damage_type = source.damage_type.to_string();
        if self.health_resources.damage.id(&damage_type).is_none() {
            return Err(DeathLootError::UnknownDamageType);
        }
        let mob = match target {
            Target::Mob(id) => Some(
                self.mobs
                    .iter()
                    .find(|m| m.id == id)
                    .ok_or(DeathLootError::StaleEntity)?,
            ),
            _ => None,
        };
        let seed = mob.map_or(0, |m| m.extra.death_table_seed);
        let table = table.map(ToString::to_string).unwrap_or_else(|| match mob {
            Some(m) => drops::mob_loot_table(m),
            None => drops::entity_type_loot_table(&self.target_type(target)),
        });
        let loot = self
            .server
            .server_loot
            .0
            .get()
            .ok_or(DeathLootError::Unavailable)?;
        if !loot.has_table(&table) {
            return Err(DeathLootError::UnknownTable);
        }

        let resolve = |handle: Option<EntityRef>| {
            handle
                .map(|e| {
                    self.native_entity_target(e)
                        .ok_or(DeathLootError::StaleEntity)
                })
                .transpose()
        };
        let attacker = resolve(source.causing)?;
        let direct = resolve(source.direct)?;
        let credited = last_damage_player
            .map(|player| {
                let read = self
                    .player_handle_read(player)
                    .ok_or(DeathLootError::StaleEntity)?;
                Ok(match read {
                    PlayerRead::Acting { body: Some(_), .. } => Target::Player,
                    PlayerRead::Guest(guest) => Target::Guest(guest.conn),
                    PlayerRead::Acting { body: None, .. } => {
                        return Err(DeathLootError::StaleEntity);
                    }
                })
            })
            .transpose()?;

        let this = self.target_loot_entity(target);
        let source_entity = attacker.map(|t| self.target_loot_entity(t));
        let direct_entity = if direct == attacker {
            source_entity.clone()
        } else {
            direct.map(|t| self.target_loot_entity(t))
        };
        let credit = credited.map(|t| {
            let luck = self
                .native_target_attribute(t, "minecraft:luck", ValueKind::Computed)
                .unwrap_or(0.0) as f32;
            (self.target_loot_entity(t), luck)
        });
        let params = EntityLootParams::death(
            this,
            Some(LootDamage {
                kind: damage_type.clone(),
                source: source_entity,
                direct: direct_entity,
                is_direct: source.causing == source.direct,
            }),
            credit,
        );
        let world_seed = self.level.seed;
        let stacks = self.caller_loot(Some(origin), None).with(|| {
            if mod_api::__private::listened(mod_api::HookId::EnchantedEntityLoot) {
                let hurt = mod_api::entity::health::HurtSource {
                    damage_type: &damage_type,
                    causing: source.causing.map_or(0, |e| e.id() as i32),
                    direct: source.direct.map_or(0, |e| e.id() as i32),
                    position: direct.and_then(|t| self.target_position(t)),
                    direct_type: None,
                };
                let mut rolls = ListenedRolls::new(self, LivingRef::new(entity), &hurt);
                drops::roll_death_table(&loot, &table, &params, &mut rolls, seed, world_seed)
            } else {
                let mut rolls = PlainRolls {
                    sequences: &mut self.server.rand_sequences,
                    level_random: &mut self.common.world.random,
                };
                drops::roll_death_table(&loot, &table, &params, &mut rolls, seed, world_seed)
            }
        });
        Ok(stacks
            .iter()
            .filter(|stack| !stack.is_empty())
            .map(from_loot)
            .collect())
    }
}
