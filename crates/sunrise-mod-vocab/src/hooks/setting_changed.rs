//! The `setting_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A registered setting changed through a typed write, Options > Mods, or a settings reload.
    /// Runs on the side that changed the value, after publishing it. Only the setting's owner
    /// hears it. `value` is the new value. The
    /// listener gets a read-only [`Host`](crate::Host): the write that caused the notice is
    /// already committed, and a listener that wrote its own key from here would notify itself
    /// without end. Make a follow-up write from the callback that made the first one.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_setting_changed",
        dispatch = "flat",
        side = "both",
        receiver = "read",
        host = "read",
        contract = "listen",
        owner = "mod_id",
        neoforge = "ModConfigEvent.Reloading",
        version = 3
    )]
    fn setting_changed(mod_id: &str, key: &str, value: SettingValue);
}
