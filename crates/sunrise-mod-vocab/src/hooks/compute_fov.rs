//! The `compute_fov` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// World field of view in degrees, after camera easing and fluid/death adjustment,
    /// before projection and Nausea spin. There is no separate engine zoom pass.
    /// Called for world rendering and near-plane overlays; hand/HUD FOV uses its own path.
    /// Only finite values in 1..=179 degrees replace the computed FOV.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_compute_fov",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeFov",
        version = 2
    )]
    fn compute_fov(partial: f32, camera: [f64; 3], degrees: &mut f32);
}
