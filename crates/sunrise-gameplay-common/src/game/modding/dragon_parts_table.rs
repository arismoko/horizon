//! Integer part registry, following the public fastutil javadoc-level description
//! of `Int2ObjectOpenHashMap` (https://fastutil.di.unimi.it/docs/).
//! Keys use multiplicative mixing, linear probing, backward-shift deletion, and
//! descending-slot iteration (with the zero key first). No Mojang implementation
//! is used here.
#[derive(Debug)]
pub struct DragonPartsTable {
    slots: Vec<Option<u32>>,
    zero: bool,
    size: usize,
}
impl Default for DragonPartsTable {
    fn default() -> Self {
        Self {
            slots: vec![None; 32],
            zero: false,
            size: 0,
        }
    }
}
impl DragonPartsTable {
    fn home(key: u32, mask: usize) -> usize {
        let h = key.wrapping_mul(0x9e3779b9);
        ((h ^ (h >> 16)) as usize) & mask
    }
    pub fn insert(&mut self, key: u32) {
        if key == 0 {
            if self.zero {
                return;
            }
            self.zero = true;
        } else {
            let mask = self.slots.len() - 1;
            let mut p = Self::home(key, mask);
            while let Some(old) = self.slots[p] {
                if old == key {
                    return;
                }
                p = (p + 1) & mask;
            }
            self.slots[p] = Some(key);
        }
        self.size += 1;
        if self.size >= self.slots.len() * 3 / 4 {
            let capacity = self.slots.len() * 2;
            let old = std::mem::replace(&mut self.slots, vec![None; capacity]);
            for key in old.into_iter().rev().flatten() {
                let mask = self.slots.len() - 1;
                let mut p = Self::home(key, mask);
                while self.slots[p].is_some() {
                    p = (p + 1) & mask;
                }
                self.slots[p] = Some(key);
            }
        }
    }
    pub fn remove(&mut self, key: u32) {
        if key == 0 {
            if self.zero {
                self.zero = false;
                self.size -= 1;
            }
            return;
        }
        let mask = self.slots.len() - 1;
        let mut hole = Self::home(key, mask);
        loop {
            match self.slots[hole] {
                None => return,
                Some(k) if k == key => break,
                _ => hole = (hole + 1) & mask,
            }
        }
        self.size -= 1;
        let mut scan = (hole + 1) & mask;
        while let Some(k) = self.slots[scan] {
            let home = Self::home(k, mask);
            if (scan.wrapping_sub(home) & mask) >= (scan.wrapping_sub(hole) & mask) {
                self.slots[hole] = Some(k);
                hole = scan;
            }
            scan = (scan + 1) & mask;
        }
        self.slots[hole] = None;
    }
    /// Start at `None`, then resume below the last slot inspected. The zero key comes first.
    pub fn next_part(&self, cursor: &mut Option<usize>) -> Option<u32> {
        let before = match *cursor {
            Some(before) => before,
            None if self.zero => {
                *cursor = Some(self.slots.len());
                return Some(0);
            }
            None => self.slots.len(),
        };
        for slot in (0..before).rev() {
            if let Some(id) = self.slots[slot] {
                *cursor = Some(slot);
                return Some(id);
            }
        }
        *cursor = Some(0);
        None
    }
}

/// The table travels with the dimension-owned singleton, including actor loans.
#[derive(Default)]
pub struct DragonSlot {
    slot: sunrise_sim::join_seq::ResidentSlot<crate::game::dragon_common::Dragon>,
    pub table: std::cell::RefCell<DragonPartsTable>,
    registered: std::cell::Cell<u32>,
}
impl std::ops::Deref for DragonSlot {
    type Target = sunrise_sim::join_seq::ResidentSlot<crate::game::dragon_common::Dragon>;
    fn deref(&self) -> &Self::Target {
        &self.slot
    }
}
impl std::ops::DerefMut for DragonSlot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slot
    }
}
impl DragonSlot {
    fn register(&self, id: u32) {
        let old = self.registered.replace(id);
        if old == id {
            return;
        }
        let mut table = self.table.borrow_mut();
        if old != 0 {
            for part in 0..8 {
                table.remove(crate::game::dragon_common::dragon_part_id(old, part));
            }
        }
        if id != 0 {
            for part in 0..8 {
                table.insert(crate::game::dragon_common::dragon_part_id(id, part));
            }
        }
    }
    // Return a copy so the caller never runs a visitor under the table borrow.
    pub fn next_part(&self, cursor: &mut Option<usize>) -> Option<u32> {
        self.table.borrow().next_part(cursor)
    }
    pub fn replace(&mut self, dragon: crate::game::dragon_common::Dragon) {
        self.register(0);
        self.register(dragon.id);
        self.slot.replace(dragon);
    }
    // Preserve ResidentSlot's ownership-returning refusal contract without allocating.
    #[allow(clippy::result_large_err)]
    pub fn install(
        &mut self,
        dragon: crate::game::dragon_common::Dragon,
    ) -> Result<(), crate::game::dragon_common::Dragon> {
        if self.slot.is_some() {
            return Err(dragon);
        }
        self.replace(dragon);
        Ok(())
    }
    pub fn discard(&mut self) {
        self.register(0);
        self.slot.discard();
    }
    pub fn id_assigned(&self) {
        self.register(self.slot.as_ref().map_or(0, |d| d.id));
        self.slot.id_assigned();
    }
    pub fn retire_extracted(&self, id: u32, join: sunrise_sim::join_seq::JoinSeq) {
        if self.registered.get() == id {
            self.register(0);
        }
        self.slot.retire_extracted(id, join);
    }
}

#[cfg(test)]
#[path = "dragon_parts_table_tests.rs"]
mod tests;
