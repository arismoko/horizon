//! Declarations carried from upstream `screen_transparent_background`; the generator owns the typed bindings.

pub trait Hooks {
    /// `Screen.renderTransparentBackground`: draws the translucent dark background
    /// behind an in-game screen (inventory, container, creative, book, sign, editor).
    /// Set `canceled` (or return `Flow::Cancel`) to suppress the vanilla dark gradient,
    /// allowing a mod to draw its own background or apply menu background blur
    /// via `Host::apply_screen_blur`. API @API:screen_transparent_background@.
    #[notes = r#"## Transparent screen backgrounds and landing changes

`Hooks::screen_transparent_background` runs immediately before an in-game screen's
translucent gradient. Returning `Flow::Cancel` suppresses that gradient, not the
screen's contents. Its `batch` is a `DrawContext`: custom drawing survives
cancellation. `ClientHost::apply_screen_blur()` requests the normal menu blur
at most once per frame; `menu_blur_radius()` reads the configured radius.
`DrawContext::draw_menu_background` and `tiled_sprite` draw menu textures through
the same atlas. The whole-screen `screen_render_pre` remains a separate moment."#]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        neoforge = "ScreenEvent.Render.Pre (transparent background)",
        cancellable = true,
        out = "canceled",
        target = "sunrise_client_gameplay::game::native_hooks::screen_transparent_background::ClientGame<'_>::native_screen_transparent_background"
    )]
    fn screen_transparent_background(
        screen: &str,
        batch: &mut dyn DrawContext,
        canceled: &mut bool,
    );
}
