# Published loops

A published loop is a hot loop the game owns, written once with a slot in it. A native mod fills the slot with a type of its own through a `[[native.loop]]` row. The launcher compiles one copy of the loop with the installed mods' slot types in it and installs that copy; the game is not rebuilt. With no mod, the loop is the game's own.

Like every `[[native.*]]` table this is the unstable, powerful route. A slot gets the game's own types, not `mod_api` values, and is pinned to the exact game build (`[native] horizon`): a change in the game shows as a compile error under the mod's id. A wasm mod cannot declare one. Prefer a hook or a Part method when one covers the behaviour.

## Declaration

```toml
[[native.loop]]
id = "throttle"
loop = "block_entities.tick"
helper = "src/loops/throttle.rs"
with = "Throttle"
priority = 1000
```

- `loop` names a loop the pinned game publishes.
- `helper` is a Rust file under `src/`, compiled beside the loop as a mixin helper is: it names the game's types and private fields as the game's source does, and it may call the mod's own crate.
- `with` names a type the helper defines. It implements the loop's slot trait and `Default`.
- `priority` is optional and defaults to 1000. Lower runs first; ties go by mod id.

## The first loop: `block_entities.tick`

The server's block entity tick. It runs on the `Game` that simulates a world: a dedicated server, the single-player server thread, or a LAN host. The window's own client `Game` never runs it.

```rust
#[derive(Default)]
pub struct Throttle {
    skipped: u64,
}

impl BlockEntityTickSlot for Throttle {
    fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
        if tick.kind != TickedKind::Hopper {
            return LoopFlow::Continue;
        }
        match tick.game.hoppers.get_mut(&tick.pos) {
            Some(state) if state.cooldown > 1 => {
                state.cooldown -= 1;
                self.skipped += 1;
                LoopFlow::Skip
            }
            _ => LoopFlow::Continue,
        }
    }
}
```

`before` runs for each block entity vanilla would tick, after vanilla's own filters (in a block-ticking chunk, inside the world border, live, not asleep) and before its tick. `after` runs once it has ticked. The cursor lends `game`, the entity's `pos` and its `kind` for one call.

| Answer from `before` | Effect |
| --- | --- |
| `Continue` | The entity ticks. |
| `Skip` | This entity does not tick this tick. |
| `Stop` | No further entity ticks this tick. The work after the loop still runs: song volumes, creaking hearts, client effects, shulker box lids. |

The game keeps the order and where the slot sits. The cursor's `game` is the whole game, so a slot can add, remove or replace block entities, with these effects:

- After every slot answered `Continue`, the loop checks the visited entity again. If it was removed, or replaced by one of another `kind`, it does not tick and no `after` runs for it.
- The pass visits the list it took when it started. An entity a slot adds is not visited in this pass and ticks from the next, as one placed by a tick does.
- An entity a slot removes later in the list is passed over when the loop reaches it, as one removed by an earlier tick is.

The game's own loop (no mod) has no `before`, so it does not make that second check.

## Several mods, state, panics

- Rows on one loop are sorted by priority, then mod id. In `before`, the first answer that is not `Continue` wins and later mods are not asked for that entity. `after` runs for every mod.
- The slot value is created with `Default` on the loop's first run, once per `Game`, and kept across ticks. Key per-dimension state on the game's dimension.
- A run started from inside a slot (the slot calls something that ticks block entities) is the game's own loop, without slots. It is a whole pass: every entity ticks again, and the work after the loop runs again. The enclosing pass's working state (its item and mob caches, the spawners it has ticked) is set aside during the nested pass and restored after it, so the enclosing pass carries on as before.
- A panic in a slot's `before`, `after` or `Default` unwinds out of the tick like any game panic: the server writes its crash report and saves the world. The report names the mod and the loop: `in native mod <id>, hook loop block_entities.tick, part <id>::<Type>`. That `Game` does not run the slots again, and the slots already made are dropped, each one attributed to its own mod.
- Each mod's slot is dropped on its own. If one mod's `Drop` panics, the other mods' slots are still dropped and the process does not abort. That panic is reported as its mod's when it happens. If it happens while cleaning up after another slot's panic, the first panic is the one that unwinds. When a `Game` is dropped normally, the first `Drop` panic unwinds after every slot has been dropped.
- The run without slots, nested or after a panic, is logged once per loop per `Game`.

## What composes with a loop

- A `HEAD` or `RETURN` mixin on the loop's entry function runs around the loop.
- An `[[inject]]` `api` listener on the entry function runs around it too.
- A mixin on a function the loop calls applies as anywhere else.
- Call-site rows inside the loop body target `…::tick_block_entities_with`. That function is copied for the installed slots, and these rows do not apply to the run without slots.

## Refusals

| Code | When |
| --- | --- |
| `E_LOOP_WASM` | A wasm mod declares `[[native.loop]]`. |
| `E_LOOP_INVALID` | `loop` is empty, `helper` is not a Rust file under `src/`, or `with` is not an identifier; the field is named. |
| `E_LOOP_UNKNOWN` | The pinned game publishes no loop of that name; the known loops are listed. |
| `E_LOOP_HELPER` | The helper is not Rust items, or `with` does not parse. An unreadable helper file is a plain file error. |
| `E_LOOP_ENTRY_REPLACED` | A mixin of any kind other than `HEAD` or `RETURN` on a filled loop's entry function, or an `OVERWRITE` of `…::tick_block_entities_with`. The error names the refused row (`mod:id`), its kind and target, the loop, and the mods filling it. |

A helper that does not compile against the pinned game is reported with the compiler's error under the mod's id.
