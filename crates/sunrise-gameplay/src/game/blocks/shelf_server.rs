//! Server-side shelf item swaps (`ShelfBlock.useItemOn`).
use super::{Hit, machines::dir_from_name};
use sunrise_core::geometry::Direction;
use sunrise_level::containers::BlockEntity;

fn accepts(limit: Option<u32>, stack: Option<&sunrise_items::items::ItemStack>) -> bool {
    limit.is_none_or(|limit| stack.is_none_or(|stack| stack.count <= limit))
}

impl crate::game::ServerGame<'_> {
    /// `SideChainPartBlock.getAllBlocksConnectedTo`: left to right, at most three.
    fn shelf_chain(&self, pos: [i32; 3], facing: Direction) -> Vec<[i32; 3]> {
        let left = match facing {
            Direction::North => Direction::East,
            Direction::East => Direction::South,
            Direction::South => Direction::West,
            Direction::West => Direction::North,
            _ => return Vec::new(),
        };
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let behaviours = self.common.res.behaviours();
        if !behaviours
            .tag("minecraft:wooden_shelves")
            .is_some_and(|tag| tag.contains(behaviours.block(state)))
        {
            return Vec::new();
        }
        let mut result = vec![pos];
        for (dir, end, prepend) in [(left, "left", true), (left.opposite(), "right", false)] {
            let normal = dir.normal();
            for step in 1..3 {
                let p = [pos[0] + normal[0] * step, pos[1], pos[2] + normal[2] * step];
                let state = self.common.world.block(p[0], p[1], p[2]);
                let Some(info) = self.common.pack.assets.registry.state(state) else {
                    break;
                };
                let behaviours = self.common.res.behaviours();
                let connectable = behaviours
                    .tag("minecraft:wooden_shelves")
                    .is_some_and(|tag| tag.contains(behaviours.block(state)))
                    && info.properties.get("powered").is_some_and(|s| s == "true")
                    && info.properties.get("facing").and_then(|s| dir_from_name(s)) == Some(facing);
                if !connectable {
                    break;
                }
                let part = info
                    .properties
                    .get("side_chain")
                    .map(String::as_str)
                    .unwrap_or("unconnected");
                if part == "center" || part == end {
                    if prepend {
                        result.insert(0, p);
                    } else {
                        result.push(p);
                    }
                }
                if part != "center" {
                    break;
                }
            }
        }
        result
    }

    /// `ShelfBlock.useItemOn`: a single swap, or a connected hotbar swap while powered.
    pub(in crate::game) fn use_shelf(&mut self, hit: &Hit) -> Option<bool> {
        use mod_api::client::first_person::Hand;
        if self.common.hand != Hand::Main {
            return None;
        }
        let (slot, facing, powered) = self.common.shelf_hit(hit)?;
        let pos = hit.pos;
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if powered {
            let chain = self.shelf_chain(pos, facing);
            let mut swapped = false;
            let count = chain.len();
            for (i, p) in chain.into_iter().enumerate() {
                let key = (p[0], p[1], p[2]);
                let limit = self.common.world.container_stack_limit(key);
                let Some(BlockEntity::Shelf(shelf)) =
                    self.common.world.block_entities.get_mut(&key)
                else {
                    continue;
                };
                for n in 0..3 {
                    let index = 9_i32 - (count as i32 - i as i32) * 3 + n as i32;
                    if !(0..9).contains(&index) {
                        continue;
                    }
                    let index = index as usize;
                    if !accepts(limit, self.common.inventory[index].as_ref()) {
                        continue;
                    }
                    swapped |= self.common.inventory[index].is_some() || shelf.items[n].is_some();
                    std::mem::swap(&mut self.common.inventory[index], &mut shelf.items[n]);
                }
                self.container_changed(key);
                let s = self.common.world.block(p[0], p[1], p[2]);
                self.post_block_game_event("minecraft:entity_interact", p, None, Some(s));
            }
            if !swapped {
                return Some(false);
            }
            self.server_play_sound(
                "minecraft:block.shelf.multi_swap",
                pos.map(|n| f64::from(n) + 0.5),
                1.0,
                1.0,
            );
            Some(true)
        } else {
            let held = self.common.inventory[self.common.selected].clone();
            let key = (pos[0], pos[1], pos[2]);
            if !accepts(self.common.world.container_stack_limit(key), held.as_ref()) {
                return Some(false);
            }
            let Some(BlockEntity::Shelf(shelf)) = self.common.world.block_entities.get_mut(&key)
            else {
                return None;
            };
            let removed = std::mem::replace(&mut shelf.items[slot], held.clone());
            self.common.inventory[self.common.selected] =
                if self.common.creative && removed.is_none() {
                    held.clone()
                } else {
                    removed.clone()
                };
            self.container_changed(key);
            let vibrations = self.common.inventory[self.common.selected]
                .as_ref()
                .is_none_or(|stack| {
                    let default = self
                        .common
                        .res
                        .items()
                        .get(&stack.item)
                        .is_none_or(|item| item.interact_vibrations);
                    let Some(data) = stack.data() else {
                        return default;
                    };
                    if data
                        .preserved_components()
                        .iter()
                        .any(|(key, _)| key == "!minecraft:use_effects")
                    {
                        return true;
                    }
                    data.preserved_components()
                        .iter()
                        .find(|(key, _)| key == "minecraft:use_effects")
                        .map_or(default, |(_, tag)| {
                            sunrise_sim::save::number(tag.get("interact_vibrations")).unwrap_or(1)
                                != 0
                        })
                });
            if vibrations {
                self.post_block_game_event(
                    "minecraft:item_interact_finish",
                    pos,
                    None,
                    Some(state),
                );
            }
            let sound = match (held.is_some(), removed.is_some()) {
                (false, false) => return None,
                (true, false) => "minecraft:block.shelf.place_item",
                (false, true) => "minecraft:block.shelf.take_item",
                (true, true) => "minecraft:block.shelf.single_swap",
            };
            self.server_play_sound(sound, pos.map(|n| f64::from(n) + 0.5), 1.0, 1.0);
            Some(true)
        }
    }
}

#[cfg(test)]
mod stack_limit_tests {
    #[test]
    fn shelf_swap_refuses_an_oversized_offer_without_capping_unmodified_shelves() {
        let stack = sunrise_items::items::ItemStack::new("minecraft:stick", 99);
        assert!(super::accepts(None, Some(&stack)));
        assert!(!super::accepts(Some(10), Some(&stack)));
        assert!(super::accepts(Some(10), None));
        let stack = sunrise_items::items::ItemStack::new("minecraft:stick", 10);
        assert!(super::accepts(Some(10), Some(&stack)));
    }
}
