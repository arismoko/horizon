//! Enchanting what mobs spawn with (`Mob.populateDefaultEquipmentEnchantments`
//! through the `mob_spawn_equipment` provider) and the raid buffs
//! (`Raider.applyRaidBuffs`).
use crate::game::ServerGame;
use sunrise_core::random::RandomSource;
use sunrise_sim::{
    mobs::{
        drops::{self, SLOTS},
        finalize::equip_stack,
        gear::{self, MAIN},
        spawner::equipment_table,
    },
    raid::{self, RaiderType},
};
use {sunrise_core::nbt::Tag, sunrise_world::loot::EntityLootParams};

/// The mobs that call `populateDefaultEquipmentEnchantments` when they spawn.
impl ServerGame<'_> {
    /// `Mob.equip(EquipmentTable)` for a spawner's `equipment`: the loot table
    /// (the mob as `this`, its random sequence or the level's random) is rolled
    /// and each stack goes in the slot `EquipmentUser.resolveSlot` picks, with
    /// the table's drop chance for that slot.
    pub(in crate::game::server_side) fn equip_mob_from_table(
        &self,
        mob: &mut sunrise_sim::mobs::Mob,
        table: &Tag,
        random: &mut sunrise_core::random::LegacyRandom,
        sequences: &mut sunrise_items::random_sequences::RandomSequences,
    ) {
        let Some((name, chances)) = equipment_table(table) else {
            return;
        };
        let Some(loot) = self.server.server_loot.0.get() else {
            return;
        };
        if !loot.has_table(&name) {
            return;
        }
        let params = EntityLootParams {
            this: drops::loot_entity_with_variants(mob, None, Some(self.world_variants())),
            ..EntityLootParams::default()
        };
        let seed = self.common.level.seed;
        let stacks = match loot.random_sequence(&name) {
            Some(sequence) => {
                let random = sequences.get(sequence, seed);
                loot.roll_entity(&name, random, &params)
            }
            None => loot.roll_entity(&name, random, &params),
        };
        let mut inserted: Vec<usize> = Vec::new();
        for stack in stacks.iter().filter(|s| !s.is_empty()) {
            // `resolveSlot`: an `equippable` item goes in its own slot, any
            // other in the main hand, each slot once.
            let slot = match self
                .common
                .res
                .items()
                .get(&stack.item)
                .and_then(|i| i.equip_slot.as_deref())
            {
                Some(name) => SLOTS.iter().position(|s| *s == name),
                None => Some(0),
            };
            let Some(slot) = slot.filter(|s| !inserted.contains(s)) else {
                continue;
            };
            equip_stack(mob, slot, stack, chances[slot]);
            inserted.push(slot);
        }
    }
    /// `SkeletonTrapGoal.enchant` (`EnchantmentHelper.enchantItemFromProvider`
    /// with `mob_spawn_equipment` and `skeleton.getRandom()`): nothing is drawn
    /// when the slot is empty.
    pub fn enchant_item(&mut self, id: u32, slot: usize, special: f32) {
        let items = server_world_items!(self);
        let enchantments = &server_world_stations!(self).enchantments;
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let Some(stack) = sunrise_sim::mobs::finalize::item_stack(m, slot) else {
            return;
        };
        let picked = enchantments.enchant_from_provider(
            "minecraft:mob_spawn_equipment",
            items,
            &mut m.random,
            &stack,
            special,
        );
        let slot = sunrise_sim::mobs::finalize::gear_slot(slot);
        for (enchantment, level) in picked {
            gear::upgrade(m, slot, &enchantment, level);
        }
    }
    /// `Pillager` / `Vindicator.applyRaidBuffs`: enchants a fresh crossbow or
    /// iron axe through the world's raid enchantment providers.
    pub(in crate::game::server_side) fn apply_raid_buffs(
        &mut self,
        id: u32,
        kind: RaiderType,
        wave: i32,
        odds: f32,
    ) {
        let Some(pos) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| [m.position.x, m.position.y, m.position.z])
        else {
            return;
        };
        let special = self.local_difficulty(pos).special();
        let items = server_world_items!(self);
        let enchantments = &server_world_stations!(self).enchantments;
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let enchant = m.random.next_float() <= odds;
        match kind {
            RaiderType::Pillager if enchant => {
                let provider = if wave > raid::num_groups(2) {
                    Some("minecraft:raid/pillager_post_wave_5")
                } else if wave > raid::num_groups(1) {
                    Some("minecraft:raid/pillager_post_wave_3")
                } else {
                    None
                };
                if let Some(provider) = provider {
                    let crossbow = sunrise_items::items::ItemStack::new("minecraft:crossbow", 1);
                    let picked = enchantments.enchant_from_provider(
                        provider,
                        items,
                        &mut m.random,
                        &crossbow,
                        special,
                    );
                    sunrise_sim::mobs::finalize::set_item(
                        m,
                        sunrise_sim::mobs::finalize::MAINHAND,
                        "minecraft:crossbow",
                    );
                    for (enchantment, level) in picked {
                        gear::upgrade(m, MAIN, &enchantment, level);
                    }
                }
            }
            RaiderType::Vindicator => {
                let axe = sunrise_items::items::ItemStack::new("minecraft:iron_axe", 1);
                let picked = if enchant {
                    let provider = if wave > raid::num_groups(2) {
                        "minecraft:raid/vindicator_post_wave_5"
                    } else {
                        "minecraft:raid/vindicator"
                    };
                    enchantments.enchant_from_provider(
                        provider,
                        items,
                        &mut m.random,
                        &axe,
                        special,
                    )
                } else {
                    Vec::new()
                };
                sunrise_sim::mobs::finalize::set_item(
                    m,
                    sunrise_sim::mobs::finalize::MAINHAND,
                    "minecraft:iron_axe",
                );
                for (enchantment, level) in picked {
                    gear::upgrade(m, MAIN, &enchantment, level);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn raid_pillager_crossbow_uses_pack_enchantment_provider() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let root = game.common.options.resources.clone();
            let dir = tempfile::tempdir().unwrap();
            let providers = dir
                .path()
                .join("datapacks/raid_override/data/minecraft/enchantment_provider/raid");
            std::fs::create_dir_all(&providers).unwrap();
            std::fs::write(
                dir.path().join("datapacks/raid_override/pack.mcmeta"),
                r#"{"pack":{"description":"raid provider override"}}"#,
            )
            .unwrap();
            std::fs::write(
                providers.join("pillager_post_wave_3.json"),
                r#"{"type":"minecraft:single","enchantment":"minecraft:multishot","level":1}"#,
            )
            .unwrap();
            std::fs::write(
                dir.path().join("datapacks.json"),
                r#"{"enabled":["file/raid_override"],"disabled":[]}"#,
            )
            .unwrap();
            game.server.world_root = dir.path().to_path_buf();
            let stack = crate::game::server_side::functions::pack_stack(
                &game.server.world_root,
                &root,
                &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
                &Default::default(),
                false,
                &[],
                &[],
            );
            game.server.world_tables.0 = game
                .stage_world_tables(&stack, false, &root)
                .unwrap()
                .tables;
            let id = game
                .spawn_mob_raw("minecraft:pillager", [8.5, 64.0, 8.5], false)
                .unwrap();
            // Wave 4 > num_groups(1) = 3; odds 1.0 guarantees enchanting.
            game.apply_raid_buffs(id, RaiderType::Pillager, 4, 1.0);
            let m = game.common.mobs.iter().find(|m| m.id == id).unwrap();
            assert_eq!(gear::level(m, MAIN, "minecraft:multishot"), 1);
            assert_eq!(gear::level(m, MAIN, "minecraft:quick_charge"), 0);
        });
    }
}
