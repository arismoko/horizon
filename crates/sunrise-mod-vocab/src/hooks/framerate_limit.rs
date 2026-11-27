//! The `framerate_limit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The frame rate this window is about to be held to: Max Framerate after the
    /// inactivity throttle and Battery Mode (260 is unlimited). `focused` is whether the
    /// window has focus, `minimized` whether it is minimised and `in_world` whether a world
    /// is open. A mod's answer takes the game's place, as a frame-rate mod does for a window
    /// in the background: lower it, or raise it above the game's own throttles for menus and
    /// idle windows, up to unlimited (260; anything above counts as 260). Set 0 to skip the
    /// frame's render: nothing is drawn or presented, and the loop waits as it would at 10
    /// frames a second, so the game goes on ticking. A skipped frame fires neither
    /// `render_frame_pre` nor `render_frame_post`. Below 0 keeps the game's limit.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_framerate_limit",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla FramerateLimitTracker.getFramerateLimit fires no event)",
        cancellable = false,
        version = 2
    )]
    fn framerate_limit(focused: bool, minimized: bool, in_world: bool, limit: &mut i32);
}
