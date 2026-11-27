/// The creative-slot wire codec bounds a stack's count to 1..=99; an item whose
/// own max stack size a native mod raised past 99 becomes the ceiling instead.
pub fn creative_count_bound(item_max_stack: u32) -> u32 {
    item_max_stack.max(99)
}

/// Whether a creative stack's count is past the bound its item's own max stack gives it.
pub fn creative_count_exceeds(count: u32, own_max: u32) -> bool {
    count > creative_count_bound(own_max)
}

impl crate::game::CommonGame<'_> {
    pub fn spawn_protects(&self, radius: i32, dimension: &str, pos: [i32; 3]) -> bool {
        let Some(spawn) = self.level.world_spawn else {
            return false;
        };
        let dx = pos[0].abs_diff(spawn[0]);
        let dz = pos[2].abs_diff(spawn[2]);
        dimension == "minecraft:overworld" && dx.max(dz) <= radius as u32
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{creative_count_bound, creative_count_exceeds};

    #[test]
    fn creative_count_bound_kicks_vanilla_100_but_not_modded_own_size() {
        // A vanilla item (own max stack 64 or 1) stays on the wire's 1..=99 bound.
        for own_max in [1, 64] {
            let bound = creative_count_bound(own_max);
            assert_eq!(bound, 99);
            assert!(99 <= bound, "99 must not kick a vanilla item");
            assert!(100 > bound, "100 must kick a vanilla item");
        }
        // An item a native mod raised past 99 is allowed up to its own max stack.
        let bound = creative_count_bound(500);
        assert_eq!(bound, 500);
        assert!(500 <= bound, "500 must not kick a modded item");
        assert!(501 > bound, "501 must kick a modded item");
    }

    #[test]
    fn creative_count_exceeds_the_bound_of_the_items_own_max() {
        assert!(creative_count_exceeds(100, 64));
        assert!(creative_count_exceeds(100, 1));
        assert!(!creative_count_exceeds(99, 64));
        assert!(!creative_count_exceeds(500, 500));
        assert!(creative_count_exceeds(501, 500));
    }
}
