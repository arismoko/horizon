#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use crate::game::boats_common::Boat;
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;

    pub fn boat() -> Boat {
        Boat::new(1, Arc::from("oak"), Vec3::ZERO, 0.0)
    }
}
