use super::*;

/// A click sent to a server: its slot, button and input, the slots it
/// changed and the carried stack after it.
pub type Predicted = (
    i16,
    i32,
    Input,
    Vec<(i16, Option<ItemStack>)>,
    Option<ItemStack>,
);

/// How many stacks the creative screen keeps in step with the server (`Remote::synced`).
pub const CREATIVE_SYNCED: usize = 41;

/// The player's stacks in `Remote::synced` order: the 36 inventory slots, the off hand, then
/// the armour from the head down. `CreativeInventoryListener.slotChanged` sends every
/// InventoryMenu slot the creative screen changes, the armour slots too.
pub fn creative_sync_stacks(
    inventory: &[Option<ItemStack>],
    offhand: &Option<ItemStack>,
    armor: &[Option<ItemStack>; 4],
) -> Vec<Option<ItemStack>> {
    let mut stacks: Vec<Option<ItemStack>> = inventory.iter().take(36).cloned().collect();
    stacks.resize(36, None);
    stacks.push(offhand.clone());
    stacks.extend(armor.iter().cloned());
    stacks
}

/// Entries of `creative_sync_stacks` that went from `before` to `now` are synced as `now`.
pub fn mark_synced(
    synced: &mut [Option<ItemStack>],
    before: &[Option<ItemStack>],
    now: Vec<Option<ItemStack>>,
) {
    for (i, stack) in now.into_iter().enumerate() {
        if before.get(i) != Some(&stack)
            && let Some(s) = synced.get_mut(i)
        {
            *s = stack;
        }
    }
}

/// InventoryMenu's slot for entry `i` of `creative_sync_stacks`: the hotbar is 36..=44, the
/// main inventory 9..=35, the off hand 45 and the armour 5..=8 (head first).
pub fn creative_sync_slot(i: usize) -> i16 {
    (match i {
        0..9 => 36 + i,
        36 => 45,
        37.. => 5 + (i - 37),
        _ => i,
    }) as i16
}

/// `MultiPlayerGameMode.handleContainerInput` for each click on the engine's
/// copy of the menu: the click with the slots it changed and the carried
/// stack after it, and whether a click dropped a stack. Every drop of
/// `AbstractContainerMenu.doClick` (a THROW, a click outside the window, a
/// swap or a result move with no room left) is `Prediction.PREDICTED`.
/// None when the engine refuses a click.
pub fn predict_clicks(
    m: &mut sunrise_sim::menu::Menu,
    env: &sunrise_sim::menu::Env,
    clicks: &[(i16, i32, Input)],
) -> Option<(Vec<Predicted>, bool)> {
    predict_clicks_with(m, env, clicks, None)
}

/// Like [`predict_clicks`], applying `last_stacked` (and `env.slot_rules`) to the final click of
/// `clicks` when client prediction decided a resolved pickup via `ItemPart::item_stacked_on_other`.
pub fn predict_clicks_with(
    m: &mut sunrise_sim::menu::Menu,
    env: &sunrise_sim::menu::Env,
    clicks: &[(i16, i32, Input)],
    last_stacked: Option<&sunrise_sim::menu::StackedOutcome>,
) -> Option<(Vec<Predicted>, bool)> {
    let mut predicted = Vec::with_capacity(clicks.len());
    let mut dropped = false;
    let last = clicks.len().saturating_sub(1);
    for (index, &(slot, button, input)) in clicks.iter().enumerate() {
        let before = menu_items(m);
        let thrown = m.dropped.len();
        let step_env = if index == last {
            sunrise_sim::menu::Env {
                stacked: last_stacked,
                ..*env
            }
        } else {
            sunrise_sim::menu::Env {
                stacked: None,
                ..*env
            }
        };
        m.click(&step_env, i32::from(slot), button, input).ok()?;
        dropped |= m.dropped.len() > thrown;
        let changed = changed_slots(&before, m);
        predicted.push((slot, button, input, changed, m.carried.clone()));
    }
    Some((predicted, dropped))
}

pub fn menu_items(m: &sunrise_sim::menu::Menu) -> Vec<Option<ItemStack>> {
    (0..m.slot_count()).map(|i| m.slot(i).cloned()).collect()
}

/// The slots whose stack is no longer `before`'s (`ItemStack.matches`).
/// `MultiPlayerGameMode.handleContainerInput`'s changed slots, in the order the packet writes them.
pub fn changed_slots(
    before: &[Option<ItemStack>],
    m: &sunrise_sim::menu::Menu,
) -> Vec<(i16, Option<ItemStack>)> {
    let changed: Vec<i16> = (0..m.slot_count())
        .filter(|&i| m.slot(i) != before.get(i).and_then(Option::as_ref))
        .map(|i| i as i16)
        .collect();
    open_hash_order(&changed)
        .into_iter()
        .map(|i| (i, m.slot(i as usize).cloned()))
        .collect()
}

/// The order a new fastutil `Int2ObjectOpenHashMap` iterates `keys` put in this order: vanilla
/// collects a click's changed slots in one and writes them as it iterates. The key 0 comes first,
/// then the table from its last position down; a key sits at `HashCommon.mix(key) & mask` or the
/// next free position. The table starts at 32 (16 expected, load factor 0.75) and doubles when an
/// insert finds it holding 24 (`size++ >= maxFill`), re-inserting from its last position down.
pub fn open_hash_order(keys: &[i16]) -> Vec<i16> {
    fn mix(key: i16) -> usize {
        let h = i32::from(key).wrapping_mul(0x9E37_79B9_u32 as i32);
        (h ^ ((h as u32) >> 16) as i32) as u32 as usize
    }
    fn put(table: &mut [Option<i16>], key: i16) {
        let mask = table.len() - 1;
        let mut pos = mix(key) & mask;
        while table[pos].is_some() {
            pos = (pos + 1) & mask;
        }
        table[pos] = Some(key);
    }
    // HashCommon.maxFill(n, 0.75): min(ceil(n * 0.75), n - 1).
    let max_fill = |n: usize| (n * 3).div_ceil(4).min(n - 1);
    let mut table = vec![None; 32];
    let (mut zero, mut size) = (false, 0);
    for &key in keys {
        if key == 0 {
            zero = true;
        } else {
            put(&mut table, key);
        }
        size += 1;
        // `size++ >= maxFill`: the size before this key.
        if size > max_fill(table.len()) {
            // HashCommon.arraySize(size + 1, 0.75): the power of two holding size + 1 at that load.
            let n = ((size + 1) * 4).div_ceil(3).next_power_of_two();
            let mut grown = vec![None; n];
            for &key in table.iter().rev().flatten() {
                put(&mut grown, key);
            }
            table = grown;
        }
    }
    let rest = table.iter().rev().flatten().copied();
    zero.then_some(0).into_iter().chain(rest).collect()
}

/// Map a menu slot to Sunrise's inventory screen slot ids.
pub fn inventory_screen_slot(menu: &str, slot: i32) -> Option<usize> {
    crate::game::inventory_screen_slot_with_type(menu, slot, || {
        sunrise_level::containers::mod_menu_type(menu)
    })
}

/// Map a slot when the caller already knows the window's registered layout.
pub fn inventory_screen_slot_with_type<'m>(
    menu: &str,
    slot: i32,
    mod_type: impl FnOnce() -> Option<&'m sunrise_level::containers::ModMenuType>,
) -> Option<usize> {
    let s = usize::try_from(slot).ok()?;
    // A container of `n` slots, then the inventory and the hotbar.
    let slots_with_base = |n: usize, base: usize| -> Option<usize> {
        match s {
            _ if s < n => Some(base + s),
            _ if s < n + 27 => Some(9 + s - n),
            _ if s < n + 36 => Some(s - n - 27),
            _ => None,
        }
    };
    let slots = |n| slots_with_base(n, 300);
    let rows = |n: usize| slots(n * 9);
    match menu {
        "inventory" => match s {
            0 => Some(200),
            1..=4 => Some(100 + s - 1),
            5..=8 => Some(500 + s - 5),
            9..=35 => Some(s),
            36..=44 => Some(s - 36),
            // InventoryMenu.SHIELD_SLOT: the off hand.
            45 => Some(504),
            _ => None,
        },
        "minecraft:crafting" => match s {
            0 => Some(200),
            1..=9 => Some(100 + s - 1),
            10..=36 => Some(9 + s - 10),
            37..=45 => Some(s - 37),
            _ => None,
        },
        "minecraft:merchant" => match s {
            0 | 1 => Some(crate::game::stations_common::INPUT + s),
            2 => Some(crate::game::stations_common::RESULT),
            3..=29 => Some(9 + s - 3),
            30..=38 => Some(s - 30),
            _ => None,
        },
        "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => match s {
            0..=2 => Some(300 + s),
            3..=29 => Some(9 + s - 3),
            30..=38 => Some(s - 30),
            _ => None,
        },
        "minecraft:generic_9x1" => rows(1),
        "minecraft:generic_9x2" => rows(2),
        "minecraft:generic_9x3" | "minecraft:shulker_box" => rows(3),
        "minecraft:generic_9x4" => rows(4),
        "minecraft:generic_9x5" => rows(5),
        "minecraft:generic_9x6" => rows(6),
        // HopperMenu; DispenserMenu (dispensers and droppers).
        "minecraft:hopper" => slots(5),
        "minecraft:generic_3x3" => slots(9),
        "minecraft:brewing_stand" => slots(5),
        // CrafterMenu: the grid, the inventory, then the result (the
        // client's own).
        "minecraft:crafter_3x3" => slots(9),
        // HorseInventoryMenu: the saddle, the body armour, the chest,
        // then the inventory.
        m if m.starts_with(MOUNT_MENU) => {
            let n = m[MOUNT_MENU.len()..].parse::<usize>().ok()? * 3;
            match s {
                0 => Some(sunrise_gameplay_common::mount_rules::SADDLE),
                1 => Some(sunrise_gameplay_common::mount_rules::ARMOR),
                _ if s < 2 + n => Some(sunrise_gameplay_common::mount_rules::CHEST + s - 2),
                _ if s < 2 + n + 27 => Some(9 + s - 2 - n),
                _ if s < 2 + n + 36 => Some(s - 2 - n - 27),
                _ => None,
            }
        }
        // A content mod's menu type: its container, then the inventory.
        // A workstation: its inputs, its result, then the inventory.
        m => {
            if let Some(t) = mod_type() {
                use crate::game::inventory_slots::MOD_MENU_SLOT_BASE;
                return if t.player_inventory {
                    slots_with_base(t.slots, MOD_MENU_SLOT_BASE)
                } else {
                    (s < t.slots).then_some(MOD_MENU_SLOT_BASE + s)
                };
            }
            let kind = crate::game::remote_station_kind(m)?;
            let inputs = kind.inputs();
            let own = inputs + usize::from(kind.has_result());
            match s {
                _ if s < inputs => Some(crate::game::stations_common::INPUT + s),
                _ if s < own => Some(crate::game::stations_common::RESULT),
                _ if s < own + 27 => Some(9 + s - own),
                _ if s < own + 36 => Some(s - own - 27),
                _ => None,
            }
        }
    }
}

/// The workstation screen of a menu type a server opens.
pub fn remote_station_kind(menu: &str) -> Option<crate::game::stations_common::Kind> {
    use crate::game::stations_common::Kind;
    Some(match menu {
        "minecraft:loom" => Kind::Loom,
        "minecraft:stonecutter" => Kind::Stonecutter,
        "minecraft:grindstone" => Kind::Grindstone,
        "minecraft:anvil" => Kind::Anvil,
        "minecraft:smithing" => Kind::Smithing,
        "minecraft:enchantment" => Kind::Enchanting,
        "minecraft:beacon" => Kind::Beacon,
        "minecraft:cartography_table" => Kind::Cartography,
        _ => return None,
    })
}
