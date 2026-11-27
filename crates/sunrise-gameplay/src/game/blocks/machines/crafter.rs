//! The crafter (`CrafterBlock`, `CrafterBlockEntity`, `CrafterMenu`,
//! `CrafterScreen`): redstone-triggered crafting of its 3x3 grid, ejecting
//! the result and remainders into the container in front or as items, the
//! crafting animation ticks, and the menu with its toggleable slots.
use super::*;
use serde_json::Value;
use sunrise_gameplay_common::game::machine_containers::{Kind, View};
#[allow(unused_imports)]
use sunrise_level::containers::Crafter;
#[allow(unused_imports)]
use sunrise_sim::{
    advancements as adv,
    menu::{Maps as _, Menu},
};

/// `CrafterBlock`'s crafting animation length.
pub(in crate::game) const CRAFTING_TICKS: i32 = 6;

pub(in crate::game::server_side) fn center(pos: Pos) -> [f64; 3] {
    [
        f64::from(pos.0) + 0.5,
        f64::from(pos.1) + 0.5,
        f64::from(pos.2) + 0.5,
    ]
}
/// Whether an advancement condition's `recipes` entry names the recipe.
pub(in crate::game::server_side) fn recipe_matches(condition: &Value, id: &str) -> bool {
    match condition {
        Value::String(s) => s == id,
        Value::Array(list) => list.iter().any(|v| v.as_str() == Some(id)),
        _ => true,
    }
}

impl crate::game::ServerGame<'_> {
    /// `Level.setBlock` of one boolean property of the block at `pos`.
    pub(in crate::game::server_side) fn set_bool_prop(
        &mut self,
        pos: Pos,
        key: &str,
        value: bool,
        flags: i32,
    ) {
        let registry = self.common.pack.assets.registry;
        let old = self.common.world.block(pos.0, pos.1, pos.2);
        let Some(mut state) = registry.state(old).cloned() else {
            return;
        };
        if !state.properties.contains_key(key) {
            return;
        }
        state.properties.insert(key.to_owned(), value.to_string());
        let Ok(id) = registry.resolve(&state) else {
            return;
        };
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, BlockPos::new(pos.0, pos.1, pos.2), id, flags)
        });
        let _ = self.server_process_level_events();
    }
    /// `CrafterBlockEntity.serverTick` of the crafter at `pos`, and the
    /// powered flag following the block state.
    pub(in crate::game) fn tick_crafter(&mut self, pos: Pos) {
        let state = self.common.world.block(pos.0, pos.1, pos.2);
        let triggered = self.containers().prop(state, "triggered") == Some("true");
        let mut finished = false;
        if let Some(BlockEntity::Crafter(c)) = self.common.world.block_entities.get_mut(&pos) {
            // CrafterBlock.neighborChanged sets the flag with the state; a
            // loaded crafter keeps its saved flag until then.
            if c.state_triggered.is_some_and(|t| t != triggered) {
                c.triggered = i32::from(triggered);
            }
            c.state_triggered = Some(triggered);
            let remaining = c.crafting_ticks - 1;
            if remaining >= 0 {
                c.crafting_ticks = remaining;
                finished = remaining == 0;
            }
        }
        if finished {
            self.set_bool_prop(
                pos,
                "crafting",
                false,
                sunrise_world::level::flags::UPDATE_ALL,
            );
        }
    }
    /// `CrafterBlock.dispenseFrom` (the scheduled tick after a redstone pulse).
    pub fn crafter_dispense(&mut self, pos: BlockPos, state: BlockStateId) -> Result<()> {
        let key = (pos.x, pos.y, pos.z);
        let Some(crafter) = self.common.crafter_at(key) else {
            return Ok(());
        };
        let grid: Vec<Option<ItemStack>> = crafter.items.to_vec();
        // Query active world recipes with this level's saved-map context.
        let maps = self.level_maps(grid.iter().flatten());
        let found = self
            .recipes()
            .find_with(&self.common.loop_context, &grid, 3, &|id| {
                maps.state(id).map(|(_, scale)| scale)
            })
            .map(|(name, result)| (name.to_owned(), result))
            .filter(|(_, result)| result.count > 0);
        let Some((recipe, mut result)) = found else {
            self.dispenser_event(1050, key, 0);
            return Ok(());
        };
        let Some(facing) = self
            .containers()
            .prop(state, "orientation")
            .and_then(|o| o.split('_').next())
            .and_then(dir_from_name)
        else {
            return Ok(());
        };
        if let Some(BlockEntity::Crafter(c)) = self.common.world.block_entities.get_mut(&key) {
            c.crafting_ticks = CRAFTING_TICKS;
        }
        self.set_bool_prop(
            key,
            "crafting",
            true,
            sunrise_world::level::flags::UPDATE_CLIENTS,
        );
        // ItemStack.onCraftedBySystem invokes MapItem.onCraftedPostProcess
        // before dispensing, registering the derived map under its new id.
        Menu::map_crafted(Some(&maps), &mut result);
        self.register_made_maps(maps);
        // The remainders come from the grid as it was before anything moved.
        let remainders =
            self.recipes()
                .remainders(&self.common.loop_context, &recipe, &grid, 3, &|s| {
                    self.crafting_remainder(s)
                });
        self.crafter_eject(key, facing, result, &recipe);
        for rest in remainders.into_iter().flatten() {
            self.crafter_eject(key, facing, rest, &recipe);
        }
        // One of each input used, from the slots that held one: a part in front may have handed
        // the output on into an empty slot of this grid.
        if let Some(BlockEntity::Crafter(c)) = self.common.world.block_entities.get_mut(&key) {
            for (slot, _) in c
                .items
                .iter_mut()
                .zip(&grid)
                .filter(|(_, used)| used.is_some())
            {
                if let Some(s) = slot {
                    s.count -= 1;
                    if s.count == 0 {
                        *slot = None;
                    }
                }
            }
        }
        self.container_changed(key);
        self.native_crafter_craft([key.0, key.1, key.2], &recipe);
        Ok(())
    }
    /// `CrafterBlock.dispenseItem`: into the container in front, else out as an item.
    pub(in crate::game::server_side) fn crafter_eject(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
        recipe: &str,
    ) {
        let target = offset(pos, facing);
        let mut rest = stack;
        let side = facing.opposite();
        // `HopperBlockEntity.getBlockContainer`: a decorated pot keeps its slot in
        // the chunk's saved tag, which `view` does not read.
        if self.item_part_at(target, side).is_none()
            && self.containers().view(target, false).is_none()
            && let Some(pot) = self.containers().hopper_insertion_view(target)
        {
            self.crafter_insert(&pot, &mut rest, side, false);
            if rest.count == 0 {
                return;
            }
            self.spawn_dispensed(pos, facing, rest);
            self.crafter_crafted_notice(pos, facing, recipe);
            return;
        }
        self.with_container_at(target, side, |game, into| match into {
            ContainerTarget::Part(part) => {
                // `insertCrafterOutput`: the handler takes what it will; the rest is dispensed.
                rest.count -= game.put_into_part(&part, &rest, |_, _| Some(()));
            }
            ContainerTarget::View(into) => {
                let into_is_crafter = game.crafter_at(target).is_some();
                game.crafter_insert(into, &mut rest, side, into_is_crafter);
            }
        });
        if rest.count == 0 {
            return;
        }
        self.spawn_dispensed(pos, facing, rest);
        self.crafter_crafted_notice(pos, facing, recipe);
    }
    /// The trigger and the sound and particle events after an ejected result.
    pub(in crate::game::server_side) fn crafter_crafted_notice(
        &mut self,
        pos: Pos,
        facing: Direction,
        recipe: &str,
    ) {
        // CrafterRecipeCraftedTrigger: every player whose box meets the 17-block
        // box around the crafter's centre (8.5 from the centre to a face: the
        // player's own width and height reach into it, not its eye point).
        // `AABB.intersects` is strict: a box that only touches a face is out.
        let near = self.common.player.as_ref().is_some_and(|p| {
            let c = center(pos);
            let b = p.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            (lo.x < c[0] + 8.5 && hi.x > c[0] - 8.5)
                && (lo.y < c[1] + 8.5 && hi.y > c[1] - 8.5)
                && (lo.z < c[2] + 8.5 && hi.z > c[2] - 8.5)
        });
        if near {
            let id = sunrise_sim::crafting::qualified(recipe);
            let me = self.player_entity_for("minecraft:crafter_recipe_crafted");
            self.trigger("minecraft:crafter_recipe_crafted", move |c, w| {
                let ctx = adv::Entities {
                    this: Some(&me),
                    origin: me.position,
                    tool: None,
                    event_block: None,
                    target: None,
                    interacting: None,
                };
                adv::conditions(&c["player"], &ctx, w) && recipe_matches(&c["recipes"], &id)
            });
        }
        self.dispenser_event(1049, pos, 0);
        self.dispenser_event(2010, pos, facing as i32);
    }
    /// The two insert loops of `dispenseItem`: one item at a time when the
    /// container holds fewer than the stack (`Container.getMaxStackSize`) or is
    /// a crafter, else the whole stack; what is left stays in `rest`.
    pub(in crate::game::server_side) fn crafter_insert(
        &mut self,
        into: &View,
        rest: &mut ItemStack,
        face: Direction,
        into_is_crafter: bool,
    ) {
        let one_by_one = into_is_crafter || rest.count > self.containers().max_stack_in(into, rest);
        // One container instance serves the whole call, and a composter's
        // `InputContainer` is `changed` once it composted: it takes no more.
        let composter = matches!(into.kind, Kind::Composter(_));
        while rest.count > 0 {
            let before = rest.count;
            if one_by_one {
                let one = ItemStack {
                    count: 1,
                    ..rest.clone()
                };
                if self.add_item(None, into, one, Some(face)).is_some() {
                    break;
                }
                rest.count -= 1;
            } else {
                match self.add_item(None, into, rest.clone(), Some(face)) {
                    Some(left) => *rest = left,
                    None => rest.count = 0,
                }
                if rest.count == before {
                    break;
                }
            }
            if composter {
                break;
            }
        }
    }

    // ---- the menu ----
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_block_states::NativeBlockStatesGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack::new(item, count))
    }
    #[test]
    fn signal_counts_filled_and_disabled_slots() {
        let mut c = Crafter::default();
        assert_eq!(c.redstone_signal(), 0);
        c.items[0] = stack("minecraft:stick", 3);
        c.set_slot_state(4, false);
        assert_eq!(c.redstone_signal(), 2);
        // A filled slot cannot be disabled.
        c.set_slot_state(0, false);
        assert!(!c.disabled[0]);
        c.set_slot_state(4, true);
        assert_eq!(c.redstone_signal(), 1);
    }
    /// `CrafterBlockEntity.setSlotState`: a slot toggled in the menu lands on
    /// the block entity itself, whose `setChanged` marks the chunk.
    #[test]
    #[ignore = "daily"]
    fn a_slot_toggled_in_the_menu_lands_on_the_crafter() {
        use crate::game::game_tests::tests::server_game;
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let key = (2, 4, 3);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:crafter")
            .unwrap();
        game.world.set(key.0, key.1, key.2, state).unwrap();
        let mut crafter = Crafter::default();
        crafter.set_slot_state(3, false);
        game.world
            .block_entities
            .insert(key, BlockEntity::Crafter(crafter));
        crate::game::server_side::native_item_places::tests::open_to_lan(&mut game);
        let (mut player, _output) =
            sunrise_gameplay_common::game::server_side::native_sound_refs::tests::guest(
                1,
                2123,
                [2.5, 4.0, 3.5],
            );
        player.menu = Some(sunrise_sim::lan::menu::Menu {
            window: 1,
            kind: 0,
            registry_id: "minecraft:crafter_3x3".into(),
            pos: Some(key),
            slots: sunrise_sim::lan::menu::Slots::Block(key),
            size: 10,
            state: 0,
            sent: vec![None; 46],
            drag: None,
            sent_data: vec![],
            reported_unsent_data_len: None,
            blockless_data: Vec::new(),
            mod_type: None,
            craft: None,
            links: vec![],
            asks_still_valid: false,
            opening_item: None,
            opener_block: Some("minecraft:crafter".into()),
            removed: false,
            slot_changed_seen: None,
        });
        game.server.lan.as_mut().unwrap().hub.guests.push(player);
        game.world.modified.clear();

        game.lan_crafter_slot_state(1, 1, 3, true);

        let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get(&key) else {
            panic!("no crafter");
        };
        assert!(!c.disabled[3], "the click enabled the slot");
        assert!(
            game.world.modified.contains(&(0, 0)),
            "the toggled slot left the chunk unmarked"
        );
    }
    #[test]
    fn hoppers_fill_the_smaller_stacks_first() {
        let mut c = Crafter::default();
        let stick = ItemStack::new("minecraft:stick", 1);
        c.items[0] = stack("minecraft:stick", 2);
        c.items[1] = stack("minecraft:stick", 1);
        // A later empty slot counts as smaller: stacks level out front to back.
        assert!(!c.can_place(0, &stick, 64));
        assert!(!c.can_place(1, &stick, 64));
        assert!(c.can_place(2, &stick, 64));
        c.disabled[5] = true;
        assert!(!c.can_place(5, &stick, 64));
        // With every later slot at least as big, a slot accepts again.
        for i in 2..9 {
            c.items[i] = stack("minecraft:stick", 2);
        }
        assert!(c.can_place(1, &stick, 64));
        // A full stack accepts nothing.
        c.items[2] = stack("minecraft:stick", 64);
        assert!(!c.can_place(2, &stick, 64));
    }
    #[test]
    fn recipes_condition_accepts_a_name_or_a_list() {
        assert!(recipe_matches(
            &serde_json::json!("minecraft:crafter"),
            "minecraft:crafter"
        ));
        assert!(!recipe_matches(
            &serde_json::json!(["minecraft:stick"]),
            "minecraft:crafter"
        ));
        assert!(recipe_matches(&Value::Null, "minecraft:crafter"));
    }
    /// `CrafterBlock.dispenseItem` ejects into the container entity in front of
    /// it (`HopperBlockEntity.getContainerAt`'s `getEntityContainer`), not
    /// beside it: a chest or hopper minecart, or a chest boat, is filled.
    #[test]
    #[ignore = "daily"]
    fn a_crafter_fills_the_cart_in_front_of_it() {
        use crate::game::game_tests::tests::server_game;
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::boats_common::Boat;
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        // One game for the three vehicles: loading the resources is the slow part.
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let at = Vec3::new(9.5, 64.0, 8.5);
        for kind in [CartKind::Chest, CartKind::Hopper] {
            game.carts.clear();
            game.boats.clear();
            game.carts.push(Minecart::new(1, kind, at));
            game.crafter_eject(
                (8, 64, 8),
                Direction::East,
                ItemStack::new("minecraft:pumpkin_pie", 3),
                "minecraft:pumpkin_pie",
            );
            let cart = game.carts.first().expect("the cart is still there");
            assert_eq!(cart.items[0].as_ref().map(|s| s.count), Some(3));
            assert!(game.dropped.is_empty(), "nothing is dropped beside it");
        }
        game.carts.clear();
        game.boats
            .push(Boat::new(1, Arc::from("oak"), at, 0.0).with_chest());
        game.crafter_eject(
            (8, 64, 8),
            Direction::East,
            ItemStack::new("minecraft:pumpkin_pie", 3),
            "minecraft:pumpkin_pie",
        );
        let boat = game.boats.first().expect("the boat is still there");
        assert_eq!(boat.items[0].as_ref().map(|s| s.count), Some(3));
        assert!(game.dropped.is_empty(), "nothing is dropped beside it");
    }
    /// One transfer into a composter is one item, however big the stack: the
    /// `InputContainer` is `changed` after it composts, and its
    /// `getMaxStackSize` of 1 keeps the crafter from pushing the stack whole.
    #[test]
    #[ignore = "daily"]
    fn one_transfer_to_a_composter_adds_one_layer_and_eats_one_item() {
        use crate::game::game_tests::tests::server_game;
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let pos = (8, 63, 8);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:composter")
            .unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        // Pumpkin pie composts one layer always (`compostable/always_add_one`).
        game.crafter_eject(
            (8, 64, 8),
            Direction::Down,
            ItemStack::new("minecraft:pumpkin_pie", 9),
            "minecraft:pumpkin_pie",
        );
        assert_eq!(game.composter_level([pos.0, pos.1, pos.2]), Some(1));
        let dropped: Vec<(String, u32)> = game
            .dropped
            .iter()
            .map(|d| (d.stack.item.to_string(), d.stack.count))
            .collect();
        assert_eq!(dropped, [("minecraft:pumpkin_pie".to_owned(), 8)]);
    }
}
