//! One living entity's draw (`LivingEntityRenderer.submit` and the layers, hooks, outline
//! and beam around it). The ordered entity pass shares one `LivingFrame` across its mob runs.

/// What the mob pass works out once a frame for every living entity it draws.
#[derive(Clone, Copy)]
pub struct LivingFrame<'a> {
    pub camera: glam::DVec3,
    pub alpha: f64,
    /// `Entity.shouldRenderAtSqrDistance`'s view scale.
    pub scale: f64,
    /// Who watches (`Entity.isInvisibleTo`): the local player's team.
    pub viewer: Option<&'a sunrise_game_data::scoreboard::Team>,
    pub skinning: bool,
    pub listened: bool,
    pub now: std::time::Instant,
}
