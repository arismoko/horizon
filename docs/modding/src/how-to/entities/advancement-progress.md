# Observing advancement progress

Subscribe to `player_advancement_progress` to inspect the whole advancement immediately after
one criterion is granted or revoked, before its completion rewards. `event.progress.criteria`
includes obtained and unobtained criteria; `obtained_at` is the original saved grant date or
`None`. `requirements` lists the OR groups, `completed_requirements` counts satisfied groups,
and `done` is the game's completion result. A player-issued `/reload` mutates and
snapshots the acting player's live progress, including during a nested turn.

Keep `event.progress.clone()` if it is needed later: the owned `AdvancementProgress` snapshot
cannot change the player's progress and is not updated by subsequent events. No-op grants and
revokes do not fire. The snapshot uses the same plain-data bindings in native and wasm mods.
