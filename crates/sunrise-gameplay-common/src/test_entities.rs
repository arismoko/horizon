//! The box arithmetic of the game tests.
use sunrise_core::bounding_box::BoundingBox;

pub type Aabb = ([f64; 3], [f64; 3]);

/// `AABB.of(BoundingBox)`.
pub fn aabb_of(b: &BoundingBox) -> Aabb {
    (b.min.map(f64::from), b.max.map(|v| f64::from(v) + 1.0))
}

/// `AABB.inflate(amount)`.
pub fn inflate(b: Aabb, amount: f64) -> Aabb {
    (b.0.map(|v| v - amount), b.1.map(|v| v + amount))
}

/// The box of an entity of `width` by `height` standing at `p`
/// (`EntityDimensions.makeBoundingBox`).
pub fn standing(p: [f64; 3], width: f64, height: f64) -> Aabb {
    let half = width / 2.0;
    (
        [p[0] - half, p[1], p[2] - half],
        [p[0] + half, p[1] + height, p[2] + half],
    )
}

/// `AABB.intersects(AABB)`: the open boxes overlap.
pub fn intersects(a: Aabb, b: Aabb) -> bool {
    (0..3).all(|i| a.0[i] < b.1[i] && a.1[i] > b.0[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boundary_box_covers_its_last_block_and_inflates_both_ways() {
        let b = BoundingBox::new([0, 1, 2], [3, 4, 5]);
        assert_eq!(aabb_of(&b), ([0.0, 1.0, 2.0], [4.0, 5.0, 6.0]));
        assert_eq!(
            inflate(aabb_of(&b), 1.0),
            ([-1.0, 0.0, 1.0], [5.0, 6.0, 7.0])
        );
    }

    #[test]
    fn boxes_that_only_touch_do_not_intersect() {
        let area = ([0.0; 3], [4.0; 3]);
        // An entity standing on the area's floor, inside it.
        assert!(intersects(standing([2.0, 0.0, 2.0], 0.6, 1.8), area));
        // Its box ends exactly where the area starts (AABB.intersects is open).
        assert!(!intersects(standing([-0.3, 0.0, 2.0], 0.6, 1.8), area));
        assert!(intersects(standing([-0.2, 0.0, 2.0], 0.6, 1.8), area));
        // Above the area.
        assert!(!intersects(standing([2.0, 4.0, 2.0], 0.6, 1.8), area));
    }
}
