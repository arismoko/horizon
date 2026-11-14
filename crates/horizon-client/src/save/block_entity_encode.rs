use super::{block_entity_id, encode_items, item_to_nbt, string, uuid_tag};
use crate::{
    beehive::BeehiveExt,
    mobs::{spawner::SpawnerExt, trial_spawner::TrialSpawnerExt},
};
use horizon_core::nbt::{Compound, Tag};

/// Chunk storage adds `keepPacked` after `BlockEntity.saveWithFullMetadata`.
pub fn encode(p: (i32, i32, i32), e: &crate::containers::BlockEntity, block: &str) -> Tag {
    let mut tag = metadata(fields(e, block, None, None, None, None), p, e, block);
    if !matches!(e, crate::containers::BlockEntity::Mod(_))
        && let Tag::Compound(map) = &mut tag
    {
        map.insert("keepPacked".into(), Tag::Byte(0));
    }
    tag
}

/// `BlockDataAccessor.getData`: write superclass/subclass fields before
/// `BlockEntity.saveWithoutMetadata`'s components and saveMetadata's id/position.
/// Borrowed external fields participate in construction, not completed-map repair.
pub fn data(
    p: (i32, i32, i32),
    e: &crate::containers::BlockEntity,
    block: &str,
    lock: Option<&Tag>,
    custom_name: Option<&Tag>,
    pending: Option<&super::PendingContainerLoot>,
    hopper_cooldown: Option<i32>,
) -> Tag {
    let mut map = fields(e, block, lock, custom_name, pending, hopper_cooldown);
    if matches!(e, crate::containers::BlockEntity::Mod(_)) {
        map.remove(&"keepPacked".into());
    }
    metadata(map, p, e, block)
}

fn fields(
    e: &crate::containers::BlockEntity,
    block: &str,
    lock: Option<&Tag>,
    custom_name: Option<&Tag>,
    pending: Option<&super::PendingContainerLoot>,
    hopper_cooldown: Option<i32>,
) -> Compound {
    use crate::containers::BlockEntity;
    let pending = if matches!(e, BlockEntity::Chest(_)) { pending } else { None };
    let components = match e {
        BlockEntity::Banner(b) => b.components.clone(),
        BlockEntity::CopperGolemStatue(components) => components.clone(),
        _ => Compound::new(),
    };
    let mut entries = Vec::new();
    match e {
        // EnderChestBlockEntity saves nothing of its own: the items are the player's.
        BlockEntity::Chest(_) if block == "minecraft:ender_chest" => {}
        BlockEntity::Chest(items) => {
            // Pending loot normally suppresses Items, but preserve a live
            // nonempty modeled inventory just as the raw inspection boundary does.
            if pending.is_none() || items.iter().any(Option::is_some) {
                let list = encode_items(items);
                // ShulkerBoxBlockEntity.saveAdditional omits its empty list.
                let empty = matches!(&list, Tag::List(l) if l.is_empty());
                if !(empty && block.ends_with("shulker_box")) {
                    entries.push(("Items", list));
                }
            }
        }
        BlockEntity::Furnace(f) => {
            entries.push(("Items", encode_items(&f.items)));
            entries.push(("lit_time_remaining", Tag::Int(f.lit_remaining)));
            entries.push(("lit_total_time", Tag::Int(f.lit_total)));
            entries.push(("cooking_time_spent", Tag::Int(f.cooking_timer)));
            entries.push(("cooking_total_time", Tag::Int(f.cooking_total)));
            entries.push(("speed_multiplier", Tag::Float(f.speed.to_bits())));
            let used: Compound = f
                .recipes_used
                .iter()
                .map(|(id, count)| (id.as_str().into(), Tag::Int(*count as i32)))
                .collect();
            entries.push(("RecipesUsed", Tag::Compound(used)));
        }
        BlockEntity::Brewing(b) => {
            entries.push(("Items", encode_items(&b.items)));
            entries.push(("BrewTime", Tag::Int(b.brew_time)));
            entries.push(("total_brew_time", Tag::Int(b.total_brew_time)));
            entries.push(("Fuel", Tag::Int(b.fuel)));
            entries.push(("total_fuel", Tag::Int(b.total_fuel)));
            entries.push(("speed_multiplier", Tag::Float(b.speed.to_bits())));
        }
        BlockEntity::Sign(sign) => entries.extend(sign.encode()),
        BlockEntity::Jukebox(j) => {
            if let Some(record) = j.record() {
                entries.push(("RecordItem", item_to_nbt(record)));
            }
            if j.song_clock_present {
                entries.push(("ticks_since_song_started", Tag::Long(j.ticks)));
            }
        }
        BlockEntity::Beacon(b) => {
            if let Some(p) = &b.primary {
                entries.push(("primary_effect", string(p)));
            }
            if let Some(s) = &b.secondary {
                entries.push(("secondary_effect", string(s)));
            }
            entries.push(("Levels", Tag::Int(b.levels)));
        }
        BlockEntity::Lectern(l) => {
            if let Some(book) = &l.book {
                entries.push(("Book", item_to_nbt(book)));
                entries.push(("Page", Tag::Int(l.page)));
            }
        }
        BlockEntity::Sculk(s) => entries.extend(s.to_nbt()),
        BlockEntity::Shelf(shelf) => {
            entries.push(("Items", encode_items(&shelf.items)));
            entries.push((
                "align_items_to_bottom",
                Tag::Byte(i8::from(shelf.align_to_bottom)),
            ));
        }
        // ChiseledBookShelfBlockEntity.saveAdditional.
        BlockEntity::ChiseledBookshelf(b) => {
            entries.push(("Items", encode_items(&b.items)));
            entries.push(("last_interacted_slot", Tag::Int(b.last_slot)));
        }
        // CampfireBlockEntity.saveAdditional.
        BlockEntity::Campfire(c) => {
            entries.push(("Items", encode_items(&c.items)));
            entries.push(("CookingTimes", Tag::IntArray(c.progress.to_vec())));
            entries.push(("CookingTotalTimes", Tag::IntArray(c.total.to_vec())));
        }
        BlockEntity::Crafter(c) => {
            entries.push(("Items", encode_items(&c.items)));
            entries.push((
                "disabled_slots",
                Tag::IntArray(
                    (0..9)
                        .filter(|&i| c.disabled[i])
                        .map(|i| i as i32)
                        .collect(),
                ),
            ));
            entries.push(("triggered", Tag::Int(c.triggered)));
            entries.push(("crafting_ticks_remaining", Tag::Int(c.crafting_ticks)));
        }
        BlockEntity::Banner(b) => {
            if !b.layers.is_empty() {
                entries.push(("patterns", crate::banner::layers_to_nbt(&b.layers)));
            }
            if let Some(name) = &b.name {
                entries.push(("CustomName", string(name)));
            }
        }
        BlockEntity::CreakingHeart(h) => {
            if let Some((_, uuid)) = h.creaking {
                entries.push(("creaking", uuid_tag(uuid)));
            }
        }
        BlockEntity::Skull(s) => {
            if let Some(profile) = &s.profile {
                entries.push(("profile", profile.clone()));
            }
            if let Some(sound) = &s.note_block_sound {
                entries.push(("note_block_sound", string(sound)));
            }
            if let Some(name) = &s.custom_name {
                entries.push(("custom_name", name.clone()));
            }
        }
        // TheEndGatewayBlockEntity.saveAdditional.
        BlockEntity::EndGateway(g) => {
            entries.push(("Age", Tag::Long(g.age)));
            if let Some(exit) = g.exit_portal {
                entries.push(("exit_portal", Tag::IntArray(exit.to_vec())));
            }
            if g.exact_teleport {
                entries.push(("ExactTeleport", Tag::Byte(1)));
            }
        }
        BlockEntity::Spawner(s) => entries.extend(s.save()),
        BlockEntity::TrialSpawner(s) => entries.extend(s.save()),
        BlockEntity::Vault(v) => entries.extend(v.save()),
        BlockEntity::Beehive(h) => entries.extend(h.to_nbt()),
        BlockEntity::CommandBlock(c) => entries.extend(c.save()),
        BlockEntity::StructureBlock(b) => entries.extend(b.save()),
        BlockEntity::JigsawBlock(b) => entries.extend(b.save()),
        BlockEntity::TestBlock(b) => entries.extend(b.save()),
        BlockEntity::TestInstanceBlock(b) => entries.extend(b.save()),
        BlockEntity::Brushable(b) => {
            if let Some(table) = &b.loot_table {
                entries.push(("LootTable", table.clone()));
                if let Some(seed) = &b.table_seed_tag {
                    entries.push(("LootTableSeed", seed.clone()));
                }
            } else if let Some(item) = &b.item {
                entries.push(("item", item_to_nbt(item)));
            }
        }
        // ConduitBlockEntity.saveAdditional: EntityReference.store.
        BlockEntity::Conduit(target) => {
            if let Some(uuid) = target {
                entries.push(("Target", Tag::IntArray(uuid.to_vec())));
            }
        }
        // An enchanting table's CustomName is the external block name.
        BlockEntity::DaylightDetector
        | BlockEntity::Bell
        | BlockEntity::EndPortal
        | BlockEntity::EnchantingTable
        | BlockEntity::CopperGolemStatue(_) => {}
        BlockEntity::Mod(m) => {
            if m.items.iter().any(Option::is_some) {
                entries.push(("Items", encode_items(&m.items)));
            }
        }
        // PotentSulfurBlockEntity.saveAdditional.
        BlockEntity::PotentSulfur(sulfur) => {
            entries.push(("countdown", Tag::Int(sulfur.waiting_countdown)))
        }
    }
    let mut map = match e {
        BlockEntity::Mod(m) => m.nbt.clone(),
        _ => Compound::new(),
    };
    // BaseContainerBlockEntity.saveAdditional writes lock, then CustomName,
    // before the subclass's inventory and other fields.
    if let Some(lock) = lock {
        map.insert("lock".into(), lock.clone());
    }
    if let Some(name) = custom_name
        && !entries.iter().any(|(key, _)| *key == "CustomName")
    {
        map.entry("CustomName".into()).or_insert_with(|| name.clone());
    }
    if let Some(pending) = pending {
        pending.write_tags(&mut map);
    }
    map.extend(entries.into_iter().map(|(key, value)| (key.into(), value)));
    if let Some(cooldown) = hopper_cooldown {
        map.insert("TransferCooldown".into(), Tag::Int(cooldown));
    }
    if !matches!(e, BlockEntity::Mod(_)) {
        map.insert("components".into(), Tag::Compound(components));
    }
    map
}

fn metadata(
    mut map: Compound,
    p: (i32, i32, i32),
    e: &crate::containers::BlockEntity,
    block: &str,
) -> Tag {
    map.insert("id".into(), string(block_entity_id(e, block)));
    for (key, value) in [("x", p.0), ("y", p.1), ("z", p.2)] {
        map.insert(key.into(), Tag::Int(value));
    }
    Tag::Compound(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::containers::BlockEntity;

    #[test]
    fn conduit_data_uses_actual_save_history_before_metadata() {
        // Vanilla 26.3's observed conduit /data display: Target and components
        // collide, and saveAdditional writes Target before the common tail.
        let entity = BlockEntity::Conduit(Some([1, 2, 3, 4]));
        let tag = data((4098, 300, 4098), &entity, "minecraft:conduit", None, None, None, None);
        assert_eq!(
            horizon_client_core::snbt::pretty(&tag).plain(),
            r#"{Target: [I; 1, 2, 3, 4], components: {}, x: 4098, y: 300, z: 4098, id: "minecraft:conduit"}"#,
        );
        assert!(tag.get("keepPacked").is_none());
        assert_eq!(encode((4098, 300, 4098), &entity, "minecraft:conduit").get("keepPacked"), Some(&Tag::Byte(0)));
    }
}
