use super::*;

/// The block entity type ID for the block it sits in (a chest-like container
/// is a chest, trapped chest or barrel).
pub fn block_entity_id<'a>(e: &'a crate::containers::BlockEntity, block: &'a str) -> &'a str {
    match e {
        crate::containers::BlockEntity::Chest(_) => chest_backed_id(e.id(), block),
        _ => e.id(),
    }
}
/// The type ID of a chest-backed container whose own ID is `id`, for the block it sits in.
pub fn chest_backed_id<'a>(id: &'a str, block: &'a str) -> &'a str {
    match crate::block_entity_types::factory_block(block) {
        block @ ("minecraft:trapped_chest"
        | "minecraft:barrel"
        | "minecraft:dropper"
        | "minecraft:ender_chest") => block,
        b if b.ends_with("shulker_box") => "minecraft:shulker_box",
        _ => id,
    }
}
/// A live block entity's `current` encoding over the `original` NBT it was loaded from:
/// what the game left unchanged keeps its original form, what it changed or cleared takes the
/// current value, and keys Sunrise never models (a stored `components` patch, a mod's
/// fields) stay. The chunk save and a loot roll's `saveWithFullMetadata` both write this.
/// `original` must have the entity's own id.
pub fn overlay_block_entity_source(
    original: &Tag,
    e: &crate::containers::BlockEntity,
    block: &str,
    current: &Tag,
    items: &crate::items::Items,
) -> Tag {
    let hopper_cooldown = (block == "minecraft:hopper")
        .then(|| number(original.get("TransferCooldown")).unwrap_or(-1) as i32);
    let items_are_payload =
        matches!(e, crate::containers::BlockEntity::Mod(_)) && block_mod_item_slots(block) == 0;
    let loaded = decode_block_entity_with_history(original, None, Some(block)).map_or_else(
        || current.clone(),
        |(q, e)| {
            // External name removal must win over the source overlay.
            let custom_name = (!matches!(
                e,
                crate::containers::BlockEntity::Banner(_)
                    | crate::containers::BlockEntity::Sign(_)
                    | crate::containers::BlockEntity::Mod(_)
            ))
            .then(|| {
                original
                    .get("CustomName")
                    .and_then(crate::block_entity_nbt::component)
            })
            .flatten();
            block_entity_encode::encode_with(
                q,
                &e,
                block,
                block_entity_encode::DataFields {
                    lock: original.get("lock"),
                    custom_name: custom_name.as_ref(),
                    pending: PendingContainerLoot::from_tag(original).as_ref(),
                    hopper_cooldown,
                },
                items,
                |_| None,
            )
        },
    );
    merge_block_entity(original, &loaded, current, items_are_payload)
}
/// `block_entity_nbt::overlay`, then a container that now holds items loses its
/// loot table (vanilla clears it when the loot is generated into the container).
pub(super) fn merge_block_entity(
    original: &Tag,
    loaded: &Tag,
    current: &Tag,
    items_are_payload: bool,
) -> Tag {
    let mut merged = crate::block_entity_nbt::overlay(original, loaded, current, items_are_payload);
    let has_items = !items_are_payload
        && matches!(current.get("Items"), Some(Tag::List(items)) if !items.is_empty());
    if has_items
        && current.get("LootTable").is_none()
        && let Tag::Compound(map) = &mut merged
    {
        map.remove(&"LootTable".into());
        map.remove(&"LootTableSeed".into());
    }
    merged
}
/// `BannerBlockEntity.loadAdditional` (and the `components` a block entity keeps).
pub fn decode_banner(tag: &Tag) -> crate::containers::Banner {
    crate::containers::Banner {
        layers: crate::banner::layers_from_nbt(tag.get("patterns")),
        name: text(tag.get("CustomName")),
        components: match tag.get("components") {
            Some(Tag::Compound(c)) => c.clone(),
            _ => Compound::new(),
        },
    }
}
/// `TheEndGatewayBlockEntity.loadAdditional`.
pub fn decode_end_gateway(tag: &Tag) -> crate::containers::EndGateway {
    crate::containers::EndGateway {
        age: number(tag.get("Age")).unwrap_or(0),
        exit_portal: tag
            .get("exit_portal")
            .and_then(crate::block_entity_nbt::gateway_exit),
        exact_teleport: crate::entity_nbt::flag(tag, "ExactTeleport"),
    }
}
/// A mod's block entity: its compound without `id` and the position, and
/// (when `mod_item_slots > 0`) the `Items` rows within `0..mod_item_slots`
/// that can be represented losslessly as modeled slots.
/// Slotless holders (`mod_item_slots == 0`) keep `Items` verbatim in `nbt`,
/// and rows outside `0..mod_item_slots`, duplicates, invalid or noncanonical
/// rows, and slotless entries stay verbatim in `nbt`.
/// The game sizes the modeled vector to its container when it uses it.
pub(super) fn decode_mod_block_entity(
    id: String,
    tag: &Tag,
    mod_item_slots: usize,
) -> Option<crate::containers::ModBlockEntity> {
    use crate::block_entity_nbt::{item_slot_tag, saved_item_slot};
    let Tag::Compound(map) = tag else {
        return None;
    };
    let mut nbt = map.clone();
    for key in ["id", "x", "y", "z"] {
        nbt.remove(&key.into());
    }
    // The published shapes are no field of the mod's: saved data that does not check is dropped.
    let shapes = nbt
        .remove(&crate::containers::SHAPES_KEY.into())
        .and_then(|tag| crate::containers::PublishedShapes::from_tag(&tag));
    let mut items = Vec::new();
    if mod_item_slots > 0 {
        let key = "Items".into();
        if let Some(Tag::List(entries)) = nbt.get_mut(&key) {
            let mut occurrences: HashMap<usize, u8> = HashMap::new();
            for slot in entries.iter().filter_map(saved_item_slot) {
                let count = occurrences.entry(slot).or_insert(0);
                *count = count.saturating_add(1);
            }
            entries.retain(|raw| {
                let Some(slot) = saved_item_slot(raw) else {
                    return true;
                };
                if slot < mod_item_slots
                    && occurrences.get(&slot) == Some(&1)
                    && let Some(slot_tag) = item_slot_tag(slot)
                    && let Some(stack) = item_from_nbt(raw)
                    && let Tag::Compound(mut canonical) = Tag::from(&stack)
                {
                    canonical.insert("Slot".into(), slot_tag);
                    if raw == &Tag::Compound(canonical) {
                        if items.len() <= slot {
                            items.resize(slot + 1, None);
                        }
                        items[slot] = Some(stack);
                        return false;
                    }
                }
                true
            });
            if entries.is_empty() && !items.is_empty() {
                nbt.remove(&key);
            }
        }
    }
    let mut entity = crate::containers::ModBlockEntity::new(
        id,
        crate::containers::ModBlockEntityFields {
            nbt,
            items,
            data: Vec::new(),
        },
    );
    entity.set_shapes(shapes.map(std::sync::Arc::new));
    Some(entity)
}
/// Compact unsigned-byte slot count (`0..=255`) decoded when a mod block's
/// declared container shape is unknown (`decode_block_entity`).
const COMPACT_MOD_ITEM_SLOTS: usize = 256;

/// The declared item slot count owned by a mod block's stock container.
pub fn block_mod_item_slots(block: &str) -> usize {
    mod_api::__private::parts_installation::block_container(block)
        .map_or(0, |container| container.slots)
}
/// A `RandomizableContainer`: a chest, barrel, shulker box, dispenser or hopper, or a mod
/// block entity whose block declares container slots. Its `LootTable` and `LootTableSeed`
/// wait beside it until its first access rolls them.
pub fn randomizable_container(e: &crate::containers::BlockEntity, block: &str) -> bool {
    match e {
        crate::containers::BlockEntity::Chest(_) => true,
        crate::containers::BlockEntity::Mod(_) => block_mod_item_slots(block) > 0,
        _ => false,
    }
}

/// `RandomizableContainerBlockEntity.loadAdditional` for a container with a valid
/// `LootTable` (`tryLoadLootTable`): the inventory starts empty and `Items` is not read. The
/// table and seed leave the payload too, to wait beside the block entity. None without a
/// valid table.
fn without_container_loot(tag: &Tag) -> Option<Tag> {
    PendingContainerLoot::from_tag(tag)?;
    let Tag::Compound(map) = tag else {
        return None;
    };
    let mut map = map.clone();
    for key in ["Items", "LootTable", "LootTableSeed"] {
        map.remove(&key.into());
    }
    Some(Tag::Compound(map))
}

/// Decode an entity at a known block, refusing undeclared IDs and invalid native membership.
/// Canonical membership wins; blocks absent from the table use vanilla membership.
pub fn decode_block_entity_for_block(
    tag: &Tag,
    block: &str,
) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    decode_block_entity_replacing(tag, block, None)
}

/// Like [`decode_block_entity_for_block`], retaining mutable set history when
/// reading new NBT onto an existing trial spawner.
pub fn decode_block_entity_replacing(
    tag: &Tag,
    block: &str,
    previous: Option<&crate::containers::BlockEntity>,
) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    let slots = block_mod_item_slots(block);
    let loot_first = (slots > 0).then(|| without_container_loot(tag)).flatten();
    let tag = loot_first.as_ref().unwrap_or(tag);
    let mut decoded = decode_block_entity_with_history(tag, previous, Some(block))?;
    // Several vanilla types share a decoded representation (for example barrel
    // and chest). Membership belongs to the saved type, not that representation.
    let saved_id = text(tag.get("id"))?;
    let valid = match &mut decoded.1 {
        crate::containers::BlockEntity::Mod(m) => {
            if crate::block_entity_types::valid(block, m.id()) != Some(true) {
                return None;
            }
            // Missing saved stacks are empty slots, whatever the declared capacity: `items` holds
            // the slots up to the last one occupied, and a slot past it reads as empty.
            // Any saved stacks past the declared size stay out of the menu but on disk.
            true
        }
        _ => crate::block_entity_types::valid(block, &saved_id)
            .unwrap_or_else(|| decoded.1.fits(block)),
    };
    valid.then_some(decoded)
}

pub fn decode_block_entity(tag: &Tag) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    decode_block_entity_with_history(tag, None, None)
}

pub(super) fn decode_block_entity_with_history(
    tag: &Tag,
    previous: Option<&crate::containers::BlockEntity>,
    block: Option<&str>,
) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    use crate::containers::BlockEntity;
    let int = |k: &str| tag.get(k).and_then(crate::nbt_path::int_value); // getIntOr
    let id = text(tag.get("id"))?;
    let pos = (int("x")?, int("y")?, int("z")?);
    // A detached decode can still hold an opaque payload without a known block.
    // At a known block, only an admitted native type gets that representation.
    if crate::block_entity_types::declared(&id)
        || (block.is_none() && id.contains(':') && !id.starts_with("minecraft:"))
    {
        return Some((
            pos,
            BlockEntity::Mod(Box::new(decode_mod_block_entity(
                id,
                tag,
                block.map_or(COMPACT_MOD_ITEM_SLOTS, block_mod_item_slots),
            )?)),
        ));
    }
    if block.is_some() && !crate::block_entity_nbt::is_known_type(&id) {
        return None;
    }
    if id == "minecraft:end_gateway" {
        return Some((pos, BlockEntity::EndGateway(decode_end_gateway(tag))));
    }
    if id == "minecraft:jukebox" {
        let record = tag.get("RecordItem").and_then(item_from_nbt);
        let clock = number(tag.get("ticks_since_song_started"));
        // `JukeboxBlockEntity.loadAdditional` -> `setSongWithoutPlaying`: only a
        // saved clock brings a song back, and it comes back without playing.
        let clocked = record.is_some() && clock.is_some();
        return Some((
            pos,
            BlockEntity::Jukebox(crate::containers::Jukebox {
                items: [record],
                ticks: clock.unwrap_or(0),
                song_clock_present: clocked,
                playing: clocked,
                loaded_clock: clocked,
            }),
        ));
    }
    if id == "minecraft:creaking_heart" {
        let heart = crate::containers::CreakingHeart {
            creaking: uuid_from_tag(tag.get("creaking")).map(|uuid| (None, uuid)),
            ..Default::default()
        };
        return Some((pos, BlockEntity::CreakingHeart(Box::new(heart))));
    }
    if id == "minecraft:lectern" {
        // `LecternBlockEntity.loadAdditional`: the page is `Mth.clamp(Page, 0,
        // getPageCount(book) - 1)`, so a stored page before the book's first
        // page or past its last comes back as the page the block holds. No
        // book, or a book with no pages, counts 0 pages: the maximum is then
        // -1, and `Mth.clamp` (`Math.min(Math.max(value, min), max)`) answers
        // it, -1.
        let book = tag.get("Book").and_then(item_from_nbt);
        let count = book.as_ref().map_or(0, crate::menu::book_pages) as i32;
        let lectern = crate::containers::Lectern {
            book,
            page: int("Page").unwrap_or(0).max(0).min(count - 1),
        };
        return Some((pos, BlockEntity::Lectern(lectern)));
    }
    if id == "minecraft:banner" {
        return Some((pos, BlockEntity::Banner(decode_banner(tag))));
    }
    if id == "minecraft:beacon" {
        let beacon = crate::containers::Beacon {
            // BeaconBlockEntity.loadAdditional never reads `Levels`: updateBase sets them.
            primary: text(tag.get("primary_effect")),
            secondary: text(tag.get("secondary_effect")),
            last_check_y: i32::MIN / 2,
            ..Default::default()
        };
        return Some((pos, BlockEntity::Beacon(Box::new(beacon))));
    }
    // A skull's block entity id is not its block's name (every head block).
    if id == "minecraft:skull" {
        return Some((
            pos,
            BlockEntity::Skull(crate::containers::Skull::new(
                tag.get("profile").cloned(),
                text(tag.get("note_block_sound")),
                tag.get("custom_name")
                    .and_then(crate::block_entity_nbt::component),
            )),
        ));
    }
    // A spawner's block entity id is not its block's name.
    let block = match id.as_str() {
        "minecraft:mob_spawner" => "minecraft:spawner",
        other => other,
    };
    let mut entity = BlockEntity::for_block(block)?;
    match &mut entity {
        BlockEntity::Chest(items) => {
            // Randomizable containers reset their inventory, then load Items only
            // when tryLoadLootTable did not read a valid loot-table key.
            let has_table = text(tag.get("LootTable"))
                .and_then(|id| sunrise_game_data::identifier::normalize_id(&id))
                .is_some();
            if !has_table {
                *items = decode_items(tag.get("Items"), items.len());
            }
        }
        BlockEntity::Furnace(f) => {
            let items = decode_items(tag.get("Items"), 3);
            f.items = [items[0].clone(), items[1].clone(), items[2].clone()];
            f.lit_remaining = int("lit_time_remaining").unwrap_or(0);
            f.lit_total = int("lit_total_time").unwrap_or(0);
            f.cooking_timer = int("cooking_time_spent").unwrap_or(0);
            f.cooking_total = int("cooking_total_time").unwrap_or(0);
            if let Some(speed) = float(tag.get("speed_multiplier")) {
                f.speed = speed as f32;
            }
            // A save without it owes nothing: the field is newer than some worlds.
            if let Some(Tag::Compound(used)) = tag.get("RecipesUsed") {
                for (id, count) in used {
                    let count = match count {
                        Tag::Int(n) => *n,
                        _ => continue,
                    };
                    // Saves from before the key was namespaced hold both forms of an id.
                    *f.recipes_used
                        .entry(crate::containers::recipe_used_key(&id.to_string_lossy()))
                        .or_insert(0) += count.max(0) as u32;
                }
            }
        }
        BlockEntity::Brewing(b) => {
            let items = decode_items(tag.get("Items"), 5);
            b.items = [
                items[0].clone(),
                items[1].clone(),
                items[2].clone(),
                items[3].clone(),
                items[4].clone(),
            ];
            b.brew_time = int("BrewTime").unwrap_or(0);
            b.total_brew_time = int("total_brew_time").unwrap_or(400);
            if b.brew_time > 0 {
                b.ingredient = b.items[3].as_ref().map(|s| s.item.clone());
            }
            b.fuel = int("Fuel").unwrap_or(0);
            b.total_fuel = int("total_fuel").unwrap_or(20);
            b.speed = float(tag.get("speed_multiplier")).map_or(1.0, |v| v as f32);
        }
        BlockEntity::Brushable(b) => {
            b.loot_table = tag.get("LootTable").cloned();
            b.table_seed_tag = tag.get("LootTableSeed").cloned();
            b.item = if b.loot_table.is_none() {
                tag.get("item").and_then(item_from_nbt)
            } else {
                None
            };
            b.hit_direction = int("hit_direction").filter(|v| (0..=5).contains(v));
        }
        BlockEntity::Sign(sign) => sign.decode(tag),
        BlockEntity::Sculk(s) => s.load(tag),
        BlockEntity::Shelf(shelf) => {
            for (slot, item) in decode_items(tag.get("Items"), 3).into_iter().enumerate() {
                shelf.items[slot] = item;
            }
            shelf.align_to_bottom = int("align_items_to_bottom").unwrap_or(0) != 0;
        }
        // ChiseledBookShelfBlockEntity.loadAdditional.
        BlockEntity::ChiseledBookshelf(b) => {
            for (slot, item) in decode_items(tag.get("Items"), 6).into_iter().enumerate() {
                b.items[slot] = item;
            }
            b.last_slot = int("last_interacted_slot").unwrap_or(-1);
        }
        // CampfireBlockEntity.loadAdditional: missing time arrays stay zeros.
        BlockEntity::Campfire(c) => {
            for (slot, item) in decode_items(tag.get("Items"), 4).into_iter().enumerate() {
                c.items[slot] = item;
            }
            let ints = |key: &str| match tag.get(key) {
                Some(Tag::IntArray(v)) => std::array::from_fn(|i| v.get(i).copied().unwrap_or(0)),
                _ => [0; 4],
            };
            c.progress = ints("CookingTimes");
            c.total = ints("CookingTotalTimes");
        }
        BlockEntity::Crafter(c) => {
            for (slot, item) in decode_items(tag.get("Items"), 9).into_iter().enumerate() {
                c.items[slot] = item;
            }
            if let Some(Tag::IntArray(slots)) = tag.get("disabled_slots") {
                for &slot in slots {
                    if (0..9).contains(&slot) && c.items[slot as usize].is_none() {
                        c.disabled[slot as usize] = true;
                    }
                }
            }
            c.triggered = int("triggered").unwrap_or(0);
            c.state_triggered = None;
            c.crafting_ticks = int("crafting_ticks_remaining").unwrap_or(0);
        }
        // PotentSulfurBlockEntity.loadAdditional.
        BlockEntity::PotentSulfur(sulfur) => {
            if let Some(countdown) = int("countdown") {
                sulfur.waiting_countdown = countdown;
            }
        }
        BlockEntity::Jukebox(_)
        | BlockEntity::Beacon(_)
        | BlockEntity::Lectern(_)
        | BlockEntity::Banner(_)
        | BlockEntity::CreakingHeart(_)
        | BlockEntity::EndGateway(_)
        | BlockEntity::Skull(_)
        | BlockEntity::DaylightDetector
        | BlockEntity::Bell
        | BlockEntity::EndPortal
        | BlockEntity::EnchantingTable
        | BlockEntity::Mod(_) => {}
        // ConduitBlockEntity.loadAdditional: EntityReference.read through
        // UUIDUtil.CODEC, an int array of four.
        BlockEntity::Conduit(target) => {
            *target = match tag.get("Target") {
                Some(Tag::IntArray(v)) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
                _ => None,
            };
        }
        // `BlockEntity.loadAdditional`: the components the block entity keeps.
        BlockEntity::CopperGolemStatue(components) => {
            *components = match tag.get("components") {
                Some(Tag::Compound(c)) => c.clone(),
                _ => Compound::new(),
            };
        }
        BlockEntity::Spawner(s) => **s = crate::mobs::spawner::Spawner::load(tag),
        BlockEntity::TrialSpawner(s) => {
            let previous = match previous {
                Some(BlockEntity::TrialSpawner(old)) => Some(old.as_ref()),
                _ => None,
            };
            **s = crate::mobs::trial_spawner::TrialSpawner::load(tag, previous);
        }
        BlockEntity::Vault(v) => **v = crate::vault::Vault::load(tag),
        BlockEntity::Beehive(h) => **h = crate::beehive::Beehive::from_nbt(tag),
        BlockEntity::CommandBlock(c) => **c = crate::command_block::CommandBlock::load(tag),
        BlockEntity::StructureBlock(b) => **b = crate::structure_block::StructureBlock::load(tag),
        BlockEntity::TestBlock(b) => **b = crate::test_block::TestBlock::load(tag),
        BlockEntity::TestInstanceBlock(b) => b.load(tag),
        // The state is not known here (Game::decode_block_entity_at applies it).
        BlockEntity::JigsawBlock(b) => {
            **b = crate::jigsaw_block::JigsawBlock::load(
                tag,
                crate::jigsaw_block::JointType::Rollable,
            )
        }
    }
    Some((pos, entity))
}
