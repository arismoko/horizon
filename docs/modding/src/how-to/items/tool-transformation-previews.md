# Tool transformation previews

`LevelRef::block_transform` previews a tool against a loaded block in the server level in place.
Pass the live player, hand, stack, position and hit; the owned result describes the new state,
relative durability cost, sound, loot and particle. The call does not apply the result. It fires
`block_tool_modification` with `ToolUse::simulated` set to `true`: listeners must not change the
level during a simulation. A stale or unavailable player or level, unloaded position, invalid face,
or tool with no transformation returns `None`. The real action fires the same hook with
`simulated` set to `false`.
