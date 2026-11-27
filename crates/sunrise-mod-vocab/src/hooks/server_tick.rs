//! The `server_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The authoritative server tick's phases: `ServerTickStart` before any
    /// level is ticked (NeoForge's `ServerTickEvent.Pre`), `ServerTickEnd`
    /// after the last (`.Post`). Runs on integrated and dedicated servers only.
    /// To read a loaded block, resolve the dimension with `host.level(dimension)`
    /// and call `level.block_state(host, pos)` on the returned handle. A paused
    /// pass fires nothing; a frozen world carries `world_running` false except
    /// during a requested step. `host.server_has_time()` says whether optional
    /// work still fits before the next tick is due (`ServerTickEvent.hasTime`).
    #[hook(target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_server_tick", dispatch = "flat", side = "server", contract = "listen", fabric = ["ServerTickEvents.START_SERVER_TICK", "ServerTickEvents.END_SERVER_TICK"], neoforge = "ServerTickEvent.Pre, ServerTickEvent.Post")]
    fn server_tick(context: &LifecycleContext, phase: ServerTickPhase);
}
