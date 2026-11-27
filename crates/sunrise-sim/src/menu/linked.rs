//! A menu's linked storage in menu order: the block entities whose containers
//! it reads through `ModLinks`.
use crate::{items::ItemStack, world::World};
use std::borrow::Cow;

pub(crate) struct ShownSlots<'a> {
    stored: &'a [Option<ItemStack>],
    count: usize,
}

impl<'a> ShownSlots<'a> {
    pub(crate) fn iter(self) -> impl Iterator<Item = &'a Option<ItemStack>> {
        let missing = self.count - self.stored.len();
        self.stored
            .iter()
            .chain(std::iter::repeat_n(&None, missing))
    }

    pub(crate) fn into_contiguous(self) -> Cow<'a, [Option<ItemStack>]> {
        if self.stored.len() == self.count {
            Cow::Borrowed(self.stored)
        } else {
            Cow::Owned(self.iter().cloned().collect())
        }
    }
}

pub(crate) fn shown(world: &World, pos: (i32, i32, i32)) -> Option<ShownSlots<'_>> {
    let stored = world.block_entities.get(&pos)?.slots();
    let count = world.container_slot_count(pos)?;
    Some(ShownSlots {
        stored: &stored[..count.min(stored.len())],
        count,
    })
}

/// A menu's linked storage in menu order. An index in the linked range without backing
/// storage is empty and unavailable, never a slot of the opening block.
#[derive(Clone, Copy)]
pub struct LinkedSlots<'a> {
    world: &'a World,
    layout: crate::containers::ModLinks,
    links: &'a [(i32, i32, i32)],
}

impl<'a> LinkedSlots<'a> {
    pub fn new(
        world: &'a World,
        layout: crate::containers::ModLinks,
        links: &'a [(i32, i32, i32)],
    ) -> Self {
        Self {
            world,
            layout,
            links,
        }
    }

    pub fn unbound(self) -> std::ops::Range<usize> {
        let range = self.layout.slots();
        range.start + linked_slot_count(self.world, self.links).min(range.len())..range.end
    }

    /// Outer None means outside the linked range; Some(None) means there is no backing.
    pub fn backing(self, slot: usize) -> Option<Option<((i32, i32, i32), usize)>> {
        let range = self.layout.slots();
        if !range.contains(&slot) {
            return None;
        }
        let mut offset = slot - range.start;
        for &pos in self.links {
            let len = self.world.container_slot_count(pos).unwrap_or(0);
            if offset < len {
                return Some(Some((pos, offset)));
            }
            offset -= len;
        }
        Some(None)
    }

    /// Outer None means outside the linked range; Some(None) means there is no backing.
    pub(crate) fn get(self, slot: usize) -> Option<Option<&'a Option<ItemStack>>> {
        let range = self.layout.slots();
        range
            .contains(&slot)
            .then(|| linked_stacks(self.world, self.links).nth(slot - range.start))
    }

    /// Stage the same live projection in the click's working storage.
    pub fn overlay(self, items: &mut [Option<ItemStack>]) {
        let slots = items
            .iter_mut()
            .skip(self.layout.from)
            .take(self.layout.count);
        for (slot, stack) in slots.zip(
            linked_stacks(self.world, self.links)
                .cloned()
                .chain(std::iter::repeat(None)),
        ) {
            *slot = stack;
        }
    }
}

pub(crate) fn linked_stacks<'a>(
    world: &'a World,
    links: &'a [(i32, i32, i32)],
) -> impl Iterator<Item = &'a Option<ItemStack>> {
    links
        .iter()
        .filter_map(|&pos| shown(world, pos))
        .flat_map(ShownSlots::iter)
}

/// How many slots the linked containers `links` have between them.
pub fn linked_slot_count(world: &World, links: &[(i32, i32, i32)]) -> usize {
    links
        .iter()
        .filter_map(|&pos| world.container_slot_count(pos))
        .sum()
}
