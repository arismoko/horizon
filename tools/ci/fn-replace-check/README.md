Two native mods whose `[[native.mixin]]` rows the launcher builds against the prebuilt game by function
replacement (no game rebuild): `tools/ci/fn_replace_check.sh` builds them and runs a dedicated server.
Every helper prints a line starting `fnr:` so the run shows what ran, in what order.

`loop_generic_head` is built on its own, never beside `loop`: a HEAD mixin on the published loop's generic
function with no `[[native.loop]]` row, so the copy is the game's own loop with the empty slot.

`loop_second` is built beside `loop`, in the workflow's main set: a second mod's ticking block entities in the same world, and a slot of
its own that counts only those.
