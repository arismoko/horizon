//! The `compute_fog_color` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Computed world fog RGB, after ordinary effects and before upload. Title panoramas skip it.
    /// Only finite RGB components in 0..=1 are accepted; alpha and distances stay unchanged.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_compute_fog_color",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeFogColor"
    )]
    fn compute_fog_color(
        partial: f32,
        camera: [f64; 3],
        fog_type: mod_api::client::fog::FogType,
        rgb: &mut [f32; 3],
    );
}
