//! The `screen_key_released_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before the currently open screen handles a named key's release.
    /// Window releases, control-socket `key ... up`, and the release phase of `key ... tap`
    /// reach the current receiving screen, even if it did not see the press. Cancel skips
    /// the screen handler and Post. A cancelled window Pre still releases the device key and
    /// applies mapped keyboard release cleanup. A cancelled socket Pre skips the ordinary
    /// control-key release; a held mapping can stay down. Cancelling an unhandled release in
    /// Post instead skips normal mapped release on either route, after the screen has declined.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_screen_key_released_pre",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyReleased.Pre",
        cancellable = true,
        version = 3
    )]
    fn screen_key_released_pre(
        screen: &ScreenView<'_>,
        key: &str,
        modifiers: u32,
        canceled: &mut bool,
    );
}
