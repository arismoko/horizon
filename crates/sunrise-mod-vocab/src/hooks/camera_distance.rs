//! The `camera_distance` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Final scaled third-person camera distance, before visual-shape collision clipping.
    /// `entity` includes its living scale/distance and optional mount scale/distance.
    /// Only finite requests in 0..=1024 blocks replace the vanilla distance.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_camera_distance",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "CalculateDetachedCameraDistanceEvent"
    )]
    fn camera_distance(
        partial: f32,
        mirrored: bool,
        entity: &mod_api::client::camera::CameraEntity,
        distance: &mut f32,
    );
}
