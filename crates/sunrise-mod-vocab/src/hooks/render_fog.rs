//! The `render_fog` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Final world fog, after the computed color hook, ordinary calculations and Sunrise's fog
    /// option, before upload. Title panoramas skip it. Invalid color or distance values retain the
    /// original whole fog.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_render_fog",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.RenderFog"
    )]
    fn render_fog(
        partial: f32,
        camera: [f64; 3],
        fog_type: mod_api::client::fog::FogType,
        fog: &mut mod_api::client::fog::FogData,
    );
}
