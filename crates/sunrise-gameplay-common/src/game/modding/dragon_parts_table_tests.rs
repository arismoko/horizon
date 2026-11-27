use super::{DragonPartsTable, DragonSlot};

impl DragonPartsTable {
    #[cfg(test)]
    fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        let mut cursor = None;
        std::iter::from_fn(move || self.next_part(&mut cursor))
    }
}

impl DragonSlot {
    #[cfg(test)]
    fn part_ids(&self) -> Vec<u32> {
        self.table.borrow().iter().collect()
    }
}

#[cfg(test)]
mod probe_order_tests {
    use super::*;
    // Literal orders were independently calculated from the specified mix/probe
    // algorithm, not obtained from DragonPartsTable. The removal calculation
    // reinserts the affected cluster, independently of our backward-shift code.
    #[test]
    fn one_dragon_descends_slots() {
        let mut t = DragonPartsTable::default();
        for k in 21..=28 {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [25, 27, 28, 24, 26, 23, 22, 21]
        );
    }
    #[test]
    fn two_dragons_collide() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38) {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 35, 25, 27, 37, 31, 32, 28, 34, 24, 38, 36, 26, 23, 22, 21
            ]
        );
    }
    #[test]
    fn removal_repairs_probe_chains() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38) {
            t.insert(k);
        }
        for k in 21..=28 {
            t.remove(k);
        }
        for k in 41..=48 {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 35, 37, 47, 48, 31, 32, 34, 46, 38, 36, 45, 44, 42, 41, 43
            ]
        );
    }
    #[test]
    fn growth_rehashes_in_descending_order() {
        let mut t = DragonPartsTable::default();
        for k in (21..=28).chain(31..=38).chain(41..=48) {
            t.insert(k);
        }
        assert_eq!(
            t.iter().collect::<Vec<_>>(),
            [
                33, 25, 37, 28, 24, 26, 23, 22, 21, 35, 27, 47, 48, 31, 32, 34, 36, 38, 46, 41, 45,
                44, 42, 43
            ]
        );
    }
    #[test]
    fn zero_is_first_and_removed() {
        let mut t = DragonPartsTable::default();
        t.insert(21);
        t.insert(0);
        t.insert(0);
        assert_eq!(t.iter().collect::<Vec<_>>(), [0, 21]);
        t.remove(0);
        assert_eq!(t.iter().collect::<Vec<_>>(), [21]);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::game::Vec3;
    use crate::game::dragon_common::Dragon;
    #[test]
    fn dragon_slot_tracks_registration_not_temporary_loans() {
        let mut slot = DragonSlot::default();
        slot.replace(Dragon::new(Vec3::ZERO, 0.0, 0));
        assert!(slot.part_ids().is_empty());
        slot.as_mut().unwrap().id = 20;
        slot.id_assigned();
        assert_eq!(slot.part_ids(), [25, 27, 28, 24, 26, 23, 22, 21]);
        let dragon = slot.extract().unwrap();
        assert_eq!(slot.part_ids().len(), 8);
        slot.put_back(dragon);
        assert_eq!(slot.part_ids(), [25, 27, 28, 24, 26, 23, 22, 21]);
        slot.discard();
        assert!(slot.part_ids().is_empty());
    }
}
