<!-- chapter: Delayed tasks -->
<!-- separator: 1 -->

A server mod asks for a call back after some server ticks with `schedule_task`, the stand-in for
vanilla's `MinecraftServer.tell(new TickTask(getTickCount() + delay, runnable))`. A closure cannot
cross into a wasm mod or outlive the host a hook borrows, so the callback is a hook: the mod passes
a `u64` token and `Hooks::scheduled_task` hands it back.

```rust,ignore
const REMINDER: u64 = 1;

impl Hooks for MyMod {
    fn player_logged_in(&self, host: &mut dyn ServerHost, player: u128, name: &str) {
        // 20 server ticks from now: one second at the normal rate.
        if let Ok(id) = host.schedule_task(20, REMINDER) {
            // Keep `id` if the reminder may need to be withdrawn: host.cancel_task(id).
        }
    }

    fn scheduled_task(&self, host: &mut dyn ServerHost, id: TaskId, owner: &str, token: u64) {
        if owner != "mymod" || token != REMINDER {
            return; // every mod is asked about every task
        }
        // The task has fired and its id is spent.
    }
}
```

| Call | Meaning |
| --- | --- |
| `host.schedule_task(delay_ticks, token) -> Result<TaskId, ScheduleError>` | A task due `delay_ticks` server ticks from now. `0` and `1` both mean the next tick. |
| `host.cancel_task(id) -> bool` | Withdraws a task. True only if it is yours and still waiting. |
| `Hooks::scheduled_task(host, id, owner, token)` | The callback, on the server thread. |

**Timing.** Delays count ticks the server runs: a paused integrated server does not advance them. A
task fires at the start of its due tick, right after `server_tick`'s start phase, in the order the
tasks were scheduled. A task never fires in the tick that scheduled it, whichever hook scheduled it,
the tick's start hook and `scheduled_task` itself included: it fires on a later tick.

**Ownership.** The task belongs to the mod whose hook called `schedule_task`; the game takes that from
the running hook, never from an argument. Outside a mod's hook the call returns
`ScheduleError::NotInMod`. `cancel_task` answers false for another mod's id, an unknown one, and one
that has fired or been cancelled. Every mod hears `scheduled_task` for every due task, so a mod
compares `owner` with its own id and ignores the rest. Ids are never reused in a world.

**Limits.** A mod may have `MAX_PENDING_TASKS` (4096) tasks waiting; the next `schedule_task` returns
`ScheduleError::TooMany` until one fires or is cancelled. A task that panics does not drop the others
due on the same tick: they still run, and the panic goes on afterwards as any hook's does.

**Lifetime.** The queue belongs to the running world. It is dropped when the world opens or closes
(including a failed open or close), and is never saved. A mod that must outlive a restart writes its
deadline to its own storage and schedules again when the world opens. A mod's tasks are not cancelled
when the mod unloads; the world's close is the end of them.

## Spare time in a tick

`host.server_has_time()` says whether the server's current tick has not yet reached the moment the next one is
due (NeoForge's `ServerTickEvent.hasTime` and `LevelTickEvent.hasTime`, vanilla's `MinecraftServer.haveTime`).
A listener of `server_tick` or of a level tick (`level_tick_pre`, `level_tick_post`) that has optional work, such
as a cache rebuild, checks it and leaves the rest for a later tick when it is false. It is always true on a
client and outside a scheduled server tick, so the same code runs on both sides.

```rust,ignore
fn level_tick_pre(&self, host: HostSide, _: &mut LevelTickPreEvent) {
    if let HostSide::Server(host) = host
        && host.server_has_time()
    {
        self.rebuild_next_cache_entry();
    }
}
```
