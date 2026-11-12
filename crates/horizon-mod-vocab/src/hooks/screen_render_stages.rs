//! The `screen_render_stages` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the open screen draws itself (`ScreenEvent.Render.Pre`, vanilla's `Screen.renderWithTooltips`): draw to put something under it, or set `canceled` and the screen is not drawn, and neither is its `screen_render_background` or `screen_render_foreground`. The world, the HUD layers and the toasts still draw, and `screen_render_post` still fires. Fires once a frame on the window, for the screens `screen_render_post` covers: a screen of the game's, the title's own screens, Win and Options. Not while playing with no screen, nor for a server dialog, the server pack, link and code-of-conduct prompts, a book or a sign editor drawn over gameplay. Draw with `native_fill`, `native_sprite` and `native_text`; `screen.rect` is the screen's container panel (or the whole window).
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.Render.Pre",
        cancellable = true
    )]
    fn screen_render_pre();
    /// The open screen is about to draw its contents (`ScreenEvent.Render.Background`, after vanilla's `renderBackground` and before `render`): draw here for what must sit under the screen's widgets, items and tooltips. Not cancellable. Horizon's screens draw their own background panel inside their drawing, so this fires before all of it; the panel is over what a mod draws here. It does not fire when `screen_render_pre` cancelled. Fires once a frame on the window, for the screens `screen_render_post` covers: a screen of the game's, the title's own screens, Win and Options. Not while playing with no screen, nor for a server dialog, the server pack, link and code-of-conduct prompts, a book or a sign editor drawn over gameplay. Draw with `native_fill`, `native_sprite` and `native_text`; `screen.rect` is the screen's container panel (or the whole window).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Render.Background",
        cancellable = false
    )]
    fn screen_render_background();
    /// The open screen has drawn its contents (`ScreenEvent.Render.Foreground`, vanilla's point before the deferred tooltips and the preedit overlay). Not cancellable. Horizon's screens draw their tooltips and the dragged stack inside their drawing and expose no boundary before them, so unlike vanilla this fires after them, immediately before `screen_render_post`; use `screen_render_post` or this hook for what goes over the screen. It does not fire when `screen_render_pre` cancelled. Fires once a frame on the window, for the screens `screen_render_post` covers: a screen of the game's, the title's own screens, Win and Options. Not while playing with no screen, nor for a server dialog, the server pack, link and code-of-conduct prompts, a book or a sign editor drawn over gameplay. Draw with `native_fill`, `native_sprite` and `native_text`; `screen.rect` is the screen's container panel (or the whole window).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Render.Foreground",
        cancellable = false
    )]
    fn screen_render_foreground();
}
