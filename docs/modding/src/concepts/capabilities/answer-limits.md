# Capability answer limits

The game never trusts an answer past what it asked or offered. A handler answering more than it was asked has its
move undone, in its transaction, and is warned once naming the mod, and `move_resources` or `move_energy` returns
`Overanswered` naming it. Returned from the closure of `host.transaction` with `?`, it aborts the whole move, so
neither end keeps any of it: a broken handler can't make a move overflow, duplicate or lose items, fluid or energy, or
panic the game.
