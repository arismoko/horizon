//! Finding a mob by its network id without walking the whole list: a
//! multiplayer client looks one up for every movement, data, motion and
//! event packet. A `Mob` is ~3 KB, so a linear search touches a cache line
//! per mob: 1000 movement packets in a 3000-mob farm cost ~5 ms a tick that
//! way, against ~0.01 ms through the index.
//!
//! The index is a cache: the mob list changes in many places (spawns,
//! removals, dimension swaps), and none of them has to tell it. A hit is
//! checked against the list, and a miss falls back to the linear search,
//! which rebuilds the index when it finds the mob. Ids are unique in the list
//! (`spawn_entity` drops a mob with the same id before adding one), so the
//! answer is always the linear search's.

use sunrise_level::fast_hash::FxHashMap;
use sunrise_sim::mobs::Mob;

#[derive(Default)]
pub struct MobIndex {
    slots: FxHashMap<u32, u32>,
}

impl MobIndex {
    /// The position of the mob with this id in `mobs`, as
    /// `mobs.iter().position(|m| m.id == id)`.
    pub fn find(&mut self, mobs: &[Mob], id: u32) -> Option<usize> {
        self.position(mobs, |m| m.id, id)
    }

    /// Read-only lookup: checks the cached slot before falling back to linear search.
    pub fn find_ref(&self, mobs: &[Mob], id: u32) -> Option<usize> {
        self.position_ref(mobs, |m| m.id, id)
    }

    fn position_ref<T>(&self, items: &[T], id_of: impl Fn(&T) -> u32, id: u32) -> Option<usize> {
        if let Some(&i) = self.slots.get(&id)
            && items.get(i as usize).is_some_and(|m| id_of(m) == id)
        {
            return Some(i as usize);
        }
        items.iter().position(|m| id_of(m) == id)
    }

    fn position<T>(&mut self, items: &[T], id_of: impl Fn(&T) -> u32, id: u32) -> Option<usize> {
        if let Some(&i) = self.slots.get(&id)
            && items.get(i as usize).is_some_and(|m| id_of(m) == id)
        {
            return Some(i as usize);
        }
        let i = items.iter().position(|m| id_of(m) == id)?;
        self.slots.clear();
        for (i, m) in items.iter().enumerate() {
            self.slots.entry(id_of(m)).or_insert(i as u32);
        }
        Some(i)
    }

    /// The mob with this id, as `mobs.iter_mut().find(|m| m.id == id)`.
    pub fn get_mut<'a>(
        &mut self,
        mobs: &'a mut sunrise_sim::join_seq::JoinList<Mob>,
        id: u32,
    ) -> Option<&'a mut Mob> {
        let i = self.find(mobs, id)?;
        Some(&mut mobs[i])
    }

    /// The mob with this id, as `mobs.iter().find(|m| m.id == id)`.
    pub fn get<'a>(&mut self, mobs: &'a [Mob], id: u32) -> Option<&'a Mob> {
        let i = self.find(mobs, id)?;
        Some(&mobs[i])
    }

    /// Immutable indexed borrow: O(1) on cache hit, falls back to linear scan on miss.
    pub fn get_ref<'a>(&self, mobs: &'a [Mob], id: u32) -> Option<&'a Mob> {
        let i = self.find_ref(mobs, id)?;
        Some(&mobs[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever the list went through since the last lookup, the index
    /// answers as the linear search does.
    #[test]
    fn finds_what_a_linear_search_finds_after_any_change() {
        let mut index = MobIndex::default();
        let check = |index: &mut MobIndex, ids: &[u32]| {
            for id in 0..20 {
                let want = ids.iter().position(|&m| m == id);
                assert_eq!(
                    index.position_ref(ids, |&m| m, id),
                    want,
                    "ref id {id} in {ids:?}"
                );
                let found = index.position(ids, |&m| m, id);
                assert_eq!(found, want, "id {id} in {ids:?}");
                assert_eq!(
                    index.position_ref(ids, |&m| m, id),
                    want,
                    "cached ref id {id} in {ids:?}"
                );
            }
        };
        let mut ids = vec![7, 3, 9, 12];
        check(&mut index, &ids);
        ids.remove(1);
        check(&mut index, &ids);
        ids.push(3);
        ids.swap(0, 2);
        check(&mut index, &ids);
        ids.retain(|&m| m != 9);
        ids.insert(0, 15);
        check(&mut index, &ids);
        // Another list altogether: a dimension swapped in.
        check(&mut index, &[12, 15, 1]);
        ids.clear();
        check(&mut index, &ids);
    }
}
